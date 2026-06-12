use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use xiaolin_core::agent_config::AgentConfig;
use xiaolin_core::tool::ToolRegistry;
use xiaolin_core::tool_runtime::ApprovalStrategy;
use xiaolin_core::types::ChatRequest;
use xiaolin_protocol::AgentEvent;

use crate::builtin_tools::{ExecutionModeState, GoalStore, TodoStore};
use crate::llm::LlmProvider;
use crate::runtime::orchestrator::ToolOrchestrator;
use crate::runtime::runtimes::RuntimeRegistry;

use super::{ExecutionParams, StreamParams};

/// Unified execution context consolidating `ExecutionParams` + `StreamParams`.
///
/// This struct replaces the 13+ parameter signatures of `execute_unified`.
/// The `tx` field is optional: when present, the compatibility layer and
/// side-path emitters (orchestrator, tools) clone it to emit `AgentEvent`s directly.
pub struct AgentContext {
    // === Required ===
    pub config: AgentConfig,
    pub request: ChatRequest,
    pub tool_registry: Arc<ToolRegistry>,

    // === Streaming (side-path emitters clone this) ===
    pub tx: Option<tokio::sync::mpsc::Sender<AgentEvent>>,

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

    // === Optional - Persistence ===
    pub session_store: Option<Arc<xiaolin_session::SessionStore>>,
    pub todo_store: Option<TodoStore>,
    pub goal_store: Option<Arc<GoalStore>>,
    pub cost_store: Option<Arc<xiaolin_session::CostStore>>,

    // === Optional - Lifecycle ===
    pub cancel_token: Option<CancellationToken>,
}

impl AgentContext {
    /// Construct from legacy `ExecutionParams` + `StreamParams` (used by the compatibility layer).
    pub fn from_params(exec: &ExecutionParams<'_>, stream: StreamParams) -> Self {
        Self {
            config: exec.config.clone(),
            request: exec.request.clone(),
            tool_registry: exec.tool_registry.clone(),
            tx: Some(stream.tx),
            llm_override: exec.llm_override.clone(),
            subagent_prompt: exec.subagent_prompt.clone(),
            mode_state: exec.mode_state.clone(),
            orchestrator: stream.orchestrator,
            interaction_handle: stream.interaction_handle,
            approval_strategy: stream.approval_strategy,
            runtime_registry: stream.runtime_registry,
            session_store: exec.session_store.clone(),
            todo_store: exec.todo_store.clone(),
            goal_store: exec.goal_store.clone(),
            cost_store: exec.cost_store.clone(),
            cancel_token: None,
        }
    }

    /// Minimal context for SubAgent spawning or testing.
    pub fn minimal(config: AgentConfig, request: ChatRequest, tool_registry: Arc<ToolRegistry>) -> Self {
        Self {
            config,
            request,
            tool_registry,
            tx: None,
            llm_override: None,
            subagent_prompt: None,
            mode_state: None,
            orchestrator: None,
            interaction_handle: None,
            approval_strategy: ApprovalStrategy::AutoApprove,
            runtime_registry: None,
            session_store: None,
            todo_store: None,
            goal_store: None,
            cost_store: None,
            cancel_token: None,
        }
    }
}
