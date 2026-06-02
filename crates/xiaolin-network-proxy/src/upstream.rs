//! Upstream proxy client for chaining through HTTP/HTTPS proxies.
//!
//! Reads proxy configuration from environment variables (HTTP_PROXY,
//! HTTPS_PROXY, ALL_PROXY) and routes outgoing requests through the
//! configured upstream proxy when available.

use anyhow::{Context as _, Result, anyhow};
use std::fmt::Write as _;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::{info, warn};
use url::Url;

/// Upstream proxy configuration read from environment variables.
#[derive(Clone, Debug, Default)]
pub struct ProxyConfig {
    pub http: Option<ProxyEndpoint>,
    pub https: Option<ProxyEndpoint>,
    pub all: Option<ProxyEndpoint>,
}

/// A parsed proxy endpoint.
#[derive(Clone, Debug)]
pub struct ProxyEndpoint {
    pub host: String,
    pub port: u16,
    pub scheme: String,
    pub auth: Option<ProxyAuth>,
}

/// Proxy authentication credentials.
#[derive(Clone, Debug)]
pub struct ProxyAuth {
    pub username: String,
    pub password: String,
}

impl ProxyConfig {
    /// Read proxy configuration from environment variables.
    pub fn from_env() -> Self {
        let http = read_proxy_env(&["HTTP_PROXY", "http_proxy"]);
        let https = read_proxy_env(&["HTTPS_PROXY", "https_proxy"]);
        let all = read_proxy_env(&["ALL_PROXY", "all_proxy"]);
        Self { http, https, all }
    }

    /// Select the appropriate proxy for the given protocol.
    pub fn proxy_for_protocol(&self, is_secure: bool) -> Option<&ProxyEndpoint> {
        if is_secure {
            self.https
                .as_ref()
                .or(self.http.as_ref())
                .or(self.all.as_ref())
        } else {
            self.http.as_ref().or(self.all.as_ref())
        }
    }

    pub fn has_proxy(&self) -> bool {
        self.http.is_some() || self.https.is_some() || self.all.is_some()
    }
}

fn read_proxy_env(keys: &[&str]) -> Option<ProxyEndpoint> {
    for key in keys {
        let Ok(value) = std::env::var(key) else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match parse_proxy_endpoint(value) {
            Ok(endpoint) => return Some(endpoint),
            Err(err) => {
                warn!("ignoring {key}: {err}");
            }
        }
    }
    None
}

fn parse_proxy_endpoint(proxy_url: &str) -> Result<ProxyEndpoint> {
    let url_str = if proxy_url.contains("://") {
        proxy_url.to_string()
    } else {
        format!("http://{proxy_url}")
    };

    let parsed = Url::parse(&url_str).context("invalid proxy URL")?;
    let scheme = parsed.scheme().to_lowercase();

    if !matches!(scheme.as_str(), "http" | "https") {
        return Err(anyhow!("unsupported proxy scheme: {scheme}"));
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow!("missing host in proxy URL"))?
        .to_string();
    let port = parsed.port().unwrap_or(if scheme == "https" { 443 } else { 80 });

    let auth = if !parsed.username().is_empty() {
        Some(ProxyAuth {
            username: parsed.username().to_string(),
            password: parsed.password().unwrap_or("").to_string(),
        })
    } else {
        None
    };

    Ok(ProxyEndpoint {
        host,
        port,
        scheme,
        auth,
    })
}

/// Upstream proxy client.
///
/// Wraps proxy configuration and provides methods for establishing
/// connections through the upstream proxy chain.
#[derive(Clone, Debug)]
pub struct UpstreamClient {
    proxy_config: ProxyConfig,
    allow_local_binding: bool,
}

impl UpstreamClient {
    /// Create a direct client (no upstream proxy).
    pub fn direct(allow_local_binding: bool) -> Self {
        Self {
            proxy_config: ProxyConfig::default(),
            allow_local_binding,
        }
    }

    /// Create a client that reads proxy config from environment.
    pub fn from_env_proxy(allow_local_binding: bool) -> Self {
        Self {
            proxy_config: ProxyConfig::from_env(),
            allow_local_binding,
        }
    }

    /// Create from explicit proxy config.
    pub fn with_config(proxy_config: ProxyConfig, allow_local_binding: bool) -> Self {
        Self {
            proxy_config,
            allow_local_binding,
        }
    }

    pub fn has_upstream_proxy(&self) -> bool {
        self.proxy_config.has_proxy()
    }

    pub fn allow_local_binding(&self) -> bool {
        self.allow_local_binding
    }

    /// Establish a TCP connection to the target, optionally through
    /// an upstream HTTP CONNECT proxy.
    pub async fn connect_tcp(&self, host: &str, port: u16, is_secure: bool) -> Result<TcpStream> {
        if let Some(proxy) = self.proxy_config.proxy_for_protocol(is_secure) {
            info!(
                "upstream proxy connect: target={host}:{port} via {}:{}",
                proxy.host, proxy.port
            );
            connect_via_http_proxy(proxy, host, port).await
        } else {
            info!("direct connect: target={host}:{port}");
            let addr = format!("{host}:{port}");
            TcpStream::connect(&addr)
                .await
                .with_context(|| format!("failed to connect to {addr}"))
        }
    }

    pub fn proxy_for_connect(&self) -> Option<&ProxyEndpoint> {
        self.proxy_config.proxy_for_protocol(true)
    }
}

/// Establish a TCP tunnel through an HTTP CONNECT proxy.
async fn connect_via_http_proxy(
    proxy: &ProxyEndpoint,
    target_host: &str,
    target_port: u16,
) -> Result<TcpStream> {
    let proxy_addr = format!("{}:{}", proxy.host, proxy.port);
    let mut stream = TcpStream::connect(&proxy_addr)
        .await
        .with_context(|| format!("failed to connect to upstream proxy {proxy_addr}"))?;

    let mut connect_request = format!(
        "CONNECT {target_host}:{target_port} HTTP/1.1\r\nHost: {target_host}:{target_port}\r\n"
    );
    if let Some(auth) = &proxy.auth {
        let credentials = format!("{}:{}", auth.username, auth.password);
        let encoded = base64_encode(credentials.as_bytes());
        write!(connect_request, "Proxy-Authorization: Basic {encoded}\r\n")
            .map_err(|e| anyhow!("failed to write auth header: {e}"))?;
    }
    connect_request.push_str("\r\n");

    stream
        .write_all(connect_request.as_bytes())
        .await
        .context("failed to send CONNECT request")?;

    let mut response_buf = vec![0u8; 4096];
    let n = stream
        .read(&mut response_buf)
        .await
        .context("failed to read CONNECT response")?;
    let response = String::from_utf8_lossy(&response_buf[..n]);

    if !response.starts_with("HTTP/1.1 200") && !response.starts_with("HTTP/1.0 200") {
        return Err(anyhow!(
            "upstream proxy CONNECT failed: {}",
            response.lines().next().unwrap_or("empty response")
        ));
    }

    info!(
        "upstream proxy tunnel established: target={target_host}:{target_port} via {proxy_addr}"
    );
    Ok(stream)
}

fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        output.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        output.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            output.push(ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(ALPHABET[(triple & 0x3F) as usize] as char);
        } else {
            output.push('=');
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_proxy_endpoint_http() {
        let ep = parse_proxy_endpoint("http://proxy.example.com:3128").unwrap();
        assert_eq!(ep.host, "proxy.example.com");
        assert_eq!(ep.port, 3128);
        assert_eq!(ep.scheme, "http");
        assert!(ep.auth.is_none());
    }

    #[test]
    fn parse_proxy_endpoint_with_auth() {
        let ep = parse_proxy_endpoint("http://user:pass@proxy.example.com:3128").unwrap();
        assert_eq!(ep.host, "proxy.example.com");
        assert_eq!(ep.port, 3128);
        let auth = ep.auth.unwrap();
        assert_eq!(auth.username, "user");
        assert_eq!(auth.password, "pass");
    }

    #[test]
    fn parse_proxy_endpoint_no_scheme() {
        let ep = parse_proxy_endpoint("proxy.example.com:8080").unwrap();
        assert_eq!(ep.host, "proxy.example.com");
        assert_eq!(ep.port, 8080);
        assert_eq!(ep.scheme, "http");
    }

    #[test]
    fn parse_proxy_endpoint_default_ports() {
        let http = parse_proxy_endpoint("http://proxy.example.com").unwrap();
        assert_eq!(http.port, 80);

        let https = parse_proxy_endpoint("https://proxy.example.com").unwrap();
        assert_eq!(https.port, 443);
    }

    #[test]
    fn parse_proxy_endpoint_unsupported_scheme() {
        assert!(parse_proxy_endpoint("socks5://proxy.example.com").is_err());
    }

    #[test]
    fn proxy_config_default_has_no_proxy() {
        let config = ProxyConfig::default();
        assert!(!config.has_proxy());
        assert!(config.proxy_for_protocol(true).is_none());
        assert!(config.proxy_for_protocol(false).is_none());
    }

    #[test]
    fn proxy_config_selects_correct_proxy() {
        let config = ProxyConfig {
            http: Some(ProxyEndpoint {
                host: "http-proxy".into(),
                port: 3128,
                scheme: "http".into(),
                auth: None,
            }),
            https: Some(ProxyEndpoint {
                host: "https-proxy".into(),
                port: 3129,
                scheme: "http".into(),
                auth: None,
            }),
            all: None,
        };
        assert_eq!(config.proxy_for_protocol(false).unwrap().host, "http-proxy");
        assert_eq!(
            config.proxy_for_protocol(true).unwrap().host,
            "https-proxy"
        );
    }

    #[test]
    fn proxy_config_falls_back_to_all() {
        let config = ProxyConfig {
            http: None,
            https: None,
            all: Some(ProxyEndpoint {
                host: "all-proxy".into(),
                port: 3130,
                scheme: "http".into(),
                auth: None,
            }),
        };
        assert_eq!(config.proxy_for_protocol(false).unwrap().host, "all-proxy");
        assert_eq!(config.proxy_for_protocol(true).unwrap().host, "all-proxy");
    }

    #[test]
    fn upstream_client_direct() {
        let client = UpstreamClient::direct(true);
        assert!(!client.has_upstream_proxy());
        assert!(client.allow_local_binding());
    }

    #[test]
    fn base64_encode_basic() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"user:pass"), "dXNlcjpwYXNz");
    }
}
