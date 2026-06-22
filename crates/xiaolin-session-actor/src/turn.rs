use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use xiaolin_protocol::approval::ApprovalDecision;
use xiaolin_protocol::event::ErrorCode;
use xiaolin_protocol::id::{SessionId, SubmissionId, TurnId};
use xiaolin_protocol::usage::TokenUsage;
use xiaolin_protocol::AgentEvent;

use crate::interaction::InteractionHandle;

/// Shared, per-session approval cache. Keyed by action cache key.
pub type SessionApprovalCache = Arc<std::sync::Mutex<HashMap<String, ApprovalDecision>>>;

/// A mid-turn input message injected via `SessionOp::SteerInput`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SteerMessage {
    pub role: String,
    pub content: String,
}

/// Parameters for executing a turn.
pub struct TurnParams {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub agent_id: String,
    pub messages: serde_json::Value,
    pub model: Option<String>,
    pub work_dir: Option<String>,
    pub extra: serde_json::Map<String, serde_json::Value>,
    /// Per-session approval cache owned by the `SessionActor`. The executor
    /// should check and populate this cache instead of maintaining its own.
    pub approval_cache: SessionApprovalCache,
    /// Receiver for mid-turn steer inputs. The executor should drain this
    /// before each LLM sampling iteration and append to messages.
    pub steer_rx: tokio::sync::mpsc::UnboundedReceiver<SteerMessage>,
    /// Type-erased data passed from the gateway to avoid JSON round-trips.
    pub typed_data: Option<Arc<dyn std::any::Any + Send + Sync>>,
}

/// Result of a completed turn execution (success case).
pub struct TurnResult {
    pub tool_calls_made: u32,
    pub iterations: u32,
    pub usage: Option<TokenUsage>,
}

/// Typed error returned when a turn fails. Aligned with Codex's `CodexErr`
/// classification for differentiated error handling.
#[derive(Debug)]
pub enum TurnError {
    /// Turn was cancelled via `CancellationToken` (user interrupt or turn
    /// replacement). The actor already emits `TurnAborted`, so no separate
    /// `Error` event is needed.
    Cancelled,
    /// Runtime error with classification for client-side handling.
    Runtime {
        message: String,
        code: ErrorCode,
    },
}

impl TurnError {
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Cancelled => false,
            Self::Runtime { code, .. } => code.is_retryable(),
        }
    }

    /// Whether the actor should emit an error event and mark the turn as failed.
    pub fn affects_turn_status(&self) -> bool {
        match self {
            Self::Cancelled => false,
            Self::Runtime { code, .. } => code.affects_turn_status(),
        }
    }
}

impl std::fmt::Display for TurnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(f, "cancelled by session actor"),
            Self::Runtime { message, code } => write!(f, "[{code:?}] {message}"),
        }
    }
}

impl std::error::Error for TurnError {}

/// Trait for executing turns. Implemented by the gateway/agent layer to
/// bridge into `AgentRuntime`.
#[async_trait::async_trait]
pub trait TurnExecutor: Send + Sync + 'static {
    /// Execute a turn. The implementation should:
    ///
    /// 1. Stream `AgentEvent`s via `tx` (content deltas, tool events, etc.)
    /// 2. Use `interaction` to request approvals/answers (blocking the turn, not the actor)
    /// 3. Check `cancel` for cooperative cancellation
    /// 4. Return `Ok(TurnResult)` on success or `Err(TurnError)` on failure
    async fn execute(
        &self,
        params: TurnParams,
        interaction: InteractionHandle,
        tx: tokio::sync::mpsc::Sender<AgentEvent>,
        cancel: CancellationToken,
    ) -> Result<TurnResult, TurnError>;
}

/// Tracks an active turn within the session actor.
pub(crate) struct ActiveTurn {
    pub(crate) sub_id: SubmissionId,
    pub(crate) turn_id: TurnId,
    pub(crate) handle: tokio_util::task::AbortOnDropHandle<()>,
    pub(crate) cancel_token: CancellationToken,
    pub(crate) done: Arc<Notify>,
    pub(crate) relay_handle: tokio::task::JoinHandle<()>,
}
