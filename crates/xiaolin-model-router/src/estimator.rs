use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

use xiaolin_core::complexity::ComplexityTier;
use xiaolin_core::types::ChatMessage;

fn default_model_tier() -> ComplexityTier {
    ComplexityTier::Medium
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    pub model: String,
    pub provider: String,
    pub input_per_1k: f64,
    pub output_per_1k: f64,
    pub max_context: u32,
    /// Relative quality score 0.0–1.0; used for routing decisions.
    #[serde(default = "default_quality")]
    pub quality: f64,
    /// Average latency in ms for first token (estimated).
    #[serde(default)]
    pub avg_latency_ms: u32,
    /// Relative model strength for tier-based routing (higher = more capable / costly).
    #[serde(default = "default_model_tier")]
    pub tier: ComplexityTier,
}

fn default_quality() -> f64 {
    0.8
}

#[derive(Debug, Clone)]
pub struct TokenEstimate {
    pub input_tokens: u32,
    pub estimated_output_tokens: u32,
}

impl TokenEstimate {
    pub fn estimated_cost(&self, pricing: &ModelPricing) -> f64 {
        let input_cost = (self.input_tokens as f64 / 1000.0) * pricing.input_per_1k;
        let output_cost = (self.estimated_output_tokens as f64 / 1000.0) * pricing.output_per_1k;
        input_cost + output_cost
    }
}

pub struct CostEstimator {
    pricing: HashMap<String, ModelPricing>,
}

impl CostEstimator {
    pub fn new() -> Self {
        Self {
            pricing: HashMap::new(),
        }
    }

    pub fn with_defaults() -> Self {
        let mut est = Self::new();
        let defaults = vec![
            ModelPricing {
                model: "gpt-4o".into(),
                provider: "openai".into(),
                input_per_1k: 0.005,
                output_per_1k: 0.015,
                max_context: 128000,
                quality: 0.95,
                avg_latency_ms: 800,
                tier: ComplexityTier::Large,
            },
            ModelPricing {
                model: "gpt-4o-mini".into(),
                provider: "openai".into(),
                input_per_1k: 0.00015,
                output_per_1k: 0.0006,
                max_context: 128000,
                quality: 0.85,
                avg_latency_ms: 400,
                tier: ComplexityTier::Small,
            },
            ModelPricing {
                model: "gpt-3.5-turbo".into(),
                provider: "openai".into(),
                input_per_1k: 0.0005,
                output_per_1k: 0.0015,
                max_context: 16385,
                quality: 0.7,
                avg_latency_ms: 300,
                tier: ComplexityTier::Tiny,
            },
            ModelPricing {
                model: "claude-sonnet-4-20250514".into(),
                provider: "anthropic".into(),
                input_per_1k: 0.003,
                output_per_1k: 0.015,
                max_context: 200000,
                quality: 0.95,
                avg_latency_ms: 900,
                tier: ComplexityTier::Frontier,
            },
            ModelPricing {
                model: "claude-3-5-haiku-20241022".into(),
                provider: "anthropic".into(),
                input_per_1k: 0.001,
                output_per_1k: 0.005,
                max_context: 200000,
                quality: 0.85,
                avg_latency_ms: 500,
                tier: ComplexityTier::Medium,
            },
            ModelPricing {
                model: "deepseek-chat".into(),
                provider: "deepseek".into(),
                input_per_1k: 0.00014,
                output_per_1k: 0.00028,
                max_context: 65536,
                quality: 0.8,
                avg_latency_ms: 600,
                tier: ComplexityTier::Medium,
            },
            ModelPricing {
                model: "gemini-2.5-pro".into(),
                provider: "google".into(),
                input_per_1k: 0.00125,
                output_per_1k: 0.01,
                max_context: 1048576,
                quality: 0.93,
                avg_latency_ms: 700,
                tier: ComplexityTier::Large,
            },
            ModelPricing {
                model: "gemini-2.5-flash".into(),
                provider: "google".into(),
                input_per_1k: 0.00015,
                output_per_1k: 0.0006,
                max_context: 1048576,
                quality: 0.85,
                avg_latency_ms: 350,
                tier: ComplexityTier::Small,
            },
            ModelPricing {
                model: "qwen-plus".into(),
                provider: "dashscope".into(),
                input_per_1k: 0.0004,
                output_per_1k: 0.0012,
                max_context: 131072,
                quality: 0.8,
                avg_latency_ms: 500,
                tier: ComplexityTier::Medium,
            },
        ];
        for p in defaults {
            est.add_pricing(p);
        }
        est
    }

    pub fn add_pricing(&mut self, pricing: ModelPricing) {
        self.pricing.insert(pricing.model.clone(), pricing);
    }

    pub fn get_pricing(&self, model: &str) -> Option<&ModelPricing> {
        self.pricing.get(model)
    }

    pub fn all_models(&self) -> Vec<&ModelPricing> {
        let mut models: Vec<_> = self.pricing.values().collect();
        models.sort_by(|a, b| a.model.cmp(&b.model));
        models
    }

    /// Conservative token count: max(bytes/4, char count).
    pub fn estimate_tokens(text: &str) -> u32 {
        let bytes = text.len() as u32;
        let chars = text.chars().count() as u32;
        bytes.div_ceil(4).max(chars)
    }

    /// Rough input-token budget for routing: message text + serialized tool calls + tool schema overhead.
    pub fn estimate_chat_complexity_tokens(
        messages: &[ChatMessage],
        tool_definition_count: usize,
    ) -> u32 {
        let mut chars: u64 = 0;
        let mut char_count: u64 = 0;
        for msg in messages {
            if let Some(ref c) = msg.content {
                if let Ok(s) = serde_json::to_string(c) {
                    chars = chars.saturating_add(s.len() as u64);
                    char_count = char_count.saturating_add(s.chars().count() as u64);
                }
            }
            if let Some(ref tcs) = msg.tool_calls {
                for tc in tcs {
                    chars = chars.saturating_add(tc.function.name.len() as u64);
                    chars = chars.saturating_add(tc.function.arguments.len() as u64);
                    char_count = char_count.saturating_add(tc.function.name.chars().count() as u64);
                    char_count =
                        char_count.saturating_add(tc.function.arguments.chars().count() as u64);
                    chars = chars.saturating_add(48);
                    char_count = char_count.saturating_add(48);
                }
            }
        }
        let capped_bytes = chars.min(u32::MAX as u64) as u32;
        let capped_chars = char_count.min(u32::MAX as u64) as u32;
        let base = capped_bytes.div_ceil(4).max(capped_chars);
        let tool_schema_overhead = (tool_definition_count as u32).saturating_mul(180);
        base.saturating_add(tool_schema_overhead).max(64)
    }

    pub fn estimate_request(
        &self,
        model: &str,
        messages: &[serde_json::Value],
    ) -> Option<(TokenEstimate, f64)> {
        let pricing = self.pricing.get(model)?;
        let mut total_chars = 0usize;
        let mut char_count = 0usize;
        for msg in messages {
            if let Some(content) = msg.get("content").and_then(|v| v.as_str()) {
                total_chars += content.len();
                char_count += content.chars().count();
            }
        }
        let input_tokens = (total_chars as u32)
            .div_ceil(4)
            .max(char_count as u32);
        let estimated_output = (input_tokens / 3).max(100);
        let est = TokenEstimate {
            input_tokens,
            estimated_output_tokens: estimated_output,
        };
        let cost = est.estimated_cost(pricing);
        Some((est, cost))
    }
}

impl Default for CostEstimator {
    fn default() -> Self {
        Self::new()
    }
}

static DEFAULT_COST_ESTIMATOR: OnceLock<CostEstimator> = OnceLock::new();

/// Best-effort dollar cost for a completed call using known pricing tables (`CostEstimator::with_defaults()`).
pub fn default_usage_charge(model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
    let est = DEFAULT_COST_ESTIMATOR.get_or_init(CostEstimator::with_defaults);
    est.get_pricing(model)
        .map(|p| {
            TokenEstimate {
                input_tokens,
                estimated_output_tokens: output_tokens,
            }
            .estimated_cost(p)
        })
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaolin_core::types::{ChatMessage, Role};

    #[test]
    fn default_pricing_has_entries() {
        let est = CostEstimator::with_defaults();
        assert!(est.all_models().len() >= 5);
    }

    #[test]
    fn cost_estimation() {
        let est = CostEstimator::with_defaults();
        let pricing = est.get_pricing("gpt-4o-mini").unwrap();
        let te = TokenEstimate {
            input_tokens: 1000,
            estimated_output_tokens: 500,
        };
        let cost = te.estimated_cost(pricing);
        assert!(cost > 0.0);
        assert!(cost < 1.0);
    }

    #[test]
    fn token_estimation() {
        let text = "hello world, this is a test";
        let tokens = CostEstimator::estimate_tokens(text);
        assert!(tokens > 0);
        assert_eq!(tokens, text.chars().count() as u32);
    }

    #[test]
    fn chat_complexity_includes_tools() {
        let msgs = vec![ChatMessage {
            role: Role::User,
            content: Some("a".repeat(400).into()),
        ..Default::default()
        }];
        let t0 = CostEstimator::estimate_chat_complexity_tokens(&msgs, 0);
        let t5 = CostEstimator::estimate_chat_complexity_tokens(&msgs, 5);
        assert!(t5 > t0);
        assert!(t0 >= 64);
    }
}
