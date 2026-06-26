use arc_swap::ArcSwap;
use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use constant_time_eq::constant_time_eq;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AuthConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub api_keys: Vec<String>,
}

#[derive(Debug, Clone)]
struct AuthSnapshot {
    enabled: bool,
    valid_keys: Vec<String>,
}

/// API key authentication layer.
/// Checks `Authorization: Bearer <key>` or `X-API-Key: <key>` headers.
#[derive(Clone)]
pub struct ApiKeyAuth {
    inner: Arc<ArcSwap<AuthSnapshot>>,
}

impl ApiKeyAuth {
    pub fn new(config: &AuthConfig) -> Self {
        Self {
            inner: Arc::new(ArcSwap::from_pointee(snapshot_from_config(config))),
        }
    }

    /// Hot-reload API keys and enabled flag (e.g. after `config.set security`).
    pub fn reload(&self, config: &AuthConfig) {
        self.inner.store(Arc::new(snapshot_from_config(config)));
    }

    pub fn is_enabled(&self) -> bool {
        self.inner.load().enabled
    }

    pub fn validate_key(&self, key: &str) -> bool {
        let snap = self.inner.load();
        if !snap.enabled {
            return true;
        }
        let key_hash = hash_api_key(key);
        let mut ok = 0u8;
        for k in &snap.valid_keys {
            ok |= constant_time_eq(&key_hash, &hash_api_key(k)) as u8;
        }
        ok == 1
    }

    /// Extract an API key for WebSocket upgrade requests.
    ///
    /// Priority: `Authorization: Bearer`, `Sec-WebSocket-Protocol` subprotocol
    /// (`bearer.<key>` or `api-key.<key>`), then deprecated query `token` /
    /// `api_key`.
    ///
    /// Query-string keys are supported only because browser WebSocket APIs cannot
    /// set custom headers on upgrade requests. Prefer `Authorization: Bearer` or
    /// `Sec-WebSocket-Protocol` when the client environment allows it.
    pub fn extract_ws_key(
        headers: &axum::http::HeaderMap,
        query_token: Option<&str>,
        query_api_key: Option<&str>,
    ) -> Option<(String, WsKeySource)> {
        if let Some(auth) = headers.get("authorization") {
            if let Ok(value) = auth.to_str() {
                if let Some(token) = value.strip_prefix("Bearer ") {
                    let token = token.trim();
                    if !token.is_empty() {
                        return Some((token.to_string(), WsKeySource::AuthorizationHeader));
                    }
                }
            }
        }

        if let Some(proto) = headers.get("sec-websocket-protocol") {
            if let Ok(value) = proto.to_str() {
                for part in value.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    if let Some(token) = part
                        .strip_prefix("bearer.")
                        .or_else(|| part.strip_prefix("api-key."))
                    {
                        if !token.is_empty() {
                            return Some((token.to_string(), WsKeySource::WebSocketProtocol));
                        }
                    }
                }
            }
        }

        if let Some(token) = query_token.map(str::trim).filter(|t| !t.is_empty()) {
            return Some((token.to_string(), WsKeySource::QueryString));
        }
        if let Some(token) = query_api_key.map(str::trim).filter(|t| !t.is_empty()) {
            return Some((token.to_string(), WsKeySource::QueryString));
        }

        None
    }

    fn extract_key_with_source(req: &Request) -> Option<(String, WsKeySource)> {
        if let Some(auth) = req.headers().get("authorization") {
            if let Ok(value) = auth.to_str() {
                if let Some(token) = value.strip_prefix("Bearer ") {
                    let token = token.trim();
                    if !token.is_empty() {
                        return Some((token.to_string(), WsKeySource::AuthorizationHeader));
                    }
                }
            }
        }

        if let Some(key) = req.headers().get("x-api-key") {
            if let Ok(value) = key.to_str() {
                let value = value.trim();
                if !value.is_empty() {
                    return Some((value.to_string(), WsKeySource::AuthorizationHeader));
                }
            }
        }

        // WebSocket upgrade: browser WebSocket APIs cannot attach custom headers, so
        // clients may pass the key in the query string as a last resort.
        if req.uri().path() == "/ws" {
            if let Some(q) = req.uri().query() {
                for pair in q.split('&') {
                    let (k, v) = match pair.split_once('=') {
                        Some((k, v)) => (k, v),
                        None => (pair, ""),
                    };
                    if k == "token" || k == "api_key" {
                        let v = decode_query_component(v);
                        let v = v.trim();
                        if !v.is_empty() {
                            return Some((v.to_string(), WsKeySource::QueryString));
                        }
                    }
                }
            }
        }

        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsKeySource {
    AuthorizationHeader,
    WebSocketProtocol,
    QueryString,
}

fn hash_api_key(key: &str) -> [u8; 32] {
    Sha256::digest(key.as_bytes()).into()
}

fn snapshot_from_config(config: &AuthConfig) -> AuthSnapshot {
    AuthSnapshot {
        enabled: config.enabled,
        valid_keys: config.api_keys.clone(),
    }
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Percent-decode a URL path segment (application/x-www-form-urlencoded rules).
fn percent_decode_path(path: &str) -> String {
    decode_query_component(path)
}

/// Collapse duplicate slashes and resolve `.` / `..` path components.
fn normalize_request_path(raw: &str) -> String {
    let decoded = percent_decode_path(raw);
    let collapsed = decoded.replace("//", "/");
    let absolute = collapsed.starts_with('/');
    let mut stack: Vec<&str> = Vec::new();

    for component in collapsed.split('/') {
        match component {
            "" | "." => continue,
            ".." => {
                if absolute {
                    stack.pop();
                } else if !stack.is_empty() {
                    stack.pop();
                }
            }
            segment => stack.push(segment),
        }
    }

    if absolute {
        if stack.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", stack.join("/"))
        }
    } else {
        stack.join("/")
    }
}

fn is_auth_exempt_path(path: &str) -> bool {
    path == "/health"
        || path == "/healthz"
        || path == "/ready"
        || path == "/"
        || path == "/ui"
        || (path.starts_with("/webhook/") && path.split('/').count() <= 4)
}

/// Minimal application/x-www-form-urlencoded decoding for query values.
fn decode_query_component(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let mut bytes = s.as_bytes().iter().copied();
    while let Some(b) = bytes.next() {
        if b == b'+' {
            out.push(b' ');
        } else if b == b'%' {
            let a = bytes.next();
            let c = bytes.next();
            match (a, c) {
                (Some(a), Some(c)) => {
                    if let (Some(hi), Some(lo)) = (hex_digit(a), hex_digit(c)) {
                        out.push(hi << 4 | lo);
                    } else {
                        tracing::debug!("auth: malformed percent-encoding in query parameter");
                        out.push(b'%');
                        out.push(a);
                        out.push(c);
                    }
                }
                (maybe_a, _) => {
                    tracing::debug!("auth: truncated percent-encoding at end of query parameter");
                    out.push(b'%');
                    if let Some(a) = maybe_a {
                        out.push(a);
                    }
                }
            }
        } else {
            out.push(b);
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Axum middleware function for API key auth.
pub async fn auth_middleware(
    auth: axum::extract::Extension<ApiKeyAuth>,
    req: Request,
    next: Next,
) -> Response {
    let auth = &auth.0;

    if !auth.is_enabled() {
        return next.run(req).await;
    }

    let raw_path = req.uri().path();
    let path = normalize_request_path(raw_path);
    if path != raw_path {
        tracing::debug!(
            raw = %raw_path,
            normalized = %path,
            "auth: request path normalized before auth check"
        );
    }
    if is_auth_exempt_path(&path) {
        return next.run(req).await;
    }

    match ApiKeyAuth::extract_key_with_source(&req) {
        Some((key, source)) if auth.validate_key(&key) => {
            if source == WsKeySource::QueryString {
                tracing::debug!(path = %path, "authenticated via query param");
            }
            next.run(req).await
        }
        Some(_) => {
            tracing::warn!(path, "invalid API key");
            (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({
                    "error": { "message": "Invalid API key", "type": "authentication_error" }
                })),
            )
                .into_response()
        }
        None => {
            tracing::warn!(path, "missing API key");
            (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({
                    "error": { "message": "API key required. Use Authorization: Bearer <key> or X-API-Key: <key>", "type": "authentication_error" }
                })),
            )
                .into_response()
        }
    }
}
