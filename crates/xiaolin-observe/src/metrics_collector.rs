//! In-memory structured metrics with Prometheus text exposition (no external TSDB).

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use dashmap::DashMap;

const MAX_HISTOGRAM_SAMPLES: usize = 50_000;

fn escape_label_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('"', "\\\"")
}

/// Simple in-memory metrics store for counters and latency samples.
pub struct MetricsCollector {
    pub counters: DashMap<String, AtomicU64>,
    pub histograms: DashMap<String, Mutex<Vec<f64>>>,
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsCollector {
    pub fn new() -> Self {
        Self {
            counters: DashMap::new(),
            histograms: DashMap::new(),
        }
    }

    fn request_counter_key(agent: &str, channel: &str) -> String {
        format!(
            "request|{}|{}",
            escape_label_value(agent),
            escape_label_value(channel)
        )
    }

    fn error_counter_key(error_type: &str) -> String {
        format!("error|{}", escape_label_value(error_type))
    }

    fn token_counter_key(model: &str) -> String {
        format!("tokens|{}", escape_label_value(model))
    }

    fn provider_request_key(provider: &str, model: &str) -> String {
        format!(
            "provider_request|{}|{}",
            escape_label_value(provider),
            escape_label_value(model)
        )
    }

    fn provider_token_key(provider: &str, model: &str) -> String {
        format!(
            "provider_tokens|{}|{}",
            escape_label_value(provider),
            escape_label_value(model)
        )
    }

    pub fn record_request(&self, agent: &str, channel: &str) {
        let key = Self::request_counter_key(agent, channel);
        self.counters
            .entry(key)
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_latency_ms(&self, endpoint: &str, ms: f64) {
        let key = format!("latency|{}", escape_label_value(endpoint));
        let entry = self
            .histograms
            .entry(key)
            .or_insert_with(|| Mutex::new(Vec::new()));
        let mut vec = match entry.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        if vec.len() < MAX_HISTOGRAM_SAMPLES {
            vec.push(ms);
        }
    }

    pub fn record_error(&self, error_type: &str) {
        let key = Self::error_counter_key(error_type);
        self.counters
            .entry(key)
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_tokens(&self, model: &str, tokens: u64) {
        let key = Self::token_counter_key(model);
        self.counters
            .entry(key)
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(tokens, Ordering::Relaxed);
    }

    /// Record a provider-level request (provider name + model).
    pub fn record_provider_request(&self, provider: &str, model: &str) {
        let key = Self::provider_request_key(provider, model);
        self.counters
            .entry(key)
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Record tokens with provider+model breakdown.
    pub fn record_provider_tokens(&self, provider: &str, model: &str, tokens: u64) {
        let key = Self::provider_token_key(provider, model);
        self.counters
            .entry(key)
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(tokens, Ordering::Relaxed);
    }

    /// Record provider-level latency.
    pub fn record_provider_latency_ms(&self, provider: &str, model: &str, ms: f64) {
        let key = format!(
            "provider_latency|{}|{}",
            escape_label_value(provider),
            escape_label_value(model)
        );
        let entry = self
            .histograms
            .entry(key)
            .or_insert_with(|| Mutex::new(Vec::new()));
        let mut vec = match entry.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        if vec.len() < MAX_HISTOGRAM_SAMPLES {
            vec.push(ms);
        }
    }

    /// Compute percentiles (p50, p95, p99) from a histogram key.
    fn percentiles(samples: &[f64]) -> (f64, f64, f64) {
        if samples.is_empty() {
            return (0.0, 0.0, 0.0);
        }
        let mut sorted = samples.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let len = sorted.len();
        let p50 = sorted[(len as f64 * 0.50) as usize];
        let p95 = sorted[((len as f64 * 0.95) as usize).min(len - 1)];
        let p99 = sorted[((len as f64 * 0.99) as usize).min(len - 1)];
        (p50, p95, p99)
    }

    /// Renders all recorded metrics in Prometheus text exposition format (0.0.5).
    pub fn render_prometheus(&self) -> String {
        let mut out = String::new();

        out.push_str("# HELP xiaolin_requests_total Requests by agent and channel\n");
        out.push_str("# TYPE xiaolin_requests_total counter\n");
        for e in self.counters.iter() {
            let key = e.key();
            if let Some(rest) = key.strip_prefix("request|") {
                let parts: Vec<&str> = rest.splitn(2, '|').collect();
                if parts.len() == 2 {
                    let _ = writeln!(
                        &mut out,
                        "xiaolin_requests_total{{agent=\"{}\",channel=\"{}\"}} {}",
                        parts[0],
                        parts[1],
                        e.value().load(Ordering::Relaxed)
                    );
                }
            }
        }

        out.push_str("# HELP xiaolin_endpoint_latency_ms Latency samples by endpoint (summary)\n");
        out.push_str("# TYPE xiaolin_endpoint_latency_ms summary\n");
        for e in self.histograms.iter() {
            let key = e.key();
            let Some(endpoint) = key.strip_prefix("latency|") else {
                continue;
            };
            let vec = match e.value().lock() {
                Ok(g) => g.clone(),
                Err(poisoned) => poisoned.into_inner().clone(),
            };
            let count = vec.len() as f64;
            let sum: f64 = vec.iter().copied().sum();
            let (p50, p95, p99) = Self::percentiles(&vec);
            let _ = writeln!(
                &mut out,
                "xiaolin_endpoint_latency_ms{{endpoint=\"{}\",quantile=\"0.5\"}} {}",
                endpoint, p50
            );
            let _ = writeln!(
                &mut out,
                "xiaolin_endpoint_latency_ms{{endpoint=\"{}\",quantile=\"0.95\"}} {}",
                endpoint, p95
            );
            let _ = writeln!(
                &mut out,
                "xiaolin_endpoint_latency_ms{{endpoint=\"{}\",quantile=\"0.99\"}} {}",
                endpoint, p99
            );
            let _ = writeln!(
                &mut out,
                "xiaolin_endpoint_latency_ms_sum{{endpoint=\"{}\"}} {}",
                endpoint, sum
            );
            let _ = writeln!(
                &mut out,
                "xiaolin_endpoint_latency_ms_count{{endpoint=\"{}\"}} {}",
                endpoint, count
            );
        }

        out.push_str("# HELP xiaolin_errors_total Errors by coarse type\n");
        out.push_str("# TYPE xiaolin_errors_total counter\n");
        for e in self.counters.iter() {
            let key = e.key();
            if let Some(typ) = key.strip_prefix("error|") {
                let _ = writeln!(
                    &mut out,
                    "xiaolin_errors_total{{type=\"{}\"}} {}",
                    typ,
                    e.value().load(Ordering::Relaxed)
                );
            }
        }

        out.push_str("# HELP xiaolin_llm_tokens_total LLM token usage by model\n");
        out.push_str("# TYPE xiaolin_llm_tokens_total counter\n");
        for e in self.counters.iter() {
            let key = e.key();
            if let Some(model) = key.strip_prefix("tokens|") {
                let _ = writeln!(
                    &mut out,
                    "xiaolin_llm_tokens_total{{model=\"{}\"}} {}",
                    model,
                    e.value().load(Ordering::Relaxed)
                );
            }
        }

        out.push_str("# HELP xiaolin_provider_requests_total Requests by provider and model\n");
        out.push_str("# TYPE xiaolin_provider_requests_total counter\n");
        for e in self.counters.iter() {
            let key = e.key();
            if let Some(rest) = key.strip_prefix("provider_request|") {
                let parts: Vec<&str> = rest.splitn(2, '|').collect();
                if parts.len() == 2 {
                    let _ = writeln!(
                        &mut out,
                        "xiaolin_provider_requests_total{{provider=\"{}\",model=\"{}\"}} {}",
                        parts[0],
                        parts[1],
                        e.value().load(Ordering::Relaxed)
                    );
                }
            }
        }

        out.push_str("# HELP xiaolin_provider_tokens_total Tokens by provider and model\n");
        out.push_str("# TYPE xiaolin_provider_tokens_total counter\n");
        for e in self.counters.iter() {
            let key = e.key();
            if let Some(rest) = key.strip_prefix("provider_tokens|") {
                let parts: Vec<&str> = rest.splitn(2, '|').collect();
                if parts.len() == 2 {
                    let _ = writeln!(
                        &mut out,
                        "xiaolin_provider_tokens_total{{provider=\"{}\",model=\"{}\"}} {}",
                        parts[0],
                        parts[1],
                        e.value().load(Ordering::Relaxed)
                    );
                }
            }
        }

        out.push_str(
            "# HELP xiaolin_provider_latency_ms Provider latency by provider and model\n",
        );
        out.push_str("# TYPE xiaolin_provider_latency_ms summary\n");
        for e in self.histograms.iter() {
            let key = e.key();
            let Some(rest) = key.strip_prefix("provider_latency|") else {
                continue;
            };
            let parts: Vec<&str> = rest.splitn(2, '|').collect();
            if parts.len() != 2 {
                continue;
            }
            let vec = match e.value().lock() {
                Ok(g) => g.clone(),
                Err(poisoned) => poisoned.into_inner().clone(),
            };
            let count = vec.len() as f64;
            let sum: f64 = vec.iter().copied().sum();
            let (p50, p95, p99) = Self::percentiles(&vec);
            let _ = writeln!(
                &mut out,
                "xiaolin_provider_latency_ms{{provider=\"{}\",model=\"{}\",quantile=\"0.5\"}} {}",
                parts[0], parts[1], p50
            );
            let _ = writeln!(
                &mut out,
                "xiaolin_provider_latency_ms{{provider=\"{}\",model=\"{}\",quantile=\"0.95\"}} {}",
                parts[0], parts[1], p95
            );
            let _ = writeln!(
                &mut out,
                "xiaolin_provider_latency_ms{{provider=\"{}\",model=\"{}\",quantile=\"0.99\"}} {}",
                parts[0], parts[1], p99
            );
            let _ = writeln!(
                &mut out,
                "xiaolin_provider_latency_ms_sum{{provider=\"{}\",model=\"{}\"}} {}",
                parts[0], parts[1], sum
            );
            let _ = writeln!(
                &mut out,
                "xiaolin_provider_latency_ms_count{{provider=\"{}\",model=\"{}\"}} {}",
                parts[0], parts[1], count
            );
        }

        out
    }
}

static DEFAULT_COLLECTOR: OnceLock<MetricsCollector> = OnceLock::new();

/// Process-wide default [`MetricsCollector`] (for gateway `/api/v1/metrics`).
pub fn default_metrics_collector() -> &'static MetricsCollector {
    DEFAULT_COLLECTOR.get_or_init(MetricsCollector::new)
}

/// Prometheus text from [`default_metrics_collector`].
pub fn render_structured_metrics_prometheus() -> String {
    default_metrics_collector().render_prometheus()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_render_contains_series() {
        let c = MetricsCollector::new();
        c.record_request("agent-a", "ws");
        c.record_request("agent-a", "ws");
        c.record_latency_ms("/api/v1/chat", 12.5);
        c.record_latency_ms("/api/v1/chat", 7.5);
        c.record_error("timeout");
        c.record_tokens("gpt-4o", 128);

        let text = c.render_prometheus();
        assert!(text.contains("xiaolin_requests_total{agent=\"agent-a\",channel=\"ws\"} 2"));
        assert!(text.contains("xiaolin_endpoint_latency_ms_sum{endpoint=\"/api/v1/chat\"} 20"));
        assert!(text.contains("xiaolin_endpoint_latency_ms_count{endpoint=\"/api/v1/chat\"} 2"));
        assert!(text.contains("xiaolin_errors_total{type=\"timeout\"} 1"));
        assert!(text.contains("xiaolin_llm_tokens_total{model=\"gpt-4o\"} 128"));
    }

    #[test]
    fn provider_metrics_recorded() {
        let c = MetricsCollector::new();
        c.record_provider_request("openai", "gpt-4o");
        c.record_provider_request("openai", "gpt-4o");
        c.record_provider_tokens("openai", "gpt-4o", 256);
        c.record_provider_latency_ms("openai", "gpt-4o", 50.0);

        let text = c.render_prometheus();
        assert!(
            text.contains(
                "xiaolin_provider_requests_total{provider=\"openai\",model=\"gpt-4o\"} 2"
            ),
            "missing provider request metric:\n{text}"
        );
        assert!(
            text.contains(
                "xiaolin_provider_tokens_total{provider=\"openai\",model=\"gpt-4o\"} 256"
            ),
            "missing provider token metric:\n{text}"
        );
        assert!(
            text.contains(
                "xiaolin_provider_latency_ms_sum{provider=\"openai\",model=\"gpt-4o\"} 50"
            ),
            "missing provider latency metric:\n{text}"
        );
    }

    #[test]
    fn percentile_calculation() {
        let c = MetricsCollector::new();
        for i in 1..=100 {
            c.record_provider_latency_ms("prov", "model", i as f64);
        }
        let text = c.render_prometheus();

        let extract = |label: &str| -> f64 {
            text.lines()
                .find(|l| l.contains(label))
                .and_then(|l| l.split_whitespace().last())
                .and_then(|v| v.parse().ok())
                .unwrap_or(-1.0)
        };

        let p50 = extract("quantile=\"0.5\"");
        let p95 = extract("quantile=\"0.95\"");
        let p99 = extract("quantile=\"0.99\"");
        assert!((45.0..=55.0).contains(&p50), "p50 out of range: {p50}");
        assert!((90.0..=100.0).contains(&p95), "p95 out of range: {p95}");
        assert!((95.0..=100.0).contains(&p99), "p99 out of range: {p99}");
    }

    #[test]
    fn multiple_providers_isolated() {
        let c = MetricsCollector::new();
        c.record_provider_request("openai", "gpt-4o");
        c.record_provider_request("anthropic", "claude");
        c.record_provider_tokens("openai", "gpt-4o", 100);
        c.record_provider_tokens("anthropic", "claude", 200);

        let text = c.render_prometheus();
        assert!(text.contains("provider=\"openai\",model=\"gpt-4o\"} 1"));
        assert!(text.contains("provider=\"anthropic\",model=\"claude\"} 1"));
        assert!(text.contains("provider=\"openai\",model=\"gpt-4o\"} 100"));
        assert!(text.contains("provider=\"anthropic\",model=\"claude\"} 200"));
    }
}
