//! LLM workload complexity tiers for model routing.

use serde::{Deserialize, Serialize};

/// Discrete complexity bands used to pick an appropriate model.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum ComplexityTier {
    /// Simple Q&A, greetings, one-line tasks.
    #[default]
    Tiny,
    /// Straightforward single-step tasks.
    Small,
    /// Multi-step reasoning or moderate code changes.
    Medium,
    /// Deep analysis, large refactors, long documents.
    Large,
    /// Hardest tasks where frontier models are justified.
    Frontier,
}

impl ComplexityTier {
    /// All tiers from least to most demanding.
    pub const ALL: [Self; 5] = [
        Self::Tiny,
        Self::Small,
        Self::Medium,
        Self::Large,
        Self::Frontier,
    ];

    /// Parse from snake_case string (for JSON APIs); unknown values map to `Medium`.
    pub fn parse_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "tiny" => Self::Tiny,
            "small" => Self::Small,
            "medium" => Self::Medium,
            "large" => Self::Large,
            "frontier" => Self::Frontier,
            _ => Self::Medium,
        }
    }
}
