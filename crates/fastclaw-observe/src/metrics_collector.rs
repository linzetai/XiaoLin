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

    /// Renders all recorded metrics in Prometheus text exposition format (0.0.4).
    pub fn render_prometheus(&self) -> String {
        let mut out = String::new();

        out.push_str("# HELP fastclaw_requests_total Requests by agent and channel\n");
        out.push_str("# TYPE fastclaw_requests_total counter\n");
        for e in self.counters.iter() {
            let key = e.key();
            if let Some(rest) = key.strip_prefix("request|") {
                let parts: Vec<&str> = rest.splitn(2, '|').collect();
                if parts.len() == 2 {
                    let _ = writeln!(
                        &mut out,
                        "fastclaw_requests_total{{agent=\"{}\",channel=\"{}\"}} {}",
                        parts[0],
                        parts[1],
                        e.value().load(Ordering::Relaxed)
                    );
                }
            }
        }

        out.push_str("# HELP fastclaw_endpoint_latency_ms Latency samples by endpoint (summary)\n");
        out.push_str("# TYPE fastclaw_endpoint_latency_ms summary\n");
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
            let _ = writeln!(
                &mut out,
                "fastclaw_endpoint_latency_ms_sum{{endpoint=\"{}\"}} {}",
                endpoint, sum
            );
            let _ = writeln!(
                &mut out,
                "fastclaw_endpoint_latency_ms_count{{endpoint=\"{}\"}} {}",
                endpoint, count
            );
        }

        out.push_str("# HELP fastclaw_errors_total Errors by coarse type\n");
        out.push_str("# TYPE fastclaw_errors_total counter\n");
        for e in self.counters.iter() {
            let key = e.key();
            if let Some(typ) = key.strip_prefix("error|") {
                let _ = writeln!(
                    &mut out,
                    "fastclaw_errors_total{{type=\"{}\"}} {}",
                    typ,
                    e.value().load(Ordering::Relaxed)
                );
            }
        }

        out.push_str("# HELP fastclaw_llm_tokens_total LLM token usage by model\n");
        out.push_str("# TYPE fastclaw_llm_tokens_total counter\n");
        for e in self.counters.iter() {
            let key = e.key();
            if let Some(model) = key.strip_prefix("tokens|") {
                let _ = writeln!(
                    &mut out,
                    "fastclaw_llm_tokens_total{{model=\"{}\"}} {}",
                    model,
                    e.value().load(Ordering::Relaxed)
                );
            }
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
        assert!(text.contains("fastclaw_requests_total{agent=\"agent-a\",channel=\"ws\"} 2"));
        assert!(text.contains("fastclaw_endpoint_latency_ms_sum{endpoint=\"/api/v1/chat\"} 20"));
        assert!(text.contains("fastclaw_endpoint_latency_ms_count{endpoint=\"/api/v1/chat\"} 2"));
        assert!(text.contains("fastclaw_errors_total{type=\"timeout\"} 1"));
        assert!(text.contains("fastclaw_llm_tokens_total{model=\"gpt-4o\"} 128"));
    }
}
