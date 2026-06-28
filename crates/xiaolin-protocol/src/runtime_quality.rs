use serde::{Deserialize, Serialize};

#[cfg(feature = "ts")]
use ts_rs::TS;

/// Rule-based diagnosis code for a completed agent turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum TurnQualityDiagnosisCode {
    Normal,
    SlowFirstToken,
    SlowTools,
    ToolFailures,
    ToolLoop,
    HighContext,
    HighCost,
    CacheMiss,
    ManyIterations,
    Aborted,
    Error,
    Unknown,
}

impl TurnQualityDiagnosisCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::SlowFirstToken => "slow_first_token",
            Self::SlowTools => "slow_tools",
            Self::ToolFailures => "tool_failures",
            Self::ToolLoop => "tool_loop",
            Self::HighContext => "high_context",
            Self::HighCost => "high_cost",
            Self::CacheMiss => "cache_miss",
            Self::ManyIterations => "many_iterations",
            Self::Aborted => "aborted",
            Self::Error => "error",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for TurnQualityDiagnosisCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Severity assigned by deterministic diagnosis rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum TurnQualitySeverity {
    Info,
    Warn,
    Error,
}

impl TurnQualitySeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

impl std::fmt::Display for TurnQualitySeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Structured runtime-quality snapshot for one completed turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct TurnQualitySummary {
    pub session_id: String,
    pub turn_id: String,
    pub agent_id: String,
    pub model_provider: String,
    pub model: String,
    pub started_at: String,
    pub ended_at: String,
    pub elapsed_ms: u64,
    pub first_delta_ms: Option<u64>,
    pub first_content_ms: Option<u64>,
    pub first_reasoning_ms: Option<u64>,
    pub first_tool_ms: Option<u64>,
    pub iterations: u32,
    pub tool_calls_total: u32,
    pub tool_failures_total: u32,
    pub tool_time_ms_total: u64,
    pub slowest_tool_name: Option<String>,
    pub slowest_tool_ms: Option<u64>,
    pub repeated_tool_warn_count: u32,
    pub repeated_tool_force_stop_count: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_creation_tokens: u32,
    pub cache_hit_pct: Option<f64>,
    pub estimated_cost_usd: Option<f64>,
    pub context_tokens: Option<u32>,
    pub context_window: Option<u32>,
    pub context_usage_pct: Option<f64>,
    pub compressed: bool,
    pub tokens_saved: Option<i64>,
    pub compact_count: u32,
    pub diagnosis_code: TurnQualityDiagnosisCode,
    pub severity: TurnQualitySeverity,
    pub evidence_json: serde_json::Value,
    /// Phase 8.3: number of tool output assets created this turn.
    pub asset_count: u32,
    /// Phase 8.3: estimated tokens of raw tool outputs this turn.
    pub raw_output_token_estimate: u64,
    /// Phase 8.3: estimated tokens after projection this turn.
    pub projected_output_tokens: u64,
    /// Phase 8.3: number of recall tool calls this turn.
    pub recall_count: u32,
    /// Phase 8.3: number of repeated tool call detections this turn.
    pub repeated_tool_call_indicators: u32,
}
