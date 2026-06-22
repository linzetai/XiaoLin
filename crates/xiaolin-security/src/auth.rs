use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use arc_swap::ArcSwap;
use constant_time_eq::constant_time_eq;
use serde::Deserialize;
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
        self.inner
            .store(Arc::new(snapshot_from_config(config)));
    }

    pub fn is_enabled(&self) -> bool {
        self.inner.load().enabled
    }

    pub fn validate_key(&self, key: &str) -> bool {
        let snap = self.inner.load();
        if !snap.enabled {
            return true;
        }
        snap.valid_keys
            .iter()
            .any(|k| constant_time_eq(k.as_bytes(), key.as_bytes()))
    }

    fn extract_key(req: &Request) -> Option<String> {
        if let Some(auth) = req.headers().get("authorization") {
            if let Ok(value) = auth.to_str() {
                if let Some(token) = value.strip_prefix("Bearer ") {
                    return Some(token.trim().to_string());
                }
            }
        }

        if let Some(key) = req.headers().get("x-api-key") {
            if let Ok(value) = key.to_str() {
                return Some(value.trim().to_string());
            }
        }

        // WebSocket upgrade: clients often cannot set custom headers; allow key in query string.
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
                            return Some(v.to_string());
                        }
                    }
                }
            }
        }

        None
    }
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
    let normalized = raw_path.replace("//", "/");
    let path = normalized.as_str();
    if path != raw_path {
        tracing::debug!(
            raw = %raw_path,
            normalized = %path,
            "auth: path normalized (double slashes removed)"
        );
    }
    if path == "/health"
        || path == "/healthz"
        || path == "/ready"
        || path == "/"
        || path == "/ui"
        || (path.starts_with("/webhook/") && !path.contains("..") && path.split('/').count() <= 4)
    {
        return next.run(req).await;
    }

    match ApiKeyAuth::extract_key(&req) {
        Some(key) if auth.validate_key(&key) => next.run(req).await,
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
