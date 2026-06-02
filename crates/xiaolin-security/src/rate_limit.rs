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

/// Multi-dimension rate limiter: per-IP, per-API-key, and per-agent token buckets.
#[derive(Clone)]
pub struct RateLimiter {
    ip_buckets: Arc<DashMap<IpAddr, TokenBucket>>,
    key_buckets: Arc<DashMap<String, TokenBucket>>,
    agent_buckets: Arc<DashMap<String, TokenBucket>>,
    max_tokens: u32,
    window: std::time::Duration,
    enabled: bool,
    trusted_proxies: Vec<IpAddr>,
}

impl RateLimiter {
    pub fn new(config: &RateLimitConfig) -> Self {
        Self {
            ip_buckets: Arc::new(DashMap::new()),
            key_buckets: Arc::new(DashMap::new()),
            agent_buckets: Arc::new(DashMap::new()),
            max_tokens: config.max_requests,
            window: std::time::Duration::from_secs(config.window_secs),
            enabled: config.enabled,
            trusted_proxies: config.trusted_proxies.clone(),
        }
    }

    fn check_bucket<K: std::hash::Hash + Eq + Clone>(
        buckets: &DashMap<K, TokenBucket>,
        key: K,
        max_tokens: u32,
        window: std::time::Duration,
    ) -> bool {
        let now = Instant::now();
        let mut entry = buckets.entry(key).or_insert_with(|| TokenBucket {
            tokens: max_tokens,
            last_refill: now,
        });
        let elapsed = now.duration_since(entry.last_refill);
        if elapsed >= window {
            entry.tokens = max_tokens;
            entry.last_refill = now;
        }
        if entry.tokens > 0 {
            entry.tokens -= 1;
            true
        } else {
            false
        }
    }

    fn check_ip(&self, ip: IpAddr) -> bool {
        Self::check_bucket(&self.ip_buckets, ip, self.max_tokens, self.window)
    }

    /// Check rate limit for an API key (uses same window/max as IP).
    pub fn check_api_key(&self, key: &str) -> bool {
        Self::check_bucket(
            &self.key_buckets,
            key.to_string(),
            self.max_tokens,
            self.window,
        )
    }

    /// Check rate limit for an agent ID.
    pub fn check_agent(&self, agent_id: &str) -> bool {
        Self::check_bucket(
            &self.agent_buckets,
            agent_id.to_string(),
            self.max_tokens * 2,
            self.window,
        )
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

    if limiter.check_ip(ip) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(max: u32, window_secs: u64) -> RateLimiter {
        RateLimiter::new(&RateLimitConfig {
            enabled: true,
            max_requests: max,
            window_secs,
            trusted_proxies: vec![],
        })
    }

    #[test]
    fn check_ip_allows_within_limit() {
        let rl = limiter(3, 60);
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        assert!(rl.check_ip(ip));
        assert!(rl.check_ip(ip));
        assert!(rl.check_ip(ip));
        assert!(!rl.check_ip(ip));
    }

    #[tokio::test]
    async fn check_ip_refills_after_window() {
        let rl = limiter(2, 1);
        let ip: IpAddr = "10.0.0.2".parse().unwrap();
        assert!(rl.check_ip(ip));
        assert!(rl.check_ip(ip));
        assert!(!rl.check_ip(ip));

        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        assert!(rl.check_ip(ip));
    }

    #[test]
    fn check_api_key_separate_buckets() {
        let rl = limiter(2, 60);
        assert!(rl.check_api_key("key-a"));
        assert!(rl.check_api_key("key-a"));
        assert!(!rl.check_api_key("key-a"));

        assert!(rl.check_api_key("key-b"));
        assert!(rl.check_api_key("key-b"));
    }

    #[test]
    fn check_agent_double_burst() {
        let rl = limiter(3, 60);
        for _ in 0..6 {
            assert!(rl.check_agent("agent-1"));
        }
        assert!(!rl.check_agent("agent-1"));
    }

    #[test]
    fn disabled_limiter_buckets_still_work() {
        let rl = RateLimiter::new(&RateLimitConfig {
            enabled: false,
            max_requests: 1,
            window_secs: 60,
            trusted_proxies: vec![],
        });
        assert!(rl.check_api_key("k"));
        assert!(!rl.check_api_key("k"));
    }

    #[test]
    fn empty_string_key() {
        let rl = limiter(1, 60);
        assert!(rl.check_api_key(""));
        assert!(!rl.check_api_key(""));
    }
}
