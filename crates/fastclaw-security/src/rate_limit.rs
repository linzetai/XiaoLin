use axum::{
    extract::{ConnectInfo, Request},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use dashmap::DashMap;
use serde::Deserialize;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Maximum requests per window.
    #[serde(default = "default_max_requests")]
    pub max_requests: u32,
    /// Window duration in seconds.
    #[serde(default = "default_window_secs")]
    pub window_secs: u64,
    #[serde(default)]
    pub trusted_proxies: Vec<IpAddr>,
}

fn default_max_requests() -> u32 {
    60
}
fn default_window_secs() -> u64 {
    60
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_requests: default_max_requests(),
            window_secs: default_window_secs(),
            trusted_proxies: Vec::new(),
        }
    }
}

struct TokenBucket {
    tokens: u32,
    last_refill: Instant,
}

/// IP-based rate limiter using a token bucket per client.
#[derive(Clone)]
pub struct RateLimiter {
    buckets: Arc<DashMap<IpAddr, TokenBucket>>,
    max_tokens: u32,
    window: std::time::Duration,
    enabled: bool,
    trusted_proxies: Vec<IpAddr>,
}

impl RateLimiter {
    pub fn new(config: &RateLimitConfig) -> Self {
        Self {
            buckets: Arc::new(DashMap::new()),
            max_tokens: config.max_requests,
            window: std::time::Duration::from_secs(config.window_secs),
            enabled: config.enabled,
            trusted_proxies: config.trusted_proxies.clone(),
        }
    }

    fn check(&self, ip: IpAddr) -> bool {
        let now = Instant::now();

        let mut entry = self.buckets.entry(ip).or_insert_with(|| TokenBucket {
            tokens: self.max_tokens,
            last_refill: now,
        });

        let elapsed = now.duration_since(entry.last_refill);
        if elapsed >= self.window {
            entry.tokens = self.max_tokens;
            entry.last_refill = now;
        }

        if entry.tokens > 0 {
            entry.tokens -= 1;
            true
        } else {
            false
        }
    }
}

fn extract_client_ip(req: &Request, trusted_proxies: &[IpAddr]) -> IpAddr {
    let connect_ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip());

    let peer_ip = connect_ip.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));

    if !trusted_proxies.is_empty() && trusted_proxies.contains(&peer_ip) {
        if let Some(xff) = req.headers().get("x-forwarded-for") {
            if let Ok(value) = xff.to_str() {
                if let Some(first_ip) = value.split(',').next() {
                    if let Ok(ip) = first_ip.trim().parse::<IpAddr>() {
                        return ip;
                    }
                }
            }
        }
    }

    peer_ip
}

/// Axum middleware function for rate limiting.
pub async fn rate_limit_middleware(
    limiter: axum::extract::Extension<RateLimiter>,
    req: Request,
    next: Next,
) -> Response {
    let limiter = &limiter.0;

    if !limiter.enabled {
        return next.run(req).await;
    }

    // Skip rate limiting for health endpoints
    let path = req.uri().path();
    if path == "/health" || path == "/healthz" || path == "/ready" {
        return next.run(req).await;
    }

    let ip = extract_client_ip(&req, &limiter.trusted_proxies);

    if limiter.check(ip) {
        next.run(req).await
    } else {
        tracing::warn!(%ip, path, "rate limit exceeded");
        (
            StatusCode::TOO_MANY_REQUESTS,
            axum::Json(serde_json::json!({
                "error": {
                    "message": "Rate limit exceeded. Please try again later.",
                    "type": "rate_limit_error"
                }
            })),
        )
            .into_response()
    }
}
