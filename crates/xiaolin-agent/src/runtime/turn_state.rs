use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use xiaolin_core::agent_config::AgentConfig;
use xiaolin_core::tool::{ToolDefinition, ToolRegistry};
use xiaolin_core::tool_runtime::ApprovalStrategy;
use xiaolin_core::types::ChatMessage;
use xiaolin_evolution::TrajectoryStep;
use xiaolin_protocol::{AgentEvent, TurnId};

use super::agent_step::AgentStep;

use super::approval_cache::ApprovalCache;
use super::cache_break_detection::CacheBreakDetector;
use super::dispatcher::ToolDispatcher;
use super::file_persistence::SessionFileTracker;
use super::observer::RuntimeObserver;
use super::permissions::DenialTracker;
use super::query_deps::ProductionDeps;
use super::query_state::QueryLoopState;
use super::runtime_quality::RuntimeQualityCollector;
use super::runtime_services::RuntimeServices;
use super::runtimes::RuntimeRegistry;
use super::token_budget::BudgetTracker;
use super::tool_result_storage::{ContentReplacementState, ToolResultStorage};
use super::undo_engine::UndoEngine;
use super::validation_pipeline::ValidationPipeline;
use super::AgentRuntime;
use crate::builtin_tools::{ExecutionModeState, GoalStore, TodoStore};
use crate::message_queue::MessageQueue;
use xiaolin_session::tool_output_store::ToolOutputAssetStore;

/// Mutable state that evolves across iterations of the agent loop.
///
/// Created by `turn_setup` at the start of execution, then passed by
/// `&mut` reference into each sub-phase (iteration_check, llm_call,
/// tool_round, post_tool).
pub(crate) struct TurnMutableState {
    pub messages: Vec<ChatMessage>,
    pub max_tokens: Option<u32>,
    pub query_loop: QueryLoopState,
    pub replacement_state: ContentReplacementState,
    pub undo_engine: UndoEngine,
    pub approval_cache: ApprovalCache,
    pub denial_tracker: DenialTracker,
    pub cache_detector: CacheBreakDetector,
    pub file_tracker: SessionFileTracker,
    pub last_seen_goal_id: Option<String>,
    pub had_tool_calls_this_round: bool,
    /// Whether any progress-making tool (write, shell, subagent) was called this round.
    pub had_progress_this_round: bool,
    /// Whether a verification command (test, build, check) was run this round.
    /// Verification counts as shallow progress — it may partially reduce the
    /// no-progress counter but should not fully reset it, preventing indefinite
    /// stalling via repeated test runs without code changes.
    pub had_verification_this_round: bool,
    pub injected_skill_ids: Vec<String>,
    pub trajectory_steps: Vec<TrajectoryStep>,
    pub budget_tracker: Option<BudgetTracker>,
    pub runtime_quality: RuntimeQualityCollector,
    /// Set to true when token budget was reached in this turn (for TurnEnd reason override).
    pub token_budget_reached: bool,
    /// Tool definitions sent to the LLM. Refreshed when the registry version changes
    /// (e.g. after `tool_search` activates a deferred tool).
    pub tool_defs: Vec<ToolDefinition>,
    pub tool_defs_est_tokens: usize,
    /// Snapshot of `ToolRegistry::version()` at the time `tool_defs` was built.
    pub registry_version_at_setup: u64,
    /// Whether mode attachment turn counter was already incremented this outer loop iteration.
    /// Prevents double-counting on `RetryIteration` paths.
    pub mode_turn_counted: bool,
    /// Extra tool definitions injected by the channel request (`request.tools`).
    /// Preserved across `tool_defs` refreshes so channel-scoped tools survive
    /// registry version changes.
    pub extra_tool_defs: Vec<ToolDefinition>,
}

/// Immutable context and service dependencies for a single agent turn.
///
/// Created during setup; shared (by reference) across all loop iterations.
/// Contains both configuration and long-lived service handles.
///
/// Stores `Arc<AgentRuntime>` so sub-functions can access runtime methods
/// (e.g. `finalize_injected_skills`, `provider()`) without a separate parameter.
pub(crate) struct TurnServices {
    // --- Runtime access (Arc-wrapped, cheap to hold) ---
    pub runtime: Arc<AgentRuntime>,

    // --- Identity ---
    pub turn_id: TurnId,
    pub stream_start: std::time::Instant,

    // --- Model/LLM config (immutable after setup) ---
    pub model: String,
    pub temperature: f32,
    pub context_window: u32,
    pub auto_compact_enabled: bool,

    // --- Request context (only fields used in the loop body) ---
    pub config: Arc<AgentConfig>,
    pub session_id: Option<String>,
    pub work_dir: Option<String>,
    pub last_user_msg: String,
    pub mode_state: Option<ExecutionModeState>,
    pub runtime_registry: Arc<RuntimeRegistry>,
    pub tool_registry: Arc<ToolRegistry>,
    pub session_store: Option<Arc<xiaolin_session::SessionStore>>,
    pub runtime_quality_store: Option<Arc<xiaolin_session::RuntimeQualityStore>>,
    pub todo_store: Option<TodoStore>,
    pub goal_store: Option<Arc<GoalStore>>,
    pub plan_file_path: Option<std::path::PathBuf>,
    pub language_preference: Option<String>,

    // --- Streaming channels ---
    /// Main-loop events (Delta, ToolResult, TurnEnd, etc.) — yielded as AgentStep from the stream.
    pub step_tx: mpsc::Sender<AgentStep>,
    /// Side-path events (ToolProgress, ApprovalRequired, SubAgent*) — forwarded to caller directly.
    pub event_tx: mpsc::Sender<AgentEvent>,
    pub approval_strategy: ApprovalStrategy,
    pub interaction_handle: Option<xiaolin_session_actor::InteractionHandle>,
    /// Live behavior overrides — enables mid-turn permission changes to take effect immediately.
    pub behavior_overrides:
        Option<Arc<dashmap::DashMap<String, xiaolin_core::agent_config::BehaviorConfig>>>,

    // --- Service handles (interior mutability where needed) ---
    pub deps: ProductionDeps,
    pub services: RuntimeServices,
    pub dispatcher: ToolDispatcher,
    pub tool_storage: ToolResultStorage,
    /// Optional asset store for handle-based output persistence (Phase 2+).
    pub tool_output_store: Option<Arc<ToolOutputAssetStore>>,
    pub skip_tool_names: HashSet<String>,
    pub validation_pipeline: ValidationPipeline,
    pub runtime_observer: RuntimeObserver,

    // --- Message Queue ---
    pub message_queue: Option<Arc<MessageQueue>>,

    // --- Lifecycle ---
    pub cancel_token: Option<CancellationToken>,
}
