//! MITM TLS interception for HTTPS CONNECT tunnels.
//!
//! When the proxy operates in `Limited` mode, plain CONNECT tunneling
//! would bypass policy enforcement on inner HTTP requests. This module
//! terminates the client TLS using a dynamically issued leaf certificate
//! (signed by the local MITM CA) and re-applies policy before forwarding
//! to the upstream server.

use crate::certs::ManagedMitmCa;
use crate::config::NetworkMode;
use crate::network_policy::HostBlockDecision;
use crate::policy::normalize_host;
use crate::reasons::REASON_METHOD_NOT_ALLOWED;
use crate::runtime::{BlockedRequestArgs, NetworkProxyState};
use crate::upstream::UpstreamClient;
use anyhow::{Context as _, Result, anyhow};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::HOST;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode, Uri};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::{info, warn};

type BoxBody = Full<Bytes>;
type HttpResponse = Response<BoxBody>;

const MITM_MAX_BODY_BYTES: usize = 4096;

/// Configuration for creating an MitmState.
pub struct MitmUpstreamConfig {
    pub allow_upstream_proxy: bool,
    pub allow_local_binding: bool,
}

/// State needed to terminate a CONNECT tunnel and enforce policy on inner
/// HTTPS requests.
pub struct MitmState {
    ca: ManagedMitmCa,
    upstream: UpstreamClient,
    inspect: bool,
    max_body_bytes: usize,
}

impl std::fmt::Debug for MitmState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MitmState")
            .field("inspect", &self.inspect)
            .field("max_body_bytes", &self.max_body_bytes)
            .finish_non_exhaustive()
    }
}

impl MitmState {
    /// Create a new MITM state, loading or generating the local CA.
    pub fn new(config: MitmUpstreamConfig) -> Result<Self> {
        let ca = ManagedMitmCa::load_or_create()?;

        let upstream = if config.allow_upstream_proxy {
            UpstreamClient::from_env_proxy(config.allow_local_binding)
        } else {
            UpstreamClient::direct(config.allow_local_binding)
        };

        Ok(Self {
            ca,
            upstream,
            inspect: false,
            max_body_bytes: MITM_MAX_BODY_BYTES,
        })
    }

    /// Create from explicit CA PEM (for testing).
    pub fn from_ca_pem(
        cert_pem: &str,
        key_pem: &str,
        allow_local_binding: bool,
    ) -> Result<Self> {
        let ca = ManagedMitmCa::from_pem(cert_pem, key_pem)?;
        Ok(Self {
            ca,
            upstream: UpstreamClient::direct(allow_local_binding),
            inspect: false,
            max_body_bytes: MITM_MAX_BODY_BYTES,
        })
    }

    /// Return the CA certificate PEM for trust-anchoring clients.
    pub fn ca_cert_pem(&self) -> &str {
        self.ca.ca_cert_pem()
    }

    pub fn inspect_enabled(&self) -> bool {
        self.inspect
    }

    pub fn max_body_bytes(&self) -> usize {
        self.max_body_bytes
    }
}

/// Policy context for a single MITM tunnel.
struct MitmPolicyContext {
    target_host: String,
    target_port: u16,
    mode: NetworkMode,
    app_state: Arc<NetworkProxyState>,
}

/// Full context for a MITM request handler.
struct MitmRequestContext {
    policy: MitmPolicyContext,
    mitm: Arc<MitmState>,
}

/// Terminate a CONNECT tunnel with MITM TLS interception.
///
/// After the client receives 200 OK for CONNECT, the upgraded stream is
/// handed to this function which:
/// 1. Generates a TLS leaf certificate for the target host
/// 2. Terminates client TLS
/// 3. Serves inner HTTP requests, applying policy and forwarding to upstream
pub async fn mitm_tunnel<S>(
    stream: S,
    target_host: &str,
    target_port: u16,
    mode: NetworkMode,
    app_state: Arc<NetworkProxyState>,
    mitm: Arc<MitmState>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let host = normalize_host(target_host);
    let acceptor = mitm
        .ca
        .tls_acceptor_for_host(&host)
        .context("failed to create TLS acceptor for MITM")?;

    let tls_stream = acceptor
        .accept(stream)
        .await
        .map_err(|e| anyhow!("MITM TLS handshake failed for {host}: {e}"))?;

    info!("MITM TLS terminated for {host}:{target_port}");

    let request_ctx = Arc::new(MitmRequestContext {
        policy: MitmPolicyContext {
            target_host: host.clone(),
            target_port,
            mode,
            app_state,
        },
        mitm,
    });

    let io = hyper_util::rt::TokioIo::new(tls_stream);
    let request_ctx_for_svc = Arc::clone(&request_ctx);

    http1::Builder::new()
        .preserve_header_case(true)
        .title_case_headers(false)
        .serve_connection(
            io,
            service_fn(move |req| {
                let ctx = Arc::clone(&request_ctx_for_svc);
                async move { handle_mitm_request(req, ctx).await }
            }),
        )
        .await
        .map_err(|e| anyhow!("MITM serve error for {host}: {e}"))?;

    Ok(())
}

async fn handle_mitm_request(
    req: Request<Incoming>,
    ctx: Arc<MitmRequestContext>,
) -> Result<HttpResponse, hyper::Error> {
    let result = forward_request(req, &ctx).await;
    Ok(result.unwrap_or_else(|e| {
        warn!("MITM request handling failed: {e}");
        text_response(StatusCode::BAD_GATEWAY, "mitm upstream error")
    }))
}

async fn forward_request(
    req: Request<Incoming>,
    ctx: &MitmRequestContext,
) -> Result<HttpResponse> {
    if let Some(response) = mitm_blocking_response(&req, &ctx.policy).await? {
        return Ok(response);
    }

    let target_host = &ctx.policy.target_host;
    let target_port = ctx.policy.target_port;
    let method = req.method().clone();
    let log_path = req.uri().path().to_string();
    let path = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());
    let authority = authority_header_value(target_host, target_port);

    let body_bytes = req.collect().await?.to_bytes();
    let uri: Uri = format!("https://{authority}{path}")
        .parse()
        .context("failed to build upstream URI")?;

    let upstream_stream = ctx
        .mitm
        .upstream
        .connect_tcp(target_host, target_port, true)
        .await?;

    let tls_connector = build_upstream_tls_connector()?;
    let server_name = rustls::pki_types::ServerName::try_from(target_host.to_string())
        .map_err(|e| anyhow!("invalid server name {target_host}: {e}"))?;
    let tls_stream = tls_connector
        .connect(server_name, upstream_stream)
        .await
        .map_err(|e| anyhow!("upstream TLS handshake failed for {target_host}: {e}"))?;

    let io = hyper_util::rt::TokioIo::new(tls_stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            warn!("MITM upstream connection error: {e}");
        }
    });

    let mut builder = Request::builder().method(&method).uri(uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/"));
    builder = builder.header(HOST, &authority);

    let upstream_req = builder
        .body(Full::new(body_bytes))
        .context("failed to build upstream request")?;

    let upstream_resp = sender
        .send_request(upstream_req)
        .await
        .context("upstream request failed")?;

    info!(
        "MITM {method} https://{authority}{log_path} -> {}",
        upstream_resp.status()
    );

    let (parts, body) = upstream_resp.into_parts();
    let body_bytes = body.collect().await?.to_bytes();

    let mut resp_builder = Response::builder().status(parts.status);
    for (name, value) in &parts.headers {
        resp_builder = resp_builder.header(name, value);
    }
    resp_builder
        .body(Full::new(body_bytes))
        .context("failed to build MITM response")
}

async fn mitm_blocking_response(
    req: &Request<Incoming>,
    policy: &MitmPolicyContext,
) -> Result<Option<HttpResponse>> {
    if req.method() == Method::CONNECT {
        return Ok(Some(text_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "CONNECT not supported inside MITM",
        )));
    }

    let method = req.method().as_str();

    if let Some(request_host) = extract_request_host(req) {
        let normalized = normalize_host(&request_host);
        if !normalized.is_empty() && normalized != policy.target_host {
            warn!(
                "MITM host mismatch (target={}, request_host={normalized})",
                policy.target_host
            );
            return Ok(Some(text_response(
                StatusCode::BAD_REQUEST,
                "host mismatch",
            )));
        }
    }

    let block_decision = policy
        .app_state
        .host_blocked(&policy.target_host, policy.target_port)
        .await?;
    if let HostBlockDecision::Blocked(ref block_reason) = block_decision {
        let reason = block_reason.as_str();
        let _ = policy
            .app_state
            .record_blocked(BlockedRequestArgs {
                host: policy.target_host.clone(),
                reason: reason.to_string(),
                client: None,
                method: Some(method.to_string()),
                protocol: "https".to_string(),
                port: Some(policy.target_port),
            })
            .await;
        warn!(
            "MITM blocked local/private target (host={}, port={})",
            policy.target_host, policy.target_port
        );
        return Ok(Some(text_response(StatusCode::FORBIDDEN, reason)));
    }

    if !policy.mode.allows_method(method) {
        let _ = policy
            .app_state
            .record_blocked(BlockedRequestArgs {
                host: policy.target_host.clone(),
                reason: REASON_METHOD_NOT_ALLOWED.to_string(),
                client: None,
                method: Some(method.to_string()),
                protocol: "https".to_string(),
                port: Some(policy.target_port),
            })
            .await;
        warn!(
            "MITM blocked by method policy (host={}, method={method}, mode={:?})",
            policy.target_host, policy.mode
        );
        return Ok(Some(text_response(
            StatusCode::FORBIDDEN,
            REASON_METHOD_NOT_ALLOWED,
        )));
    }

    Ok(None)
}

fn build_upstream_tls_connector() -> Result<tokio_rustls::TlsConnector> {
    let root_store = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    Ok(tokio_rustls::TlsConnector::from(Arc::new(config)))
}

fn extract_request_host(req: &Request<Incoming>) -> Option<String> {
    req.headers()
        .get(HOST)
        .and_then(|v| v.to_str().ok())
        .map(|h| h.split(':').next().unwrap_or(h).to_string())
        .or_else(|| req.uri().host().map(String::from))
}

fn authority_header_value(host: &str, port: u16) -> String {
    if host.contains(':') {
        if port == 443 {
            format!("[{host}]")
        } else {
            format!("[{host}]:{port}")
        }
    } else if port == 443 {
        host.to_string()
    } else {
        format!("{host}:{port}")
    }
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
    fn authority_header_value_standard_port() {
        assert_eq!(authority_header_value("example.com", 443), "example.com");
    }

    #[test]
    fn authority_header_value_non_standard_port() {
        assert_eq!(
            authority_header_value("example.com", 8443),
            "example.com:8443"
        );
    }

    #[test]
    fn authority_header_value_ipv6() {
        assert_eq!(authority_header_value("::1", 443), "[::1]");
        assert_eq!(authority_header_value("::1", 8443), "[::1]:8443");
    }

    #[test]
    fn text_response_basic() {
        let resp = text_response(StatusCode::FORBIDDEN, "blocked");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }
}
