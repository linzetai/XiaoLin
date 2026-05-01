use std::collections::HashMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use dashmap::DashMap;
use fastclaw_core::agent_config::AgentConfig;
use fastclaw_core::tool::ToolRegistry;
use fastclaw_core::types::{
    ChatMessage, ChatRequest, ChatResponse, Role, StreamEvent, ToolCall,
};
use fastclaw_core::types::ExecutionMode;

use prompt_engine::{PromptContext, PromptEngine, PromptSection};
#[cfg(feature = "evolution")]
use fastclaw_evolution::{
    format_candidate_skills_for_prompt, format_skills_for_prompt, infer_task_type, SkillStatus,
    SkillStore, Trajectory, TrajectoryOutcome, TrajectoryStep, TrajectoryStore,
};

#[cfg(not(feature = "evolution"))]
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct TrajectoryStep {
    role: String,
    action_type: String,
    tool_name: Option<String>,
    summary: String,
    success: Option<bool>,
}
#[cfg(feature = "self-iter")]
use fastclaw_self_iter::{SelfIterEngine, ToolCallTrace};
#[cfg(not(feature = "self-iter"))]
use stream_engine::ToolCallTrace;
use futures::StreamExt;

use crate::builtin_tools::{with_file_access_mode, with_work_dir};
use crate::llm::{CompletionParams, LlmProvider};
use base64::Engine as _;

#[allow(dead_code)]
pub mod api_errors;
#[allow(dead_code)]
pub mod cache_break_detection;
#[allow(dead_code)]
pub mod cost_tracker;
#[allow(dead_code)]
pub mod retry;
pub mod streaming_tool_executor;
mod accumulator;
pub mod query_engine;
pub(crate) mod context_compressor;
pub mod prompt_engine;
pub mod prompt_sections;
mod prompt_builder;
pub(crate) mod query_deps;
#[allow(dead_code)]
pub mod file_state_cache;
#[allow(dead_code)]
pub mod hook_config;
#[allow(dead_code)]
pub mod hook_events;
#[allow(dead_code)]
pub mod hook_executor;
#[allow(dead_code)]
pub mod permissions;
mod query_state;
#[allow(dead_code)]
pub mod side_query;
#[allow(dead_code)]
pub mod memory_selection;
#[allow(dead_code)]
pub mod prompt_suggestion;
#[allow(dead_code)]
pub mod magic_docs;
#[allow(dead_code)]
pub mod file_persistence;
mod session_memory;
#[allow(dead_code)]
mod stop_hooks;
mod stream_engine;
mod tool_executor;
pub mod tool_result_storage;
mod trajectory;
mod unified_compact;

pub use prompt_builder::{build_subagent_prompt_block, SubAgentPromptContext};

use accumulator::{accumulate_tool_call, ToolCallAccumulator};
#[cfg(feature = "evolution")]
use prompt_builder::SKILL_MANAGEMENT_GUIDANCE;
use query_state::QueryLoopState;
use query_deps::QueryDeps;
use stream_engine::send_stream_event;
use tool_executor::execute_tool_batch;
use tool_executor::filter_tool_definitions;
use tool_executor::semantic_header;
#[allow(deprecated)]
use tool_executor::truncate_tool_result_output_with_limit;
use tool_result_storage::{
    ContentReplacementState, ToolResultEntry, ToolResultStorage,
    MAX_TOOL_RESULTS_PER_MESSAGE_CHARS, reconstruct_state,
};
#[cfg(any(feature = "evolution", feature = "self-iter"))]
use trajectory::append_text_to_chat_content;
#[cfg(feature = "evolution")]
use trajectory::last_user_turn_text;
use trajectory::truncate_for_trajectory;

/// Create a ToolResultStorage for the current invocation session.
///
/// - With `session_id`: uses `~/.fastclaw/sessions/<session_id>/` so tool results
///   persist across process restarts and can be recovered on session resume.
/// - Without `session_id`: uses an ephemeral temp directory that lives only as
///   long as the current process.
fn create_tool_result_storage(session_id: Option<&str>) -> ToolResultStorage {
    let session_dir = match session_id {
        Some(sid) => dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(".fastclaw")
            .join("sessions")
            .join(sid),
        None => std::env::temp_dir()
            .join("fastclaw_sessions")
            .join(format!("ephemeral_{}", std::process::id())),
    };
    ToolResultStorage::new(session_dir)
}

/// Build the set of tool names whose results should skip budget enforcement.
/// These are tools with `max_result_size_chars() == usize::MAX`.
fn build_skip_tool_names(tool_registry: &fastclaw_core::tool::ToolRegistry) -> std::collections::HashSet<String> {
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
    messages: &mut [fastclaw_core::types::ChatMessage],
    state: &mut ContentReplacementState,
    skip_tool_names: &std::collections::HashSet<String>,
) -> Vec<tool_result_storage::ContentReplacementRecord> {
    let mut tool_entries: Vec<ToolResultEntry> = Vec::new();
    let mut entry_indices: Vec<usize> = Vec::new();

    for (i, msg) in messages.iter().enumerate() {
        if msg.role == fastclaw_core::types::Role::Tool {
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
fn tool_result_content(
    text: &str,
    result: &fastclaw_core::tool::ToolResult,
) -> serde_json::Value {
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
    pub session_store: Option<Arc<fastclaw_session::SessionStore>>,
}

/// Additional parameters specific to the streaming execution path.
pub struct StreamParams {
    pub tx: tokio::sync::mpsc::Sender<StreamEvent>,
    pub confirm_pending: Option<Arc<DashMap<String, tokio::sync::oneshot::Sender<String>>>>,
}

/// Manages the execution of a single agent invocation, including
/// the tool-calling loop: LLM → tool_calls → execute → inject result → repeat.
pub struct AgentRuntime {
    default_provider: Arc<dyn LlmProvider>,
    agent_providers: ArcSwap<HashMap<String, Arc<dyn LlmProvider>>>,
    prompt_engine: PromptEngine,
    #[cfg(feature = "self-iter")]
    self_iter_engine: Option<Arc<SelfIterEngine>>,
    #[cfg(feature = "self-iter")]
    self_iter_max_recovery_attempts: u32,
    #[cfg(feature = "evolution")]
    skill_store: ArcSwap<Option<Arc<SkillStore>>>,
    #[cfg(feature = "evolution")]
    trajectory_store: ArcSwap<Option<Arc<TrajectoryStore>>>,
}

impl AgentRuntime {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self {
            default_provider: provider,
            agent_providers: ArcSwap::new(Arc::new(HashMap::new())),
            prompt_engine: Self::default_prompt_engine(),
            #[cfg(feature = "self-iter")]
            self_iter_engine: None,
            #[cfg(feature = "self-iter")]
            self_iter_max_recovery_attempts: 3,
            #[cfg(feature = "evolution")]
            skill_store: ArcSwap::new(Arc::new(None)),
            #[cfg(feature = "evolution")]
            trajectory_store: ArcSwap::new(Arc::new(None)),
        }
    }

    fn default_prompt_engine() -> PromptEngine {
        use prompt_sections::{
            actions_section, doing_tasks_section, intro_section, output_efficiency_section,
            system_section, tone_and_style_section, using_tools_section,
        };
        use prompt_sections::dynamic::{
            environment_section, frc_section, language_section, mcp_instructions_section,
            memory_section, session_guidance_section, token_budget_section,
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
            frc_section(),
        ];

        PromptEngine::new(static_sections, dynamic_sections)
    }

    #[cfg(feature = "evolution")]
    pub fn with_skill_store(self, store: Arc<SkillStore>) -> Self {
        self.skill_store.store(Arc::new(Some(store)));
        self
    }

    #[cfg(feature = "evolution")]
    pub fn with_trajectory_store(self, store: Arc<TrajectoryStore>) -> Self {
        self.trajectory_store.store(Arc::new(Some(store)));
        self
    }

    #[cfg(feature = "evolution")]
    pub fn attach_evolution_stores(&self, skill: Arc<SkillStore>, trajectory: Arc<TrajectoryStore>) {
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

    pub fn provider(&self) -> &dyn LlmProvider {
        &*self.default_provider
    }

    /// Get a shared reference to the default LLM provider.
    pub fn default_provider_arc(&self) -> Arc<dyn LlmProvider> {
        self.default_provider.clone()
    }

    pub fn register_provider(&self, agent_id: &str, provider: Arc<dyn LlmProvider>) {
        let mut m = self.agent_providers.load().as_ref().clone();
        m.insert(agent_id.to_string(), provider);
        self.agent_providers.store(Arc::new(m));
    }

    /// Drop all per-agent provider overrides.
    ///
    /// The runtime-level default provider remains unchanged and is still used as
    /// fallback when an agent has no dedicated provider entry.
    pub fn clear_registered_providers(&self) {
        self.agent_providers.store(Arc::new(HashMap::new()));
    }

    fn resolve_provider(&self, agent_id: &str) -> anyhow::Result<Arc<dyn LlmProvider>> {
        let guard = self.agent_providers.load();
        Ok(guard
            .get(agent_id)
            .cloned()
            .unwrap_or_else(|| self.default_provider.clone()))
    }

    pub async fn execute(
        &self,
        config: &AgentConfig,
        request: &ChatRequest,
        tool_registry: &Arc<ToolRegistry>,
        llm_override: Option<Arc<dyn LlmProvider>>,
    ) -> anyhow::Result<ExecutionResult> {
        let params = ExecutionParams { config, request, tool_registry, llm_override, subagent_prompt: None, mode_state: None, session_store: None };
        self.execute_inner(&params).await
    }

    pub async fn execute_with_subagent_prompt(
        &self,
        config: &AgentConfig,
        request: &ChatRequest,
        tool_registry: &Arc<ToolRegistry>,
        llm_override: Option<Arc<dyn LlmProvider>>,
        subagent_prompt: Option<String>,
    ) -> anyhow::Result<ExecutionResult> {
        let params = ExecutionParams { config, request, tool_registry, llm_override, subagent_prompt, mode_state: None, session_store: None };
        self.execute_inner(&params).await
    }

    async fn execute_inner(
        &self,
        params: &ExecutionParams<'_>,
    ) -> anyhow::Result<ExecutionResult> {
        let ExecutionParams { config, request, tool_registry, ref llm_override, subagent_prompt: _, ref mode_state, ref session_store } = *params;
        let max_iterations = config.behavior.max_tool_calls_per_turn;
        let max_errors = config.behavior.max_consecutive_errors;

        let t0 = std::time::Instant::now();
        let mut messages = self.build_messages(params);
        tracing::info!(elapsed_ms = t0.elapsed().as_millis() as u64, "perf: build_messages");

        #[allow(unused_mut)]
        let mut injected_skill_ids: Vec<String> = Vec::new();
        #[cfg(feature = "evolution")]
        {
            let t0_skills = std::time::Instant::now();
            if let Err(e) = self
                .inject_relevant_skills(&mut messages, request, &mut injected_skill_ids)
                .await
            {
                tracing::warn!(error = %e, "skill injection skipped");
            }
            tracing::info!(elapsed_ms = t0_skills.elapsed().as_millis() as u64, "perf: inject_relevant_skills");
        }

        let mut trajectory_steps: Vec<TrajectoryStep> = Vec::new();
        let t0 = std::time::Instant::now();
        let all_tool_defs = tool_registry.definitions();
        let tool_defs = filter_tool_definitions(&all_tool_defs, config);
        tracing::info!(elapsed_ms = t0.elapsed().as_millis() as u64, count = tool_defs.len(), "perf: tool_definitions");
        let tools_for_llm = if tool_defs.is_empty() {
            None
        } else {
            Some(tool_defs.as_slice())
        };

        let temperature = request.temperature.unwrap_or(config.model.temperature);
        let model = request.model.as_deref().unwrap_or(&config.model.model);
        let max_tokens = request.max_tokens.or(config.model.max_tokens).or_else(|| {
            let inferred = fastclaw_context::infer_output_limit_from_model(model);
            if inferred > 0 { Some(inferred) } else { None }
        });

        let mut state = QueryLoopState::new(max_iterations);
        let tool_storage = create_tool_result_storage(request.session_id.as_deref());
        let skip_tool_names = build_skip_tool_names(tool_registry);

        // Reconstruct ContentReplacementState from persisted records on session resume
        let mut replacement_state = Self::load_or_create_replacement_state(
            session_store,
            request.session_id.as_deref(),
            &request.messages,
        ).await;

        loop {
            if let Some(query_state::LoopTransition::Terminal(_)) = state.check_pre_iteration() {
                tracing::warn!(
                    agent_id = %config.agent_id,
                    consecutive_errors = state.consecutive_errors,
                    "stopping outer loop — consecutive error limit reached"
                );
                self.finalize_injected_skills(&injected_skill_ids, false).await;
                self.record_completed_trajectory(request, config, &trajectory_steps, false)
                    .await;
                anyhow::bail!(
                    "agent '{}' stopped: {} consecutive tool errors",
                    config.agent_id,
                    state.consecutive_errors
                );
            }

            state.begin_iteration();
            state.iteration_msg_boundaries.push((messages.len(), std::time::Instant::now()));

            tracing::info!(
                agent_id = %config.agent_id,
                model,
                iteration = state.iteration,
                msg_count = messages.len(),
                "LLM call"
            );

            let newly_replaced = apply_message_budget(&tool_storage, &mut messages, &mut replacement_state, &skip_tool_names);
            Self::persist_replacement_records(session_store, request.session_id.as_deref(), &newly_replaced).await;

            let params = CompletionParams {
                model,
                messages: &messages,
                temperature,
                max_tokens,
                tools: tools_for_llm,
            };

            let provider = match &llm_override {
                Some(p) => p.clone(),
                None => self.resolve_provider(&config.agent_id)?,
            };
            let llm_t0 = std::time::Instant::now();
            let response = provider.chat_completion(&params).await?;
            {
                let llm_ms = llm_t0.elapsed().as_millis() as f64;
                let pname = provider.provider_name();
                let mc = fastclaw_observe::default_metrics_collector();
                mc.record_provider_request(pname, &response.model);
                mc.record_provider_latency_ms(pname, &response.model, llm_ms);
                if let Some(ref u) = response.usage {
                    mc.record_provider_tokens(pname, &response.model, u.total_tokens as u64);
                }
            }

            let choice = response
                .choices
                .first()
                .ok_or_else(|| anyhow::anyhow!("LLM returned no choices"))?;

            let has_tool_calls = choice
                .message
                .tool_calls
                .as_ref()
                .is_some_and(|tc| !tc.is_empty());

            let transition = state.determine_post_llm_transition(has_tool_calls);
            match transition {
                query_state::LoopTransition::Terminal(ref reason) => {
                    tracing::info!(
                        agent_id = %config.agent_id,
                        reason = %reason,
                        iterations = state.iteration,
                        total_tool_calls = state.total_tool_calls,
                        "agent execution complete"
                    );
                    self.finalize_injected_skills(&injected_skill_ids, true).await;
                    self.record_completed_trajectory(request, config, &trajectory_steps, true)
                        .await;
                    return Ok(ExecutionResult {
                        response,
                        tool_calls_made: state.total_tool_calls,
                        iterations: state.iteration,
                    });
                }
                query_state::LoopTransition::Continue(_) => {}
            }

            messages.push(choice.message.clone());

            let tool_calls = choice
                .message
                .tool_calls
                .as_ref()
                .filter(|t| !t.is_empty())
                .ok_or_else(|| anyhow::anyhow!("LLM reported tool_calls but none were present"))?;

            let results = execute_tool_batch(
                tool_calls, tool_registry, &config.behavior, &request.work_dir, "", mode_state.as_ref(),
            ).await;

            for (tool_name, call_id, arguments, result) in results {
                state.total_tool_calls += 1;

                trajectory_steps.push(TrajectoryStep {
                    role: "assistant".into(),
                    action_type: "tool_call".into(),
                    tool_name: Some(tool_name.clone()),
                    summary: truncate_for_trajectory(""),
                    success: None,
                });

                if !result.success {
                    state.record_tool_error(&tool_name, &result.output);
                } else {
                    state.clear_error_streak();
                }

                let max_chars = tool_registry
                    .get(&tool_name)
                    .map(|t| t.max_result_size_chars())
                    .unwrap_or(100_000);
                let processed = process_tool_output(
                    &tool_storage, &tool_name, &call_id, &result.output, max_chars,
                );
                let header = semantic_header(&tool_name, &arguments, &result.output, result.success);
                let out = format!("{header}\n{processed}");
                let content = tool_result_content(&out, &result);
                messages.push(ChatMessage {
                    role: Role::Tool,
                    content: Some(content),
                    name: Some(tool_name.clone()),
                    tool_calls: None,
                    tool_call_id: Some(call_id),
                });

                trajectory_steps.push(TrajectoryStep {
                    role: "tool".into(),
                    action_type: "tool_result".into(),
                    tool_name: Some(tool_name.clone()),
                    summary: truncate_for_trajectory(&result.output),
                    success: Some(result.success),
                });

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
                        query_state::LoopTransition::Terminal(query_state::TerminalReason::ConsecutiveErrors) => {
                            tracing::warn!(
                                agent_id = %config.agent_id,
                                consecutive_errors = error_count,
                                "consecutive error limit reached after grace turn (non-stream)"
                            );
                            break;
                        }
                        _ => break,
                    }
                } else if state.grace_turn_active {
                    tracing::info!(
                        agent_id = %config.agent_id,
                        consecutive_errors = error_count,
                        "consecutive error limit reached — entering grace turn (non-stream)"
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
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
                    });
                    break;
                }
            }

            if choice.finish_reason.as_deref() == Some("length") {
                let has_write_tools = tool_calls.iter().any(|tc| {
                    let n = tc.function.name.as_str();
                    n == "write_file" || n == "edit_file" || n == "apply_patch"
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
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
            }
        }
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
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
        llm_override: Option<Arc<dyn LlmProvider>>,
    ) -> anyhow::Result<(u32, u32)> {
        let exec = ExecutionParams { config, request, tool_registry, llm_override, subagent_prompt: None, mode_state: None, session_store: None };
        let stream = StreamParams { tx, confirm_pending: None };
        self.execute_stream_inner(&exec, stream).await
    }

    pub async fn execute_stream_with_subagent_prompt(
        &self,
        config: &AgentConfig,
        request: &ChatRequest,
        tool_registry: &Arc<ToolRegistry>,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
        llm_override: Option<Arc<dyn LlmProvider>>,
        subagent_prompt: Option<String>,
    ) -> anyhow::Result<(u32, u32)> {
        let exec = ExecutionParams { config, request, tool_registry, llm_override, subagent_prompt, mode_state: None, session_store: None };
        let stream = StreamParams { tx, confirm_pending: None };
        self.execute_stream_inner(&exec, stream).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn execute_stream_with_confirm(
        &self,
        config: &AgentConfig,
        request: &ChatRequest,
        tool_registry: &Arc<ToolRegistry>,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
        llm_override: Option<Arc<dyn LlmProvider>>,
        confirm_pending: Arc<DashMap<String, tokio::sync::oneshot::Sender<String>>>,
        subagent_prompt: Option<String>,
        mode_state: Option<crate::builtin_tools::ExecutionModeState>,
        session_store: Option<Arc<fastclaw_session::SessionStore>>,
    ) -> anyhow::Result<(u32, u32)> {
        let exec = ExecutionParams { config, request, tool_registry, llm_override, subagent_prompt, mode_state, session_store };
        let stream = StreamParams { tx, confirm_pending: Some(confirm_pending) };
        self.execute_stream_inner(&exec, stream).await
    }

    async fn execute_stream_inner(
        &self,
        params: &ExecutionParams<'_>,
        stream_params: StreamParams,
    ) -> anyhow::Result<(u32, u32)> {
        let ExecutionParams { config, request, tool_registry, ref llm_override, subagent_prompt: _, ref mode_state, ref session_store } = *params;
        let StreamParams { ref tx, ref confirm_pending } = stream_params;
        let max_iterations = config.behavior.max_tool_calls_per_turn;
        let max_errors = config.behavior.max_consecutive_errors;

        let t0 = std::time::Instant::now();
        let mut messages = self.build_messages(params);
        tracing::info!(elapsed_ms = t0.elapsed().as_millis() as u64, "perf: build_messages (stream)");

        let t0 = std::time::Instant::now();
        let mut injected_skill_ids: Vec<String> = Vec::new();
        if let Err(e) = self
            .inject_relevant_skills(&mut messages, request, &mut injected_skill_ids)
            .await
        {
            tracing::warn!(error = %e, "skill injection skipped (stream)");
        }
        tracing::info!(elapsed_ms = t0.elapsed().as_millis() as u64, "perf: inject_relevant_skills (stream)");

        let mut trajectory_steps: Vec<TrajectoryStep> = Vec::new();
        let t0 = std::time::Instant::now();
        let all_tool_defs = tool_registry.definitions();
        let tool_defs = filter_tool_definitions(&all_tool_defs, config);
        let tool_defs_json_chars: usize = tool_defs.iter().map(|td| {
            serde_json::to_string(td).map(|s| s.len()).unwrap_or(0)
        }).sum();
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
            let inferred = fastclaw_context::infer_output_limit_from_model(&model);
            if inferred > 0 { Some(inferred) } else { None }
        });

        let mut state = QueryLoopState::new(max_iterations);
        let tool_storage = create_tool_result_storage(request.session_id.as_deref());
        let skip_tool_names = build_skip_tool_names(tool_registry);
        let stream_start = std::time::Instant::now();

        // Context window — constant across iterations
        let context_window = config.model.context_window.unwrap_or(
            fastclaw_context::infer_context_window_from_model(&config.model.model),
        );

        // QueryDeps: unified dependency injection for LLM calls + compression
        let provider_for_deps: Arc<dyn LlmProvider> = match &llm_override {
            Some(p) => p.clone(),
            None => self.resolve_provider(&config.agent_id)?,
        };
        let pipeline_config = fastclaw_context::PipelineConfig {
            snip_max_tokens: context_window as usize,
            reactive_target_tokens: context_window as usize,
            ..Default::default()
        };
        let auto_compact_enabled = pipeline_config.enable_auto_compact;
        let compact_pipeline = fastclaw_context::ContextPipeline::new(pipeline_config);
        let deps = query_deps::ProductionDeps::new(provider_for_deps, compact_pipeline);

        // Reconstruct ContentReplacementState from persisted records on session resume
        let mut replacement_state = Self::load_or_create_replacement_state(
            session_store,
            request.session_id.as_deref(),
            &request.messages,
        ).await;

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
                    StreamEvent::Error(user_msg.clone()),
                    false,
                )
                .await;
                self.finalize_injected_skills(&injected_skill_ids, false).await;
                return Err(anyhow::anyhow!(
                    "agent '{}' stopped: {} consecutive tool errors",
                    config.agent_id,
                    state.consecutive_errors
                ));
            }

            state.begin_iteration();
            state.iteration_msg_boundaries.push((messages.len(), std::time::Instant::now()));

            // ── Unified context compaction (via QueryDeps) ─────────────────
            let compact_result = deps.pre_query_compact(
                &mut messages,
                context_window,
                max_tokens,
                &model,
                state.last_estimated_tokens,
                &state.iteration_msg_boundaries,
            ).await;
            state.last_estimated_tokens = compact_result.estimated_tokens;
            let estimated_tokens = compact_result.estimated_tokens;

            // Emit live context usage update to frontend
            let _ = send_stream_event(
                tx,
                StreamEvent::ContextUsageUpdate {
                    used_tokens: estimated_tokens as u32,
                    limit_tokens: context_window,
                    compressed: compact_result.compressed_by_llm,
                    tokens_saved: compact_result.tokens_saved_by_llm as u32,
                },
                false,
            ).await;

            let usage_ratio = estimated_tokens as f32 / context_window.max(1) as f32;

            // Compact warning at 85%: suggest /compact (sent once per session)
            if usage_ratio > 0.85 && !state.compact_warning_sent {
                state.compact_warning_sent = true;
                let _ = send_stream_event(
                    tx,
                    StreamEvent::CompactWarning {
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
                    StreamEvent::ContextLimitWarning {
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
            // Blocking limit: if tokens >= 95% of context window and
            // auto-compact is off, stop and tell the user to run /compact.
            let just_compacted = compact_result.compressed_by_llm || compact_result.pipeline_applied;
            if let Some(query_state::LoopTransition::Terminal(query_state::TerminalReason::BlockingLimit)) =
                state.check_blocking_limit(estimated_tokens, context_window, auto_compact_enabled, just_compacted)
            {
                tracing::warn!(
                    agent_id = %config.agent_id,
                    estimated_tokens,
                    context_window,
                    "blocking limit reached (>= 95% context window) — stopping"
                );
                let _ = send_stream_event(
                    tx,
                    StreamEvent::Error(format!(
                        "Context window is nearly full ({}/{} tokens, {:.0}%). \
                         Please run /compact to free space, or start a new session.",
                        estimated_tokens, context_window, usage_ratio * 100.0,
                    )),
                    false,
                ).await;
                self.finalize_injected_skills(&injected_skill_ids, false).await;
                return Ok((state.total_tool_calls, state.iteration));
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

            let newly_replaced = apply_message_budget(&tool_storage, &mut messages, &mut replacement_state, &skip_tool_names);
            Self::persist_replacement_records(session_store, request.session_id.as_deref(), &newly_replaced).await;

            let mut accumulated_content = String::new();
            let mut tool_call_accum: Vec<ToolCallAccumulator> = Vec::new();
            let mut stream_errored = false;
            let mut last_finish_reason: Option<String> = None;
            let mut withheld_prompt_too_long: Option<String> = None;

            // Streaming tool execution: create executor and track submission state.
            // A new executor is created per iteration since it's consumed via drain_remaining().
            let streaming_exec_enabled = config.behavior.streaming_tool_execution;
            let mut streaming_executor = if streaming_exec_enabled {
                let exec_config = streaming_tool_executor::StreamingExecutorConfig {
                    sibling_cancel_on_error: true,
                    work_dir: request.work_dir.as_ref().map(std::path::PathBuf::from),
                    file_access: config.behavior.file_access,
                    additional_allowed_paths: Vec::new(),
                };
                Some(streaming_tool_executor::StreamingToolExecutor::new(
                    Arc::clone(tool_registry),
                    exec_config,
                ))
            } else {
                None
            };
            let mut last_submitted_tool_idx: Option<usize> = None;

            'stream_try: loop {
                let params = CompletionParams {
                    model: &model,
                    messages: &messages,
                    temperature,
                    max_tokens,
                    tools: tools_for_llm,
                };

                let llm_call_t0 = std::time::Instant::now();
                let stream_result = deps.call_model_stream(&params).await;
                let mut stream = match stream_result {
                    Ok(s) => s,
                    Err(e) => {
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
                tracing::info!(elapsed_ms = llm_call_t0.elapsed().as_millis() as u64, "perf: stream_connect");

                let mut first_chunk = true;
                let mut should_resume = false;
                while let Some(result) = stream.next().await {
                    if first_chunk {
                        tracing::info!(elapsed_ms = llm_call_t0.elapsed().as_millis() as u64, "perf: time_to_first_chunk");
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
                                messages.push(ChatMessage {
                                    role: Role::Assistant,
                                    content: Some(serde_json::Value::String(
                                        std::mem::take(&mut accumulated_content),
                                    )),
                                    name: None,
                                    tool_calls: None,
                                    tool_call_id: None,
                                });
                                stream_resume_attempts += 1;
                                should_resume = true;
                                break;
                            }
                            let _ = send_stream_event(
                                tx,
                                StreamEvent::Error(e.to_string()),
                                false,
                            )
                            .await;
                            stream_errored = true;
                            break;
                        }
                    };

                    if let Some(choice) = delta.choices.first() {
                        if let Some(ref content) = choice.delta.content {
                            accumulated_content.push_str(content);
                        }

                        if let Some(ref tc_deltas) = choice.delta.tool_calls {
                            for tc_delta in tc_deltas {
                                // In streaming mode: when a new tool index appears, all
                                // prior tools are fully accumulated and can start executing.
                                if let Some(ref mut executor) = streaming_executor {
                                    let new_idx = tc_delta.index as usize;
                                    let submit_start = last_submitted_tool_idx
                                        .map(|i| i + 1)
                                        .unwrap_or(0);
                                    if new_idx > 0 && submit_start < new_idx {
                                        for si in submit_start..new_idx {
                                            if let Some(acc) = tool_call_accum.get(si) {
                                                if !acc.name.is_empty() {
                                                    executor.add_tool(acc.to_tool_call());
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
                    }

                    if tool_call_accum.is_empty() {
                        let _ = send_stream_event(tx, StreamEvent::Delta(delta), true).await;
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
                    StreamEvent::Error(withheld_err.clone()),
                    false,
                ).await;
                self.finalize_injected_skills(&injected_skill_ids, false).await;
                return Err(anyhow::anyhow!("prompt_too_long: recovery failed"));
            }

            if stream_errored {
                self.finalize_injected_skills(&injected_skill_ids, false).await;
                return Err(anyhow::anyhow!(
                    "provider stream error (already sent to client)"
                ));
            }

            // max_output_tokens recovery: when finish_reason=length and no
            // tool calls, the model's output was truncated by the token limit.
            // Escalate max_tokens and retry up to MAX_OUTPUT_TOKENS_RECOVERY_LIMIT times.
            let has_valid_tool_calls = tool_call_accum.iter().any(|a| !a.name.is_empty());
            if last_finish_reason.as_deref() == Some("length") && !has_valid_tool_calls {
                if let Some(query_state::LoopTransition::Continue(query_state::ContinueReason::MaxOutputTokensRecovery)) =
                    state.try_max_output_tokens_recovery()
                {
                    let escalated = query_state::ESCALATED_MAX_TOKENS;
                    tracing::warn!(
                        agent_id = %config.agent_id,
                        attempt = state.max_output_tokens_recovery_count,
                        escalated_max_tokens = escalated,
                        "max_output_tokens recovery — retrying with escalated limit"
                    );
                    max_tokens = Some(escalated);
                    messages.push(ChatMessage {
                        role: Role::Assistant,
                        content: Some(serde_json::Value::String(
                            std::mem::take(&mut accumulated_content),
                        )),
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
                    });
                    continue;
                }
            }

            let transition = state.determine_post_llm_transition(has_valid_tool_calls);

            match transition {
                query_state::LoopTransition::Terminal(ref reason) => {
                    let final_tc: Option<Vec<ToolCall>> = if matches!(reason, query_state::TerminalReason::MaxIterations) {
                        let tc: Vec<ToolCall> = tool_call_accum
                            .iter()
                            .filter(|a| !a.name.is_empty())
                            .map(|a| a.to_tool_call())
                            .collect();
                        if tc.is_empty() { None } else { Some(tc) }
                    } else {
                        None
                    };
                    if matches!(reason, query_state::TerminalReason::MaxIterations) {
                        tracing::warn!(
                            agent_id = %config.agent_id,
                            max_iterations,
                            "streaming tool call limit reached"
                        );
                    }
                    let _ = send_stream_event(
                        tx,
                        StreamEvent::Done {
                            session_id: request.session_id.clone(),
                            tool_calls_made: state.total_tool_calls,
                            iterations: state.iteration,
                            final_tool_calls: final_tc,
                            usage: state.build_usage(),
                            elapsed_ms: stream_start.elapsed().as_millis() as u64,
                            context_tokens: Some(state.last_estimated_tokens as u32),
                            context_window: Some(context_window),
                        },
                        false,
                    )
                    .await;
                    self.finalize_injected_skills(&injected_skill_ids, true).await;
                    self.record_completed_trajectory(request, config, &trajectory_steps, true)
                        .await;
                    return Ok((state.total_tool_calls, state.iteration));
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
                    StreamEvent::Done {
                        session_id: request.session_id.clone(),
                        tool_calls_made: state.total_tool_calls,
                        iterations: state.iteration,
                        final_tool_calls: None,
                        usage: state.build_usage(),
                        elapsed_ms: stream_start.elapsed().as_millis() as u64,
                        context_tokens: Some(state.last_estimated_tokens as u32),
                        context_window: Some(context_window),
                    },
                    false,
                )
                .await;
                self.finalize_injected_skills(&injected_skill_ids, true).await;
                self.record_completed_trajectory(request, config, &trajectory_steps, true)
                    .await;
                return Ok((state.total_tool_calls, state.iteration));
            }

            messages.push(ChatMessage {
                role: Role::Assistant,
                content: if accumulated_content.is_empty() {
                    None
                } else {
                    Some(serde_json::Value::String(accumulated_content.clone()))
                },
                name: None,
                tool_calls: Some(assembled_calls.clone()),
                tool_call_id: None,
            });

            // Emit ToolExecuting events for all tool calls first.
            for tc in &assembled_calls {
                let args_str = if tc.function.arguments.is_empty() { None } else { Some(tc.function.arguments.clone()) };
                let _ = send_stream_event(
                    tx,
                    StreamEvent::ToolExecuting {
                        tool_name: tc.function.name.clone(),
                        call_id: tc.id.clone(),
                        args: args_str,
                    },
                    false,
                )
                .await;
            }

            let mode_before = mode_state.as_ref().map(|ms| ms.current_mode());

            // Choose execution path based on streaming_tool_execution config.
            let stream_results = if let Some(mut executor) = streaming_executor.take() {
                // Submit any remaining unsubmitted tools to the streaming executor
                let submit_start = last_submitted_tool_idx.map(|i| i + 1).unwrap_or(0);
                for si in submit_start..tool_call_accum.len() {
                    if let Some(acc) = tool_call_accum.get(si) {
                        if !acc.name.is_empty() {
                            executor.add_tool(acc.to_tool_call());
                        }
                    }
                }

                // Drain all results (tools already started during streaming)
                let completed = executor.drain_remaining().await;
                completed
                    .into_iter()
                    .map(|ct| {
                        let tc = &assembled_calls[ct.index];
                        (
                            ct.tool_name,
                            ct.call_id,
                            tc.function.arguments.clone(),
                            ct.result,
                        )
                    })
                    .collect::<Vec<_>>()
            } else {
                execute_tool_batch(
                    &assembled_calls, tool_registry, &config.behavior, &request.work_dir, " (stream)", mode_state.as_ref(),
                ).await
            };

            for (tool_name, call_id, arguments, mut result) in stream_results {
                state.total_tool_calls += 1;

                trajectory_steps.push(TrajectoryStep {
                    role: "assistant".into(),
                    action_type: "tool_call".into(),
                    tool_name: Some(tool_name.clone()),
                    summary: truncate_for_trajectory(&arguments),
                    success: None,
                });

                // ── Runtime-driven confirmation flow (sequential, requires user interaction) ──
                if result.needs_confirmation {
                    if let Some(ref pending_map) = confirm_pending {
                        use fastclaw_core::types::AskQuestionOption;

                        let confirm_request_id = uuid::Uuid::new_v4().to_string();
                        let (answer_tx, answer_rx) = tokio::sync::oneshot::channel::<String>();
                        pending_map.insert(confirm_request_id.clone(), answer_tx);

                        let _ = send_stream_event(
                            tx,
                            StreamEvent::AskQuestion {
                                request_id: confirm_request_id.clone(),
                                question: result.output.clone(),
                                options: vec![
                                    AskQuestionOption { id: "allow".into(), label: "Allow".into() },
                                    AskQuestionOption { id: "deny".into(), label: "Deny".into() },
                                ],
                                timeout_secs: 0,
                                allow_multiple: false,
                            },
                            true,
                        )
                        .await;

                        let user_answer = answer_rx.await;

                        pending_map.remove(&confirm_request_id);

                        let approved = matches!(user_answer, Ok(ref a) if a == "allow");

                        if approved {
                            let mut args: serde_json::Value =
                                serde_json::from_str(&arguments).unwrap_or_default();
                            if let Some(obj) = args.as_object_mut() {
                                obj.insert("confirmed".into(), serde_json::Value::Bool(true));
                            }
                            let confirmed_args = serde_json::to_string(&args).unwrap_or_default();

                            if let Some(tool) = tool_registry.get(&tool_name) {
                                let work_dir_path = request.work_dir.as_ref().map(std::path::PathBuf::from);
                                result = with_file_access_mode(
                                    config.behavior.file_access,
                                    crate::builtin_tools::with_additional_allowed_paths(
                                        Vec::new(),
                                        with_work_dir(work_dir_path, tool.execute(&confirmed_args)),
                                    ),
                                )
                                .await;
                            }
                        } else {
                            result = fastclaw_core::tool::ToolResult::err(
                                "User denied the operation."
                            );
                        }
                    }
                }

                if !result.success {
                    state.record_tool_error(&tool_name, &result.output);
                } else {
                    state.clear_error_streak();
                }

                let max_chars = tool_registry
                    .get(&tool_name)
                    .map(|t| t.max_result_size_chars())
                    .unwrap_or(100_000);
                let processed = process_tool_output(
                    &tool_storage, &tool_name, &call_id, &result.output, max_chars,
                );
                let header = semantic_header(&tool_name, &arguments, &result.output, result.success);
                let llm_out = format!("{header}\n{processed}");
                let _ = send_stream_event(
                    tx,
                    StreamEvent::ToolResult {
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

                let content = tool_result_content(&llm_out, &result);
                messages.push(ChatMessage {
                    role: Role::Tool,
                    content: Some(content),
                    name: Some(tool_name.clone()),
                    tool_calls: None,
                    tool_call_id: Some(call_id),
                });

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
                        query_state::LoopTransition::Terminal(query_state::TerminalReason::ConsecutiveErrors) => {
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
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
                    });
                    break;
                }
            }

            if let (Some(before), Some(ms)) = (mode_before, mode_state.as_ref()) {
                let after = ms.current_mode();
                if before != after {
                    let _ = send_stream_event(
                        tx,
                        StreamEvent::ModeChange { from: before, to: after },
                        false,
                    ).await;
                }
            }

            if let Some(ref reason) = last_finish_reason {
                if reason == "length" {
                    let has_write_tools = assembled_calls.iter().any(|tc| {
                        let n = tc.function.name.as_str();
                        n == "write_file" || n == "edit_file" || n == "apply_patch"
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
                            name: None,
                            tool_calls: None,
                            tool_call_id: None,
                        });
                    }
                }
            }
        }
    }

    #[cfg(feature = "self-iter")]
    fn inject_tool_recovery_guidance(messages: &mut Vec<ChatMessage>, guidance: &str) {
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
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
        );
    }

    #[cfg(feature = "self-iter")]
    #[allow(clippy::too_many_arguments)]
    fn try_self_iter_tool_recovery(
        &self,
        messages: &mut Vec<ChatMessage>,
        config: &AgentConfig,
        request: &ChatRequest,
        loop_iteration: u32,
        consecutive_errors: u32,
        max_errors: u32,
        failure_streak: &[ToolCallTrace],
        recovery_attempts: &mut u32,
    ) -> bool {
        let Some(engine) = self.self_iter_engine.as_ref() else {
            return false;
        };
        if *recovery_attempts >= self.self_iter_max_recovery_attempts {
            return false;
        }
        let trigger = std::cmp::min(2, max_errors.max(1));
        if consecutive_errors < trigger || failure_streak.is_empty() {
            return false;
        }

        let session = request
            .session_id
            .clone()
            .unwrap_or_else(|| "default".to_string());
        let diagnoses = engine.diagnose_tool_failure_streak(
            &config.agent_id,
            &session,
            loop_iteration,
            failure_streak,
        );
        let Some(guidance) = SelfIterEngine::format_recovery_guidance(&diagnoses) else {
            return false;
        };

        Self::inject_tool_recovery_guidance(messages, &guidance);
        *recovery_attempts += 1;
        tracing::info!(
            agent_id = %config.agent_id,
            recovery_attempt = *recovery_attempts,
            "self-iter: merged tool recovery guidance into system prompt"
        );
        true
    }

    #[cfg(not(feature = "self-iter"))]
    fn try_self_iter_tool_recovery(
        &self,
        _messages: &mut Vec<ChatMessage>,
        _config: &AgentConfig,
        _request: &ChatRequest,
        _loop_iteration: u32,
        _consecutive_errors: u32,
        _max_errors: u32,
        _failure_streak: &[ToolCallTrace],
        _recovery_attempts: &mut u32,
    ) -> bool {
        false
    }

    #[cfg(feature = "evolution")]
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

    #[cfg(not(feature = "evolution"))]
    async fn finalize_injected_skills(&self, _injected_skill_ids: &[String], _success: bool) {}

    #[cfg(feature = "evolution")]
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

    #[cfg(not(feature = "evolution"))]
    async fn record_completed_trajectory(
        &self,
        _request: &ChatRequest,
        _config: &AgentConfig,
        _steps: &[TrajectoryStep],
        _run_succeeded: bool,
    ) {}

    #[cfg(feature = "evolution")]
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
            tracing::debug!(task = trimmed, "inject_relevant_skills: skipping trivial query");
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

    #[cfg(not(feature = "evolution"))]
    async fn inject_relevant_skills(
        &self,
        _messages: &mut Vec<ChatMessage>,
        _request: &ChatRequest,
        _injected_skill_ids: &mut Vec<String>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    #[cfg(feature = "evolution")]
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
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
        );
    }

    fn build_messages(
        &self,
        params: &ExecutionParams<'_>,
    ) -> Vec<ChatMessage> {
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
            name: None,
            tool_calls: None,
            tool_call_id: None,
        });

        messages.extend_from_slice(user_messages);
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

        let model_id = format!(
            "{}/{}",
            params.config.model.provider, params.config.model.model
        );

        let cwd = std::env::current_dir().unwrap_or_default();
        let is_git = cwd.join(".git").exists();
        let platform = std::env::consts::OS.to_string();
        let shell = std::env::var("SHELL")
            .unwrap_or_else(|_| "sh".to_string());
        let date = chrono::Local::now().format("%Y-%m-%d").to_string();

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
        }
    }

    /// Load persisted ContentReplacementState from session store, or create fresh.
    /// On resume, collects all tool_use_ids from existing messages and loads persisted
    /// replacement records to reconstruct byte-identical state.
    async fn load_or_create_replacement_state(
        session_store: &Option<Arc<fastclaw_session::SessionStore>>,
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
        session_store: &Option<Arc<fastclaw_session::SessionStore>>,
        session_id: Option<&str>,
        records: &[tool_result_storage::ContentReplacementRecord],
    ) {
        if records.is_empty() {
            return;
        }
        let Some(store) = session_store else { return; };
        let Some(sid) = session_id else { return; };

        let rows: Vec<fastclaw_session::ContentReplacementRow> = records
            .iter()
            .map(|r| fastclaw_session::ContentReplacementRow {
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
    use fastclaw_core::agent_config::{AgentConfig, AgentModelConfig, BehaviorConfig};
    use fastclaw_core::tool::ToolRegistry;
    use fastclaw_core::types::{
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
                    tool_calls: None,
                },
                finish_reason: None,
            }],
            usage: None,
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
                    tool_calls: None,
                },
                finish_reason: Some("stop".into()),
            }],
            usage: None,
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
        ) -> anyhow::Result<futures::stream::BoxStream<'static, anyhow::Result<StreamDelta>>> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            let s = if n == 0 {
                stream::iter(vec![
                    Ok(stream_delta_text("hello")),
                    Err(anyhow::anyhow!("simulated drop")),
                ])
                .boxed()
            } else {
                stream::iter(vec![Ok(stream_delta_text(" world")), Ok(stream_delta_stop())]).boxed()
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
        let (tx, mut rx) = tokio::sync::mpsc::channel::<StreamEvent>(64);

        let req = ChatRequest {
            model: None,
            messages: vec![ChatMessage {
                role: Role::User,
                content: Some("hi".into()),
                name: None,
                tool_calls: None,
                tool_call_id: None,
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
                    StreamEvent::Delta(d) => {
                        if let Some(c) = d.choices.first().and_then(|x| x.delta.content.as_ref()) {
                            s.push_str(c);
                        }
                    }
                    StreamEvent::Done { .. } => break,
                    StreamEvent::Error(e) => panic!("unexpected stream error: {e}"),
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
