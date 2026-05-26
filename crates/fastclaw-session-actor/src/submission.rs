use serde::{Deserialize, Serialize};

use fastclaw_protocol::approval::ApprovalDecision;
use fastclaw_protocol::id::{SessionId, SubmissionId};

/// A submission queued for processing by a [`SessionActor`](crate::SessionActor).
///
/// Mirrors Codex's `Submission` — every operation sent to the actor carries
/// a unique ID for event correlation.
#[derive(Debug, Clone)]
pub struct Submission {
    pub id: SubmissionId,
    pub op: SessionOp,
}

impl Submission {
    pub fn new(op: SessionOp) -> Self {
        Self {
            id: SubmissionId::generate(),
            op,
        }
    }
}

/// Operations that can be submitted to a session actor.
///
/// Aligned with Codex's `Op` enum but extended with FastClaw-specific
/// operations (multi-channel, context engine, memory).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SessionOp {
    /// Start a new user turn. Aborts any active turn first (Codex invariant).
    UserTurn {
        messages: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        work_dir: Option<String>,
        #[serde(default, flatten)]
        extra: serde_json::Map<String, serde_json::Value>,
    },

    /// Abort the current turn without terminating the session.
    Interrupt,

    /// Resolve a pending approval request (approval/deny/abort).
    ResolveApproval {
        interaction_id: String,
        decision: ApprovalDecision,
    },

    /// Answer a pending question from a tool (ask_question / confirm).
    ResolveAnswer {
        interaction_id: String,
        answer: String,
    },

    /// Inject input into an in-flight turn (mid-turn steering).
    SteerInput { messages: Vec<crate::turn::SteerMessage> },

    /// Trigger context compaction.
    Compact,

    /// Fork the session into a new session with a copy of history up to a
    /// specified point. Returns the new session ID via event.
    ForkSession {
        #[serde(skip_serializing_if = "Option::is_none")]
        fork_point_turn_id: Option<String>,
    },

    /// Roll back the session to a previous turn, discarding later history.
    RollbackTurns { to_turn_id: String },

    /// Update per-session settings (model, temperature, etc.) without
    /// starting a new turn.
    UpdateSettings {
        #[serde(flatten)]
        settings: serde_json::Map<String, serde_json::Value>,
    },

    /// Graceful shutdown — drain active turn, emit ShutdownComplete.
    Shutdown,
}

/// An event emitted by the session actor, correlated to a submission.
#[derive(Debug, Clone)]
pub struct SessionEvent {
    /// The submission ID this event correlates to.
    pub id: SubmissionId,
    /// The session that produced this event.
    pub session_id: SessionId,
    /// The event payload.
    pub msg: fastclaw_protocol::AgentEvent,
}
