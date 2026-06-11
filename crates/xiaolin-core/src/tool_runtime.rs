use std::path::Path;

use async_trait::async_trait;
use xiaolin_protocol::approval::PendingAction;
use xiaolin_protocol::id::{SessionId, TurnId};
use serde::{Deserialize, Serialize};

/// What the orchestrator should do regarding approval for a particular tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecApprovalRequirement {
    /// Tool call is safe — skip the approval pipeline entirely.
    Skip,
    /// Tool call needs explicit approval before execution.
    NeedsApproval { reason: String },
    /// Tool call is categorically forbidden — reject without asking.
    Forbidden { reason: String },
}

/// How the orchestrator should handle sandbox selection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SandboxPreference {
    /// Automatically select the best available sandbox.
    #[default]
    Auto,
    /// Require a real sandbox; fail the call if none is available.
    Required,
    /// Skip sandboxing (e.g. file I/O that doesn't spawn processes).
    Skip,
}

/// Describes the sandbox environment selected for a tool execution.
#[derive(Debug, Clone)]
pub struct SandboxAttempt {
    /// Which sandbox backend was selected.
    pub sandbox_type: SandboxBackend,
    /// Working directory for the sandboxed process.
    pub cwd: std::path::PathBuf,
}

/// Sandbox backend type — mirrors `xiaolin_sandbox::SandboxType` but lives in core
/// so that the trait layer doesn't depend on the sandbox crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SandboxBackend {
    Landlock,
    ExternalBinary,
    Seatbelt,
    RestrictedToken,
    None,
}

impl std::fmt::Display for SandboxBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Landlock => write!(f, "landlock"),
            Self::ExternalBinary => write!(f, "external_binary"),
            Self::Seatbelt => write!(f, "seatbelt"),
            Self::RestrictedToken => write!(f, "restricted_token"),
            Self::None => write!(f, "none"),
        }
    }
}

/// Progress update sent by a tool during streaming execution.
#[derive(Debug, Clone)]
pub struct ToolProgressEvent {
    pub message: String,
    pub partial_output: Option<String>,
    pub progress: Option<f64>,
}

/// Sender for tool progress events (optional, used by streaming tools).
pub type ToolProgressTx = tokio::sync::mpsc::Sender<ToolProgressEvent>;

/// Context provided to a `ToolRuntime` during execution.
#[derive(Debug, Clone)]
pub struct ToolExecContext {
    pub turn_id: TurnId,
    pub session_id: SessionId,
    pub call_id: String,
    pub cwd: std::path::PathBuf,
    /// Optional channel for emitting progress updates during execution.
    /// If `None`, the tool executes in batch mode (no streaming).
    pub progress_tx: Option<ToolProgressTx>,
}

/// Errors that can occur during orchestrated tool execution.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ToolRuntimeError {
    #[error("tool call rejected: {reason}")]
    Rejected { reason: String },

    #[error("sandbox denied execution: {reason}")]
    SandboxDenied { reason: String },

    #[error("tool execution timed out after {elapsed_ms}ms")]
    Timeout { elapsed_ms: u64 },

    #[error("internal error: {message}")]
    Internal { message: String },
}

/// How the orchestrator resolves approval based on the entry point.
#[derive(Debug, Clone)]
pub enum ApprovalStrategy {
    /// A human is in the loop — send approval requests via the interaction handle.
    /// The contained value is an opaque token identifying the interaction channel.
    Interactive,
    /// Automatically approve all tool calls (e.g. CLI `--auto-approve`).
    AutoApprove,
    /// Deny all tool calls that require approval (security audit mode).
    DenyAll,
    /// Rely solely on ExecPolicy rules: Allow → pass, Prompt/Forbid → reject.
    /// Used for non-interactive entry points (Feishu, HTTP API).
    PolicyBased,
}

/// Where the approval decision came from (for audit/diagnostics).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionSource {
    /// ExecPolicy explicitly allowed.
    PolicyAllowed,
    /// Retrieved from session-level approval cache.
    Cached,
    /// User approved interactively.
    UserApproved,
    /// User approved for the entire session.
    UserApprovedForSession,
    /// Auto-approve strategy.
    AutoApproved,
    /// Guardian LLM allowed.
    GuardianAllowed,
    /// Tool didn't need approval (Skip).
    NotRequired,
}

/// Result of a successful orchestrated execution.
#[derive(Debug, Clone)]
pub struct OrchestratorResult {
    /// The tool's output as a string (for inclusion in LLM messages).
    pub output: String,
    /// How the approval was resolved.
    pub decision_source: DecisionSource,
    /// Which sandbox backend was used.
    pub sandbox_used: SandboxBackend,
}

/// Trait for tools that can declare approval requirements.
pub trait Approvable {
    /// Compute approval keys for session-level caching.
    /// Calls with identical keys reuse a prior "ApprovedForSession" decision.
    fn approval_keys(&self, args: &serde_json::Value) -> Vec<String>;

    /// Determine what approval is needed for this specific invocation.
    fn exec_requirement(
        &self,
        args: &serde_json::Value,
        cwd: &Path,
    ) -> ExecApprovalRequirement;

    /// Map this tool call to a `PendingAction` for the approval UI.
    fn to_pending_action(
        &self,
        args: &serde_json::Value,
        cwd: &Path,
    ) -> PendingAction;
}

/// Trait for tools that can be sandboxed.
pub trait Sandboxable {
    /// Preferred sandbox strategy for this tool.
    fn sandbox_preference(&self) -> SandboxPreference;

    /// Whether to automatically retry without sandbox if sandbox denies execution.
    fn escalate_on_sandbox_failure(&self) -> bool {
        false
    }

    /// Whether escalation (retry without sandbox) should skip re-prompting the user.
    fn bypass_approval_on_escalation(&self) -> bool {
        false
    }
}

/// Unified trait for tools managed by the `ToolOrchestrator`.
///
/// A `ToolRuntime` combines approval logic, sandbox preferences, and execution
/// into a single cohesive interface. The orchestrator calls methods in order:
/// 1. `exec_requirement()` — decide if approval is needed
/// 2. `sandbox_preference()` — decide sandbox strategy
/// 3. `run()` — execute the tool
#[async_trait]
pub trait ToolRuntime: Approvable + Sandboxable + Send + Sync {
    /// Execute the tool with the given arguments and sandbox context.
    async fn run(
        &self,
        args: &serde_json::Value,
        sandbox: &SandboxAttempt,
        ctx: &ToolExecContext,
    ) -> Result<String, ToolRuntimeError>;

    /// Human-readable name for logging/diagnostics.
    fn name(&self) -> &str;
}
