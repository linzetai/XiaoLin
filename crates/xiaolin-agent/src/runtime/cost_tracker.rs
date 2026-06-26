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
}

#[cfg(test)]
impl ModelStats {
    pub fn total_tokens(&self) -> u64 {
        self.total_prompt_tokens + self.total_completion_tokens
    }
}

/// Per-model cost rates (USD per 1M tokens).
#[derive(Debug, Clone)]
pub struct ModelCostRate {
    pub input_per_1m: f64,
    pub output_per_1m: f64,
    pub cache_read_per_1m: f64,
    pub cache_write_per_1m: f64,
}

impl Default for ModelCostRate {
    fn default() -> Self {
        Self {
            input_per_1m: 0.14,
            output_per_1m: 0.28,
            cache_read_per_1m: 0.0028,
            cache_write_per_1m: 0.14,
        }
    }
}

impl ModelCostRate {
    pub fn builtin_rates() -> HashMap<String, ModelCostRate> {
        let mut m = HashMap::new();
        let add = |m: &mut HashMap<String, ModelCostRate>, names: &[&str], rate: ModelCostRate| {
            for name in names {
                m.insert((*name).to_string(), rate.clone());
            }
        };

        // DeepSeek V4 — cache hit reduced to 2% of input on 2026-04-26
        add(
            &mut m,
            &[
                "deepseek-v4-flash",
                "deepseek/deepseek-v4-flash",
                "deepseek-chat",
                "deepseek-reasoner",
            ],
            ModelCostRate {
                input_per_1m: 0.14,
                output_per_1m: 0.28,
                cache_read_per_1m: 0.0028,
                cache_write_per_1m: 0.14,
            },
        );
        add(
            &mut m,
            &["deepseek-v4-pro", "deepseek/deepseek-v4-pro"],
            ModelCostRate {
                input_per_1m: 0.435,
                output_per_1m: 0.87,
                cache_read_per_1m: 0.003625,
                cache_write_per_1m: 0.435,
            },
        );
        // Anthropic Claude — cache read = 10% of input, 5-min write = 125% of input
        add(
            &mut m,
            &[
                "claude-sonnet-4-6",
                "claude-sonnet-4-6-20250514",
                "claude-sonnet-4-5",
                "claude-sonnet-4-20250514",
                "anthropic/claude-sonnet-4-6",
            ],
            ModelCostRate {
                input_per_1m: 3.0,
                output_per_1m: 15.0,
                cache_read_per_1m: 0.30,
                cache_write_per_1m: 3.75,
            },
        );
        add(
            &mut m,
            &[
                "claude-opus-4-7",
                "claude-opus-4-8",
                "claude-opus-4-6",
                "anthropic/claude-opus-4-7",
            ],
            ModelCostRate {
                input_per_1m: 5.0,
                output_per_1m: 25.0,
                cache_read_per_1m: 0.50,
                cache_write_per_1m: 6.25,
            },
        );
        add(
            &mut m,
            &[
                "claude-haiku-4-5",
                "claude-haiku-4-5-20251001",
                "claude-3-5-haiku-20241022",
            ],
            ModelCostRate {
                input_per_1m: 1.0,
                output_per_1m: 5.0,
                cache_read_per_1m: 0.10,
                cache_write_per_1m: 1.25,
            },
        );
        // OpenAI — cache read = 50% of input (GPT-4o/mini), 75-90% off (newer models)
        add(
            &mut m,
            &["gpt-4o", "openai/gpt-4o"],
            ModelCostRate {
                input_per_1m: 2.50,
                output_per_1m: 10.0,
                cache_read_per_1m: 1.25,
                cache_write_per_1m: 2.50,
            },
        );
        add(
            &mut m,
            &["gpt-4o-mini", "openai/gpt-4o-mini"],
            ModelCostRate {
                input_per_1m: 0.15,
                output_per_1m: 0.60,
                cache_read_per_1m: 0.075,
                cache_write_per_1m: 0.15,
            },
        );
        add(
            &mut m,
            &["gpt-4.1", "openai/gpt-4.1"],
            ModelCostRate {
                input_per_1m: 2.0,
                output_per_1m: 8.0,
                cache_read_per_1m: 0.50,
                cache_write_per_1m: 2.0,
            },
        );
        add(
            &mut m,
            &["gpt-4.1-mini", "openai/gpt-4.1-mini"],
            ModelCostRate {
                input_per_1m: 0.40,
                output_per_1m: 1.60,
                cache_read_per_1m: 0.10,
                cache_write_per_1m: 0.40,
            },
        );
        add(
            &mut m,
            &["gpt-4.1-nano", "openai/gpt-4.1-nano"],
            ModelCostRate {
                input_per_1m: 0.10,
                output_per_1m: 0.40,
                cache_read_per_1m: 0.025,
                cache_write_per_1m: 0.10,
            },
        );
        add(
            &mut m,
            &["gpt-5", "openai/gpt-5"],
            ModelCostRate {
                input_per_1m: 1.25,
                output_per_1m: 10.0,
                cache_read_per_1m: 0.125,
                cache_write_per_1m: 1.25,
            },
        );
        add(
            &mut m,
            &["gpt-5-mini", "openai/gpt-5-mini"],
            ModelCostRate {
                input_per_1m: 0.25,
                output_per_1m: 2.0,
                cache_read_per_1m: 0.025,
                cache_write_per_1m: 0.25,
            },
        );
        add(
            &mut m,
            &["gpt-5.5", "openai/gpt-5.5"],
            ModelCostRate {
                input_per_1m: 5.0,
                output_per_1m: 30.0,
                cache_read_per_1m: 0.50,
                cache_write_per_1m: 5.0,
            },
        );
        add(
            &mut m,
            &["gpt-5.4", "openai/gpt-5.4"],
            ModelCostRate {
                input_per_1m: 2.50,
                output_per_1m: 15.0,
                cache_read_per_1m: 0.25,
                cache_write_per_1m: 2.50,
            },
        );
        // Alibaba Qwen — cache hit = 10% of input (prices in USD converted from CNY at ~7.2)
        add(
            &mut m,
            &[
                "qwen3-max",
                "qwen3.7-max",
                "ali/qwen3-max",
                "alibaba/qwen3-max",
                "ali/qwen3.7-max",
            ],
            ModelCostRate {
                input_per_1m: 0.35,
                output_per_1m: 1.39,
                cache_read_per_1m: 0.035,
                cache_write_per_1m: 0.44,
            },
        );
        add(
            &mut m,
            &["qwen3.5-plus", "ali/qwen3.5-plus", "alibaba/qwen3.5-plus"],
            ModelCostRate {
                input_per_1m: 0.11,
                output_per_1m: 0.67,
                cache_read_per_1m: 0.011,
                cache_write_per_1m: 0.14,
            },
        );
        // Zhipu GLM — cache read ~20% of input, storage free (limited time)
        add(
            &mut m,
            &["glm-5.1", "zhipu/glm-5.1"],
            ModelCostRate {
                input_per_1m: 1.40,
                output_per_1m: 4.40,
                cache_read_per_1m: 0.26,
                cache_write_per_1m: 1.40,
            },
        );
        add(
            &mut m,
            &["glm-5", "zhipu/glm-5"],
            ModelCostRate {
                input_per_1m: 1.00,
                output_per_1m: 3.20,
                cache_read_per_1m: 0.20,
                cache_write_per_1m: 1.00,
            },
        );
        add(
            &mut m,
            &["glm-4.7", "zhipu/glm-4.7"],
            ModelCostRate {
                input_per_1m: 0.60,
                output_per_1m: 2.20,
                cache_read_per_1m: 0.11,
                cache_write_per_1m: 0.60,
            },
        );
        add(
            &mut m,
            &["glm-4.7-flash", "zhipu/glm-4.7-flash"],
            ModelCostRate {
                input_per_1m: 0.0,
                output_per_1m: 0.0,
                cache_read_per_1m: 0.0,
                cache_write_per_1m: 0.0,
            },
        );
        // ByteDance Doubao/Seed — cache hit = 20% of input (via Volcano Ark, CNY/7.1)
        add(
            &mut m,
            &["doubao-seed-2.0-pro", "bytedance/doubao-seed-2.0-pro"],
            ModelCostRate {
                input_per_1m: 0.45,
                output_per_1m: 2.25,
                cache_read_per_1m: 0.09,
                cache_write_per_1m: 0.45,
            },
        );
        add(
            &mut m,
            &["doubao-seed-2.0-lite", "bytedance/doubao-seed-2.0-lite"],
            ModelCostRate {
                input_per_1m: 0.085,
                output_per_1m: 0.51,
                cache_read_per_1m: 0.017,
                cache_write_per_1m: 0.085,
            },
        );
        add(
            &mut m,
            &["doubao-seed-1.6", "bytedance/doubao-seed-1.6"],
            ModelCostRate {
                input_per_1m: 0.113,
                output_per_1m: 1.13,
                cache_read_per_1m: 0.023,
                cache_write_per_1m: 0.113,
            },
        );
        add(
            &mut m,
            &["doubao-seed-1.6-flash", "bytedance/doubao-seed-1.6-flash"],
            ModelCostRate {
                input_per_1m: 0.022,
                output_per_1m: 0.22,
                cache_read_per_1m: 0.004,
                cache_write_per_1m: 0.022,
            },
        );
        // Moonshot Kimi
        add(
            &mut m,
            &["kimi-k2", "moonshot/kimi-k2"],
            ModelCostRate {
                input_per_1m: 0.60,
                output_per_1m: 2.40,
                cache_read_per_1m: 0.12,
                cache_write_per_1m: 0.60,
            },
        );
        // Google Gemini
        add(
            &mut m,
            &["gemini-2.5-pro", "google/gemini-2.5-pro"],
            ModelCostRate {
                input_per_1m: 1.25,
                output_per_1m: 10.0,
                cache_read_per_1m: 0.315,
                cache_write_per_1m: 1.25,
            },
        );
        add(
            &mut m,
            &["gemini-2.5-flash", "google/gemini-2.5-flash"],
            ModelCostRate {
                input_per_1m: 0.15,
                output_per_1m: 0.60,
                cache_read_per_1m: 0.03,
                cache_write_per_1m: 0.15,
            },
        );

        m
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

    fn emit_metrics(&self, usage: &CallUsage) {
        let model_label = sanitize_model_label(&usage.model);

        counter!("xiaolin_llm_prompt_tokens_total", "model" => model_label.clone())
            .increment(usage.prompt_tokens as u64);
        counter!("xiaolin_llm_completion_tokens_total", "model" => model_label.clone())
            .increment(usage.completion_tokens as u64);
        counter!("xiaolin_llm_cache_read_tokens_total", "model" => model_label.clone())
            .increment(usage.cache_read_tokens as u64);
        counter!("xiaolin_llm_cache_creation_tokens_total", "model" => model_label.clone())
            .increment(usage.cache_creation_tokens as u64);
        counter!("xiaolin_llm_calls_total", "model" => model_label.clone()).increment(1);
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

        // Cache hit tokens are charged at the lower cache_read rate, not the full input rate.
        // DeepSeek: prompt_tokens = cache_hit + cache_miss; only cache_miss pays full price.
        // Anthropic: prompt_tokens excludes cache_read (counted separately).
        let non_cached_input = usage.prompt_tokens.saturating_sub(usage.cache_read_tokens);
        let input_cost = (non_cached_input as f64 / 1_000_000.0) * rate.input_per_1m;
        let output_cost = (usage.completion_tokens as f64 / 1_000_000.0) * rate.output_per_1m;
        let cache_read_cost =
            (usage.cache_read_tokens as f64 / 1_000_000.0) * rate.cache_read_per_1m;
        let cache_write_cost =
            (usage.cache_creation_tokens as f64 / 1_000_000.0) * rate.cache_write_per_1m;

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
            gauge!("xiaolin_budget_exceeded").set(1.0);
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
        counter!("xiaolin_llm_cost_usd_total").increment((call_cost * 10000.0) as u64);
        gauge!("xiaolin_llm_accumulated_cost_usd").set(self.accumulated_cost_usd);
    }

    fn check_cache_hit_rate(&self, model: &str, rate: f64, total_calls: u64) {
        if total_calls < self.config.min_calls_for_cache_warning {
            return;
        }

        let model_label = sanitize_model_label(model);

        gauge!("xiaolin_llm_cache_hit_rate", "model" => model_label).set(rate);

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
impl CostTracker {
    pub fn model_stats(&self, model: &str) -> Option<&ModelStats> {
        self.per_model.get(model)
    }

    pub fn all_stats(&self) -> &HashMap<String, ModelStats> {
        &self.per_model
    }

    pub fn total_calls(&self) -> u64 {
        self.global_calls.load(Ordering::Relaxed)
    }

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
            budget_limit_usd: Some(0.01),
            budget_warn_threshold: 0.8,
            ..Default::default()
        };
        let mut tracker = CostTracker::new(config);

        // Default rate: input=$0.14/1M, output=$0.28/1M
        // 100_000 prompt: (100_000/1_000_000)*0.14 = 0.014
        // 50_000 completion: (50_000/1_000_000)*0.28 = 0.014
        // total = 0.028 => exceeds limit of 0.01
        let big_usage = CallUsage {
            model: "unknown-model".into(),
            prompt_tokens: 100_000,
            completion_tokens: 50_000,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
        };

        let alert = tracker.record(&big_usage);
        assert_eq!(alert, Some(BudgetAlert::Exceeded));
    }

    #[test]
    fn budget_warning_fires_before_exceeded() {
        let config = CostTrackerConfig {
            budget_limit_usd: Some(1.0),
            budget_warn_threshold: 0.5,
            ..Default::default()
        };
        let mut tracker = CostTracker::new(config);

        // Default rate: input=$0.14/1M, output=$0.28/1M
        // 100_000 prompt: 0.014, 50_000 completion: 0.014 => 0.028/call
        // Need >= 0.5 (50% of 1.0) => ~18 calls
        let small_usage = CallUsage {
            model: "unknown-model".into(),
            prompt_tokens: 100_000,
            completion_tokens: 50_000,
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
        assert!(tracker.accumulated_cost_usd() >= 0.5);
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
        // Default rate: input=$0.14/1M, output=$0.28/1M
        // 1000 prompt: (1000/1_000_000)*0.14 = 0.00014
        // 500 completion: (500/1_000_000)*0.28 = 0.00014
        // total = 0.00028
        tracker.record(&usage("unknown-model", 1000, 500, 0));
        let expected = 0.00014 + 0.00014;
        assert!((tracker.accumulated_cost_usd() - expected).abs() < 0.00001);
    }

    #[test]
    fn custom_model_rates_applied() {
        let mut rates = HashMap::new();
        rates.insert(
            "custom-model".into(),
            ModelCostRate {
                input_per_1m: 10.0,
                output_per_1m: 30.0,
                cache_read_per_1m: 5.0,
                cache_write_per_1m: 7.5,
            },
        );
        let config = CostTrackerConfig {
            model_cost_rates: rates,
            ..Default::default()
        };
        let mut tracker = CostTracker::new(config);

        // 2_000_000 prompt: (2_000_000/1_000_000)*10.0 = 20.0
        // 1_000_000 completion: (1_000_000/1_000_000)*30.0 = 30.0
        tracker.record(&CallUsage {
            model: "custom-model".into(),
            prompt_tokens: 2_000_000,
            completion_tokens: 1_000_000,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
        });
        let expected = 20.0 + 30.0;
        assert!((tracker.accumulated_cost_usd() - expected).abs() < 0.01);
    }
}
