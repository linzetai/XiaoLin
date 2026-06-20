use xiaolin_protocol::event::GoalData;
use xiaolin_protocol::{AgentEvent, ContextWarningLevel, ErrorCode, ExecutionMode, TurnId, TurnSummary};

use super::query_state::TerminalReason;

/// Events produced by the main agent loop (`execute_as_stream`).
///
/// Side-path events (ToolProgress, ApprovalRequired, SubAgent*, etc.) are emitted
/// directly to `tx` by orchestrator/tools and are NOT represented here.
#[derive(Debug, Clone)]
pub enum AgentStep {
    TurnStart {
        turn_id: TurnId,
        session_id: Option<String>,
    },

    /// LLM streaming content delta. `delta` matches `AgentEvent::ContentDelta.delta`.
    Delta {
        turn_id: TurnId,
        delta: serde_json::Value,
        #[allow(dead_code)]
        raw_bytes: Option<bytes::Bytes>,
    },

    /// Reasoning/thinking content (separate from visible content).
    ReasoningDelta {
        turn_id: TurnId,
        content: String,
    },

    ToolExecuting {
        turn_id: TurnId,
        call_id: String,
        tool_name: String,
        args: Option<String>,
    },

    ToolResult {
        turn_id: TurnId,
        call_id: String,
        tool_name: String,
        output: String,
        display_output: Option<String>,
        success: bool,
        metadata: Option<serde_json::Value>,
    },

    /// Marks the boundary between tool rounds (after all results, before next LLM call).
    ToolRoundBoundary {
        turn_id: TurnId,
        iteration: u32,
    },

    /// Steering messages were injected at a tool-round boundary.
    SteeringInjected {
        count: usize,
        sources: Vec<String>,
    },

    ContextUsage {
        turn_id: TurnId,
        used_tokens: u32,
        limit_tokens: u32,
        compressed: bool,
        tokens_saved: u32,
    },

    ContextWarning {
        turn_id: TurnId,
        level: ContextWarningLevel,
        used_tokens: u32,
        limit_tokens: u32,
        message: String,
    },

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
        content: Option<String>,
    },

    PlanDelta {
        turn_id: TurnId,
        delta: String,
    },

    GoalUpdated {
        turn_id: TurnId,
        goal: GoalData,
    },

    GoalCleared {
        turn_id: TurnId,
        goal_id: String,
    },

    TurnEnd {
        turn_id: TurnId,
        reason: TurnEndReason,
        summary: TurnSummary,
        session_id: Option<String>,
    },

    Error {
        turn_id: TurnId,
        message: String,
        error_code: Option<ErrorCode>,
        recoverable: bool,
    },
}

/// Why the agent loop terminated (aligned with `TerminalReason`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnEndReason {
    Completed,
    MaxTurns,
    Cancelled,
    ContextLimit,
    BudgetExceeded,
    TokenBudgetReached,
    ConsecutiveErrors,
    DiminishingReturns,
    PlanApprovalPending,
    Error(String),
}

impl From<TerminalReason> for TurnEndReason {
    fn from(r: TerminalReason) -> Self {
        match r {
            TerminalReason::EndTurn => Self::Completed,
            TerminalReason::MaxIterations => Self::MaxTurns,
            TerminalReason::Aborted => Self::Cancelled,
            TerminalReason::BlockingLimit => Self::ContextLimit,
            TerminalReason::BudgetExhausted => Self::BudgetExceeded,
            TerminalReason::ConsecutiveErrors => Self::ConsecutiveErrors,
            TerminalReason::DiminishingReturns => Self::DiminishingReturns,
        }
    }
}

impl AgentStep {
    /// Convert this step into zero or more `AgentEvent`s for the compatibility layer.
    ///
    /// Internal markers (`ToolRoundBoundary`, `SteeringInjected`) produce no events.
    /// Most steps produce exactly one event; `Delta` with reasoning may produce two.
    pub fn into_agent_events(self) -> Vec<AgentEvent> {
        match self {
            Self::TurnStart { turn_id, session_id } => {
                vec![AgentEvent::TurnStart { turn_id, session_id }]
            }

            Self::Delta { turn_id, delta, raw_bytes } => {
                vec![AgentEvent::ContentDelta { turn_id, delta, raw_bytes }]
            }

            Self::ReasoningDelta { turn_id, content } => {
                vec![AgentEvent::ReasoningDelta { turn_id, content }]
            }

            Self::ToolExecuting { turn_id, call_id, tool_name, args } => {
                vec![AgentEvent::ToolExecuting { turn_id, tool_name, call_id, args }]
            }

            Self::ToolResult { turn_id, call_id, tool_name, output, display_output, success, metadata } => {
                vec![AgentEvent::ToolResult {
                    turn_id,
                    tool_name,
                    call_id,
                    output,
                    display_output,
                    success,
                    metadata,
                }]
            }

            Self::ContextUsage { turn_id, used_tokens, limit_tokens, compressed, tokens_saved } => {
                vec![AgentEvent::ContextUsageUpdate {
                    turn_id,
                    used_tokens,
                    limit_tokens,
                    compressed,
                    tokens_saved,
                }]
            }

            Self::ContextWarning { turn_id, level, used_tokens, limit_tokens, message } => {
                vec![AgentEvent::ContextWarning {
                    turn_id,
                    level,
                    used_tokens,
                    limit_tokens,
                    message,
                }]
            }

            Self::ModeChange { turn_id, from, to } => {
                vec![AgentEvent::ModeChange { turn_id, from, to }]
            }

            Self::PlanFileUpdate { turn_id, session_id, path, exists, content } => {
                vec![AgentEvent::PlanFileUpdate { turn_id, session_id, path, exists, content }]
            }

            Self::PlanDelta { turn_id, delta } => {
                vec![AgentEvent::PlanDelta { turn_id, delta }]
            }

            Self::GoalUpdated { turn_id, goal } => {
                vec![AgentEvent::GoalUpdated { turn_id, goal }]
            }

            Self::GoalCleared { turn_id, goal_id } => {
                vec![AgentEvent::GoalCleared { turn_id, goal_id }]
            }

            Self::TurnEnd { turn_id, reason, summary, session_id } => {
                let reason_str = match &reason {
                    TurnEndReason::TokenBudgetReached => Some("token_budget_reached".to_string()),
                    TurnEndReason::Completed => None,
                    TurnEndReason::MaxTurns => Some("max_turns".to_string()),
                    TurnEndReason::Cancelled => Some("cancelled".to_string()),
                    TurnEndReason::ContextLimit => Some("context_limit".to_string()),
                    TurnEndReason::BudgetExceeded => Some("budget_exceeded".to_string()),
                    TurnEndReason::ConsecutiveErrors => Some("consecutive_errors".to_string()),
                    TurnEndReason::DiminishingReturns => Some("diminishing_returns".to_string()),
                    TurnEndReason::PlanApprovalPending => Some("plan_approval_pending".to_string()),
                    TurnEndReason::Error(e) => Some(format!("error: {e}")),
                };
                vec![AgentEvent::TurnEnd {
                    turn_id,
                    summary,
                    session_id,
                    final_tool_calls: None,
                    reason: reason_str,
                }]
            }

            Self::Error { turn_id, message, error_code, .. } => {
                vec![AgentEvent::Error { turn_id, message, error_code }]
            }

            Self::ToolRoundBoundary { turn_id, iteration } => {
                vec![AgentEvent::IterationBoundary { turn_id, iteration }]
            }
            Self::SteeringInjected { .. } => vec![],
        }
    }

    /// Whether this step can be dropped when the channel is full.
    pub fn is_lossy(&self) -> bool {
        matches!(self, Self::ContextUsage { .. } | Self::ContextWarning { .. })
    }

}
