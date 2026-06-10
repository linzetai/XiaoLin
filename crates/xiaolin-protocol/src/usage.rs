use serde::{Deserialize, Serialize};

#[cfg(feature = "ts")]
use ts_rs::TS;

/// Accumulated LLM token usage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    #[serde(default)]
    pub cached_input_tokens: u32,
}

impl TokenUsage {
    pub fn merge(&mut self, other: &Self) {
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
        self.total_tokens += other.total_tokens;
        self.cached_input_tokens += other.cached_input_tokens;
    }

    /// Compute the effective token delta for goal accounting:
    /// (input_tokens - cached_input_tokens) + output_tokens
    pub fn goal_token_delta(&self) -> u64 {
        let non_cached_input = self.prompt_tokens.saturating_sub(self.cached_input_tokens);
        (non_cached_input as u64) + (self.completion_tokens as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_usage_serde_roundtrip() {
        let u = TokenUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            cached_input_tokens: 0,
        };
        let json = serde_json::to_string(&u).unwrap();
        let back: TokenUsage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.prompt_tokens, 100);
        assert_eq!(back.total_tokens, 150);
    }

    #[test]
    fn token_usage_merge() {
        let mut a = TokenUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
            cached_input_tokens: 0,
        };
        let b = TokenUsage {
            prompt_tokens: 20,
            completion_tokens: 10,
            total_tokens: 30,
            cached_input_tokens: 0,
        };
        a.merge(&b);
        assert_eq!(a.prompt_tokens, 30);
        assert_eq!(a.total_tokens, 45);
    }

    #[test]
    fn goal_token_delta_deducts_cached() {
        let u = TokenUsage {
            prompt_tokens: 1000,
            completion_tokens: 200,
            total_tokens: 1200,
            cached_input_tokens: 800,
        };
        assert_eq!(u.goal_token_delta(), 400);
    }

    #[test]
    fn goal_token_delta_no_cached() {
        let u = TokenUsage {
            prompt_tokens: 500,
            completion_tokens: 100,
            total_tokens: 600,
            cached_input_tokens: 0,
        };
        assert_eq!(u.goal_token_delta(), 600);
    }
}
