use crate::connect_policy::TargetCheckedTcpConnector;
use crate::network_policy::{NetworkDecision, NetworkDecisionSource};
use crate::policy::normalize_host;
use crate::reasons::*;
use crate::runtime::{BlockedRequestArgs, ConfigSnapshot, NetworkProxyState};
use anyhow::{Context, Result};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{info, warn};

// SOCKS5 protocol constants
const SOCKS5_VERSION: u8 = 0x05;
const AUTH_NO_AUTH: u8 = 0x00;
const AUTH_NO_ACCEPTABLE: u8 = 0xFF;
const CMD_CONNECT: u8 = 0x01;
const CMD_UDP_ASSOCIATE: u8 = 0x03;
const ATYP_IPV4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_IPV6: u8 = 0x04;

// SOCKS5 reply codes
const REP_SUCCEEDED: u8 = 0x00;
const _REP_GENERAL_FAILURE: u8 = 0x01;
const REP_NOT_ALLOWED: u8 = 0x02;
const REP_HOST_UNREACHABLE: u8 = 0x04;
const REP_CMD_NOT_SUPPORTED: u8 = 0x07;
const _REP_ATYP_NOT_SUPPORTED: u8 = 0x08;

/// Run the SOCKS5 proxy engine on a pre-bound listener.
pub async fn run_socks5_proxy_on_listener(
    state: Arc<NetworkProxyState>,
    listener: TcpListener,
) -> Result<()> {
    let addr = listener.local_addr().context("get listener local addr")?;
    info!("SOCKS5 proxy engine active on {addr}");

    loop {
        let (stream, client_addr) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                warn!("SOCKS5 accept error: {e}");
                continue;
            }
        };

        let state = Arc::clone(&state);
        tokio::spawn(async move {
            if let Err(e) = handle_socks5_connection(state, stream, client_addr).await {
                warn!("SOCKS5 error from {client_addr}: {e}");
            }
        });
    }
}

/// Bind and run the SOCKS5 proxy engine (convenience wrapper).
pub async fn run_socks5_proxy(
    state: Arc<NetworkProxyState>,
    addr: SocketAddr,
) -> Result<()> {
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind SOCKS5 proxy: {addr}"))?;
    run_socks5_proxy_on_listener(state, listener).await
}

async fn handle_socks5_connection(
    state: Arc<NetworkProxyState>,
    mut stream: TcpStream,
    client_addr: SocketAddr,
) -> Result<()> {
    let version = stream.read_u8().await?;
    if version != SOCKS5_VERSION {
        return Err(anyhow::anyhow!("unsupported SOCKS version: {version}"));
    }

    let nmethods = stream.read_u8().await?;
    let mut methods = vec![0u8; nmethods as usize];
    stream.read_exact(&mut methods).await?;

    if !methods.contains(&AUTH_NO_AUTH) {
        stream.write_all(&[SOCKS5_VERSION, AUTH_NO_ACCEPTABLE]).await?;
        return Err(anyhow::anyhow!("no acceptable auth method"));
    }

    stream.write_all(&[SOCKS5_VERSION, AUTH_NO_AUTH]).await?;

    let version = stream.read_u8().await?;
    if version != SOCKS5_VERSION {
        return Err(anyhow::anyhow!("unexpected version in request: {version}"));
    }

    let cmd = stream.read_u8().await?;
    let _rsv = stream.read_u8().await?;
    let atyp = stream.read_u8().await?;

    let (host, port) = read_address(&mut stream, atyp).await?;
    let normalized_host = normalize_host(&host);

    let snap = state.snapshot().await;

    match cmd {
        CMD_CONNECT => {
            handle_connect_cmd(&state, &snap, &mut stream, client_addr, &normalized_host, port)
                .await
        }
        CMD_UDP_ASSOCIATE => {
            send_reply(&mut stream, REP_CMD_NOT_SUPPORTED, "0.0.0.0", 0).await?;
            Ok(())
        }
        _ => {
            send_reply(&mut stream, REP_CMD_NOT_SUPPORTED, "0.0.0.0", 0).await?;
            Err(anyhow::anyhow!("unsupported SOCKS5 command: {cmd}"))
        }
    }
}

async fn handle_connect_cmd(
    state: &NetworkProxyState,
    snap: &ConfigSnapshot,
    stream: &mut TcpStream,
    client_addr: SocketAddr,
    host: &str,
    port: u16,
) -> Result<()> {
    if !snap.enabled {
        send_reply(stream, REP_NOT_ALLOWED, "0.0.0.0", 0).await?;
        return Ok(());
    }

    let decision = evaluate_socks_policy(snap, host);
    if let NetworkDecision::Deny { reason, .. } = &decision {
        let _ = state
            .record_blocked(BlockedRequestArgs {
                host: host.to_string(),
                reason: reason.clone(),
                client: Some(client_addr.to_string()),
                method: None,
                protocol: "socks5".to_string(),
                port: Some(port),
            })
            .await;
        warn!("SOCKS5 blocked (host={host}, reason={reason})");
        send_reply(stream, REP_NOT_ALLOWED, "0.0.0.0", 0).await?;
        return Ok(());
    }

    info!("SOCKS5 CONNECT -> {host}:{port}");

    let connector = TargetCheckedTcpConnector::new(snap.allow_local_binding);
    let target_addr: SocketAddr = match tokio::net::lookup_host(format!("{host}:{port}")).await {
        Ok(mut addrs) => match addrs.next() {
            Some(addr) => addr,
            None => {
                send_reply(stream, REP_HOST_UNREACHABLE, "0.0.0.0", 0).await?;
                return Ok(());
            }
        },
        Err(_) => {
            send_reply(stream, REP_HOST_UNREACHABLE, "0.0.0.0", 0).await?;
            return Ok(());
        }
    };

    let mut upstream = match connector.connect(target_addr).await {
        Ok(s) => s,
        Err(e) => {
            let code = if e.kind() == std::io::ErrorKind::PermissionDenied {
                REP_NOT_ALLOWED
            } else {
                REP_HOST_UNREACHABLE
            };
            send_reply(stream, code, "0.0.0.0", 0).await?;
            return Err(anyhow::anyhow!("connect to {host}:{port}: {e}"));
        }
    };

    let bound = upstream
        .local_addr()
        .unwrap_or_else(|_| "0.0.0.0:0".parse().unwrap());
    send_reply(stream, REP_SUCCEEDED, &bound.ip().to_string(), bound.port()).await?;

    let _ = tokio::io::copy_bidirectional(stream, &mut upstream).await;
    Ok(())
}

fn evaluate_socks_policy(snap: &ConfigSnapshot, host: &str) -> NetworkDecision {
    if let Some(ref deny_set) = snap.deny_globset {
        if deny_set.is_match(host) {
            return NetworkDecision::deny_with_source(
                REASON_DENIED,
                NetworkDecisionSource::BaselinePolicy,
            );
        }
    }

    if let Some(ref allow_set) = snap.allow_globset {
        if !allow_set.is_match(host) {
            return NetworkDecision::deny_with_source(
                REASON_CONNECT_BLOCKED,
                NetworkDecisionSource::BaselinePolicy,
            );
        }
    }

    NetworkDecision::allow()
}

async fn read_address(stream: &mut TcpStream, atyp: u8) -> Result<(String, u16)> {
    match atyp {
        ATYP_IPV4 => {
            let mut addr = [0u8; 4];
            stream.read_exact(&mut addr).await?;
            let port = stream.read_u16().await?;
            let ip = Ipv4Addr::from(addr);
            Ok((ip.to_string(), port))
        }
        ATYP_DOMAIN => {
            let len = stream.read_u8().await? as usize;
            let mut domain = vec![0u8; len];
            stream.read_exact(&mut domain).await?;
            let port = stream.read_u16().await?;
            let domain = String::from_utf8(domain)
                .map_err(|_| anyhow::anyhow!("invalid UTF-8 domain"))?;
            Ok((domain, port))
        }
        ATYP_IPV6 => {
            let mut addr = [0u8; 16];
            stream.read_exact(&mut addr).await?;
            let port = stream.read_u16().await?;
            let ip = Ipv6Addr::from(addr);
            Ok((ip.to_string(), port))
        }
        _ => Err(anyhow::anyhow!("unsupported address type: {atyp}")),
    }
}

async fn send_reply(
    stream: &mut TcpStream,
    reply: u8,
    bind_addr: &str,
    bind_port: u16,
) -> Result<()> {
    let mut buf = vec![SOCKS5_VERSION, reply, 0x00]; // VER, REP, RSV

    if let Ok(ipv4) = bind_addr.parse::<Ipv4Addr>() {
        buf.push(ATYP_IPV4);
        buf.extend_from_slice(&ipv4.octets());
    } else if let Ok(ipv6) = bind_addr.parse::<Ipv6Addr>() {
        buf.push(ATYP_IPV6);
        buf.extend_from_slice(&ipv6.octets());
    } else {
        buf.push(ATYP_IPV4);
        buf.extend_from_slice(&[0, 0, 0, 0]);
    }

    buf.extend_from_slice(&bind_port.to_be_bytes());
    stream.write_all(&buf).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socks5_constants() {
        assert_eq!(SOCKS5_VERSION, 0x05);
        assert_eq!(AUTH_NO_AUTH, 0x00);
        assert_eq!(CMD_CONNECT, 0x01);
    }

    #[test]
    fn reply_codes() {
        assert_eq!(REP_SUCCEEDED, 0x00);
        assert_eq!(REP_NOT_ALLOWED, 0x02);
        assert_eq!(REP_CMD_NOT_SUPPORTED, 0x07);
    }

    #[tokio::test]
    async fn socks5_handshake_rejects_wrong_version() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let client_task = tokio::spawn(async move {
            let mut client = TcpStream::connect(addr).await.unwrap();
            // Send wrong version
            client.write_all(&[0x04, 0x01, 0x00]).await.unwrap();
            let mut buf = [0u8; 2];
            let n = client.read(&mut buf).await.unwrap_or(0);
            (n, buf)
        });

        let (stream, client_addr) = listener.accept().await.unwrap();
        let state = Arc::new(NetworkProxyState::for_settings(
            crate::config::NetworkProxySettings::default(),
        ));
        let result = handle_socks5_connection(state, stream, client_addr).await;
        assert!(result.is_err());
        let _ = client_task.await;
    }

    #[tokio::test]
    async fn socks5_handshake_no_acceptable_method() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let client_task = tokio::spawn(async move {
            let mut client = TcpStream::connect(addr).await.unwrap();
            // Send version 5, 1 method: username/password (0x02) only
            client.write_all(&[0x05, 0x01, 0x02]).await.unwrap();
            let mut buf = [0u8; 2];
            client.read_exact(&mut buf).await.unwrap();
            buf
        });

        let (stream, client_addr) = listener.accept().await.unwrap();
        let state = Arc::new(NetworkProxyState::for_settings(
            crate::config::NetworkProxySettings::default(),
        ));
        let result = handle_socks5_connection(state, stream, client_addr).await;
        assert!(result.is_err());
        let buf = client_task.await.unwrap();
        assert_eq!(buf, [SOCKS5_VERSION, AUTH_NO_ACCEPTABLE]);
    }
}
