//! Per-model API cost tracking with Prometheus metrics integration.
//!
//! Records prompt_tokens, completion_tokens, cache_read_tokens, and
//! cache_creation_tokens for each LLM call. Computes per-model cumulative
//! statistics and prompt cache hit rate. Emits a warning when cache hit
//! rate drops below a configurable threshold.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use metrics::{counter, gauge};

/// Token usage from a single LLM API call, including cache-specific counts.
#[derive(Debug, Clone, Default)]
pub struct CallUsage {
    pub model: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_creation_tokens: u32,
}

/// Cumulative per-model statistics.
#[derive(Debug, Clone, Default)]
pub struct ModelStats {
    pub total_calls: u64,
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cache_creation_tokens: u64,
}

impl ModelStats {
    /// Prompt cache hit rate: cache_read_tokens / total_prompt_tokens.
    /// Returns 0.0 if no prompt tokens have been consumed.
    pub fn cache_hit_rate(&self) -> f64 {
        if self.total_prompt_tokens == 0 {
            return 0.0;
        }
        self.total_cache_read_tokens as f64 / self.total_prompt_tokens as f64
    }

    pub fn total_tokens(&self) -> u64 {
        self.total_prompt_tokens + self.total_completion_tokens
    }
}

/// Configuration for the cost tracker.
#[derive(Debug, Clone)]
pub struct CostTrackerConfig {
    /// Warn when cache hit rate drops below this threshold (0.0–1.0).
    pub cache_hit_rate_warn_threshold: f64,
    /// Minimum calls before evaluating cache hit rate (avoids noise).
    pub min_calls_for_cache_warning: u64,
}

impl Default for CostTrackerConfig {
    fn default() -> Self {
        Self {
            cache_hit_rate_warn_threshold: 0.3,
            min_calls_for_cache_warning: 5,
        }
    }
}

/// Tracks API call costs across all models in a session.
#[derive(Debug)]
pub struct CostTracker {
    config: CostTrackerConfig,
    per_model: HashMap<String, ModelStats>,
    global_calls: AtomicU64,
}

impl CostTracker {
    pub fn new(config: CostTrackerConfig) -> Self {
        Self {
            config,
            per_model: HashMap::new(),
            global_calls: AtomicU64::new(0),
        }
    }

    /// Record a completed API call's token usage.
    pub fn record(&mut self, usage: &CallUsage) {
        self.global_calls.fetch_add(1, Ordering::Relaxed);

        let stats = self.per_model.entry(usage.model.clone()).or_default();
        stats.total_calls += 1;
        stats.total_prompt_tokens += usage.prompt_tokens as u64;
        stats.total_completion_tokens += usage.completion_tokens as u64;
        stats.total_cache_read_tokens += usage.cache_read_tokens as u64;
        stats.total_cache_creation_tokens += usage.cache_creation_tokens as u64;

        let rate = stats.cache_hit_rate();
        let total_calls = stats.total_calls;

        self.emit_metrics(usage);
        self.check_cache_hit_rate(&usage.model, rate, total_calls);
    }

    /// Get stats for a specific model.
    pub fn model_stats(&self, model: &str) -> Option<&ModelStats> {
        self.per_model.get(model)
    }

    /// Get all per-model stats.
    pub fn all_stats(&self) -> &HashMap<String, ModelStats> {
        &self.per_model
    }

    /// Global total calls.
    pub fn total_calls(&self) -> u64 {
        self.global_calls.load(Ordering::Relaxed)
    }

    /// Aggregate cache hit rate across all models.
    pub fn global_cache_hit_rate(&self) -> f64 {
        let total_prompt: u64 = self.per_model.values().map(|s| s.total_prompt_tokens).sum();
        let total_cache: u64 = self.per_model.values().map(|s| s.total_cache_read_tokens).sum();
        if total_prompt == 0 {
            return 0.0;
        }
        total_cache as f64 / total_prompt as f64
    }

    fn emit_metrics(&self, usage: &CallUsage) {
        let model_label = sanitize_model_label(&usage.model);

        counter!("fastclaw_llm_prompt_tokens_total", "model" => model_label.clone())
            .increment(usage.prompt_tokens as u64);
        counter!("fastclaw_llm_completion_tokens_total", "model" => model_label.clone())
            .increment(usage.completion_tokens as u64);
        counter!("fastclaw_llm_cache_read_tokens_total", "model" => model_label.clone())
            .increment(usage.cache_read_tokens as u64);
        counter!("fastclaw_llm_cache_creation_tokens_total", "model" => model_label.clone())
            .increment(usage.cache_creation_tokens as u64);
        counter!("fastclaw_llm_calls_total", "model" => model_label.clone())
            .increment(1);
    }

    fn check_cache_hit_rate(&self, model: &str, rate: f64, total_calls: u64) {
        if total_calls < self.config.min_calls_for_cache_warning {
            return;
        }

        let model_label = sanitize_model_label(model);

        gauge!("fastclaw_llm_cache_hit_rate", "model" => model_label)
            .set(rate);

        if rate < self.config.cache_hit_rate_warn_threshold {
            tracing::warn!(
                model = %model,
                cache_hit_rate = format!("{:.1}%", rate * 100.0),
                threshold = format!("{:.1}%", self.config.cache_hit_rate_warn_threshold * 100.0),
                total_calls = total_calls,
                "prompt cache hit rate below threshold"
            );
        }
    }
}

impl Default for CostTracker {
    fn default() -> Self {
        Self::new(CostTrackerConfig::default())
    }
}

fn sanitize_model_label(model: &str) -> String {
    model
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == '.' || *c == '/')
        .take(64)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(model: &str, prompt: u32, completion: u32, cache_read: u32) -> CallUsage {
        CallUsage {
            model: model.to_string(),
            prompt_tokens: prompt,
            completion_tokens: completion,
            cache_read_tokens: cache_read,
            cache_creation_tokens: 0,
        }
    }

    #[test]
    fn records_per_model_stats() {
        let mut tracker = CostTracker::default();
        tracker.record(&usage("claude-3", 1000, 200, 800));
        tracker.record(&usage("claude-3", 1000, 300, 700));
        tracker.record(&usage("gpt-4", 500, 100, 0));

        let claude = tracker.model_stats("claude-3").unwrap();
        assert_eq!(claude.total_calls, 2);
        assert_eq!(claude.total_prompt_tokens, 2000);
        assert_eq!(claude.total_completion_tokens, 500);
        assert_eq!(claude.total_cache_read_tokens, 1500);

        let gpt = tracker.model_stats("gpt-4").unwrap();
        assert_eq!(gpt.total_calls, 1);
        assert_eq!(gpt.total_prompt_tokens, 500);
    }

    #[test]
    fn cache_hit_rate_calculation() {
        let mut tracker = CostTracker::default();
        tracker.record(&usage("claude-3", 1000, 200, 800));
        tracker.record(&usage("claude-3", 1000, 200, 600));

        let stats = tracker.model_stats("claude-3").unwrap();
        let rate = stats.cache_hit_rate();
        // (800 + 600) / (1000 + 1000) = 1400 / 2000 = 0.7
        assert!((rate - 0.7).abs() < 0.001);
    }

    #[test]
    fn cache_hit_rate_zero_when_no_cache() {
        let mut tracker = CostTracker::default();
        tracker.record(&usage("gpt-4", 1000, 200, 0));

        let stats = tracker.model_stats("gpt-4").unwrap();
        assert_eq!(stats.cache_hit_rate(), 0.0);
    }

    #[test]
    fn global_cache_hit_rate_aggregates() {
        let mut tracker = CostTracker::default();
        tracker.record(&usage("claude-3", 1000, 200, 800));
        tracker.record(&usage("gpt-4", 1000, 200, 0));

        // 800 / 2000 = 0.4
        let rate = tracker.global_cache_hit_rate();
        assert!((rate - 0.4).abs() < 0.001);
    }

    #[test]
    fn total_calls_tracks_globally() {
        let mut tracker = CostTracker::default();
        tracker.record(&usage("a", 100, 50, 0));
        tracker.record(&usage("b", 200, 100, 0));
        tracker.record(&usage("a", 100, 50, 0));

        assert_eq!(tracker.total_calls(), 3);
    }

    #[test]
    fn model_stats_total_tokens() {
        let mut tracker = CostTracker::default();
        tracker.record(&usage("claude-3", 1000, 200, 0));

        let stats = tracker.model_stats("claude-3").unwrap();
        assert_eq!(stats.total_tokens(), 1200);
    }

    #[test]
    fn cache_creation_tokens_tracked() {
        let mut tracker = CostTracker::default();
        tracker.record(&CallUsage {
            model: "claude-3".into(),
            prompt_tokens: 1000,
            completion_tokens: 200,
            cache_read_tokens: 0,
            cache_creation_tokens: 500,
        });

        let stats = tracker.model_stats("claude-3").unwrap();
        assert_eq!(stats.total_cache_creation_tokens, 500);
    }

    #[test]
    fn empty_tracker_returns_none_for_unknown_model() {
        let tracker = CostTracker::default();
        assert!(tracker.model_stats("nonexistent").is_none());
    }
}
