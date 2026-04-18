mod metrics_collector;

pub use metrics_collector::{
    default_metrics_collector, render_structured_metrics_prometheus, MetricsCollector,
};

use metrics::{counter, gauge, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::sync::OnceLock;
use std::time::Instant;

static PROM_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Initializes global tracing + Prometheus metrics recorder.
/// Returns a handle that can render `/metrics` output; `None` if already initialized.
///
/// `log_level` overrides the default env filter if set (e.g. "info", "debug").
pub fn init_observability(log_format: &str) -> Option<PrometheusHandle> {
    init_observability_with_level(log_format, None)
}

/// Same as `init_observability` but accepts an optional log level override.
pub fn init_observability_with_level(
    log_format: &str,
    log_level: Option<&str>,
) -> Option<PrometheusHandle> {
    let filter = if let Some(level) = log_level {
        tracing_subscriber::EnvFilter::try_new(level)
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::from_default_env())
    } else {
        tracing_subscriber::EnvFilter::from_default_env()
    };

    let _ = match log_format.to_ascii_lowercase().as_str() {
        "json" => tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .try_init(),
        _ => tracing_subscriber::fmt().with_env_filter(filter).try_init(),
    };

    let handle = PrometheusBuilder::new().install_recorder().ok()?;

    PROM_HANDLE.set(handle.clone()).ok();
    Some(handle)
}

/// Legacy alias.
pub fn init_tracing(format: &str) {
    init_observability(format);
}

/// Retrieve the global Prometheus handle (after `init_observability`).
pub fn prometheus_handle() -> Option<&'static PrometheusHandle> {
    PROM_HANDLE.get()
}

/// Render Prometheus text exposition format.
pub fn render_metrics() -> String {
    PROM_HANDLE.get().map(|h| h.render()).unwrap_or_default()
}

// ── Pre-defined metric helpers ──────────────────────────────────────

fn sanitize_label(s: &str, max_len: usize) -> String {
    let truncated: String = s
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
        .take(max_len)
        .collect();
    if truncated.is_empty() {
        "_unknown_".to_string()
    } else {
        truncated
    }
}

pub fn record_chat_request(agent_id: &str, streaming: bool) {
    let mode = if streaming { "stream" } else { "sync" };
    counter!("fastclaw_chat_requests_total", "agent" => sanitize_label(agent_id, 64), "mode" => mode)
        .increment(1);
}

pub fn record_chat_latency(agent_id: &str, start: Instant) {
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    histogram!("fastclaw_chat_latency_ms", "agent" => sanitize_label(agent_id, 64)).record(ms);
}

pub fn record_tool_call(tool_name: &str, success: bool) {
    let ok = if success { "true" } else { "false" };
    counter!("fastclaw_tool_calls_total", "tool" => sanitize_label(tool_name, 64), "success" => ok)
        .increment(1);
}

pub fn record_ws_connection(delta: i64) {
    if delta > 0 {
        gauge!("fastclaw_ws_connections").increment(delta as f64);
    } else {
        gauge!("fastclaw_ws_connections").decrement((-delta) as f64);
    }
}

pub fn record_session_count(count: u64) {
    gauge!("fastclaw_sessions_active").set(count as f64);
}

pub fn record_agent_reload(count: usize) {
    counter!("fastclaw_agent_reloads_total").increment(1);
    gauge!("fastclaw_agents_loaded").set(count as f64);
}

pub fn record_memory_operation(kind: &str) {
    counter!("fastclaw_memory_ops_total", "kind" => kind.to_string()).increment(1);
}

pub fn record_plugin_invocation(plugin_id: &str, success: bool) {
    let ok = if success { "true" } else { "false" };
    counter!("fastclaw_plugin_invocations_total", "plugin" => sanitize_label(plugin_id, 64), "success" => ok).increment(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_returns_string_before_init() {
        let out = render_metrics();
        assert!(out.is_empty() || out.contains("fastclaw"));
    }
}
