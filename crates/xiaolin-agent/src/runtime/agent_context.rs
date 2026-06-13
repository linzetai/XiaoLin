use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use xiaolin_core::agent_config::AgentConfig;
use xiaolin_core::tool::ToolRegistry;
use xiaolin_core::tool_runtime::ApprovalStrategy;
use xiaolin_core::types::ChatRequest;
use xiaolin_protocol::AgentEvent;

use super::agent_step::AgentStep;
use crate::builtin_tools::{ExecutionModeState, GoalStore, TodoStore};
use crate::llm::LlmProvider;
use crate::message_queue::MessageQueue;
use crate::runtime::orchestrator::ToolOrchestrator;
use crate::runtime::runtimes::RuntimeRegistry;

/// Unified execution context for an agent turn.
///
/// This struct replaces the 13+ parameter signatures of `execute_unified`.
///
/// Two channels carry events out of the execution loop:
/// - `step_tx`: main-loop events (Delta, ToolResult, TurnEnd, etc.) yielded as `AgentStep`
/// - `event_tx`: side-path events (ToolProgress, ApprovalRequired, SubAgent*) forwarded to caller
pub struct AgentContext {
    // === Required ===
    pub config: AgentConfig,
    pub request: ChatRequest,
    pub tool_registry: Arc<ToolRegistry>,

    // === Streaming channels ===
    /// Main-loop events yielded as `AgentStep` from the stream.
    pub step_tx: Option<tokio::sync::mpsc::Sender<AgentStep>>,
    /// Side-path events forwarded directly to the caller's channel.
    pub event_tx: Option<tokio::sync::mpsc::Sender<AgentEvent>>,

    // === Optional - LLM ===
    pub llm_override: Option<Arc<dyn LlmProvider>>,

    // === Optional - SubAgent ===
    pub subagent_prompt: Option<String>,

    // === Optional - Execution control ===
    pub mode_state: Option<ExecutionModeState>,
    pub orchestrator: Option<Arc<ToolOrchestrator>>,
    pub interaction_handle: Option<xiaolin_session_actor::InteractionHandle>,
    pub approval_strategy: ApprovalStrategy,
    pub runtime_registry: Option<Arc<RuntimeRegistry>>,
    /// Live behavior overrides — enables mid-turn permission changes.
    pub behavior_overrides:
        Option<Arc<dashmap::DashMap<String, xiaolin_core::agent_config::BehaviorConfig>>>,

    // === Optional - Persistence ===
    pub session_store: Option<Arc<xiaolin_session::SessionStore>>,
    pub todo_store: Option<TodoStore>,
    pub goal_store: Option<Arc<GoalStore>>,
    pub cost_store: Option<Arc<xiaolin_session::CostStore>>,

    // === Optional - Message Queue (for steering injection) ===
    pub message_queue: Option<Arc<MessageQueue>>,

    // === Optional - Plan file path (passes through tokio::spawn boundary) ===
    pub plan_file_path: Option<std::path::PathBuf>,

    // === Optional - Lifecycle ===
    pub cancel_token: Option<CancellationToken>,
}

impl AgentContext {
    /// Minimal context for SubAgent spawning or testing.
    pub fn minimal(config: AgentConfig, request: ChatRequest, tool_registry: Arc<ToolRegistry>) -> Self {
        Self {
            config,
            request,
            tool_registry,
            step_tx: None,
            event_tx: None,
            llm_override: None,
            subagent_prompt: None,
            mode_state: None,
            orchestrator: None,
            interaction_handle: None,
            approval_strategy: ApprovalStrategy::AutoApprove,
            runtime_registry: None,
            behavior_overrides: None,
            session_store: None,
            todo_store: None,
            goal_store: None,
            cost_store: None,
            plan_file_path: None,
            message_queue: None,
            cancel_token: None,
        }
    }
}
