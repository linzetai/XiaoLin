use std::collections::HashMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use xiaolin_core::agent_config::AgentConfig;
use xiaolin_core::tool::{ToolProfile, ToolRegistry};
use xiaolin_core::types::{ChatMessage, ChatRequest, ChatResponse, Role, ToolCall};
use xiaolin_protocol::{
    AgentEvent, ContextWarningLevel, ErrorCode, ExecutionMode,
    TokenUsage, ToolCallData, ToolCallFunction, TurnId, TurnSummary, WarningCategory,
};

use xiaolin_evolution::{
    format_candidate_skills_for_prompt, format_skills_for_prompt, infer_task_type, SkillStatus,
    SkillStore, Trajectory, TrajectoryOutcome, TrajectoryStep, TrajectoryStore,
};
#[cfg(feature = "self-iter")]
use xiaolin_self_iter::{SelfIterEngine, ToolCallTrace};
use futures::StreamExt;
use prompt_engine::{PromptContext, PromptEngine, PromptSection};
#[cfg(not(feature = "self-iter"))]
use stream_engine::ToolCallTrace;

use crate::llm::{CompletionParams, LlmProvider};
use base64::Engine as _;

mod accumulator;
pub mod api_errors;
pub mod approval_cache;
pub mod runtimes;
pub mod cache_break_detection;
#[allow(dead_code)] // TODO(integrate): assemble related files at query start
pub mod context_assembly;
pub(crate) mod context_budget;
pub(crate) mod context_compressor;
pub mod cost_tracker;
pub mod file_persistence;
pub mod file_state_cache;
pub mod hook_config;
pub mod hook_events;
pub mod hook_executor;
#[allow(dead_code)]
pub mod lsp_actions;
pub mod magic_docs;
pub mod mode_attachments;
#[allow(dead_code)]
pub mod memory_selection;
pub mod model_critic;
pub(crate) mod observer;
pub mod dispatcher;
pub mod orchestrator;
pub mod permissions;
mod post_compact_restore;
mod prompt_builder;
pub mod prompt_engine;
pub mod prompt_sections;
#[allow(dead_code)] // TODO(integrate): wire into AgentEvent::Suggestions
pub mod prompt_suggestion;
pub(crate) mod query_deps;
pub mod query_engine;
mod query_state;
pub mod retry;
pub(crate) mod runtime_services;
mod session_memory;
#[allow(dead_code)] // TODO(integrate): side-query tool handle for auxiliary LLM calls
pub mod side_query;
mod stop_hooks;
mod stream_engine;
pub mod streaming_tool_executor;
pub mod task_decomposer;
mod tool_executor;
pub mod tool_result_storage;
mod trajectory;
pub mod undo_engine;
mod unified_compact;
#[allow(dead_code)]
pub mod validation_pipeline;

pub use prompt_builder::{build_subagent_prompt_block, ActiveRunSummary, SubAgentPromptContext};

use accumulator::{accumulate_tool_call, ToolCallAccumulator};
use prompt_builder::SKILL_MANAGEMENT_GUIDANCE;
use query_deps::QueryDeps;
use query_state::QueryLoopState;
use stream_engine::send_stream_event;
use tool_executor::filter_tool_definitions;
use tool_executor::semantic_header;
#[allow(deprecated)]
use tool_executor::truncate_tool_result_output_with_limit;
use tool_result_storage::{
    reconstruct_state, ContentReplacementState, ToolResultEntry, ToolResultStorage,
    MAX_TOOL_RESULTS_PER_MESSAGE_CHARS,
};
use trajectory::append_text_to_chat_content;
use trajectory::last_user_turn_text;
use trajectory::truncate_for_trajectory;

/// Track restoration state from tool execution.
/// Extracts file reads, skill invocations, and plan content for post-compact recovery.
fn track_restoration_state(
    restoration_state: &mut post_compact_restore::RestorationState,
    tool_name: &str,
    arguments: &str,
    output: &str,
    success: bool,
) {
    // Only track successful tool executions
    if !success {
        return;
    }

    match tool_name {
        // Track file reads
        "Read" => {
            if let Ok(args) = serde_json::from_str::<serde_json::Value>(arguments) {
                if let Some(path) = args.get("file_path").and_then(|p| p.as_str()) {
                    restoration_state.add_file(std::path::PathBuf::from(path), output.to_string());
                }
            }
        }
        // Track skill invocations
        "Skill" => {
            if let Ok(args) = serde_json::from_str::<serde_json::Value>(arguments) {
                if let Some(skill_name) = args.get("skill").and_then(|s| s.as_str()) {
                    restoration_state.add_skill(
                        skill_name.to_string(),
                        std::path::PathBuf::from(format!(".claude/skills/{}.md", skill_name)),
                        output.to_string(),
                    );
                }
            }
        }
        // Track plan mode - when entering plan mode, mark it
        "EnterPlanMode" => {
            restoration_state.is_plan_mode = true;
        }
        "ExitPlanMode" => {
            restoration_state.is_plan_mode = false;
            // Clear plan content when exiting plan mode
            restoration_state.clear_plan();
        }
        _ => {}
    }
}

/// Create a ToolResultStorage for the current invocation session.
///
/// - With `session_id`: uses `~/.xiaolin/sessions/<session_id>/` so tool results
///   persist across process restarts and can be recovered on session resume.
/// - Without `session_id`: uses an ephemeral temp directory that lives only as
///   long as the current process.
fn create_tool_result_storage(session_id: Option<&str>) -> ToolResultStorage {
    let session_dir = match session_id {
        Some(sid) => dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(".xiaolin")
            .join("sessions")
            .join(sid),
        None => std::env::temp_dir()
            .join("xiaolin_sessions")
            .join(format!("ephemeral_{}", std::process::id())),
    };
    ToolResultStorage::new(session_dir)
}

/// Build the set of tool names whose results should skip budget enforcement.
/// These are tools with `max_result_size_chars() == usize::MAX`.
fn build_skip_tool_names(
    tool_registry: &xiaolin_core::tool::ToolRegistry,
) -> std::collections::HashSet<String> {
    tool_registry
        .tool_names()
        .into_iter()
        .filter(|name| {
            tool_registry
                .get(name)
                .map(|t| t.max_result_size_chars() == usize::MAX)
                .unwrap_or(false)
        })
        .collect()
}

fn classify_stream_error_code(message: &str) -> Option<ErrorCode> {
    Some(ErrorCode::classify(message))
}

/// Process a tool result: try ToolResultStorage.process_result() first,
/// fall back to truncate_tool_result_output_with_limit on error.
#[allow(deprecated)]
fn process_tool_output(
    storage: &ToolResultStorage,
    tool_name: &str,
    call_id: &str,
    output: &str,
    max_result_size_chars: usize,
) -> String {
    let threshold = tool_result_storage::get_persistence_threshold(max_result_size_chars);
    match storage.process_result(tool_name, call_id, output, threshold) {
        Ok(Some(replacement)) => replacement,
        Ok(None) => output.to_string(),
        Err(e) => {
            tracing::warn!(error = %e, tool = tool_name, "ToolResultStorage failed, falling back to truncation");
            truncate_tool_result_output_with_limit(output, tool_name, Some(max_result_size_chars))
        }
    }
}

/// Apply enforce_per_message_budget on messages before sending to LLM.
/// Modifies messages in-place by replacing oversized tool results with previews.
/// Returns any newly created replacement records for session persistence.
fn apply_message_budget(
    storage: &ToolResultStorage,
    messages: &mut [xiaolin_core::types::ChatMessage],
    state: &mut ContentReplacementState,
    skip_tool_names: &std::collections::HashSet<String>,
) -> Vec<tool_result_storage::ContentReplacementRecord> {
    let mut tool_entries: Vec<ToolResultEntry> = Vec::new();
    let mut entry_indices: Vec<usize> = Vec::new();

    for (i, msg) in messages.iter().enumerate() {
        if msg.role == xiaolin_core::types::Role::Tool {
            if let Some(content) = msg.text_content() {
                let tool_use_id = msg.tool_call_id.clone().unwrap_or_default();
                let tool_name = msg.name.clone().unwrap_or_default();
                tool_entries.push(ToolResultEntry {
                    tool_use_id,
                    tool_name,
                    content: content.to_string(),
                });
                entry_indices.push(i);
            }
        }
    }

    if tool_entries.is_empty() {
        return Vec::new();
    }

    let result = storage.enforce_per_message_budget(
        tool_entries,
        state,
        skip_tool_names,
        MAX_TOOL_RESULTS_PER_MESSAGE_CHARS,
    );

    if result.replacements.is_empty() {
        return Vec::new();
    }

    for &idx in &entry_indices {
        let msg = &messages[idx];
        if let Some(tool_call_id) = &msg.tool_call_id {
            if let Some(replacement) = result.replacements.get(tool_call_id) {
                messages[idx].content = Some(serde_json::Value::String(replacement.clone()));
            }
        }
    }

    if !result.newly_replaced.is_empty() {
        tracing::info!(
            count = result.newly_replaced.len(),
            "Per-message budget: persisted tool results"
        );
    }

    result.newly_replaced
}

/// Build ChatMessage content for a tool result. When the result carries images,
/// constructs a multimodal content array so the LLM can visually interpret them.
fn tool_result_content(text: &str, result: &xiaolin_core::tool::ToolResult) -> serde_json::Value {
    if result.images.is_empty() {
        return serde_json::Value::String(text.to_string());
    }
    let mut parts = vec![serde_json::json!({"type": "text", "text": text})];
    for img in &result.images {
        let b64 = base64::engine::general_purpose::STANDARD.encode(&img.data);
        parts.push(serde_json::json!({
            "type": "image_url",
            "image_url": {
                "url": format!("data:{};base64,{b64}", img.mime_type)
            }
        }));
    }
    serde_json::Value::Array(parts)
}

/// Execution result containing the final response and tool-call trace.
pub struct ExecutionResult {
    pub response: ChatResponse,
    pub tool_calls_made: u32,
    pub iterations: u32,
}

/// Shared parameters for both streaming and non-streaming execution paths.
pub struct ExecutionParams<'a> {
    pub config: &'a AgentConfig,
    pub request: &'a ChatRequest,
    pub tool_registry: &'a Arc<ToolRegistry>,
    pub llm_override: Option<Arc<dyn LlmProvider>>,
    /// Pre-built sub-agent prompt block to append to the system message.
    /// Built by the caller (gateway) using `build_subagent_prompt_block`.
    pub subagent_prompt: Option<String>,
    /// Shared execution mode state for plan-mode blocking.
    /// When `Some`, tools of kind Edit/Execute are blocked in Plan mode.
    pub mode_state: Option<crate::builtin_tools::ExecutionModeState>,
    /// Optional session store for persisting content replacement records.
    /// When provided with a session_id, enables byte-stable resume.
    pub session_store: Option<Arc<xiaolin_session::SessionStore>>,
    /// Shared todo store so stop-hooks can check for incomplete todos.
    pub todo_store: Option<crate::builtin_tools::TodoStore>,
}

/// Additional parameters specific to the streaming execution path.
pub struct StreamParams {
    pub tx: tokio::sync::mpsc::Sender<AgentEvent>,
    pub orchestrator: Option<Arc<crate::runtime::orchestrator::ToolOrchestrator>>,
    /// When running inside a Session Actor, this handle lets the orchestrator
    /// wait for approval/answer resolution directly via the actor, bypassing
    /// the legacy DashMap + polling relay.
    pub interaction_handle: Option<xiaolin_session_actor::InteractionHandle>,
    /// Approval strategy for the unified pipeline. Determines how tool
    /// approval is resolved (auto-approve, deny-all, policy-based, interactive).
    pub approval_strategy: xiaolin_core::tool_runtime::ApprovalStrategy,
    /// Runtime registry for guarded tools. When present, tools registered here
    /// go through the orchestrator's 5-phase pipeline instead of direct execution.
    pub runtime_registry: Option<Arc<crate::runtime::runtimes::RuntimeRegistry>>,
}

fn tool_calls_to_data(calls: Vec<ToolCall>) -> Vec<ToolCallData> {
    calls
        .into_iter()
        .map(|tc| ToolCallData {
            id: tc.id,
            call_type: tc.call_type,
            function: ToolCallFunction {
                name: tc.function.name,
                arguments: tc.function.arguments,
            },
            output: tc.output,
            success: tc.success,
            duration_ms: tc.duration_ms,
        })
        .collect()
}

fn make_turn_end_event(
    turn_id: &TurnId,
    request: &ChatRequest,
    state: &QueryLoopState,
    stream_start: std::time::Instant,
    context_window: u32,
    final_tool_calls: Option<Vec<ToolCallData>>,
) -> AgentEvent {
    AgentEvent::TurnEnd {
        turn_id: turn_id.clone(),
        summary: TurnSummary {
            turn_id: turn_id.clone(),
            tool_calls_made: state.total_tool_calls,
            iterations: state.iteration,
            usage: state.build_usage().map(|u| TokenUsage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
                total_tokens: u.total_tokens,
            }),
            elapsed_ms: stream_start.elapsed().as_millis() as u64,
            context_tokens: Some(state.last_estimated_tokens as u32),
            context_window: Some(context_window),
        },
        session_id: request.session_id.clone().map(Into::into),
        final_tool_calls,
    }
}

fn make_turn_summary(
    turn_id: &TurnId,
    state: &QueryLoopState,
    stream_start: std::time::Instant,
    context_window: u32,
) -> TurnSummary {
    TurnSummary {
        turn_id: turn_id.clone(),
        tool_calls_made: state.total_tool_calls,
        iterations: state.iteration,
        usage: state.build_usage().map(|u| TokenUsage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
        }),
        elapsed_ms: stream_start.elapsed().as_millis() as u64,
        context_tokens: Some(state.last_estimated_tokens as u32),
        context_window: Some(context_window),
    }
}

/// Build recovery guidance from a streak of consecutive tool failures.
///
/// Returns `None` if the streak is empty. Otherwise produces actionable
/// suggestions tailored to the failing tool categories.
pub(crate) fn format_basic_recovery_guidance(failure_streak: &[ToolCallTrace]) -> Option<String> {
    if failure_streak.is_empty() {
        return None;
    }

    let mut tool_errors: Vec<String> = Vec::new();
    let mut seen_tools = std::collections::HashSet::new();
    for trace in failure_streak {
        let err = trace.error.as_deref().unwrap_or("unknown error");
        let truncated = if err.len() > 150 {
            format!("{}...", &err[..err.floor_char_boundary(150)])
        } else {
            err.to_string()
        };
        tool_errors.push(format!("- `{}`: {}", trace.tool_name, truncated));
        seen_tools.insert(trace.tool_name.as_str());
    }

    let mut guidance = format!(
        "The following tool calls have failed consecutively:\n{}\n\n",
        tool_errors.join("\n")
    );

    guidance.push_str("Before retrying, consider:\n");
    for tool in &seen_tools {
        match *tool {
            "read_file" | "list_dir" | "list_directory" =>
                guidance.push_str("- File/path errors: verify the path exists, check spelling, use `glob` or `list_dir` to discover correct paths\n"),
            "shell_exec" | "shell" | "run_command" =>
                guidance.push_str("- Command errors: check command syntax, verify required tools are installed, try simpler alternatives\n"),
            "write_file" | "edit_file" | "apply_patch" | "multi_edit" =>
                guidance.push_str("- Write errors: ensure the target directory exists, check permissions, verify the file content/diff is correct\n"),
            "grep" | "ripgrep" =>
                guidance.push_str("- Search errors: simplify the pattern, check regex syntax, try broader search scope\n"),
            _ =>
                guidance.push_str(&format!("- `{tool}` errors: review the error message carefully and try a different approach\n")),
        }
    }
    guidance.push_str("\nDo NOT repeat the same failing calls. Try an alternative approach or explain the issue to the user.");

    Some(guidance)
}

/// Inject tool recovery guidance into the system message, or prepend a new
/// system message if none exists.
pub(crate) fn inject_tool_recovery_guidance(messages: &mut Vec<ChatMessage>, guidance: &str) {
    let block = format!(
        "\n\n---\n[Tool execution recovery — review before your next tool_calls]\n{guidance}\n---\n"
    );
    if let Some(first) = messages.first_mut() {
        if matches!(first.role, Role::System) {
            append_text_to_chat_content(&mut first.content, &block);
            return;
        }
    }
    messages.insert(
        0,
        ChatMessage {
            role: Role::System,
            content: Some(serde_json::Value::String(format!(
                "[Tool execution recovery — review before your next tool_calls]\n{guidance}"
            ))),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
            compact_metadata: None,
        },
    );
}

/// Append a block to the system message (first message), or create one.
fn inject_system_block(messages: &mut Vec<ChatMessage>, block: &str) {
    if let Some(first) = messages.first_mut() {
        if matches!(first.role, Role::System) {
            append_text_to_chat_content(&mut first.content, block);
            return;
        }
    }
    messages.insert(
        0,
        ChatMessage {
            role: Role::System,
            content: Some(serde_json::Value::String(block.to_string())),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
            compact_metadata: None,
        },
    );
}

/// Extract file path from tool arguments JSON (looks for "path" or "file_path" fields).
fn extract_file_path_from_args(arguments: &str) -> Option<std::path::PathBuf> {
    let v: serde_json::Value = serde_json::from_str(arguments).ok()?;
    v.get("path")
        .or_else(|| v.get("file_path"))
        .or_else(|| v.get("file"))
        .and_then(|p| p.as_str())
        .map(std::path::PathBuf::from)
}

/// Manages the execution of a single agent invocation, including
/// the tool-calling loop: LLM → tool_calls → execute → inject result → repeat.
/// Internal key for the default/fallback provider inside `agent_providers`.
const DEFAULT_PROVIDER_KEY: &str = "";

pub struct AgentRuntime {
    agent_providers: ArcSwap<HashMap<String, Arc<dyn LlmProvider>>>,
    prompt_engine: PromptEngine,
    #[cfg(feature = "self-iter")]
    self_iter_engine: Option<Arc<SelfIterEngine>>,
    #[cfg(feature = "self-iter")]
    self_iter_max_recovery_attempts: u32,
    skill_store: ArcSwap<Option<Arc<SkillStore>>>,
    trajectory_store: ArcSwap<Option<Arc<TrajectoryStore>>>,
    cached_runtime_registry: Arc<runtimes::RuntimeRegistry>,
}

impl AgentRuntime {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        let mut initial = HashMap::new();
        initial.insert(DEFAULT_PROVIDER_KEY.to_string(), provider);

        // Kick off background symbol indexing if we're in a workspace.
        if let Ok(root) = std::env::current_dir() {
            let index = crate::symbol_index::SymbolIndex::global().clone();
            crate::symbol_index::start_background_scan(root.clone(), index.clone());
            crate::symbol_index::start_watcher(root, index);
        }

        Self {
            agent_providers: ArcSwap::new(Arc::new(initial)),
            prompt_engine: Self::default_prompt_engine(),
            #[cfg(feature = "self-iter")]
            self_iter_engine: None,
            #[cfg(feature = "self-iter")]
            self_iter_max_recovery_attempts: 3,
            skill_store: ArcSwap::new(Arc::new(None)),
            trajectory_store: ArcSwap::new(Arc::new(None)),
            cached_runtime_registry: Arc::new(runtimes::register_default_runtimes()),
        }
    }

    fn default_prompt_engine() -> PromptEngine {
        use prompt_sections::dynamic::{
            code_context_section, environment_section, frc_section, language_section,
            mcp_instructions_section, memory_section, session_guidance_section,
            token_budget_section,
        };
        use prompt_sections::{
            actions_section, doing_tasks_section, intro_section, output_efficiency_section,
            system_section, tone_and_style_section, using_tools_section,
        };

        let static_sections: Vec<PromptSection> = vec![
            intro_section(),
            system_section(),
            doing_tasks_section(),
            actions_section(),
            using_tools_section(),
            tone_and_style_section(),
            output_efficiency_section(),
        ];

        let dynamic_sections: Vec<PromptSection> = vec![
            session_guidance_section(),
            environment_section(),
            memory_section(),
            language_section(),
            mcp_instructions_section(),
            token_budget_section(),
            code_context_section(),
            frc_section(),
        ];

        PromptEngine::new(static_sections, dynamic_sections)
    }

    pub fn with_skill_store(self, store: Arc<SkillStore>) -> Self {
        self.skill_store.store(Arc::new(Some(store)));
        self
    }

    pub fn with_trajectory_store(self, store: Arc<TrajectoryStore>) -> Self {
        self.trajectory_store.store(Arc::new(Some(store)));
        self
    }

    pub fn attach_evolution_stores(
        &self,
        skill: Arc<SkillStore>,
        trajectory: Arc<TrajectoryStore>,
    ) {
        self.skill_store.store(Arc::new(Some(skill)));
        self.trajectory_store.store(Arc::new(Some(trajectory)));
    }

    #[cfg(feature = "self-iter")]
    pub fn with_self_iter_engine(mut self, engine: Arc<SelfIterEngine>) -> Self {
        self.self_iter_engine = Some(engine);
        self
    }

    #[cfg(feature = "self-iter")]
    pub fn with_self_iter_max_recovery_attempts(mut self, n: u32) -> Self {
        self.self_iter_max_recovery_attempts = n.max(1);
        self
    }

    pub fn provider(&self) -> Arc<dyn LlmProvider> {
        self.default_provider_arc()
    }

    /// Get a shared reference to the default LLM provider.
    pub fn default_provider_arc(&self) -> Arc<dyn LlmProvider> {
        let guard = self.agent_providers.load();
        guard
            .get(DEFAULT_PROVIDER_KEY)
            .cloned()
            .expect("default provider must exist")
    }

    /// Atomically replace the default LLM provider used as fallback when no
    /// per-agent provider is registered.
    pub fn set_default_provider(&self, provider: Arc<dyn LlmProvider>) {
        let mut m = self.agent_providers.load().as_ref().clone();
        m.insert(DEFAULT_PROVIDER_KEY.to_string(), provider);
        self.agent_providers.store(Arc::new(m));
        tracing::info!("default LLM provider hot-swapped");
    }

    pub fn register_provider(&self, agent_id: &str, provider: Arc<dyn LlmProvider>) {
        let mut m = self.agent_providers.load().as_ref().clone();
        m.insert(agent_id.to_string(), provider);
        self.agent_providers.store(Arc::new(m));
    }

    /// Drop all per-agent provider overrides, keeping only the default provider.
    pub fn clear_registered_providers(&self) {
        let guard = self.agent_providers.load();
        let default = guard.get(DEFAULT_PROVIDER_KEY).cloned();
        let mut fresh = HashMap::new();
        if let Some(p) = default {
            fresh.insert(DEFAULT_PROVIDER_KEY.to_string(), p);
        }
        self.agent_providers.store(Arc::new(fresh));
    }

    fn resolve_provider(&self, agent_id: &str) -> anyhow::Result<Arc<dyn LlmProvider>> {
        let guard = self.agent_providers.load();
        guard
            .get(agent_id)
            .or_else(|| guard.get(DEFAULT_PROVIDER_KEY))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no provider found for agent '{agent_id}'"))
    }

    pub async fn execute(
        &self,
        config: &AgentConfig,
        request: &ChatRequest,
        tool_registry: &Arc<ToolRegistry>,
        llm_override: Option<Arc<dyn LlmProvider>>,
    ) -> anyhow::Result<ExecutionResult> {
        self.execute_with_subagent_prompt(config, request, tool_registry, llm_override, None)
            .await
    }

    pub async fn execute_with_subagent_prompt(
        &self,
        config: &AgentConfig,
        request: &ChatRequest,
        tool_registry: &Arc<ToolRegistry>,
        llm_override: Option<Arc<dyn LlmProvider>>,
        subagent_prompt: Option<String>,
    ) -> anyhow::Result<ExecutionResult> {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(512);
        let orchestrator = Arc::new(orchestrator::ToolOrchestrator::new());
        let approval_strategy =
            xiaolin_core::tool_runtime::ApprovalStrategy::AutoApprove;

        let summary = self
            .execute_unified(
                config,
                request,
                tool_registry,
                tx,
                approval_strategy,
                llm_override,
                orchestrator,
                None,
                subagent_prompt,
                None,
                None,
                None,
            )
            .await?;

        // Collect streamed content to reconstruct ExecutionResult
        let mut text = String::new();
        while let Ok(evt) = rx.try_recv() {
            if let AgentEvent::ContentDelta { ref delta, .. } = evt {
                if let Some(content) = delta
                    .get("choices")
                    .and_then(|c| c.get(0))
                    .and_then(|c| c.get("delta"))
                    .and_then(|d| d.get("content"))
                    .and_then(|c| c.as_str())
                {
                    text.push_str(content);
                }
            }
        }

        let model = request
            .model
            .clone()
            .unwrap_or_else(|| config.model.model.clone());
        let usage = summary.usage.map(|u| {
            xiaolin_core::types::Usage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
                total_tokens: u.total_tokens,
            }
        });
        let response = ChatResponse {
            id: summary.turn_id.to_string(),
            object: "chat.completion".to_string(),
            created: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            model,
            choices: vec![xiaolin_core::types::ChatChoice {
                index: 0,
                finish_reason: Some("stop".to_string()),
                message: ChatMessage {
                    role: Role::Assistant,
                    content: Some(serde_json::Value::String(text)),
                    reasoning_content: None,
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                    compact_metadata: None,
                },
            }],
            usage,
        };

        Ok(ExecutionResult {
            response,
            tool_calls_made: summary.tool_calls_made,
            iterations: summary.iterations,
        })
    }

    /// Streaming agentic loop: streams text deltas to the caller while handling
    /// tool calling iterations transparently.
    ///
    /// **Stream resume (best effort):** if the SSE stream yields an error after
    /// some text deltas (e.g. connection drop) and there is no in-flight tool-call
    /// assembly, the partial assistant text is appended to `messages` and the
    /// stream is re-opened on the same turn (bounded retries). The model may
    /// repeat a prefix of the answer; the goal is not to lose prior context.
    pub async fn execute_stream(
        &self,
        config: &AgentConfig,
        request: &ChatRequest,
        tool_registry: &Arc<ToolRegistry>,
        tx: tokio::sync::mpsc::Sender<AgentEvent>,
        llm_override: Option<Arc<dyn LlmProvider>>,
    ) -> anyhow::Result<TurnSummary> {
        let exec = ExecutionParams {
            config,
            request,
            tool_registry,
            llm_override,
            subagent_prompt: None,
            mode_state: None,
            session_store: None,
            todo_store: None,
        };
        let stream = StreamParams {
            tx,
            orchestrator: None,
            interaction_handle: None,
            approval_strategy: xiaolin_core::tool_runtime::ApprovalStrategy::AutoApprove,
            runtime_registry: None,
        };
        self.execute_stream_inner(&exec, stream).await
    }


    /// Unified execution entry point for all callers.
    ///
    /// All entry points (Gateway WS, HTTP, CLI, Feishu, Tauri, SubAgent) should
    /// converge on this method. The `ApprovalStrategy` determines how tool
    /// approval is handled; the `RuntimeRegistry` is used internally by the
    /// orchestrator.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_unified(
        &self,
        config: &AgentConfig,
        request: &ChatRequest,
        tool_registry: &Arc<ToolRegistry>,
        tx: tokio::sync::mpsc::Sender<AgentEvent>,
        approval_strategy: xiaolin_core::tool_runtime::ApprovalStrategy,
        llm_override: Option<Arc<dyn LlmProvider>>,
        orchestrator: Arc<crate::runtime::orchestrator::ToolOrchestrator>,
        interaction_handle: Option<xiaolin_session_actor::InteractionHandle>,
        subagent_prompt: Option<String>,
        mode_state: Option<crate::builtin_tools::ExecutionModeState>,
        session_store: Option<Arc<xiaolin_session::SessionStore>>,
        todo_store: Option<crate::builtin_tools::TodoStore>,
    ) -> anyhow::Result<TurnSummary> {
        let exec = ExecutionParams {
            config,
            request,
            tool_registry,
            llm_override,
            subagent_prompt,
            mode_state,
            session_store,
            todo_store,
        };
        let runtime_registry = self.cached_runtime_registry.clone();
        let stream = StreamParams {
            tx,
            orchestrator: Some(orchestrator),
            interaction_handle,
            approval_strategy,
            runtime_registry: Some(runtime_registry),
        };
        self.execute_stream_inner(&exec, stream).await
    }

    async fn execute_stream_inner(
        &self,
        params: &ExecutionParams<'_>,
        stream_params: StreamParams,
    ) -> anyhow::Result<TurnSummary> {
        let ExecutionParams {
            config,
            request,
            tool_registry,
            ref llm_override,
            subagent_prompt: _,
            ref mode_state,
            ref session_store,
            ref todo_store,
        } = *params;
        let StreamParams {
            ref tx,
            ref orchestrator,
            ref interaction_handle,
            ref approval_strategy,
            ref runtime_registry,
        } = stream_params;
        let turn_id = TurnId::generate();
        let max_iterations = config.behavior.max_tool_calls_per_turn;
        let max_errors = config.behavior.max_consecutive_errors;

        let t0 = std::time::Instant::now();
        let mut messages = self.build_messages(params);
        tracing::info!(
            elapsed_ms = t0.elapsed().as_millis() as u64,
            "perf: build_messages (stream)"
        );

        let t0 = std::time::Instant::now();
        let mut injected_skill_ids: Vec<String> = Vec::new();
        if let Err(e) = self
            .inject_relevant_skills(&mut messages, request, &mut injected_skill_ids)
            .await
        {
            tracing::warn!(error = %e, "skill injection skipped (stream)");
        }
        tracing::info!(
            elapsed_ms = t0.elapsed().as_millis() as u64,
            "perf: inject_relevant_skills (stream)"
        );

        // ── Context Assembly: inject project hints ──────────────────────
        if let Some(ref wd) = request.work_dir {
            let hints = context_assembly::detect_project_hints(std::path::Path::new(wd));
            if !hints.is_empty() {
                let hints_block = format!(
                    "\n─── Project Context ───\n{}\n───────────────────────\n",
                    hints
                        .iter()
                        .map(|h| format!("• {}", h))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
                inject_system_block(&mut messages, &hints_block);
                tracing::info!(
                    hint_count = hints.len(),
                    "context_assembly: project hints injected"
                );
            }
        }

        // ── Extract last user message for downstream injections ─────────
        let last_user_msg = request
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .and_then(|m| m.text_content())
            .unwrap_or_default();

        // ── Task Decomposer: decompose complex requests ─────────────────
        // This block runs once before the tool-call loop, so it naturally
        // only fires on the first iteration. We raise the threshold from 80
        // to 200 chars to avoid unnecessary LLM calls for routine messages.
        if last_user_msg.len() >= 200 {
            let decomp_provider = self.provider();
            let decomp_config = task_decomposer::TaskDecomposerConfig {
                model: config.model.model.clone(),
                ..Default::default()
            };
            if let Some(decomp) =
                task_decomposer::decompose_task(&decomp_provider, &last_user_msg, &decomp_config)
                    .await
            {
                if let Some(block) = task_decomposer::format_decomposition_for_prompt(&decomp) {
                    inject_system_block(&mut messages, &block);
                    tracing::info!(
                        task_type = decomp.task_type.as_str(),
                        steps = decomp.steps.len(),
                        "task_decomposer: plan injected"
                    );
                }
            }
        }

        let mut trajectory_steps: Vec<TrajectoryStep> = Vec::new();
        let t0 = std::time::Instant::now();
        let mode_profile = mode_state
            .as_ref()
            .map(|ms| match ms.current_mode() {
                ExecutionMode::Plan => ToolProfile::plan_mode(),
                _ => ToolProfile::default(),
            })
            .unwrap_or_default();
        let mut all_tool_defs = tool_registry.definitions_with_profile(&mode_profile);
        if let Some(extra) = &request.tools {
            all_tool_defs.extend(extra.iter().cloned());
        }
        let tool_defs = filter_tool_definitions(&all_tool_defs, config);
        let tool_defs_json_chars: usize = tool_defs
            .iter()
            .map(|td| serde_json::to_string(td).map(|s| s.len()).unwrap_or(0))
            .sum();
        let tool_defs_est_tokens = tool_defs_json_chars / 4;
        tracing::info!(
            elapsed_ms = t0.elapsed().as_millis() as u64,
            count = tool_defs.len(),
            json_chars = tool_defs_json_chars,
            est_tokens = tool_defs_est_tokens,
            "perf: tool_definitions (stream)"
        );
        let tools_for_llm = if tool_defs.is_empty() {
            None
        } else {
            Some(tool_defs.as_slice())
        };

        let temperature = request.temperature.unwrap_or(config.model.temperature);
        let model = request
            .model
            .as_deref()
            .unwrap_or(&config.model.model)
            .to_string();
        let mut max_tokens = request.max_tokens.or(config.model.max_tokens).or_else(|| {
            let inferred = xiaolin_context::infer_output_limit_from_model(&model);
            if inferred > 0 {
                Some(inferred)
            } else {
                None
            }
        });

        let mut state = QueryLoopState::new(max_iterations);
        let tool_storage = create_tool_result_storage(request.session_id.as_deref());
        let skip_tool_names = build_skip_tool_names(tool_registry);
        let stream_start = std::time::Instant::now();

        // Load session memory from store if resuming a session
        if let (Some(store), Some(sid)) = (session_store, request.session_id.as_deref()) {
            if let Some(mem) = session_memory::load_session_memory(store.as_ref(), sid).await {
                state.session_memory = Some(mem);
            }
        }

        // Context window — constant across iterations
        let context_window = config.model.context_window.unwrap_or(
            xiaolin_context::infer_context_window_from_model(&config.model.model),
        );

        // QueryDeps: unified dependency injection for LLM calls + compression
        let provider_for_deps: Arc<dyn LlmProvider> = match &llm_override {
            Some(p) => p.clone(),
            None => {
                let r = self.resolve_provider(&config.agent_id);
                r?
            }
        };
        let pipeline_config = xiaolin_context::PipelineConfig {
            snip_max_tokens: context_window as usize,
            reactive_target_tokens: context_window as usize,
            ..Default::default()
        };
        let auto_compact_enabled = pipeline_config.enable_auto_compact;
        let compact_pipeline = xiaolin_context::ContextPipeline::new(pipeline_config);
        let deps = query_deps::ProductionDeps::new(provider_for_deps, compact_pipeline);

        // Reconstruct ContentReplacementState from persisted records on session resume
        let mut replacement_state = Self::load_or_create_replacement_state(
            session_store,
            request.session_id.as_deref(),
            &request.messages,
        )
        .await;

        // RuntimeServices: hook system, cost tracking, magic docs, permissions
        let abort_token = tokio_util::sync::CancellationToken::new();
        let workspace_dir = request.work_dir.as_ref().map(std::path::Path::new);
        let budget_limit = config.behavior.budget_limit_usd;
        let services = runtime_services::RuntimeServices::from_config(
            workspace_dir,
            budget_limit,
            abort_token,
        );

        // ── ValidationPipeline: post-tool output validation ─────────────
        let validation_pipeline = validation_pipeline::ValidationPipeline::default();

        // ── UndoEngine: file snapshot & rollback on consecutive failures ─
        let undo_config = undo_engine::UndoEngineConfig::default();
        let mut undo_engine = undo_engine::UndoEngine::new(undo_config);

        // ── Orchestrator context: per-turn approval cache + denial tracker ──
        let mut orch_approval_cache = approval_cache::ApprovalCache::new();
        let mut orch_denial_tracker = permissions::DenialTracker::new();

        // ── ToolDispatcher: unified tool routing through orchestrator/direct ──
        let rt_reg = runtime_registry
            .as_ref()
            .map(Arc::clone)
            .unwrap_or_else(|| Arc::new(runtimes::register_default_runtimes()));
        let orch = orchestrator
            .as_ref()
            .map(Arc::clone)
            .unwrap_or_else(|| Arc::new(orchestrator::ToolOrchestrator::new()));
        let dispatcher = dispatcher::ToolDispatcher::new(
            Arc::clone(tool_registry),
            rt_reg,
            orch,
        );

        // ── Observer: runtime event collection for evolution pipeline ────
        let runtime_observer = observer::RuntimeObserver::new(
            request.session_id.as_deref().unwrap_or("anonymous"),
            &config.agent_id,
            None,
        );

        // ── CacheBreakDetector: track prompt cache effectiveness ─────────
        let mut cache_detector = cache_break_detection::CacheBreakDetector::new();

        // ── FilePersistence: track all file changes in this session ──────
        let mut file_tracker = file_persistence::SessionFileTracker::new();

        // ── Magic Docs: inject relevant documentation ───────────────────
        {
            let keywords: Vec<&str> = last_user_msg
                .split_whitespace()
                .filter(|w| w.len() > 3)
                .take(10)
                .collect();
            if !keywords.is_empty() {
                let docs_content = services.query_magic_docs(&keywords, 2000);
                if !docs_content.is_empty() {
                    let docs_block = format!(
                        "\n─── Relevant Documentation ───\n{}\n──────────────────────────────\n",
                        docs_content
                    );
                    inject_system_block(&mut messages, &docs_block);
                    tracing::info!(
                        chars = docs_content.len(),
                        "magic_docs: documentation injected"
                    );
                }
            }
        }

        send_stream_event(
            tx,
            AgentEvent::TurnStart {
                turn_id: turn_id.clone(),
                session_id: request.session_id.as_ref().map(|s| s.to_string()),
            },
            false,
        )
        .await;

        loop {
            if let Some(query_state::LoopTransition::Terminal(_)) = state.check_pre_iteration() {
                tracing::warn!(
                    agent_id = %config.agent_id,
                    consecutive_errors = state.consecutive_errors,
                    "stopping outer stream loop — consecutive error limit reached"
                );
                let failure_detail = state.format_failure_summary();
                let user_msg = if failure_detail.is_empty() {
                    format!(
                        "执行过程中遇到连续 {} 次工具错误，已自动停止。请检查工具配置或尝试换一种方式描述任务。",
                        state.consecutive_errors
                    )
                } else {
                    format!(
                        "执行过程中遇到连续工具错误，已自动停止。\n出错的工具调用：\n{}\n\n请检查相关配置或尝试换一种方式。",
                        failure_detail
                    )
                };
                let _ = send_stream_event(
                    tx,
                    AgentEvent::Error {
                        turn_id: turn_id.clone(),
                        message: user_msg.clone(),
                        error_code: None,
                    },
                    false,
                )
                .await;
                self.finalize_injected_skills(&injected_skill_ids, false)
                    .await;
                return Err(anyhow::anyhow!(
                    "agent '{}' stopped: {} consecutive tool errors",
                    config.agent_id,
                    state.consecutive_errors
                ));
            }

            state.begin_iteration();
            state
                .iteration_msg_boundaries
                .push((messages.len(), std::time::Instant::now()));

            // ── Populate plan content for restoration ───────────────────────
            // Read plan file content before compression so it can be restored.
            if let Some(ref session_id) = request.session_id {
                let plan_store = crate::builtin_tools::PlanFileStore::new(None);
                state
                    .restoration_state
                    .populate_plan_from_store(session_id, &plan_store);
            }

            // ── Unified context compaction (via QueryDeps) ─────────────────
            let compact_t0 = std::time::Instant::now();
            let compact_result = deps
                .pre_query_compact(
                    &mut messages,
                    context_window,
                    max_tokens,
                    &model,
                    state.last_estimated_tokens,
                    &state.iteration_msg_boundaries,
                    todo_store.as_ref(),
                    config.behavior.enable_smart_compression,
                    Some(&state.restoration_state),
                    state.session_memory.as_ref(),
                )
                .await;
            tracing::info!(
                elapsed_ms = compact_t0.elapsed().as_millis() as u64,
                iteration = state.iteration,
                "perf: pre_query_compact"
            );
            state.last_estimated_tokens = compact_result.estimated_tokens;
            let estimated_tokens = compact_result.estimated_tokens;

            // Persist session memory if extracted/updated
            if let Some(ref mem) = compact_result.extracted_memory {
                state.session_memory = Some(mem.clone());
                if let (Some(store), Some(sid)) = (session_store, request.session_id.as_deref()) {
                    session_memory::persist_session_memory(store.as_ref(), sid, mem).await;
                }
            }

            // Emit live context usage update to frontend
            let _ = send_stream_event(
                tx,
                AgentEvent::ContextUsageUpdate {
                    turn_id: turn_id.clone(),
                    used_tokens: estimated_tokens as u32,
                    limit_tokens: context_window,
                    compressed: compact_result.compressed_by_llm,
                    tokens_saved: compact_result.tokens_saved_by_llm as u32,
                },
                false,
            )
            .await;

            let usage_ratio = estimated_tokens as f32 / context_window.max(1) as f32;

            // Compact warning at 85%: suggest /compact (sent once per session)
            if usage_ratio > 0.85 && !state.compact_warning_sent {
                state.compact_warning_sent = true;
                let _ = send_stream_event(
                    tx,
                    AgentEvent::ContextWarning {
                        turn_id: turn_id.clone(),
                        level: ContextWarningLevel::Soft,
                        used_tokens: estimated_tokens as u32,
                        limit_tokens: context_window,
                        message: format!(
                            "Context is {:.0}% full ({}/{} tokens). \
                             Run /compact to free space, or the system will auto-compact if enabled.",
                            usage_ratio * 100.0,
                            estimated_tokens,
                            context_window,
                        ),
                    },
                    false,
                ).await;
            }

            // Critical warning at 90%
            if usage_ratio > 0.90 {
                let _ = send_stream_event(
                    tx,
                    AgentEvent::ContextWarning {
                        turn_id: turn_id.clone(),
                        level: ContextWarningLevel::Hard,
                        used_tokens: estimated_tokens as u32,
                        limit_tokens: context_window,
                        message: format!(
                            "Context usage is at {:.0}% ({}/{} tokens). Consider starting a new session.",
                            usage_ratio * 100.0,
                            estimated_tokens,
                            context_window,
                        ),
                    },
                    false,
                ).await;
            }
            if compact_result.compressed_by_llm || compact_result.pipeline_applied {
                let method = if compact_result.compressed_by_llm {
                    "llm"
                } else {
                    "pipeline"
                };
                runtime_observer
                    .record_compact(
                        state.last_estimated_tokens,
                        compact_result.estimated_tokens,
                        method,
                    )
                    .await;
            }

            // Blocking limit: if tokens >= 95% of context window and
            // auto-compact is off, stop and tell the user to run /compact.
            let just_compacted =
                compact_result.compressed_by_llm || compact_result.pipeline_applied;
            if let Some(query_state::LoopTransition::Terminal(
                query_state::TerminalReason::BlockingLimit,
            )) = state.check_blocking_limit(
                estimated_tokens,
                context_window,
                auto_compact_enabled,
                just_compacted,
            ) {
                tracing::warn!(
                    agent_id = %config.agent_id,
                    estimated_tokens,
                    context_window,
                    "blocking limit reached (>= 95% context window) — stopping"
                );
                let _ = send_stream_event(
                    tx,
                    AgentEvent::Error {
                        turn_id: turn_id.clone(),
                        message: format!(
                            "Context window is nearly full ({}/{} tokens, {:.0}%). \
                             Please run /compact to free space, or start a new session.",
                            estimated_tokens,
                            context_window,
                            usage_ratio * 100.0,
                        ),
                        error_code: Some(xiaolin_protocol::ErrorCode::ContextWindowExceeded),
                    },
                    false,
                )
                .await;
                self.finalize_injected_skills(&injected_skill_ids, false)
                    .await;
                return Ok(make_turn_summary(
                    &turn_id,
                    &state,
                    stream_start,
                    context_window,
                ));
            }

            let total_est_with_tools = estimated_tokens + tool_defs_est_tokens;
            tracing::info!(
                agent_id = %config.agent_id,
                model = %model,
                iteration = state.iteration,
                msg_count = messages.len(),
                msg_tokens = estimated_tokens,
                tool_def_tokens = tool_defs_est_tokens,
                total_est = total_est_with_tools,
                context_window,
                "streaming LLM call"
            );

            const MAX_STREAM_RESUME_ATTEMPTS: u32 = 5;
            let mut stream_resume_attempts: u32 = 0;

            let newly_replaced = apply_message_budget(
                &tool_storage,
                &mut messages,
                &mut replacement_state,
                &skip_tool_names,
            );
            Self::persist_replacement_records(
                session_store,
                request.session_id.as_deref(),
                &newly_replaced,
            )
            .await;

            // ── Mode Attachment: inject plan mode instructions per-turn ──────
            if let Some(ref ms) = mode_state {
                if ms.current_mode() == ExecutionMode::Plan {
                    let turn_count = ms.plan_turn_count();
                    let plan_path_str = request.session_id.as_ref().map(|sid| {
                        let ps = crate::builtin_tools::PlanFileStore::new(None);
                        ps.plan_path(sid).display().to_string()
                    });
                    let plan_exists = request.session_id.as_ref().is_some_and(|sid| {
                        let ps = crate::builtin_tools::PlanFileStore::new(None);
                        ps.plan_exists(sid)
                    });
                    let lang: Option<&str> = None;
                    let attachment = mode_attachments::plan_mode_attachment(
                        plan_path_str.as_deref(),
                        plan_exists,
                        lang,
                    );
                    if let Some(text) = attachment.text_for_turn(turn_count) {
                        let mut inject_text = String::new();
                        if turn_count == 0 && ms.has_exited_plan() {
                            inject_text.push_str(&mode_attachments::plan_reentry_notice(lang));
                            inject_text.push('\n');
                        }
                        inject_text.push_str(text);
                        messages.push(ChatMessage {
                            role: Role::User,
                            content: Some(serde_json::Value::String(inject_text)),
                            reasoning_content: None,
                            name: None,
                            tool_calls: None,
                            tool_call_id: None,
                            compact_metadata: None,
                        });
                        tracing::debug!(
                            turn_count,
                            is_reentry = ms.has_exited_plan() && turn_count == 0,
                            "mode_attachment: injected plan mode instructions"
                        );
                    }
                    ms.increment_plan_turn();
                }
            }

            xiaolin_context::compressor::sanitize_tool_call_pairing(&mut messages);
            xiaolin_context::compressor::ensure_valid_assistant_messages(&mut messages);

            if !xiaolin_context::model_supports_vision_with_caps(
                &model,
                config.model.capabilities.as_ref(),
            ) {
                xiaolin_context::compressor::strip_image_content(&mut messages);
            }

            let mut accumulated_content = String::new();
            let mut accumulated_reasoning = String::new();
            let mut tool_call_accum: Vec<ToolCallAccumulator> = Vec::new();
            let mut stream_errored = false;
            let mut force_stop = false;
            let mut last_finish_reason: Option<String> = None;
            let mut withheld_prompt_too_long: Option<String> = None;

            // Streaming tool execution: create executor and track submission state.
            // A new executor is created per iteration since it's consumed via drain_remaining().
            let streaming_exec_enabled = config.behavior.streaming_tool_execution;
            let mut streaming_executor = if streaming_exec_enabled {
                let streaming_plan_fp = crate::builtin_tools::plan_mode::current_plan_context()
                    .map(|pc| pc.store.plan_path(&pc.session_id));
                let exec_config = streaming_tool_executor::StreamingExecutorConfig {
                    sibling_cancel_on_error: true,
                    work_dir: request.work_dir.clone(),
                    behavior: config.behavior.clone(),
                    execution_mode: mode_state.as_ref().map(|ms| ms.current_mode()),
                    plan_file_path: streaming_plan_fp,
                };
                Some(streaming_tool_executor::StreamingToolExecutor::new(
                    Arc::clone(tool_registry),
                    exec_config,
                ))
            } else {
                None
            };
            let mut last_submitted_tool_idx: Option<usize> = None;

            let stream_consume_t0 = std::time::Instant::now();
            'stream_try: loop {
                let params = CompletionParams {
                    model: &model,
                    messages: &messages,
                    temperature,
                    max_tokens,
                    tools: tools_for_llm,
                };

                let llm_call_t0 = std::time::Instant::now();
                tracing::info!(
                    model = %model,
                    msg_count = messages.len(),
                    provider = %deps.provider_name(),
                    "LLM call starting"
                );
                let stream_result = deps.call_model_stream(&params).await;
                let mut stream = match stream_result {
                    Ok(s) => {
                        tracing::info!(
                            elapsed_ms = llm_call_t0.elapsed().as_millis() as u64,
                            "perf: stream_connect_success"
                        );
                        s
                    }
                    Err(e) => {
                        tracing::error!(
                            error = %e,
                            model = %model,
                            provider = %deps.provider_name(),
                            "LLM stream call failed"
                        );
                        if query_state::is_prompt_too_long_error(&e.to_string()) {
                            tracing::warn!(
                                error = %e,
                                "prompt_too_long detected — attempting reactive compaction"
                            );
                            let reactive_result = deps.reactive_compact(&messages);
                            if reactive_result.recovered {
                                tracing::info!(
                                    level = ?reactive_result.level_used,
                                    tokens_after = reactive_result.tokens_after,
                                    "reactive compaction recovered — retrying LLM call"
                                );
                                messages = reactive_result.messages;
                                continue 'stream_try;
                            }
                        }
                        return Err(e);
                    }
                };
                tracing::info!(
                    elapsed_ms = llm_call_t0.elapsed().as_millis() as u64,
                    "perf: stream_connect"
                );

                let mut first_chunk = true;
                let mut should_resume = false;
                let mut delta_count: u64 = 0;
                while let Some(result) = stream.next().await {
                    delta_count += 1;
                    if first_chunk {
                        tracing::info!(
                            elapsed_ms = llm_call_t0.elapsed().as_millis() as u64,
                            "perf: time_to_first_chunk"
                        );
                        first_chunk = false;
                    }
                    let delta = match result {
                        Ok(d) => d,
                        Err(e) => {
                            if query_state::is_prompt_too_long_error(&e.to_string()) {
                                tracing::warn!(
                                    error = %e,
                                    "prompt_too_long during stream — withholding error for recovery attempt"
                                );
                                withheld_prompt_too_long = Some(e.to_string());
                                break;
                            }

                            if tool_call_accum.is_empty()
                                && !accumulated_content.is_empty()
                                && stream_resume_attempts < MAX_STREAM_RESUME_ATTEMPTS
                            {
                                tracing::warn!(
                                    error = %e,
                                    attempt = stream_resume_attempts + 1,
                                    partial_len = accumulated_content.len(),
                                    "streaming LLM interrupted; best-effort resume with partial assistant context"
                                );
                                let rc = std::mem::take(&mut accumulated_reasoning);
                                let partial = std::mem::take(&mut accumulated_content);
                                if !partial.is_empty() || !rc.is_empty() {
                                    messages.push(ChatMessage {
                                        role: Role::Assistant,
                                        content: if partial.is_empty() {
                                            None
                                        } else {
                                            Some(serde_json::Value::String(partial))
                                        },
                                        reasoning_content: if rc.is_empty() {
                                            None
                                        } else {
                                            Some(rc)
                                        },
                                        name: None,
                                        tool_calls: None,
                                        tool_call_id: None,
                                        compact_metadata: None,
                                    });
                                }
                                stream_resume_attempts += 1;
                                should_resume = true;
                                break;
                            }
                            let err_msg = e.to_string();
                            let _ = send_stream_event(
                                tx,
                                AgentEvent::StreamError {
                                    turn_id: turn_id.clone(),
                                    message: err_msg.clone(),
                                    error_code: classify_stream_error_code(&err_msg),
                                    retry_attempt: stream_resume_attempts,
                                },
                                false,
                            )
                            .await;
                            stream_errored = true;
                            break;
                        }
                    };

                    if delta_count <= 3 || delta.choices.is_empty() {
                        let preview_content = delta
                            .choices
                            .first()
                            .and_then(|c| c.delta.content.as_deref());
                        let preview_rc = delta
                            .choices
                            .first()
                            .and_then(|c| c.delta.reasoning_content.as_deref());
                        let has_tc = delta
                            .choices
                            .first()
                            .map(|c| c.delta.tool_calls.is_some())
                            .unwrap_or(false);
                        let fr = delta
                            .choices
                            .first()
                            .and_then(|c| c.finish_reason.as_deref());
                        tracing::info!(
                            delta_count,
                            choices_len = delta.choices.len(),
                            content_preview = ?preview_content.map(|s| &s[..s.floor_char_boundary(60)]),
                            reasoning_preview = ?preview_rc.map(|s| &s[..s.floor_char_boundary(60)]),
                            has_tool_calls = has_tc,
                            finish_reason = ?fr,
                            has_usage = delta.usage.is_some(),
                            "stream delta inspect"
                        );
                    }

                    if let Some(choice) = delta.choices.first() {
                        if let Some(ref content) = choice.delta.content {
                            accumulated_content.push_str(content);
                        }
                        if let Some(ref rc) = choice.delta.reasoning_content {
                            accumulated_reasoning.push_str(rc);
                        }

                        if let Some(ref tc_deltas) = choice.delta.tool_calls {
                            for tc_delta in tc_deltas {
                                // In streaming mode: when a new tool index appears, all
                                // prior tools are fully accumulated and can start executing.
                                // Guarded tools (in RuntimeRegistry) are NOT submitted here
                                // — they'll go through orchestrator after stream completes.
                                if let Some(ref mut executor) = streaming_executor {
                                    let new_idx = tc_delta.index as usize;
                                    let submit_start =
                                        last_submitted_tool_idx.map(|i| i + 1).unwrap_or(0);
                                    if new_idx > 0 && submit_start < new_idx {
                                        for si in submit_start..new_idx {
                                            if let Some(acc) = tool_call_accum.get(si) {
                                                if !acc.name.is_empty() {
                                                    if !runtime_registry.as_ref().is_some_and(|r| r.has(&acc.name)) {
                                                        executor.add_tool(acc.to_tool_call());
                                                    }
                                                    last_submitted_tool_idx = Some(si);
                                                }
                                            }
                                        }
                                    }
                                }
                                accumulate_tool_call(&mut tool_call_accum, tc_delta);
                            }
                        }

                        if let Some(ref reason) = choice.finish_reason {
                            last_finish_reason = Some(reason.clone());
                        }
                    }

                    if let Some(ref u) = delta.usage {
                        state.acc_prompt_tokens += u.prompt_tokens;
                        state.acc_completion_tokens += u.completion_tokens;
                        if u.prompt_tokens > 0 {
                            state.last_estimated_tokens = u.prompt_tokens as usize;
                        }

                        // ── Cost tracker: record LLM usage ──
                        if u.prompt_tokens > 0 || u.completion_tokens > 0 {
                            let call_usage = cost_tracker::CallUsage {
                                model: model.clone(),
                                prompt_tokens: u.prompt_tokens,
                                completion_tokens: u.completion_tokens,
                                cache_read_tokens: 0,
                                cache_creation_tokens: 0,
                            };
                            if let Some(alert) = services.record_llm_usage(call_usage).await {
                                match alert {
                                    cost_tracker::BudgetAlert::Warning => {
                                        let cost = services.accumulated_cost_usd().await;
                                        let _ = send_stream_event(
                                            tx,
                                            AgentEvent::Warning {
                                                turn_id: turn_id.clone(),
                                                message: format!(
                                                    "Budget warning: accumulated cost ${:.4} is approaching the limit.",
                                                    cost,
                                                ),
                                                category: WarningCategory::Budget,
                                            },
                                            false,
                                        ).await;
                                    }
                                    cost_tracker::BudgetAlert::Exceeded => {
                                        let cost = services.accumulated_cost_usd().await;
                                        let _ = send_stream_event(
                                            tx,
                                            AgentEvent::Error {
                                                turn_id: turn_id.clone(),
                                                message: format!(
                                                    "Budget exceeded: accumulated cost ${:.4}. Stopping execution.",
                                                    cost,
                                                ),
                                                error_code: Some(xiaolin_protocol::ErrorCode::UsageLimitExceeded),
                                            },
                                            false,
                                        ).await;
                                        force_stop = true;
                                    }
                                }
                            }

                            // ── Observer: record LLM call ──
                            runtime_observer
                                .record_llm_call(
                                    &model,
                                    u.prompt_tokens,
                                    u.completion_tokens,
                                    llm_call_t0.elapsed(),
                                )
                                .await;

                            // ── CacheBreakDetector: check for cache invalidation ──
                            let cache_usage = cache_break_detection::CacheAwareUsage {
                                prompt_tokens: u.prompt_tokens,
                                completion_tokens: u.completion_tokens,
                                cache_read_tokens: 0,
                                cache_creation_tokens: 0,
                            };
                            let cache_snapshot =
                                cache_detector.pre_call_snapshot("", "", &model, false, false);
                            if let Some(report) =
                                cache_detector.post_call_analyze(&cache_snapshot, &cache_usage)
                            {
                                tracing::warn!(
                                    cause = %report.summary(),
                                    "cache_break_detection: prompt cache break detected"
                                );
                            }
                        }
                    }

                    if tool_call_accum.is_empty() {
                        let _ = send_stream_event(
                            tx,
                            AgentEvent::ContentDelta {
                                turn_id: turn_id.clone(),
                                delta: serde_json::to_value(&delta).unwrap_or_default(),
                                raw_bytes: delta.raw_sse_json.clone(),
                            },
                            true,
                        )
                        .await;
                    }
                }

                if stream_errored {
                    break 'stream_try;
                }
                if should_resume {
                    continue 'stream_try;
                }
                break 'stream_try;
            }

            tracing::info!(
                elapsed_ms = stream_consume_t0.elapsed().as_millis() as u64,
                agent_id = %config.agent_id,
                iteration = state.iteration,
                accumulated_content_len = accumulated_content.len(),
                accumulated_reasoning_len = accumulated_reasoning.len(),
                tool_calls_count = tool_call_accum.len(),
                last_finish_reason = ?last_finish_reason,
                stream_errored,
                "perf: stream_consumed"
            );

            // Withheld prompt_too_long recovery: attempt reactive compact
            // before surfacing the error to the client.
            if let Some(ref withheld_err) = withheld_prompt_too_long {
                if !state.has_attempted_reactive_compact {
                    state.has_attempted_reactive_compact = true;
                    let reactive_result = deps.reactive_compact(&messages);
                    if reactive_result.recovered {
                        tracing::info!(
                            level = ?reactive_result.level_used,
                            tokens_after = reactive_result.tokens_after,
                            "withheld prompt_too_long recovered via reactive compact — retrying"
                        );
                        messages = reactive_result.messages;
                        continue;
                    }
                }
                tracing::error!(
                    error = %withheld_err,
                    "withheld prompt_too_long: reactive compact failed — yielding error to client"
                );
                let _ = send_stream_event(
                    tx,
                    AgentEvent::Error {
                        turn_id: turn_id.clone(),
                        message: withheld_err.clone(),
                        error_code: None,
                    },
                    false,
                )
                .await;
                self.finalize_injected_skills(&injected_skill_ids, false)
                    .await;
                return Err(anyhow::anyhow!("prompt_too_long: recovery failed"));
            }

            if stream_errored {
                self.finalize_injected_skills(&injected_skill_ids, false)
                    .await;
                return Err(anyhow::anyhow!(
                    "provider stream error (already sent to client)"
                ));
            }

            if force_stop {
                tracing::warn!("budget exceeded — stopping execution");
                self.finalize_injected_skills(&injected_skill_ids, false)
                    .await;
                return Ok(make_turn_summary(
                    &turn_id,
                    &state,
                    stream_start,
                    context_window,
                ));
            }

            // max_output_tokens recovery: when finish_reason=length and no
            // tool calls, the model's output was truncated by the token limit.
            // Escalate max_tokens and retry up to MAX_OUTPUT_TOKENS_RECOVERY_LIMIT times.
            let has_valid_tool_calls = tool_call_accum.iter().any(|a| !a.name.is_empty());
            if last_finish_reason.as_deref() == Some("length") && !has_valid_tool_calls {
                if let Some(query_state::LoopTransition::Continue(
                    query_state::ContinueReason::MaxOutputTokensRecovery,
                )) = state.try_max_output_tokens_recovery()
                {
                    let escalated = query_state::ESCALATED_MAX_TOKENS;
                    tracing::warn!(
                        agent_id = %config.agent_id,
                        attempt = state.max_output_tokens_recovery_count,
                        escalated_max_tokens = escalated,
                        "max_output_tokens recovery — retrying with escalated limit"
                    );
                    max_tokens = Some(escalated);
                    let rc = std::mem::take(&mut accumulated_reasoning);
                    let partial = std::mem::take(&mut accumulated_content);
                    if !partial.is_empty() || !rc.is_empty() {
                        messages.push(ChatMessage {
                            role: Role::Assistant,
                            content: if partial.is_empty() {
                                None
                            } else {
                                Some(serde_json::Value::String(partial))
                            },
                            reasoning_content: if rc.is_empty() { None } else { Some(rc) },
                            name: None,
                            tool_calls: None,
                            tool_call_id: None,
                            compact_metadata: None,
                        });
                    }
                    continue;
                }
            }

            let transition = state.determine_post_llm_transition(has_valid_tool_calls);

            match transition {
                query_state::LoopTransition::Terminal(ref reason) => {
                    if matches!(reason, query_state::TerminalReason::EndTurn) {
                        // ── Model Critic: review final output before accepting ──
                        let critic_config = model_critic::CriticConfig {
                            model: config.model.model.clone(),
                            ..Default::default()
                        };
                        if critic_config.enabled && !accumulated_content.is_empty() {
                            let critic_provider = self.provider();
                            let task_type =
                                task_decomposer::TaskType::from_str_loose_pub(&last_user_msg);
                            if let Some(review) = model_critic::run_critic(
                                &critic_provider,
                                task_type,
                                &accumulated_content,
                                &critic_config,
                            )
                            .await
                            {
                                if !review.approved {
                                    if let Some(feedback) = review.format_for_injection() {
                                        tracing::info!(
                                            issues = review.issues.len(),
                                            "model_critic: output rejected, injecting feedback"
                                        );
                                        if !accumulated_content.is_empty() || !accumulated_reasoning.is_empty() {
                                            messages.push(ChatMessage {
                                                role: Role::Assistant,
                                                content: if accumulated_content.is_empty() {
                                                    None
                                                } else {
                                                    Some(serde_json::Value::String(
                                                        std::mem::take(&mut accumulated_content),
                                                    ))
                                                },
                                                reasoning_content: if accumulated_reasoning.is_empty() {
                                                    None
                                                } else {
                                                    Some(std::mem::take(&mut accumulated_reasoning))
                                                },
                                                name: None,
                                                tool_calls: None,
                                                tool_call_id: None,
                                                compact_metadata: None,
                                            });
                                        }
                                        inject_tool_recovery_guidance(&mut messages, &feedback);
                                        continue;
                                    }
                                }
                            }
                        }

                        let hook_result = stop_hooks::evaluate_stop_hooks(
                            &accumulated_content,
                            last_finish_reason.as_deref(),
                            todo_store.as_ref(),
                            &[],
                        )
                        .await;

                        if hook_result.should_continue {
                            tracing::info!(
                                agent_id = %config.agent_id,
                                reason = hook_result.reason,
                                "stop hook triggered continuation"
                            );
                            if !accumulated_content.is_empty() || !accumulated_reasoning.is_empty() {
                                messages.push(ChatMessage {
                                    role: Role::Assistant,
                                    content: if accumulated_content.is_empty() {
                                        None
                                    } else {
                                        Some(serde_json::Value::String(
                                            std::mem::take(&mut accumulated_content),
                                        ))
                                    },
                                    reasoning_content: if accumulated_reasoning.is_empty() {
                                        None
                                    } else {
                                        Some(std::mem::take(&mut accumulated_reasoning))
                                    },
                                    name: None,
                                    tool_calls: None,
                                    tool_call_id: None,
                                    compact_metadata: None,
                                });
                            }
                            if let Some(msg) = hook_result.continuation_message {
                                messages.push(ChatMessage {
                                    role: Role::User,
                                    content: Some(serde_json::Value::String(msg)),
                                    reasoning_content: None,
                                    name: None,
                                    tool_calls: None,
                                    tool_call_id: None,
                                    compact_metadata: None,
                                });
                            }
                            continue;
                        }
                    }

                    let final_tc: Option<Vec<ToolCall>> =
                        if matches!(reason, query_state::TerminalReason::MaxIterations) {
                            let tc: Vec<ToolCall> = tool_call_accum
                                .iter()
                                .filter(|a| !a.name.is_empty())
                                .map(|a| a.to_tool_call())
                                .collect();
                            if tc.is_empty() {
                                None
                            } else {
                                Some(tc)
                            }
                        } else {
                            None
                        };
                    if matches!(reason, query_state::TerminalReason::MaxIterations) {
                        tracing::warn!(
                            agent_id = %config.agent_id,
                            max_iterations,
                            "streaming tool call limit reached — requesting progress summary"
                        );

                        messages.push(ChatMessage {
                            role: Role::User,
                            content: Some(serde_json::Value::String(
                                "[SYSTEM] Tool call limit reached. You MUST now:\n\
                                 1. Summarize your progress so far\n\
                                 2. List any unfinished tasks\n\
                                 3. Explain what remains to be done\n\
                                 Do NOT call any tools — just output text."
                                    .to_string(),
                            )),
                            reasoning_content: None,
                            name: None,
                            tool_calls: None,
                            tool_call_id: None,
                            compact_metadata: None,
                        });

                        let summary_params = CompletionParams {
                            model: &model,
                            messages: &messages,
                            temperature: 0.0,
                            max_tokens: Some(2048),
                            tools: None,
                        };
                        if let Ok(resp) = self.provider().chat_completion(&summary_params).await {
                            if let Some(text) =
                                resp.choices.first().and_then(|c| c.message.text_content())
                            {
                                use xiaolin_core::types::{
                                    DeltaContent, StreamChoice, StreamDelta,
                                };
                                let summary_delta = StreamDelta {
                                    id: String::new(),
                                    object: "chat.completion.chunk".to_string(),
                                    created: 0,
                                    model: model.clone(),
                                    choices: vec![StreamChoice {
                                        index: 0,
                                        delta: DeltaContent {
                                            role: Some(Role::Assistant),
                                            content: Some(text.into_owned()),
                                            reasoning_content: None,
                                            tool_calls: None,
                                        },
                                        finish_reason: Some("stop".to_string()),
                                    }],
                                    usage: None,
                                    raw_sse_json: None,
                                };
                                let _ = send_stream_event(
                                    tx,
                                    AgentEvent::ContentDelta {
                                        turn_id: turn_id.clone(),
                                        delta: serde_json::to_value(&summary_delta)
                                            .unwrap_or_default(),
                                        raw_bytes: None,
                                    },
                                    false,
                                )
                                .await;
                            }
                        }
                    }
                    // Auto-exit Plan mode when turn ends without a plan file
                    if let Some(ref ms) = mode_state {
                        if ms.current_mode() == ExecutionMode::Plan {
                            let has_plan = request.session_id.as_ref().is_some_and(|sid| {
                                let ps = crate::builtin_tools::PlanFileStore::new(None);
                                ps.plan_exists(sid)
                            });
                            if !has_plan {
                                ms.transition(ExecutionMode::Agent);
                                tracing::info!(
                                    agent_id = %config.agent_id,
                                    "auto-exited Plan mode — no plan file produced, returning to Agent mode"
                                );
                            }
                        }
                    }

                    tracing::info!(
                        agent_id = %config.agent_id,
                        reason = %reason,
                        iterations = state.iteration,
                        total_tool_calls = state.total_tool_calls,
                        content_len = accumulated_content.len(),
                        "streaming execution complete — sending Done"
                    );
                    let final_tool_calls = final_tc.map(tool_calls_to_data);
                    let _ = send_stream_event(
                        tx,
                        make_turn_end_event(
                            &turn_id,
                            request,
                            &state,
                            stream_start,
                            context_window,
                            final_tool_calls,
                        ),
                        false,
                    )
                    .await;
                    services.fire_stop_hooks(&messages, &[]).await;
                    self.finalize_injected_skills(&injected_skill_ids, true)
                        .await;
                    self.record_completed_trajectory(request, config, &trajectory_steps, true)
                        .await;
                    let _obs = runtime_observer.summary().await;
                    runtime_observer
                        .clone()
                        .finalize(xiaolin_evolution::TrajectoryOutcome::Success {
                            user_rating: None,
                        })
                        .await;
                    return Ok(make_turn_summary(
                        &turn_id,
                        &state,
                        stream_start,
                        context_window,
                    ));
                }
                query_state::LoopTransition::Continue(_) => {}
            }

            let assembled_calls: Vec<ToolCall> = tool_call_accum
                .iter()
                .filter(|a| !a.name.is_empty())
                .map(|a| a.to_tool_call())
                .collect();

            if assembled_calls.is_empty() {
                tracing::warn!("stream tool call deltas produced no valid tool calls, stopping");
                let _ = send_stream_event(
                    tx,
                    make_turn_end_event(
                        &turn_id,
                        request,
                        &state,
                        stream_start,
                        context_window,
                        None,
                    ),
                    false,
                )
                .await;
                services.fire_stop_hooks(&messages, &[]).await;
                self.finalize_injected_skills(&injected_skill_ids, true)
                    .await;
                self.record_completed_trajectory(request, config, &trajectory_steps, true)
                    .await;
                return Ok(make_turn_summary(
                    &turn_id,
                    &state,
                    stream_start,
                    context_window,
                ));
            }

            messages.push(ChatMessage {
                role: Role::Assistant,
                content: if accumulated_content.is_empty() {
                    None
                } else {
                    Some(serde_json::Value::String(accumulated_content.clone()))
                },
                reasoning_content: if accumulated_reasoning.is_empty() {
                    None
                } else {
                    Some(accumulated_reasoning.clone())
                },
                name: None,
                tool_calls: Some(assembled_calls.clone()),
                tool_call_id: None,
                compact_metadata: None,
            });

            // Emit ToolExecuting events for all tool calls first.
            for tc in &assembled_calls {
                let args_str = if tc.function.arguments.is_empty() {
                    None
                } else {
                    Some(tc.function.arguments.clone())
                };
                let _ = send_stream_event(
                    tx,
                    AgentEvent::ToolExecuting {
                        turn_id: turn_id.clone(),
                        tool_name: tc.function.name.clone(),
                        call_id: tc.id.clone(),
                        args: args_str,
                    },
                    false,
                )
                .await;
            }

            let mode_before = mode_state.as_ref().map(|ms| ms.current_mode());

            // Execute tool calls through the ToolDispatcher (unified pipeline).
            //
            // When a streaming executor exists, drain it for tools already submitted
            // and route remaining guarded tools through dispatch_one.
            // When no streaming executor, dispatch_batch handles everything.
            let tool_dispatch_t0 = std::time::Instant::now();
            let tool_count = assembled_calls.len();
            let stream_results = if let Some(mut executor) = streaming_executor.take() {
                // Streaming path: some tools were already submitted during streaming.
                let submit_start = last_submitted_tool_idx.map(|i| i + 1).unwrap_or(0);
                for tc in &assembled_calls[submit_start..] {
                    if !dispatcher.is_guarded(&tc.function.name) && !tc.function.name.is_empty() {
                        executor.add_tool(tc.clone());
                    }
                }
                let completed = executor.drain_remaining().await;
                let mut all_results: Vec<Option<(String, String, String, xiaolin_core::tool::ToolResult)>> =
                    vec![None; assembled_calls.len()];

                // Place streaming results
                let mut completed_iter = completed.into_iter()
                    .map(|ct| (ct.tool_name, ct.call_id, ct.result));
                for (i, tc) in assembled_calls.iter().enumerate() {
                    if !dispatcher.is_guarded(&tc.function.name) {
                        if let Some((name, id, result)) = completed_iter.next() {
                            all_results[i] = Some((name, id, tc.function.arguments.clone(), result));
                        }
                    }
                }

                // Dispatch guarded tools through ToolDispatcher
                let plan_file_path_for_ctx = crate::builtin_tools::plan_mode::current_plan_context()
                    .map(|pc| pc.store.plan_path(&pc.session_id));
                for (i, tc) in assembled_calls.iter().enumerate() {
                    if dispatcher.is_guarded(&tc.function.name) {
                        let mut dispatch_ctx = dispatcher::DispatchContext {
                            turn_id: &turn_id,
                            behavior: &config.behavior,
                            work_dir: &request.work_dir,
                            mode_state: mode_state.as_ref(),
                            plan_file_path: plan_file_path_for_ctx.clone(),
                            event_tx: tx,
                            approval_strategy,
                            interaction_handle: interaction_handle.as_ref(),
                            approval_cache: &mut orch_approval_cache,
                            denial_tracker: &mut orch_denial_tracker,
                            agent_id: &config.agent_id,
                        };
                        let result = dispatcher.dispatch_one(tc, &mut dispatch_ctx).await;
                        all_results[i] = Some(result);
                    }
                }

                all_results.into_iter().flatten().collect::<Vec<_>>()
            } else {
                // Non-streaming path: dispatch_batch handles everything.
                let plan_file_path_batch = crate::builtin_tools::plan_mode::current_plan_context()
                    .map(|pc| pc.store.plan_path(&pc.session_id));
                let mut dispatch_ctx = dispatcher::DispatchContext {
                    turn_id: &turn_id,
                    behavior: &config.behavior,
                    work_dir: &request.work_dir,
                    mode_state: mode_state.as_ref(),
                    plan_file_path: plan_file_path_batch,
                    event_tx: tx,
                    approval_strategy,
                    interaction_handle: interaction_handle.as_ref(),
                    approval_cache: &mut orch_approval_cache,
                    denial_tracker: &mut orch_denial_tracker,
                    agent_id: &config.agent_id,
                };
                dispatcher.dispatch_batch(&assembled_calls, &mut dispatch_ctx).await
            };
            tracing::info!(
                elapsed_ms = tool_dispatch_t0.elapsed().as_millis() as u64,
                tool_count,
                "perf: tool_dispatch"
            );

            let mut force_stop_loop = false;
            let mut plan_approval_pending = false;
            for (tool_name, call_id, arguments, mut result) in stream_results {
                let tool_start_time = std::time::Instant::now();
                state.total_tool_calls += 1;
                let rep_action = state.record_tool_call(&tool_name, &arguments);

                // ── Permission check ──
                if let Some(permissions::PermissionDecision::Denied(reason)) =
                    services.check_permission(&tool_name)
                {
                    let msg = reason.unwrap_or_else(|| {
                        format!("Tool '{}' is denied by permission rules", tool_name)
                    });
                    tracing::warn!(tool = %tool_name, %msg, "tool blocked by permission engine");
                    result = xiaolin_core::tool::ToolResult::err(&msg);
                }

                // ── UndoEngine + FilePersistence: capture file snapshot before edit ──
                if matches!(
                    tool_name.as_str(),
                    "edit_file" | "write_file" | "create_file" | "str_replace_editor"
                ) {
                    if let Some(file_path) = extract_file_path_from_args(&arguments) {
                        let file_exists = file_path.exists();
                        if let Ok(content) = std::fs::read_to_string(&file_path) {
                            undo_engine.capture_before_edit(&file_path, &content);
                        }
                        let op = if file_exists {
                            file_persistence::FileOp::Modified
                        } else {
                            file_persistence::FileOp::Created
                        };
                        file_tracker.record(file_path, op, &tool_name);
                    }
                }

                // ── Pre-tool hook ──
                let input_json: serde_json::Value =
                    serde_json::from_str(&arguments).unwrap_or_default();
                if let Some(hook_result) = services
                    .fire_pre_tool_hooks(&tool_name, &call_id, &input_json)
                    .await
                {
                    if let Some(err) = hook_result.blocking_error {
                        tracing::warn!(tool = %tool_name, %err, "tool blocked by pre-hook");
                        result = xiaolin_core::tool::ToolResult::err(&err);
                    }
                }

                // ── Track restoration state for post-compact recovery ──
                track_restoration_state(
                    &mut state.restoration_state,
                    &tool_name,
                    &arguments,
                    &result.output,
                    result.success,
                );

                // ── Post-tool hook (fire-and-forget) ──
                let output_json =
                    serde_json::Value::String(result.output.chars().take(2000).collect::<String>());
                services
                    .fire_post_tool_hooks(
                        &tool_name,
                        &call_id,
                        &input_json,
                        &output_json,
                        tool_start_time.elapsed(),
                    )
                    .await;

                // ── Observer: record tool call observation ──
                runtime_observer
                    .record_tool_call(
                        &tool_name,
                        result.success,
                        tool_start_time.elapsed(),
                        &result.output.chars().take(200).collect::<String>(),
                    )
                    .await;

                trajectory_steps.push(TrajectoryStep {
                    role: "assistant".into(),
                    action_type: "tool_call".into(),
                    tool_name: Some(tool_name.clone()),
                    summary: truncate_for_trajectory(&arguments),
                    success: None,
                });

                match rep_action {
                    query_state::ToolRepetitionAction::ForceStop => {
                        if let Some(nudge) = state.build_repetition_nudge(true) {
                            inject_tool_recovery_guidance(&mut messages, &nudge);
                        }
                        force_stop_loop = true;
                    }
                    query_state::ToolRepetitionAction::Warn => {
                        if let Some(nudge) = state.build_repetition_nudge(false) {
                            inject_tool_recovery_guidance(&mut messages, &nudge);
                        }
                    }
                    query_state::ToolRepetitionAction::None => {}
                }

                // NOTE: The legacy `needs_confirmation` block has been removed.
                // Guarded tools now go through orchestrator.run() which handles
                // approval internally before execution.

                if !result.success {
                    state.record_tool_error(&tool_name, &result.output);
                    undo_engine.record_failure(&tool_name);
                } else {
                    state.clear_error_streak();
                    undo_engine.record_success();
                }

                // ── ValidationPipeline: append findings to tool output ───
                let work_dir_for_validation = request
                    .work_dir
                    .as_ref()
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| std::path::PathBuf::from("."));
                let val_ctx = validation_pipeline::ValidationContext {
                    tool_name: &tool_name,
                    arguments: &arguments,
                    output: &result.output,
                    success: result.success,
                    work_dir: &work_dir_for_validation,
                };
                let val_result = validation_pipeline.validate(&val_ctx);
                let validation_suffix = if !val_result.findings.is_empty() {
                    let msgs: Vec<String> = val_result
                        .findings
                        .iter()
                        .map(|f| format!("[{:?}] {}", f.severity, f.message))
                        .collect();
                    tracing::info!(
                        tool = %tool_name,
                        findings = msgs.len(),
                        "validation_pipeline: findings appended"
                    );
                    format!("\n\n─── Validation Findings ───\n{}", msgs.join("\n"))
                } else {
                    String::new()
                };

                // ── UndoEngine: rollback on excessive failures ───────────
                if undo_engine.should_rollback() {
                    if let Some(rb) = undo_engine.execute_rollback() {
                        for file_path in &rb.restored_files {
                            if let Some(content) = undo_engine.get_restore_content(file_path) {
                                let _ = std::fs::write(file_path, content);
                            }
                        }
                        inject_tool_recovery_guidance(&mut messages, &rb.guidance);
                        tracing::warn!(
                            restored = rb.restored_files.len(),
                            "undo_engine: auto-rollback triggered"
                        );
                    }
                }

                let max_chars = tool_registry
                    .get(&tool_name)
                    .map(|t| t.max_result_size_chars())
                    .unwrap_or(100_000);
                let tool_output_with_validation = if validation_suffix.is_empty() {
                    result.output.clone()
                } else {
                    format!("{}{}", result.output, validation_suffix)
                };
                let processed = process_tool_output(
                    &tool_storage,
                    &tool_name,
                    &call_id,
                    &tool_output_with_validation,
                    max_chars,
                );
                let header = semantic_header(
                    &tool_name,
                    &arguments,
                    &tool_output_with_validation,
                    result.success,
                );
                let llm_out = format!("{header}\n{processed}");
                let _ = send_stream_event(
                    tx,
                    AgentEvent::ToolResult {
                        turn_id: turn_id.clone(),
                        tool_name: tool_name.clone(),
                        call_id: call_id.clone(),
                        output: result.ui_output().to_string(),
                        display_output: result.display_output.clone(),
                        success: result.success,
                        metadata: result.metadata.clone(),
                    },
                    false,
                )
                .await;

                trajectory_steps.push(TrajectoryStep {
                    role: "tool".into(),
                    action_type: "tool_result".into(),
                    tool_name: Some(tool_name.clone()),
                    summary: truncate_for_trajectory(&result.output),
                    success: Some(result.success),
                });

                if result.metadata.as_ref().and_then(|m| m.get("approval_pending")).and_then(|v| v.as_bool()) == Some(true) {
                    tracing::info!(
                        agent_id = %config.agent_id,
                        tool = %tool_name,
                        "plan approval pending — ending turn to wait for user decision"
                    );
                    plan_approval_pending = true;
                }

                let content = tool_result_content(&llm_out, &result);
                messages.push(ChatMessage {
                    role: Role::Tool,
                    content: Some(content),
                    name: Some(tool_name.clone()),
                    tool_call_id: Some(call_id),
                    ..Default::default()
                });

                // ── Auto-fix loop (streaming path) ──
                if !result.success {
                    if let Some(build_cmd) =
                        crate::autofix::extract_build_command(&tool_name, &arguments)
                    {
                        if let Some(guide) = crate::autofix::detect_and_plan(
                            &build_cmd,
                            &result.output,
                            -1,
                            state.autofix.iteration,
                        ) {
                            let error_count_for_state = guide.diagnostics.len();
                            state.autofix.record_build_result(
                                &build_cmd,
                                -1,
                                error_count_for_state,
                            );
                            inject_tool_recovery_guidance(&mut messages, &guide.formatted);
                            tracing::info!(
                                compiler = %crate::autofix::compiler_name(guide.compiler),
                                errors = error_count_for_state,
                                iteration = guide.iteration,
                                "auto-fix guidance injected (stream)"
                            );
                        }
                    }
                } else if crate::autofix::extract_build_command(&tool_name, &arguments).is_some() {
                    state.autofix.reset();
                }

                if self.try_self_iter_tool_recovery(
                    &mut messages,
                    config,
                    request,
                    state.iteration,
                    state.consecutive_errors,
                    max_errors,
                    &state.failure_streak_traces,
                    &mut state.self_iter_recovery_used,
                ) {
                    state.clear_error_streak();
                }

                let failure_summary = state.format_failure_summary();
                let error_count = state.consecutive_errors;
                if let Some(transition) = state.check_error_limit(max_errors) {
                    match transition {
                        query_state::LoopTransition::Terminal(
                            query_state::TerminalReason::ConsecutiveErrors,
                        ) => {
                            tracing::warn!(
                                agent_id = %config.agent_id,
                                consecutive_errors = error_count,
                                "consecutive error limit reached after grace turn"
                            );
                            break;
                        }
                        _ => break,
                    }
                } else if state.grace_turn_active {
                    tracing::info!(
                        agent_id = %config.agent_id,
                        consecutive_errors = error_count,
                        "consecutive error limit reached — entering grace turn"
                    );
                    messages.push(ChatMessage {
                        role: Role::System,
                        content: Some(serde_json::Value::String(format!(
                            "[TOOL ERROR LIMIT] You have hit {error_count} consecutive tool errors. \
                             The failing calls were:\n{failure_summary}\n\n\
                             STOP calling the tools that keep failing. Instead:\n\
                             1. Explain to the user what you were trying to do and what went wrong.\n\
                             2. Suggest how to fix the issue (e.g. correct file paths, adjust permissions, change approach).\n\
                             3. Ask the user if they want you to try a different approach.\n\n\
                             Do NOT retry the same failing tool calls.",
                        ))),
                        reasoning_content: None,
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
            compact_metadata: None,
                    });
                    break;
                }
            }

            if plan_approval_pending {
                tracing::info!(
                    agent_id = %config.agent_id,
                    "breaking execution loop — plan approval pending, waiting for user"
                );
                let _ = send_stream_event(
                    tx,
                    make_turn_end_event(&turn_id, request, &state, stream_start, context_window, None),
                    false,
                )
                .await;
                self.finalize_injected_skills(&injected_skill_ids, true)
                    .await;
                return Ok(make_turn_summary(
                    &turn_id,
                    &state,
                    stream_start,
                    context_window,
                ));
            }

            if force_stop_loop {
                tracing::warn!(
                    agent_id = %config.agent_id,
                    "tool repetition hard limit reached — giving LLM one final turn to explain (stream)"
                );
                continue;
            }

            // Post-tool-call context usage update — run lightweight microcompact
            // first so the reported token count reflects what the next LLM call
            // will actually see, rather than the raw uncompressed accumulation.
            {
                let keep_recent = tool_executor::keep_recent_for_context_window(context_window);
                tool_executor::microcompact_tool_results(&mut messages, keep_recent);
                tool_executor::dedup_repeated_tool_calls(&mut messages);
                let post_tool_tokens = xiaolin_context::estimate_messages_tokens(&messages);
                state.last_estimated_tokens = post_tool_tokens;
                let _ = send_stream_event(
                    tx,
                    AgentEvent::ContextUsageUpdate {
                        turn_id: turn_id.clone(),
                        used_tokens: post_tool_tokens as u32,
                        limit_tokens: context_window,
                        compressed: false,
                        tokens_saved: 0,
                    },
                    false,
                )
                .await;
            }

            if let (Some(before), Some(ms)) = (mode_before, mode_state.as_ref()) {
                let after = ms.current_mode();
                if before != after {
                    let _ = send_stream_event(
                        tx,
                        AgentEvent::ModeChange {
                            turn_id: turn_id.clone(),
                            from: before,
                            to: after,
                        },
                        false,
                    )
                    .await;

                    if let Some(pc) = crate::builtin_tools::plan_mode::current_plan_context() {
                        let path = pc.store.plan_path(&pc.session_id);
                        let exists = pc.store.plan_exists(&pc.session_id);
                        let _ = send_stream_event(
                            tx,
                            AgentEvent::PlanFileUpdate {
                                turn_id: turn_id.clone(),
                                session_id: pc.session_id.clone(),
                                path: path.to_string_lossy().to_string(),
                                exists,
                            },
                            false,
                        )
                        .await;
                    }
                }
            }

            if let Some(ref reason) = last_finish_reason {
                if reason == "length" {
                    let has_write_tools = assembled_calls.iter().any(|tc| {
                        let n = tc.function.name.as_str();
                        n == "write_file" || n == "edit_file" || n == "multi_edit"
                    });
                    if has_write_tools {
                        tracing::warn!(
                            agent_id = %config.agent_id,
                            "LLM output truncated (finish_reason=length) with write/edit tool calls — injecting retry guidance"
                        );
                        messages.push(ChatMessage {
                            role: Role::System,
                            content: Some(serde_json::Value::String(
                                "[WARNING] Your previous response was truncated (finish_reason=length). \
                                The file content you wrote may be incomplete. Please verify the file \
                                with read_file and fix any truncated content. When writing large files, \
                                break the work into smaller edit_file calls instead of one large write_file."
                                    .to_string(),
                            )),
                            reasoning_content: None,
                            name: None,
                            tool_calls: None,
                            tool_call_id: None,
            compact_metadata: None,
                        });
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn try_self_iter_tool_recovery(
        &self,
        messages: &mut Vec<ChatMessage>,
        config: &AgentConfig,
        #[cfg(feature = "self-iter")] request: &ChatRequest,
        #[cfg(not(feature = "self-iter"))] _request: &ChatRequest,
        #[cfg(feature = "self-iter")] loop_iteration: u32,
        #[cfg(not(feature = "self-iter"))] _loop_iteration: u32,
        consecutive_errors: u32,
        max_errors: u32,
        failure_streak: &[ToolCallTrace],
        recovery_attempts: &mut u32,
    ) -> bool {
        let max_attempts = {
            #[cfg(feature = "self-iter")]
            {
                self.self_iter_max_recovery_attempts
            }
            #[cfg(not(feature = "self-iter"))]
            {
                3u32
            }
        };
        if *recovery_attempts >= max_attempts {
            return false;
        }
        let trigger = std::cmp::min(2, max_errors.max(1));
        if consecutive_errors < trigger || failure_streak.is_empty() {
            return false;
        }

        // Try advanced SelfIterEngine diagnosis first (when available),
        // then fall back to basic guidance.
        let guidance: String;

        #[cfg(feature = "self-iter")]
        {
            let advanced = self.self_iter_engine.as_ref().and_then(|engine| {
                let session = request
                    .session_id
                    .clone()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "default".to_string());
                let diagnoses = engine.diagnose_tool_failure_streak(
                    &config.agent_id,
                    &session,
                    loop_iteration,
                    failure_streak,
                );
                SelfIterEngine::format_recovery_guidance(&diagnoses)
            });
            match advanced {
                Some(g) => guidance = g,
                None => match format_basic_recovery_guidance(failure_streak) {
                    Some(g) => guidance = g,
                    None => return false,
                },
            }
        }
        #[cfg(not(feature = "self-iter"))]
        {
            match format_basic_recovery_guidance(failure_streak) {
                Some(g) => guidance = g,
                None => return false,
            }
        }

        inject_tool_recovery_guidance(messages, &guidance);
        *recovery_attempts += 1;
        tracing::info!(
            agent_id = %config.agent_id,
            recovery_attempt = *recovery_attempts,
            "tool recovery guidance injected into system prompt"
        );
        true
    }

    async fn finalize_injected_skills(&self, injected_skill_ids: &[String], success: bool) {
        let store: Arc<SkillStore> = match (*self.skill_store.load()).as_ref() {
            Some(s) => s.clone(),
            None => return,
        };
        for id in injected_skill_ids {
            if let Err(e) = store.record_usage(id, success).await {
                tracing::warn!(skill_id = %id, error = %e, "skill usage record failed");
            }
        }
    }

    async fn record_completed_trajectory(
        &self,
        request: &ChatRequest,
        config: &AgentConfig,
        steps: &[TrajectoryStep],
        run_succeeded: bool,
    ) {
        let store: Arc<TrajectoryStore> = match (*self.trajectory_store.load()).as_ref() {
            Some(s) => s.clone(),
            None => return,
        };

        let session_id = request
            .session_id
            .clone()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "default".to_string());

        let outcome = if run_succeeded {
            TrajectoryOutcome::Success { user_rating: None }
        } else {
            let reason = steps
                .iter()
                .rev()
                .find(|s| s.success == Some(false))
                .map(|s| s.summary.clone())
                .unwrap_or_else(|| "unknown failure".to_string());
            TrajectoryOutcome::Failure { reason }
        };

        let trajectory = Trajectory {
            id: uuid::Uuid::new_v4().to_string(),
            agent_id: config.agent_id.to_string(),
            session_id,
            task_type: infer_task_type(steps),
            steps: steps.to_vec(),
            outcome,
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        if let Err(e) = store.record_trajectory(&trajectory).await {
            tracing::warn!(error = %e, "trajectory record failed");
        }
    }

    async fn inject_relevant_skills(
        &self,
        messages: &mut Vec<ChatMessage>,
        request: &ChatRequest,
        injected_skill_ids: &mut Vec<String>,
    ) -> anyhow::Result<()> {
        let store: Arc<SkillStore> = match (*self.skill_store.load()).as_ref() {
            Some(s) => s.clone(),
            None => return Ok(()),
        };
        let task = last_user_turn_text(&request.messages);
        let trimmed = task.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        if trimmed.split_whitespace().count() < 3 && trimmed.len() < 12 {
            tracing::debug!(
                task = trimmed,
                "inject_relevant_skills: skipping trivial query"
            );
            return Ok(());
        }
        let skills = store.find_similar(&task, 16).await?;
        let active: Vec<_> = skills
            .iter()
            .filter(|s| matches!(s.status, SkillStatus::Active))
            .take(5)
            .cloned()
            .collect();
        let candidates: Vec<_> = skills
            .iter()
            .filter(|s| matches!(s.status, SkillStatus::Candidate))
            .take(2)
            .cloned()
            .collect();
        if active.is_empty() && candidates.is_empty() {
            return Ok(());
        }
        for s in &active {
            injected_skill_ids.push(s.id.clone());
        }
        for s in &candidates {
            injected_skill_ids.push(s.id.clone());
        }

        let mut block = String::new();
        if !active.is_empty() {
            block.push_str(&format_skills_for_prompt(&active));
        }
        if !candidates.is_empty() {
            block.push_str(&format_candidate_skills_for_prompt(&candidates));
        }
        block.push_str(SKILL_MANAGEMENT_GUIDANCE);
        Self::inject_skill_block_into_system(messages, &block);

        let session_key = request.session_id.as_deref().unwrap_or("default");
        if let Err(e) = store
            .register_session_skills(session_key, injected_skill_ids)
            .await
        {
            tracing::warn!(error = %e, "register_session_skills failed");
        }
        Ok(())
    }

    fn inject_skill_block_into_system(messages: &mut Vec<ChatMessage>, block: &str) {
        if block.trim().is_empty() {
            return;
        }
        if let Some(first) = messages.first_mut() {
            if matches!(first.role, Role::System) {
                append_text_to_chat_content(&mut first.content, block);
                return;
            }
        }
        messages.insert(
            0,
            ChatMessage {
                role: Role::System,
                content: Some(serde_json::Value::String(block.to_string())),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
                compact_metadata: None,
            },
        );
    }

    fn build_messages(&self, params: &ExecutionParams<'_>) -> Vec<ChatMessage> {
        let config = params.config;
        let user_messages = &params.request.messages;

        let mut messages = Vec::with_capacity(user_messages.len() + 1);

        let agent_prompt = config
            .system_prompt
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());

        let ctx = self.build_prompt_context(params);
        let parts = self.prompt_engine.build_effective_prompt(
            &ctx,
            None,
            agent_prompt,
            None,
            params.subagent_prompt.as_deref(),
        );

        let system_text = parts.join("\n\n");

        messages.push(ChatMessage {
            role: Role::System,
            content: Some(serde_json::Value::String(system_text)),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
            compact_metadata: None,
        });

        messages.extend_from_slice(user_messages);

        // When the user explicitly selected a model different from the agent's default,
        // scan the conversation history for conflicting model identity claims and inject
        // a reminder right before the last user message to prevent the LLM from parroting
        // the old model's identity from history.
        if let Some(ref req_model) = params.request.model {
            if !req_model.is_empty() {
                let has_conflicting_identity = messages.iter().any(|m| {
                    if m.role != Role::Assistant {
                        return false;
                    }
                    if let Some(text) = m.text_content() {
                        let lower = text.to_lowercase();
                        (lower.contains("我是") || lower.contains("i am") || lower.contains("i'm"))
                            && !lower.contains(&req_model.to_lowercase())
                    } else {
                        false
                    }
                });
                if has_conflicting_identity {
                    if let Some(last_user_idx) = messages.iter().rposition(|m| m.role == Role::User)
                    {
                        let reminder = format!(
                            "[Model Switch Notice] The model has been switched. You are now {}. \
                             Disregard any previous assistant messages claiming a different model identity.",
                            req_model
                        );
                        messages.insert(
                            last_user_idx,
                            ChatMessage {
                                role: Role::System,
                                content: Some(serde_json::Value::String(reminder)),
                                reasoning_content: None,
                                name: None,
                                tool_calls: None,
                                tool_call_id: None,
                                compact_metadata: None,
                            },
                        );
                    }
                }
            }
        }

        messages
    }

    fn build_prompt_context(&self, params: &ExecutionParams<'_>) -> PromptContext {
        let tool_names = params.tool_registry.tool_names();
        let deferred_count = params.tool_registry.deferred_count();

        let mode = params
            .mode_state
            .as_ref()
            .map(|ms| ms.current_mode())
            .unwrap_or(ExecutionMode::Agent);

        let model_id = if let Some(ref req_model) = params.request.model {
            if !req_model.is_empty() {
                req_model.clone()
            } else {
                format!(
                    "{}/{}",
                    params.config.model.provider, params.config.model.model
                )
            }
        } else {
            format!(
                "{}/{}",
                params.config.model.provider, params.config.model.model
            )
        };

        let cwd = params
            .request
            .work_dir
            .as_ref()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let is_git = cwd.join(".git").exists();
        let platform = std::env::consts::OS.to_string();
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string());
        let date = chrono::Local::now().format("%Y-%m-%d").to_string();

        let pending_todo_summary = if mode == ExecutionMode::Agent {
            params
                .todo_store
                .as_ref()
                .and_then(|ts| ts.pending_summary())
        } else {
            None
        };

        let (plan_file_path, plan_file_exists) =
            crate::builtin_tools::plan_mode::current_plan_context()
                .map(|pc| {
                    let path = pc.store.plan_path(&pc.session_id);
                    let exists = pc.store.plan_exists(&pc.session_id);
                    (Some(path.display().to_string()), exists)
                })
                .unwrap_or((None, false));

        PromptContext {
            agent_config: Arc::new(params.config.clone()),
            enabled_tools: tool_names,
            deferred_tool_count: deferred_count,
            model_id,
            cwd,
            is_git,
            platform,
            shell,
            execution_mode: mode,
            mcp_servers: vec![],
            language_preference: None,
            token_budget: None,
            memory_prompt: None,
            session_start_date: date,
            pending_todo_summary,
            plan_file_path,
            plan_file_exists,
            system_base_prompt: Some(
                xiaolin_core::workspace::EMBEDDED_SYSTEM_BASE_PROMPT.to_string(),
            ),
        }
    }

    /// Load persisted ContentReplacementState from session store, or create fresh.
    /// On resume, collects all tool_use_ids from existing messages and loads persisted
    /// replacement records to reconstruct byte-identical state.
    async fn load_or_create_replacement_state(
        session_store: &Option<Arc<xiaolin_session::SessionStore>>,
        session_id: Option<&str>,
        messages: &[ChatMessage],
    ) -> ContentReplacementState {
        let Some(store) = session_store else {
            return ContentReplacementState::new();
        };
        let Some(sid) = session_id else {
            return ContentReplacementState::new();
        };

        let records = match store.load_replacement_records(sid).await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = %e, session_id = sid, "failed to load replacement records, starting fresh");
                return ContentReplacementState::new();
            }
        };

        if records.is_empty() {
            return ContentReplacementState::new();
        }

        let message_tool_use_ids: Vec<String> = messages
            .iter()
            .filter(|m| m.role == Role::Tool)
            .filter_map(|m| m.tool_call_id.clone())
            .collect();

        let cr_records: Vec<tool_result_storage::ContentReplacementRecord> = records
            .into_iter()
            .map(|r| tool_result_storage::ContentReplacementRecord {
                tool_use_id: r.tool_use_id,
                replacement: r.replacement,
            })
            .collect();

        let state = reconstruct_state(&message_tool_use_ids, &cr_records);
        tracing::info!(
            session_id = sid,
            seen_ids = state.seen_ids.len(),
            replacements = state.replacements.len(),
            "reconstructed ContentReplacementState from persisted records"
        );
        state
    }

    /// Persist newly created replacement records to session store.
    /// Fails silently (logs warning) — the fallback is truncation on next turn.
    async fn persist_replacement_records(
        session_store: &Option<Arc<xiaolin_session::SessionStore>>,
        session_id: Option<&str>,
        records: &[tool_result_storage::ContentReplacementRecord],
    ) {
        if records.is_empty() {
            return;
        }
        let Some(store) = session_store else {
            return;
        };
        let Some(sid) = session_id else {
            return;
        };

        let rows: Vec<xiaolin_session::ContentReplacementRow> = records
            .iter()
            .map(|r| xiaolin_session::ContentReplacementRow {
                tool_use_id: r.tool_use_id.clone(),
                replacement: r.replacement.clone(),
            })
            .collect();

        if let Err(e) = store.save_replacement_records(sid, &rows).await {
            tracing::warn!(
                error = %e,
                session_id = sid,
                count = rows.len(),
                "failed to persist replacement records"
            );
        }
    }
}

#[cfg(test)]
mod stream_resume_tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    use super::*;
    use async_trait::async_trait;
    use xiaolin_core::agent_config::{AgentConfig, AgentModelConfig, BehaviorConfig};
    use xiaolin_core::tool::ToolRegistry;
    use xiaolin_core::types::{
        ChatMessage, ChatRequest, ChatResponse, DeltaContent, Role, StreamChoice, StreamDelta,
    };
    use futures::stream::{self, StreamExt};

    fn test_agent_config() -> AgentConfig {
        AgentConfig {
            agent_id: "t1".into(),
            name: None,
            description: None,
            model: AgentModelConfig {
                provider: "openai".into(),
                model: "mock".into(),
                temperature: 0.0,
                max_tokens: None,
                context_window: None,
                cost_per_1k_input: None,
                cost_per_1k_output: None,
                supports_reasoning: None,
                capabilities: None,
                fallbacks: Vec::new(),
                max_concurrent_requests: 10,
            },
            system_prompt: Some("You are a test assistant.".into()),
            tools: Vec::new(),
            behavior: BehaviorConfig::default(),
            mcp_servers: Vec::new(),
            min_tier: None,
            max_tier: None,
            avatar: None,
            channels: std::collections::HashMap::new(),
        }
    }

    struct FlakyStreamProvider {
        calls: Arc<AtomicU32>,
    }

    fn stream_delta_text(text: &str) -> StreamDelta {
        StreamDelta {
            id: "id-m".into(),
            object: "chat.completion.chunk".into(),
            created: 0,
            model: "mock".into(),
            choices: vec![StreamChoice {
                index: 0,
                delta: DeltaContent {
                    role: None,
                    content: Some(text.into()),
                    reasoning_content: None,
                    tool_calls: None,
                },
                finish_reason: None,
            }],
            usage: None,
            raw_sse_json: None,
        }
    }

    fn stream_delta_stop() -> StreamDelta {
        StreamDelta {
            id: "id-m".into(),
            object: "chat.completion.chunk".into(),
            created: 0,
            model: "mock".into(),
            choices: vec![StreamChoice {
                index: 0,
                delta: DeltaContent {
                    role: None,
                    content: None,
                    reasoning_content: None,
                    tool_calls: None,
                },
                finish_reason: Some("stop".into()),
            }],
            usage: None,
            raw_sse_json: None,
        }
    }

    #[async_trait]
    impl LlmProvider for FlakyStreamProvider {
        async fn chat_completion(&self, _: &CompletionParams<'_>) -> anyhow::Result<ChatResponse> {
            anyhow::bail!("not used")
        }

        async fn chat_completion_stream(
            &self,
            _: &CompletionParams<'_>,
        ) -> anyhow::Result<futures::stream::BoxStream<'static, anyhow::Result<StreamDelta>>>
        {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            let s = if n == 0 {
                stream::iter(vec![
                    Ok(stream_delta_text("hello")),
                    Err(anyhow::anyhow!("simulated drop")),
                ])
                .boxed()
            } else {
                stream::iter(vec![
                    Ok(stream_delta_text(" world")),
                    Ok(stream_delta_stop()),
                ])
                .boxed()
            };
            Ok(s)
        }
    }

    #[tokio::test]
    async fn execute_stream_resumes_after_interrupt_with_partial_context() {
        let config = test_agent_config();
        let calls = Arc::new(AtomicU32::new(0));
        let provider: Arc<dyn LlmProvider> = Arc::new(FlakyStreamProvider {
            calls: calls.clone(),
        });
        let runtime = AgentRuntime::new(provider);
        let registry = Arc::new(ToolRegistry::new());
        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);

        let req = ChatRequest {
            model: None,
            messages: vec![ChatMessage {
                role: Role::User,
                content: Some("hi".into()),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
                compact_metadata: None,
            }],
            agent_id: None,
            session_id: None,
            stream: true,
            temperature: None,
            max_tokens: None,
            tools: None,
            slash_intent: None,
            work_dir: None,
        };

        let res = runtime
            .execute_stream(&config, &req, &registry, tx, None)
            .await;

        assert!(res.is_ok(), "{res:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 2, "expected stream reconnect");

        let seen = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            let mut s = String::new();
            while let Some(ev) = rx.recv().await {
                match ev {
                    AgentEvent::ContentDelta { delta, .. } => {
                        if let Some(c) = delta
                            .get("choices")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("delta"))
                            .and_then(|d| d.get("content"))
                            .and_then(|c| c.as_str())
                        {
                            s.push_str(c);
                        }
                    }
                    AgentEvent::TurnEnd { .. } => break,
                    AgentEvent::Error { message, .. } => {
                        panic!("unexpected stream error: {message}")
                    }
                    _ => {}
                }
            }
            s
        })
        .await
        .expect("timeout waiting for stream events");

        assert!(seen.contains("hello"), "concatenated deltas: {seen}");
        assert!(seen.contains("world"), "concatenated deltas: {seen}");
    }
}
