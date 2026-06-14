use serde::{Deserialize, Serialize};

#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::id::TurnId;
use crate::message::{AskQuestionOption, CompactTrigger, ExecutionMode};
use crate::usage::TokenUsage;

/// Severity of a context-window warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "snake_case")]
pub enum ContextWarningLevel {
    /// ~85% usage — suggest /compact.
    Soft,
    /// ~95% usage — automatic compaction triggered.
    Hard,
}

/// Reason a turn was aborted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "snake_case")]
pub enum AbortReason {
    Interrupted,
    Replaced,
    BudgetLimited,
}

/// Structured error codes for agent events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorCode {
    ContextWindowExceeded,
    UsageLimitExceeded,
    ServerOverloaded,
    ConnectionFailed,
    StreamDisconnected,
    SandboxError,
    Unauthorized,
    BadRequest,
    Other,
}

impl ErrorCode {
    /// Whether the error is transient and the operation can be retried.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::ServerOverloaded | Self::ConnectionFailed | Self::StreamDisconnected
        )
    }

    /// Whether this error should mark the turn as failed.
    /// Non-affecting errors (e.g. steer rejections) leave the turn status intact.
    pub fn affects_turn_status(&self) -> bool {
        true
    }

    /// Classify an error message string into an `ErrorCode` via heuristics.
    pub fn classify(message: &str) -> Self {
        let lower = message.to_lowercase();
        if lower.contains("rate_limit")
            || lower.contains("rate limit")
            || lower.contains("overloaded")
            || lower.contains("capacity")
        {
            Self::ServerOverloaded
        } else if lower.contains("timeout") || lower.contains("timed out") {
            Self::ConnectionFailed
        } else if lower.contains("unauthorized") || lower.contains("401") {
            Self::Unauthorized
        } else if lower.contains("context") && lower.contains("exceed") {
            Self::ContextWindowExceeded
        } else if lower.contains("sandbox") {
            Self::SandboxError
        } else {
            Self::StreamDisconnected
        }
    }
}

/// Category of warning event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "snake_case")]
pub enum WarningCategory {
    Budget,
    ContextPressure,
    ToolFailure,
}

/// Risk level assessed by Guardian LLM review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// Outcome of a Guardian review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "snake_case")]
pub enum GuardianOutcome {
    Allow,
    Deny,
}

/// Summary emitted at the end of a turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct TurnSummary {
    pub turn_id: TurnId,
    pub tool_calls_made: u32,
    pub iterations: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    pub elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
}

/// Persistent record of a turn's execution context, used for session resume.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct TurnContextItem {
    pub turn_id: TurnId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub model: String,
    pub execution_mode: ExecutionMode,
    pub agent_id: String,
}

/// Runtime events emitted by the agent.
///
/// Each variant carries a `turn_id` where applicable so consumers can
/// Goal state snapshot sent to the frontend via `GoalUpdated` events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct GoalData {
    pub id: String,
    pub description: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<u64>,
    pub tokens_used: u64,
    pub time_used_seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pause_reason: Option<String>,
    pub continuation_rounds: u32,
    pub created_at: u64,
    pub updated_at: u64,
}

/// correlate events to their originating turn without out-of-band state.
///
/// Serialized with `#[serde(tag = "type")]` for discriminated-union
/// consumption on the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum AgentEvent {
    // ── Turn lifecycle ──────────────────────────────────────────────
    TurnStart {
        turn_id: TurnId,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
    TurnEnd {
        turn_id: TurnId,
        summary: TurnSummary,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        final_tool_calls: Option<Vec<crate::ToolCallData>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    // ── Content streaming ───────────────────────────────────────────
    ContentDelta {
        turn_id: TurnId,
        #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
        delta: serde_json::Value,
        #[serde(skip)]
        raw_bytes: Option<bytes::Bytes>,
    },
    ReasoningDelta {
        turn_id: TurnId,
        content: String,
    },

    // ── Tool lifecycle ──────────────────────────────────────────────
    ToolExecuting {
        turn_id: TurnId,
        tool_name: String,
        call_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        args: Option<String>,
    },
    ToolResult {
        turn_id: TurnId,
        tool_name: String,
        call_id: String,
        output: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        display_output: Option<String>,
        success: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown> | null"))]
        metadata: Option<serde_json::Value>,
    },
    ToolProgress {
        turn_id: TurnId,
        tool_name: String,
        call_id: String,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        progress: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        partial_output: Option<String>,
    },
    IterationBoundary {
        turn_id: TurnId,
        iteration: u32,
    },

    // ── Approval / user interaction ─────────────────────────────────
    AskQuestion {
        turn_id: TurnId,
        request_id: String,
        question: String,
        options: Vec<AskQuestionOption>,
        timeout_secs: u32,
        allow_multiple: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
    BriefMessage {
        turn_id: TurnId,
        content: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<String>,
        mode: String,
    },
    Suggestions {
        turn_id: TurnId,
        items: Vec<String>,
    },

    // ── Context management ──────────────────────────────────────────
    ContextWarning {
        turn_id: TurnId,
        level: ContextWarningLevel,
        used_tokens: u32,
        limit_tokens: u32,
        message: String,
    },
    ContextUsageUpdate {
        turn_id: TurnId,
        used_tokens: u32,
        limit_tokens: u32,
        compressed: bool,
        tokens_saved: u32,
    },
    CompactBoundary {
        turn_id: TurnId,
        trigger: CompactTrigger,
        pre_compact_tokens: usize,
        post_compact_tokens: usize,
        messages_removed: usize,
    },

    // ── Mode changes ────────────────────────────────────────────────
    ModeChange {
        turn_id: TurnId,
        from: ExecutionMode,
        to: ExecutionMode,
    },
    PlanFileUpdate {
        turn_id: TurnId,
        session_id: String,
        path: String,
        exists: bool,
    },

    // ── Sub-agent events ────────────────────────────────────────────
    SubAgentStart {
        turn_id: TurnId,
        run_id: String,
        agent_id: String,
        subagent_type: String,
        task: String,
        depth: u32,
    },
    SubAgentDelta {
        turn_id: TurnId,
        run_id: String,
        content: String,
    },
    SubAgentToolExecuting {
        turn_id: TurnId,
        run_id: String,
        tool_name: String,
        call_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        args: Option<String>,
    },
    SubAgentToolResult {
        turn_id: TurnId,
        run_id: String,
        tool_name: String,
        call_id: String,
        output: String,
        success: bool,
    },
    SubAgentComplete {
        turn_id: TurnId,
        run_id: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<String>,
        tool_calls_made: u32,
        iterations: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<TokenUsage>,
        elapsed_ms: u64,
    },
    /// Emitted when the harness reactive loop re-prompts the LLM after
    /// one or more sub-agents complete.
    SubAgentNotification {
        turn_id: TurnId,
        completions: Vec<CompletionSummary>,
        remaining_active: u32,
    },

    // ── Approval / policy ────────────────────────────────────────────
    ApprovalRequired {
        turn_id: TurnId,
        approval_id: String,
        action: crate::approval::PendingAction,
        reason: String,
        available_decisions: Vec<crate::approval::ApprovalDecision>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        risk_level: Option<crate::approval::ActionRiskLevel>,
    },
    ApprovalResolved {
        turn_id: TurnId,
        approval_id: String,
        decision: crate::approval::ApprovalDecision,
        source: String,
    },

    // ── Error ───────────────────────────────────────────────────────
    TurnAborted {
        turn_id: TurnId,
        reason: AbortReason,
        #[serde(skip_serializing_if = "Option::is_none")]
        completed_at: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
    },
    StreamError {
        turn_id: TurnId,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        error_code: Option<ErrorCode>,
        #[serde(default)]
        retry_attempt: u32,
    },
    Warning {
        turn_id: TurnId,
        message: String,
        category: WarningCategory,
    },
    Error {
        turn_id: TurnId,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        error_code: Option<ErrorCode>,
    },

    // ── Guardian LLM review ──────────────────────────────────────────
    GuardianAssessment {
        turn_id: TurnId,
        review_id: String,
        risk_level: RiskLevel,
        outcome: GuardianOutcome,
        rationale: String,
    },
    GuardianWarning {
        turn_id: TurnId,
        message: String,
    },

    // ── Memory (XiaoLin-specific) ──────────────────────────────────
    MemoryStored {
        turn_id: TurnId,
        fragment_id: String,
        summary: String,
    },
    MemoryRecalled {
        turn_id: TurnId,
        fragment_ids: Vec<String>,
    },

    // ── Goal lifecycle ──────────────────────────────────────────────
    GoalUpdated {
        turn_id: TurnId,
        goal: GoalData,
    },
    GoalCleared {
        turn_id: TurnId,
        goal_id: String,
    },
}

impl AgentEvent {
    pub fn turn_id(&self) -> &TurnId {
        match self {
            Self::TurnStart { turn_id, .. }
            | Self::TurnEnd { turn_id, .. }
            | Self::ContentDelta { turn_id, .. }
            | Self::ReasoningDelta { turn_id, .. }
            | Self::ToolExecuting { turn_id, .. }
            | Self::ToolResult { turn_id, .. }
            | Self::ToolProgress { turn_id, .. }
            | Self::AskQuestion { turn_id, .. }
            | Self::BriefMessage { turn_id, .. }
            | Self::Suggestions { turn_id, .. }
            | Self::ContextWarning { turn_id, .. }
            | Self::ContextUsageUpdate { turn_id, .. }
            | Self::CompactBoundary { turn_id, .. }
            | Self::ModeChange { turn_id, .. }
            | Self::PlanFileUpdate { turn_id, .. }
            | Self::SubAgentStart { turn_id, .. }
            | Self::SubAgentDelta { turn_id, .. }
            | Self::SubAgentToolExecuting { turn_id, .. }
            | Self::SubAgentToolResult { turn_id, .. }
            | Self::SubAgentComplete { turn_id, .. }
            | Self::SubAgentNotification { turn_id, .. }
            | Self::ApprovalRequired { turn_id, .. }
            | Self::ApprovalResolved { turn_id, .. }
            | Self::GuardianAssessment { turn_id, .. }
            | Self::GuardianWarning { turn_id, .. }
            | Self::TurnAborted { turn_id, .. }
            | Self::StreamError { turn_id, .. }
            | Self::Warning { turn_id, .. }
            | Self::Error { turn_id, .. }
            | Self::MemoryStored { turn_id, .. }
            | Self::MemoryRecalled { turn_id, .. }
            | Self::GoalUpdated { turn_id, .. }
            | Self::GoalCleared { turn_id, .. }
            | Self::IterationBoundary { turn_id, .. } => turn_id,
        }
    }
}

/// Summary of a completed sub-agent run, used for reactive loop notifications.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct CompletionSummary {
    pub run_id: String,
    pub agent_id: String,
    pub subagent_type: String,
    pub task: String,
    pub status: String,
    pub elapsed_ms: u64,
    pub tool_call_count: u32,
    /// Truncated result preview (max 2000 chars).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Lightweight data for a tool call, carried in `TurnEnd`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ToolCallData {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: ToolCallFunction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_turn_id() -> TurnId {
        TurnId::new("turn-test-1")
    }

    #[test]
    fn agent_event_content_delta_roundtrip() {
        let evt = AgentEvent::ContentDelta {
            turn_id: sample_turn_id(),
            delta: serde_json::json!({"content": "hello"}),
            raw_bytes: None,
        };
        let json = serde_json::to_string(&evt).unwrap();
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.turn_id().as_str(), "turn-test-1");
    }

    #[test]
    fn agent_event_tool_result_roundtrip() {
        let evt = AgentEvent::ToolResult {
            turn_id: sample_turn_id(),
            tool_name: "read_file".into(),
            call_id: "tc-1".into(),
            output: "file content".into(),
            display_output: None,
            success: true,
            metadata: None,
        };
        let json = serde_json::to_string(&evt).unwrap();
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        if let AgentEvent::ToolResult { success, .. } = back {
            assert!(success);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn agent_event_turn_end_roundtrip() {
        let evt = AgentEvent::TurnEnd {
            turn_id: sample_turn_id(),
            summary: TurnSummary {
                turn_id: sample_turn_id(),
                tool_calls_made: 3,
                iterations: 2,
                usage: None,
                elapsed_ms: 1500,
                context_tokens: Some(4000),
                context_window: Some(128_000),
            },
            session_id: Some("sess-1".into()),
            final_tool_calls: None,
            reason: None,
        };
        let json = serde_json::to_string(&evt).unwrap();
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        if let AgentEvent::TurnEnd { summary, .. } = back {
            assert_eq!(summary.tool_calls_made, 3);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn agent_event_error_roundtrip() {
        let evt = AgentEvent::Error {
            turn_id: sample_turn_id(),
            message: "something went wrong".into(),
            error_code: None,
        };
        let json = serde_json::to_string(&evt).unwrap();
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        if let AgentEvent::Error { message, .. } = back {
            assert_eq!(message, "something went wrong");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn agent_event_subagent_complete_roundtrip() {
        let evt = AgentEvent::SubAgentComplete {
            turn_id: sample_turn_id(),
            run_id: "run-1".into(),
            status: "completed".into(),
            result: Some("done".into()),
            tool_calls_made: 5,
            iterations: 3,
            usage: Some(TokenUsage {
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: 150,
                cached_input_tokens: 0,
            }),
            elapsed_ms: 2000,
        };
        let json = serde_json::to_string(&evt).unwrap();
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        if let AgentEvent::SubAgentComplete { elapsed_ms, .. } = back {
            assert_eq!(elapsed_ms, 2000);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn agent_event_tagged_serde() {
        let evt = AgentEvent::ModeChange {
            turn_id: sample_turn_id(),
            from: ExecutionMode::Agent,
            to: ExecutionMode::Plan,
        };
        let val = serde_json::to_value(&evt).unwrap();
        assert_eq!(val["type"], "mode_change");
    }

    #[test]
    fn context_warning_level_roundtrip() {
        let soft = ContextWarningLevel::Soft;
        let json = serde_json::to_string(&soft).unwrap();
        let back: ContextWarningLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ContextWarningLevel::Soft);
    }
}
