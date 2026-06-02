use crate::config::NetworkMode;
use crate::connect_policy::TargetCheckedTcpConnector;
use crate::mitm;
use crate::network_policy::{
    NetworkDecision, NetworkDecisionSource, NetworkPolicyDecision,
    NetworkProtocol,
};
use crate::policy::normalize_host;
use crate::reasons::*;
use crate::responses::{PolicyDecisionDetails, blocked_header_value, blocked_message};
use crate::runtime::{BlockedRequestArgs, ConfigSnapshot, NetworkProxyState};
use anyhow::{Context, Result};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tracing::{error, info, warn};

type BoxBody = Full<Bytes>;
type HttpResponse = Response<BoxBody>;

const HOP_BY_HOP_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
    "proxy-connection",
];

const X_PROXY_ERROR: &str = "x-proxy-error";
const X_UNIX_SOCKET: &str = "x-unix-socket";

/// Run the HTTP proxy engine on a pre-bound listener.
pub async fn run_http_proxy_on_listener(
    state: Arc<NetworkProxyState>,
    listener: TcpListener,
) -> Result<()> {
    let addr = listener.local_addr().context("get listener local addr")?;
    info!("HTTP proxy engine active on {addr}");

    loop {
        let (stream, client_addr) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                warn!("accept error: {e}");
                continue;
            }
        };

        let state = Arc::clone(&state);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(state, stream, client_addr).await {
                warn!("connection error from {client_addr}: {e}");
            }
        });
    }
}

/// Bind and run the HTTP proxy engine (convenience wrapper).
pub async fn run_http_proxy(
    state: Arc<NetworkProxyState>,
    addr: SocketAddr,
) -> Result<()> {
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind HTTP proxy: {addr}"))?;
    run_http_proxy_on_listener(state, listener).await
}

async fn handle_connection(
    state: Arc<NetworkProxyState>,
    stream: TcpStream,
    client_addr: SocketAddr,
) -> Result<()> {
    let io = hyper_util::rt::TokioIo::new(stream);
    let state_for_svc = Arc::clone(&state);

    http1::Builder::new()
        .preserve_header_case(true)
        .title_case_headers(false)
        .serve_connection(
            io,
            service_fn(move |req| {
                let state = Arc::clone(&state_for_svc);
                let client = client_addr;
                async move { handle_request(state, req, client).await }
            }),
        )
        .with_upgrades()
        .await
        .map_err(|e| anyhow::anyhow!("hyper serve: {e}"))
}

async fn handle_request(
    state: Arc<NetworkProxyState>,
    req: Request<Incoming>,
    client_addr: SocketAddr,
) -> Result<HttpResponse, hyper::Error> {
    let snap = state.snapshot().await;
    let result = if req.method() == Method::CONNECT {
        handle_connect(&state, &snap, req, client_addr).await
    } else {
        handle_plain_proxy(&state, &snap, req, client_addr).await
    };

    Ok(result.unwrap_or_else(|e| {
        error!("proxy error: {e}");
        text_response(StatusCode::INTERNAL_SERVER_ERROR, "proxy error")
    }))
}

async fn handle_connect(
    state: &Arc<NetworkProxyState>,
    snap: &ConfigSnapshot,
    req: Request<Incoming>,
    client_addr: SocketAddr,
) -> Result<HttpResponse> {
    let authority = req
        .uri()
        .authority()
        .map(|a| a.to_string())
        .or_else(|| {
            req.uri()
                .host()
                .map(|h| format!("{}:{}", h, req.uri().port_u16().unwrap_or(443)))
        })
        .unwrap_or_default();

    let (raw_host, port) = parse_authority(&authority)?;
    let host = normalize_host(&raw_host);

    if host.is_empty() {
        return Ok(text_response(StatusCode::BAD_REQUEST, "invalid host"));
    }

    if !snap.enabled {
        let details = make_details(
            NetworkPolicyDecision::Deny,
            REASON_NOT_ENABLED,
            NetworkDecisionSource::ProxyState,
            NetworkProtocol::HttpsConnect,
            &host,
            port,
        );
        return Ok(blocked_response(&details));
    }

    let decision = evaluate_policy(snap, &host, port, NetworkProtocol::HttpsConnect);
    if let NetworkDecision::Deny {
        reason,
        source,
        decision: policy_decision,
    } = &decision
    {
        let details = make_details(
            *policy_decision,
            reason,
            *source,
            NetworkProtocol::HttpsConnect,
            &host,
            port,
        );
        record_blocked(state, &host, reason, &client_addr, Some("CONNECT"), port).await;
        warn!("CONNECT blocked (host={host}, reason={reason})");
        return Ok(blocked_response(&details));
    }

    if snap.mode == NetworkMode::Limited && !snap.mitm_enabled {
        let details = make_details(
            NetworkPolicyDecision::Deny,
            REASON_MODE_GUARD,
            NetworkDecisionSource::ModeGuard,
            NetworkProtocol::HttpsConnect,
            &host,
            port,
        );
        record_blocked(state, &host, REASON_MODE_GUARD, &client_addr, Some("CONNECT"), port)
            .await;
        warn!("CONNECT blocked; MITM required for limited mode (host={host})");
        return Ok(blocked_response(&details));
    }

    let use_mitm = snap.mitm_enabled
        && snap.mode == NetworkMode::Limited
        && state.mitm_state().is_some();

    if use_mitm {
        info!("CONNECT MITM tunnel -> {host}:{port}");
        let mitm_state = state.mitm_state().unwrap().clone();
        let mode = snap.mode;
        let app_state = Arc::clone(state);
        let host_owned = host.clone();
        tokio::spawn(async move {
            match hyper::upgrade::on(req).await {
                Ok(upgraded) => {
                    let stream = hyper_util::rt::TokioIo::new(upgraded);
                    if let Err(e) = mitm::mitm_tunnel(
                        stream,
                        &host_owned,
                        port,
                        mode,
                        app_state,
                        mitm_state,
                    )
                    .await
                    {
                        warn!("MITM tunnel error for {host_owned}:{port}: {e}");
                    }
                }
                Err(e) => warn!("upgrade failed: {e}"),
            }
        });
    } else {
        info!("CONNECT tunnel -> {host}:{port}");

        let connector = TargetCheckedTcpConnector::new(snap.allow_local_binding);
        let target_addr: SocketAddr = tokio::net::lookup_host(format!("{host}:{port}"))
            .await?
            .next()
            .ok_or_else(|| anyhow::anyhow!("DNS lookup failed for {host}:{port}"))?;

        let upstream = connector
            .connect(target_addr)
            .await
            .map_err(|e| anyhow::anyhow!("connect to {host}:{port}: {e}"))?;

        tokio::spawn(async move {
            match hyper::upgrade::on(req).await {
                Ok(upgraded) => {
                    let mut upgraded = hyper_util::rt::TokioIo::new(upgraded);
                    let mut upstream = upstream;
                    let _ = tokio::io::copy_bidirectional(&mut upgraded, &mut upstream).await;
                }
                Err(e) => warn!("upgrade failed: {e}"),
            }
        });
    }

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Full::new(Bytes::new()))
        .unwrap())
}

async fn handle_plain_proxy(
    state: &NetworkProxyState,
    snap: &ConfigSnapshot,
    req: Request<Incoming>,
    client_addr: SocketAddr,
) -> Result<HttpResponse> {
    let uri = req.uri().clone();
    let method = req.method().clone();

    let raw_host = uri
        .host()
        .map(String::from)
        .or_else(|| {
            req.headers()
                .get("host")
                .and_then(|v| v.to_str().ok())
                .map(|h| h.split(':').next().unwrap_or(h).to_string())
        })
        .unwrap_or_default();
    let port = uri.port_u16().unwrap_or(80);
    let host = normalize_host(&raw_host);

    if host.is_empty() {
        return Ok(text_response(StatusCode::BAD_REQUEST, "missing host"));
    }

    if let Some(host_hdr) = req.headers().get("host").and_then(|v| v.to_str().ok()) {
        let hdr_host = normalize_host(host_hdr);
        if !hdr_host.is_empty() && hdr_host != host {
            warn!("Host header mismatch: URI={host} Header={hdr_host}");
            return Ok(text_response(
                StatusCode::BAD_REQUEST,
                "host header does not match request URI",
            ));
        }
    }

    if req.headers().contains_key(X_UNIX_SOCKET) {
        #[cfg(not(target_os = "macos"))]
        {
            let details = make_details(
                NetworkPolicyDecision::Deny,
                REASON_UNIX_SOCKET_BLOCKED,
                NetworkDecisionSource::BaselinePolicy,
                NetworkProtocol::Http,
                &host,
                port,
            );
            return Ok(blocked_response(&details));
        }
    }

    if !snap.enabled {
        let details = make_details(
            NetworkPolicyDecision::Deny,
            REASON_NOT_ENABLED,
            NetworkDecisionSource::ProxyState,
            NetworkProtocol::Http,
            &host,
            port,
        );
        return Ok(blocked_response(&details));
    }

    let decision = evaluate_policy(snap, &host, port, NetworkProtocol::Http);
    if let NetworkDecision::Deny {
        reason,
        source,
        decision: policy_decision,
    } = &decision
    {
        let details = make_details(
            *policy_decision,
            reason,
            *source,
            NetworkProtocol::Http,
            &host,
            port,
        );
        record_blocked(
            state,
            &host,
            reason,
            &client_addr,
            Some(method.as_str()),
            port,
        )
        .await;
        warn!("HTTP blocked (host={host}, reason={reason})");
        return Ok(blocked_response(&details));
    }

    if !snap.mode.allows_method(method.as_str()) {
        let details = make_details(
            NetworkPolicyDecision::Deny,
            REASON_MODE_GUARD,
            NetworkDecisionSource::ModeGuard,
            NetworkProtocol::Http,
            &host,
            port,
        );
        record_blocked(
            state,
            &host,
            REASON_MODE_GUARD,
            &client_addr,
            Some(method.as_str()),
            port,
        )
        .await;
        warn!(
            "HTTP method blocked (host={host}, method={method}, mode={:?})",
            snap.mode
        );
        return Ok(blocked_response(&details));
    }

    info!("HTTP proxy -> {method} {host}:{port}{}", uri.path());

    let connector = TargetCheckedTcpConnector::new(snap.allow_local_binding);
    let target_addr: SocketAddr = tokio::net::lookup_host(format!("{host}:{port}"))
        .await?
        .next()
        .ok_or_else(|| anyhow::anyhow!("DNS lookup failed for {host}:{port}"))?;

    let upstream = connector
        .connect(target_addr)
        .await
        .map_err(|e| anyhow::anyhow!("connect to {host}:{port}: {e}"))?;

    let io = hyper_util::rt::TokioIo::new(upstream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            warn!("upstream connection error: {e}");
        }
    });

    let path = uri
        .path_and_query()
        .map(|pq| pq.to_string())
        .unwrap_or_else(|| "/".to_string());
    let mut builder = Request::builder().method(method).uri(path);

    for (name, value) in req.headers() {
        let name_lower = name.as_str().to_ascii_lowercase();
        if HOP_BY_HOP_HEADERS.contains(&name_lower.as_str()) {
            continue;
        }
        builder = builder.header(name, value);
    }

    let body = req.collect().await?.to_bytes();
    let upstream_req = builder.body(Full::new(body))?;
    let upstream_resp = sender.send_request(upstream_req).await?;

    let (parts, body) = upstream_resp.into_parts();
    let body_bytes = body.collect().await?.to_bytes();

    let mut resp_builder = Response::builder().status(parts.status);
    for (name, value) in &parts.headers {
        let name_lower = name.as_str().to_ascii_lowercase();
        if HOP_BY_HOP_HEADERS.contains(&name_lower.as_str()) {
            continue;
        }
        resp_builder = resp_builder.header(name, value);
    }
    Ok(resp_builder.body(Full::new(body_bytes))?)
}

fn evaluate_policy(
    snap: &ConfigSnapshot,
    host: &str,
    _port: u16,
    _protocol: NetworkProtocol,
) -> NetworkDecision {
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

fn parse_authority(authority: &str) -> Result<(String, u16)> {
    // Handle [ipv6]:port
    if authority.starts_with('[') {
        if let Some(bracket_end) = authority.find(']') {
            let host = &authority[1..bracket_end];
            let port = if authority.len() > bracket_end + 1 && authority.as_bytes()[bracket_end + 1] == b':' {
                authority[bracket_end + 2..].parse().unwrap_or(443)
            } else {
                443
            };
            return Ok((host.to_string(), port));
        }
    }

    // Handle host:port
    if let Some((host, port_str)) = authority.rsplit_once(':') {
        if let Ok(port) = port_str.parse::<u16>() {
            return Ok((host.to_string(), port));
        }
    }

    Ok((authority.to_string(), 443))
}

fn make_details<'a>(
    decision: NetworkPolicyDecision,
    reason: &'a str,
    source: NetworkDecisionSource,
    protocol: NetworkProtocol,
    host: &'a str,
    port: u16,
) -> PolicyDecisionDetails<'a> {
    PolicyDecisionDetails {
        decision,
        reason,
        source,
        protocol,
        host,
        port,
    }
}

async fn record_blocked(
    state: &NetworkProxyState,
    host: &str,
    reason: &str,
    client_addr: &SocketAddr,
    method: Option<&str>,
    port: u16,
) {
    let _ = state
        .record_blocked(BlockedRequestArgs {
            host: host.to_string(),
            reason: reason.to_string(),
            client: Some(client_addr.to_string()),
            method: method.map(String::from),
            protocol: "http".to_string(),
            port: Some(port),
        })
        .await;
}

fn blocked_response(details: &PolicyDecisionDetails<'_>) -> HttpResponse {
    let status = match details.reason {
        REASON_NOT_ENABLED => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::FORBIDDEN,
    };

    let header_value = blocked_header_value(details.reason);
    let body = blocked_message(details);

    Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .header(X_PROXY_ERROR, header_value)
        .body(Full::new(Bytes::from(body)))
        .unwrap()
}

fn text_response(status: StatusCode, body: &str) -> HttpResponse {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .body(Full::new(Bytes::from(body.to_string())))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_authority_host_port() {
        let (host, port) = parse_authority("example.com:8080").unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(port, 8080);
    }

    #[test]
    fn parse_authority_ipv6() {
        let (host, port) = parse_authority("[::1]:443").unwrap();
        assert_eq!(host, "::1");
        assert_eq!(port, 443);
    }

    #[test]
    fn parse_authority_no_port() {
        let (host, port) = parse_authority("example.com").unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(port, 443);
    }

    #[test]
    fn hop_by_hop_list_is_complete() {
        assert!(HOP_BY_HOP_HEADERS.contains(&"connection"));
        assert!(HOP_BY_HOP_HEADERS.contains(&"proxy-authorization"));
        assert!(HOP_BY_HOP_HEADERS.contains(&"transfer-encoding"));
        assert!(!HOP_BY_HOP_HEADERS.contains(&"content-type"));
    }

    #[test]
    fn blocked_response_returns_403() {
        let details = PolicyDecisionDetails {
            decision: NetworkPolicyDecision::Deny,
            reason: REASON_DENIED,
            source: NetworkDecisionSource::BaselinePolicy,
            protocol: NetworkProtocol::Http,
            host: "evil.com",
            port: 80,
        };
        let resp = blocked_response(&details);
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert!(resp.headers().contains_key(X_PROXY_ERROR));
    }

    #[test]
    fn blocked_response_returns_503_when_disabled() {
        let details = PolicyDecisionDetails {
            decision: NetworkPolicyDecision::Deny,
            reason: REASON_NOT_ENABLED,
            source: NetworkDecisionSource::ProxyState,
            protocol: NetworkProtocol::Http,
            host: "example.com",
            port: 80,
        };
        let resp = blocked_response(&details);
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
