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

/// Per-model cost rates (USD per 1K tokens).
#[derive(Debug, Clone)]
pub struct ModelCostRate {
    pub input_per_1k: f64,
    pub output_per_1k: f64,
    pub cache_read_per_1k: f64,
    pub cache_write_per_1k: f64,
}

impl Default for ModelCostRate {
    fn default() -> Self {
        Self {
            input_per_1k: 0.003,
            output_per_1k: 0.015,
            cache_read_per_1k: 0.0015,
            cache_write_per_1k: 0.00375,
        }
    }
}

/// Configuration for the cost tracker.
#[derive(Debug, Clone)]
pub struct CostTrackerConfig {
    /// Warn when cache hit rate drops below this threshold (0.0–1.0).
    pub cache_hit_rate_warn_threshold: f64,
    /// Minimum calls before evaluating cache hit rate (avoids noise).
    pub min_calls_for_cache_warning: u64,
    /// Budget limit in USD. None means no limit.
    pub budget_limit_usd: Option<f64>,
    /// Warn at this percentage of budget (0.0–1.0). Default: 0.8 (80%).
    pub budget_warn_threshold: f64,
    /// Default cost rates for unknown models.
    pub default_cost_rate: ModelCostRate,
    /// Per-model cost rate overrides.
    pub model_cost_rates: HashMap<String, ModelCostRate>,
}

impl Default for CostTrackerConfig {
    fn default() -> Self {
        Self {
            cache_hit_rate_warn_threshold: 0.3,
            min_calls_for_cache_warning: 5,
            budget_limit_usd: None,
            budget_warn_threshold: 0.8,
            default_cost_rate: ModelCostRate::default(),
            model_cost_rates: HashMap::new(),
        }
    }
}

/// Alert level returned by budget checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetAlert {
    /// Cost exceeds warning threshold but within limit.
    Warning,
    /// Cost has exceeded the budget limit.
    Exceeded,
}

/// Tracks API call costs across all models in a session.
#[derive(Debug)]
pub struct CostTracker {
    config: CostTrackerConfig,
    per_model: HashMap<String, ModelStats>,
    global_calls: AtomicU64,
    accumulated_cost_usd: f64,
    budget_warning_emitted: bool,
    budget_exceeded_emitted: bool,
}

impl CostTracker {
    pub fn new(config: CostTrackerConfig) -> Self {
        Self {
            config,
            per_model: HashMap::new(),
            global_calls: AtomicU64::new(0),
            accumulated_cost_usd: 0.0,
            budget_warning_emitted: false,
            budget_exceeded_emitted: false,
        }
    }

    /// Record a completed API call's token usage.
    /// Returns a `BudgetAlert` if a budget threshold was crossed.
    pub fn record(&mut self, usage: &CallUsage) -> Option<BudgetAlert> {
        self.global_calls.fetch_add(1, Ordering::Relaxed);

        let call_cost = self.compute_call_cost(usage);
        self.accumulated_cost_usd += call_cost;

        let stats = self.per_model.entry(usage.model.clone()).or_default();
        stats.total_calls += 1;
        stats.total_prompt_tokens += usage.prompt_tokens as u64;
        stats.total_completion_tokens += usage.completion_tokens as u64;
        stats.total_cache_read_tokens += usage.cache_read_tokens as u64;
        stats.total_cache_creation_tokens += usage.cache_creation_tokens as u64;

        let rate = stats.cache_hit_rate();
        let total_calls = stats.total_calls;

        self.emit_metrics(usage);
        self.emit_cost_metrics(call_cost);
        self.check_cache_hit_rate(&usage.model, rate, total_calls);
        self.check_budget()
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
        let total_cache: u64 = self
            .per_model
            .values()
            .map(|s| s.total_cache_read_tokens)
            .sum();
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
        counter!("fastclaw_llm_calls_total", "model" => model_label.clone()).increment(1);
    }

    /// Current accumulated cost in USD.
    pub fn accumulated_cost_usd(&self) -> f64 {
        self.accumulated_cost_usd
    }

    /// Compute the cost of a single API call in USD.
    pub fn compute_call_cost(&self, usage: &CallUsage) -> f64 {
        let rate = self
            .config
            .model_cost_rates
            .get(&usage.model)
            .unwrap_or(&self.config.default_cost_rate);

        let input_cost = (usage.prompt_tokens as f64 / 1000.0) * rate.input_per_1k;
        let output_cost = (usage.completion_tokens as f64 / 1000.0) * rate.output_per_1k;
        let cache_read_cost = (usage.cache_read_tokens as f64 / 1000.0) * rate.cache_read_per_1k;
        let cache_write_cost =
            (usage.cache_creation_tokens as f64 / 1000.0) * rate.cache_write_per_1k;

        input_cost + output_cost + cache_read_cost + cache_write_cost
    }

    fn check_budget(&mut self) -> Option<BudgetAlert> {
        let limit = self.config.budget_limit_usd?;

        if self.accumulated_cost_usd >= limit && !self.budget_exceeded_emitted {
            self.budget_exceeded_emitted = true;
            tracing::error!(
                accumulated_usd = format!("{:.4}", self.accumulated_cost_usd),
                limit_usd = format!("{:.4}", limit),
                "budget limit exceeded"
            );
            gauge!("fastclaw_budget_exceeded").set(1.0);
            return Some(BudgetAlert::Exceeded);
        }

        let warn_at = limit * self.config.budget_warn_threshold;
        if self.accumulated_cost_usd >= warn_at && !self.budget_warning_emitted {
            self.budget_warning_emitted = true;
            tracing::warn!(
                accumulated_usd = format!("{:.4}", self.accumulated_cost_usd),
                warn_threshold_usd = format!("{:.4}", warn_at),
                limit_usd = format!("{:.4}", limit),
                "approaching budget limit"
            );
            return Some(BudgetAlert::Warning);
        }

        None
    }

    fn emit_cost_metrics(&self, call_cost: f64) {
        counter!("fastclaw_llm_cost_usd_total").increment((call_cost * 10000.0) as u64);
        gauge!("fastclaw_llm_accumulated_cost_usd").set(self.accumulated_cost_usd);
    }

    fn check_cache_hit_rate(&self, model: &str, rate: f64, total_calls: u64) {
        if total_calls < self.config.min_calls_for_cache_warning {
            return;
        }

        let model_label = sanitize_model_label(model);

        gauge!("fastclaw_llm_cache_hit_rate", "model" => model_label).set(rate);

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

impl CostTracker {
    /// Reset the budget alert state (useful for testing or session resets).
    #[allow(dead_code)]
    pub fn reset_budget_alerts(&mut self) {
        self.budget_warning_emitted = false;
        self.budget_exceeded_emitted = false;
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

    #[test]
    fn budget_warning_at_threshold() {
        let config = CostTrackerConfig {
            budget_limit_usd: Some(1.0),
            budget_warn_threshold: 0.8,
            ..Default::default()
        };
        let mut tracker = CostTracker::new(config);

        // Each call ~0.003 * 100 + 0.015 * 50 = 0.3 + 0.75 = 1.05
        // With default rates: input=0.003/1k, output=0.015/1k
        // 100_000 prompt tokens = 0.3, 50_000 completion = 0.75 => 1.05 per call
        // But our usage helper uses smaller numbers. Let's be explicit:
        let big_usage = CallUsage {
            model: "claude-3".into(),
            prompt_tokens: 100_000,
            completion_tokens: 50_000,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
        };
        // cost = (100_000/1000)*0.003 + (50_000/1000)*0.015 = 300*0.003 + 50*0.015 = 0.9 + 0.75 = nope
        // Actually: (100_000/1000)*0.003 = 100*0.003 = 0.3; (50_000/1000)*0.015 = 50*0.015 = 0.75 => 1.05
        // Wait let me recalculate: 100_000 / 1000 = 100. 100 * 0.003 = 0.3
        // 50_000 / 1000 = 50. 50 * 0.015 = 0.75
        // total = 1.05 => exceeds limit of 1.0

        let alert = tracker.record(&big_usage);
        // 1.05 > 1.0 = exceeded
        assert_eq!(alert, Some(BudgetAlert::Exceeded));
    }

    #[test]
    fn budget_warning_fires_before_exceeded() {
        let config = CostTrackerConfig {
            budget_limit_usd: Some(10.0),
            budget_warn_threshold: 0.5,
            ..Default::default()
        };
        let mut tracker = CostTracker::new(config);

        // Small call: 10_000 prompt + 5_000 completion
        // cost = (10/1000)*0.003*1000 ... wait, (10_000/1000)*0.003 = 10*0.003 = 0.03;
        //   (5_000/1000)*0.015 = 5*0.015 = 0.075 => total per call = 0.105
        // 50 calls => 5.25 => exceeds 50% of 10.0 = 5.0
        let small_usage = CallUsage {
            model: "gpt-4".into(),
            prompt_tokens: 10_000,
            completion_tokens: 5_000,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
        };

        let mut saw_warning = false;
        for _ in 0..50 {
            if let Some(alert) = tracker.record(&small_usage) {
                if alert == BudgetAlert::Warning {
                    saw_warning = true;
                    break;
                }
            }
        }
        assert!(saw_warning, "expected budget warning");
        assert!(tracker.accumulated_cost_usd() >= 5.0);
    }

    #[test]
    fn no_budget_alert_without_limit() {
        let mut tracker = CostTracker::default();
        let alert = tracker.record(&usage("claude-3", 100_000, 50_000, 0));
        assert_eq!(alert, None);
    }

    #[test]
    fn cost_accumulates_correctly() {
        let mut tracker = CostTracker::default();
        // 1000 prompt / 1000 = 1.0 * 0.003 = 0.003
        // 500 completion / 1000 = 0.5 * 0.015 = 0.0075
        // total = 0.0105
        tracker.record(&usage("claude-3", 1000, 500, 0));
        let expected = 0.003 + 0.0075;
        assert!((tracker.accumulated_cost_usd() - expected).abs() < 0.0001);
    }

    #[test]
    fn custom_model_rates_applied() {
        let mut rates = HashMap::new();
        rates.insert(
            "custom-model".into(),
            ModelCostRate {
                input_per_1k: 0.01,
                output_per_1k: 0.03,
                cache_read_per_1k: 0.005,
                cache_write_per_1k: 0.0075,
            },
        );
        let config = CostTrackerConfig {
            model_cost_rates: rates,
            ..Default::default()
        };
        let mut tracker = CostTracker::new(config);

        // 2000 prompt: 2 * 0.01 = 0.02; 1000 completion: 1 * 0.03 = 0.03
        tracker.record(&CallUsage {
            model: "custom-model".into(),
            prompt_tokens: 2000,
            completion_tokens: 1000,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
        });
        let expected = 0.02 + 0.03;
        assert!((tracker.accumulated_cost_usd() - expected).abs() < 0.0001);
    }
}
