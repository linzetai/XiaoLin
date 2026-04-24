use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use fastclaw_evolution::{
    format_candidate_skills_for_prompt, format_skills_for_prompt, infer_task_type, SkillStatus,
    SkillStore, Trajectory, TrajectoryOutcome, TrajectoryStep, TrajectoryStore,
};
use fastclaw_core::agent_config::AgentConfig;
use fastclaw_core::tool::ToolRegistry;
use fastclaw_core::workspace::default_runtime_system_prompt_for_agent;
use fastclaw_core::types::{
    ChatMessage, ChatRequest, ChatResponse, FunctionCall, Role, StreamEvent, ToolCall,
};
use fastclaw_self_iter::{SelfIterEngine, ToolCallTrace};
use futures::StreamExt;

use crate::llm::{CompletionParams, LlmProvider};
use crate::builtin_tools::{with_file_access_mode, with_work_dir};

/// Max characters of tool output embedded in chat history (per tool message).
pub const MAX_TOOL_RESULT_CHARS: usize = 8000;

fn truncate_tool_result_output(output: &str) -> String {
    let total = output.chars().count();
    if total <= MAX_TOOL_RESULT_CHARS {
        return output.to_string();
    }
    let head: String = output.chars().take(MAX_TOOL_RESULT_CHARS).collect();
    format!("{head}\n... (truncated, showing first {MAX_TOOL_RESULT_CHARS} of {total} chars)")
}

fn memory_tool_suffix(agent_id: &str) -> String {
    agent_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Per-agent visibility for scoped memory tools (`memory_search__{agent}` style).
/// Append plain text to a message `content`, preserving prior text via [`ChatMessage::text_content`].
fn append_text_to_chat_content(content: &mut Option<serde_json::Value>, block: &str) {
    let tmp = ChatMessage {
        role: Role::System,
        content: content.clone(),
        name: None,
        tool_calls: None,
        tool_call_id: None,
    };
    let mut s = tmp.text_content().unwrap_or_default();
    s.push_str(block);
    *content = if s.is_empty() {
        None
    } else {
        Some(serde_json::Value::String(s))
    };
}

fn memory_tool_visible_for_agent(name: &str, agent_id: &str) -> bool {
    let sfx = memory_tool_suffix(agent_id);
    if let Some(rest) = name.strip_prefix("memory_search__") {
        return rest == sfx;
    }
    if let Some(rest) = name.strip_prefix("memory_store__") {
        return rest == sfx;
    }
    if name == "memory_search" || name == "memory_store" {
        return true;
    }
    true
}

fn is_tool_allowed(tool_name: &str, behavior: &fastclaw_core::agent_config::BehaviorConfig) -> bool {
    behavior.is_tool_allowed(tool_name)
}

async fn send_stream_event(
    tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ev: StreamEvent,
    lossy: bool,
) -> bool {
    let dur = if lossy {
        Duration::from_millis(200)
    } else {
        Duration::from_secs(30)
    };
    match tokio::time::timeout(dur, tx.send(ev)).await {
        Ok(Ok(())) => true,
        Ok(Err(_)) => false,
        Err(_) => {
            if lossy {
                tracing::warn!("stream sink slow: dropped a token delta (backpressure)");
            } else {
                tracing::warn!("stream sink slow: timed out sending control event");
            }
            false
        }
    }
}

/// Execution result containing the final response and tool-call trace.
pub struct ExecutionResult {
    pub response: ChatResponse,
    pub tool_calls_made: u32,
    pub iterations: u32,
}

/// Manages the execution of a single agent invocation, including
/// the tool-calling loop: LLM → tool_calls → execute → inject result → repeat.
pub struct AgentRuntime {
    default_provider: Arc<dyn LlmProvider>,
    agent_providers: std::sync::RwLock<HashMap<String, Arc<dyn LlmProvider>>>,
    self_iter_engine: Option<Arc<SelfIterEngine>>,
    self_iter_max_recovery_attempts: u32,
    skill_store: RwLock<Option<Arc<SkillStore>>>,
    trajectory_store: RwLock<Option<Arc<TrajectoryStore>>>,
}

impl AgentRuntime {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self {
            default_provider: provider,
            agent_providers: std::sync::RwLock::new(HashMap::new()),
            self_iter_engine: None,
            self_iter_max_recovery_attempts: 3,
            skill_store: RwLock::new(None),
            trajectory_store: RwLock::new(None),
        }
    }

    /// Optional Hermes-style skill store: matching **active** skills are injected into the system prompt.
    pub fn with_skill_store(self, store: Arc<SkillStore>) -> Self {
        if let Ok(mut g) = self.skill_store.write() {
            *g = Some(store);
        } else {
            tracing::error!("skill_store RwLock poisoned during with_skill_store");
        }
        self
    }

    /// Optional trajectory store: successful runs append [`Trajectory`] rows for evolution.
    pub fn with_trajectory_store(self, store: Arc<TrajectoryStore>) -> Self {
        if let Ok(mut g) = self.trajectory_store.write() {
            *g = Some(store);
        } else {
            tracing::error!("trajectory_store RwLock poisoned during with_trajectory_store");
        }
        self
    }

    /// Late-bind evolution stores after the runtime is already wrapped in [`Arc`] (production gateway wiring).
    ///
    /// Equivalent to calling [`Self::with_skill_store`] and [`Self::with_trajectory_store`] before wrapping.
    pub fn attach_evolution_stores(&self, skill: Arc<SkillStore>, trajectory: Arc<TrajectoryStore>) {
        if let Ok(mut g) = self.skill_store.write() {
            *g = Some(skill);
        } else {
            tracing::error!("skill_store RwLock poisoned during attach_evolution_stores");
        }
        if let Ok(mut g) = self.trajectory_store.write() {
            *g = Some(trajectory);
        } else {
            tracing::error!("trajectory_store RwLock poisoned during attach_evolution_stores");
        }
    }

    /// Attach the self-iteration / diagnosis engine for automatic tool-failure recovery hints.
    pub fn with_self_iter_engine(mut self, engine: Arc<SelfIterEngine>) -> Self {
        self.self_iter_engine = Some(engine);
        self
    }

    /// Cap how many recovery guidance injections are allowed per single `execute` / `execute_stream` run (default 3).
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
        let mut guard = self
            .agent_providers
            .write()
            .unwrap_or_else(|e| e.into_inner());
        guard.insert(agent_id.to_string(), provider);
    }

    /// Drop all per-agent provider overrides.
    ///
    /// The runtime-level default provider remains unchanged and is still used as
    /// fallback when an agent has no dedicated provider entry.
    pub fn clear_registered_providers(&self) {
        let mut guard = self
            .agent_providers
            .write()
            .unwrap_or_else(|e| e.into_inner());
        guard.clear();
    }

    fn resolve_provider(&self, agent_id: &str) -> anyhow::Result<Arc<dyn LlmProvider>> {
        let guard = self
            .agent_providers
            .read()
            .map_err(|e| anyhow::anyhow!("RwLock poisoned: {e}"))?;
        Ok(guard
            .get(agent_id)
            .cloned()
            .unwrap_or_else(|| self.default_provider.clone()))
    }

    pub async fn execute(
        &self,
        config: &AgentConfig,
        request: &ChatRequest,
        tool_registry: &ToolRegistry,
        llm_override: Option<Arc<dyn LlmProvider>>,
    ) -> anyhow::Result<ExecutionResult> {
        let max_iterations = config.behavior.max_tool_calls_per_turn;
        let max_errors = config.behavior.max_consecutive_errors;

        let mut messages = self.build_messages(config, &request.messages);
        let mut injected_skill_ids: Vec<String> = Vec::new();
        if let Err(e) = self
            .inject_relevant_skills(&mut messages, request, &mut injected_skill_ids)
            .await
        {
            tracing::warn!(error = %e, "skill injection skipped");
        }
        let mut trajectory_steps: Vec<TrajectoryStep> = Vec::new();
        let all_tool_defs = tool_registry.definitions();
        let tool_defs: Vec<_> = all_tool_defs
            .iter()
            .filter(|td| {
                let name = &td.function.name;
                if !memory_tool_visible_for_agent(name, &config.agent_id) {
                    return false;
                }
                if !config.behavior.tools_deny.is_empty()
                    && config.behavior.tools_deny.iter().any(|d| d == name)
                {
                    return false;
                }
                if !config.behavior.tools_allow.is_empty()
                    && !config.behavior.tools_allow.iter().any(|a| a == name)
                {
                    return false;
                }
                true
            })
            .cloned()
            .collect();
        let tools_for_llm = if tool_defs.is_empty() {
            None
        } else {
            Some(tool_defs.as_slice())
        };

        let temperature = request.temperature.unwrap_or(config.model.temperature);
        let max_tokens = request.max_tokens.or(config.model.max_tokens);
        let model = request.model.as_deref().unwrap_or(&config.model.model);

        let mut total_tool_calls: u32 = 0;
        let mut consecutive_errors: u32 = 0;
        let mut iteration: u32 = 0;
        let mut failure_streak_traces: Vec<ToolCallTrace> = Vec::new();
        let mut self_iter_recovery_used: u32 = 0;
        let mut error_limit_reached = false;

        loop {
            if error_limit_reached {
                tracing::warn!(
                    agent_id = %config.agent_id,
                    consecutive_errors,
                    "stopping outer loop — consecutive error limit reached"
                );
                self.finalize_injected_skills(&injected_skill_ids, false).await;
                self.record_completed_trajectory(request, config, &trajectory_steps, false)
                    .await;
                anyhow::bail!(
                    "agent '{}' stopped: {} consecutive tool errors",
                    config.agent_id,
                    consecutive_errors
                );
            }

            iteration += 1;

            tracing::info!(
                agent_id = %config.agent_id,
                model,
                iteration,
                msg_count = messages.len(),
                "LLM call"
            );

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
            let response = provider.chat_completion(&params).await?;

            let choice = response
                .choices
                .first()
                .ok_or_else(|| anyhow::anyhow!("LLM returned no choices"))?;

            let has_tool_calls = choice
                .message
                .tool_calls
                .as_ref()
                .is_some_and(|tc| !tc.is_empty());

            // Prefer tool_calls over finish_reason: some models send finish_reason "stop"
            // while still returning tool_calls; those tools must run.
            if !has_tool_calls {
                tracing::info!(
                    agent_id = %config.agent_id,
                    iterations = iteration,
                    total_tool_calls,
                    "agent execution complete"
                );
                self.finalize_injected_skills(&injected_skill_ids, true).await;
                self.record_completed_trajectory(request, config, &trajectory_steps, true)
                    .await;
                return Ok(ExecutionResult {
                    response,
                    tool_calls_made: total_tool_calls,
                    iterations: iteration,
                });
            }

            if iteration >= max_iterations {
                tracing::warn!(
                    agent_id = %config.agent_id,
                    max_iterations,
                    "tool call limit reached, returning last response"
                );
                self.finalize_injected_skills(&injected_skill_ids, true).await;
                self.record_completed_trajectory(request, config, &trajectory_steps, true)
                    .await;
                return Ok(ExecutionResult {
                    response,
                    tool_calls_made: total_tool_calls,
                    iterations: iteration,
                });
            }

            // Append assistant message with tool_calls
            messages.push(choice.message.clone());

            let tool_calls = choice
                .message
                .tool_calls
                .as_ref()
                .filter(|t| !t.is_empty())
                .ok_or_else(|| anyhow::anyhow!("LLM reported tool_calls but none were present"))?;

            // Execute tool calls in parallel (fork-join): spawn all concurrently,
            // then process results sequentially for correct message ordering.
            let tool_futures: Vec<_> = tool_calls.iter().map(|tc| {
                let tool_name = tc.function.name.clone();
                let call_id = tc.id.clone();
                let arguments = tc.function.arguments.clone();
                let registry = tool_registry.clone();
                let behavior = config.behavior.clone();
                let work_dir = request.work_dir.clone();
                async move {
                    if !is_tool_allowed(&tool_name, &behavior) {
                        tracing::warn!(tool = %tool_name, "tool blocked by allow/deny policy");
                        let msg = format!("tool '{}' is not allowed by agent policy", tool_name);
                        return (tool_name, call_id, fastclaw_core::tool::ToolResult::err(msg));
                    }
                    let result = match registry.get(&tool_name) {
                        Some(tool) => {
                            let work_dir_path = work_dir.as_ref().map(std::path::PathBuf::from);
                            with_file_access_mode(
                                behavior.file_access,
                                with_work_dir(work_dir_path, tool.execute(&arguments)),
                            )
                            .await
                        }
                        None => {
                            let msg = format!("tool not found: {}", tool_name);
                            fastclaw_core::tool::ToolResult::err(msg)
                        }
                    };
                    tracing::info!(
                        tool = %tool_name, success = result.success,
                        output_len = result.output.len(), "tool result"
                    );
                    (tool_name, call_id, result)
                }
            }).collect();

            let results = futures::future::join_all(tool_futures).await;

            for (tool_name, call_id, result) in results {
                total_tool_calls += 1;

                trajectory_steps.push(TrajectoryStep {
                    role: "assistant".into(),
                    action_type: "tool_call".into(),
                    tool_name: Some(tool_name.clone()),
                    summary: truncate_for_trajectory(""),
                    success: None,
                });

                if !result.success {
                    consecutive_errors += 1;
                    failure_streak_traces.push(ToolCallTrace {
                        tool_name: tool_name.clone(),
                        success: false,
                        latency_ms: 0,
                        error: Some(result.output.clone()),
                    });
                } else {
                    consecutive_errors = 0;
                    failure_streak_traces.clear();
                }

                let out = truncate_tool_result_output(&result.output);
                messages.push(ChatMessage {
                    role: Role::Tool,
                    content: Some(serde_json::Value::String(out)),
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
                    iteration,
                    consecutive_errors,
                    max_errors,
                    &failure_streak_traces,
                    &mut self_iter_recovery_used,
                ) {
                    consecutive_errors = 0;
                    failure_streak_traces.clear();
                }

                if consecutive_errors >= max_errors {
                    tracing::warn!(
                        agent_id = %config.agent_id,
                        consecutive_errors,
                        "consecutive error limit reached"
                    );
                    error_limit_reached = true;
                    break;
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
        tool_registry: &ToolRegistry,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
        llm_override: Option<Arc<dyn LlmProvider>>,
    ) -> anyhow::Result<(u32, u32)> {
        self.execute_stream_inner(config, request, tool_registry, tx, llm_override, None)
            .await
    }

    /// Same as `execute_stream` but accepts an optional `confirm_pending` map so the runtime
    /// can automatically present user-confirmation dialogs when tools return `needs_confirmation`.
    pub async fn execute_stream_with_confirm(
        &self,
        config: &AgentConfig,
        request: &ChatRequest,
        tool_registry: &ToolRegistry,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
        llm_override: Option<Arc<dyn LlmProvider>>,
        confirm_pending: Arc<tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<String>>>>,
    ) -> anyhow::Result<(u32, u32)> {
        self.execute_stream_inner(config, request, tool_registry, tx, llm_override, Some(confirm_pending))
            .await
    }

    async fn execute_stream_inner(
        &self,
        config: &AgentConfig,
        request: &ChatRequest,
        tool_registry: &ToolRegistry,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
        llm_override: Option<Arc<dyn LlmProvider>>,
        confirm_pending: Option<Arc<tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<String>>>>>,
    ) -> anyhow::Result<(u32, u32)> {
        let max_iterations = config.behavior.max_tool_calls_per_turn;
        let max_errors = config.behavior.max_consecutive_errors;

        let mut messages = self.build_messages(config, &request.messages);
        let mut injected_skill_ids: Vec<String> = Vec::new();
        if let Err(e) = self
            .inject_relevant_skills(&mut messages, request, &mut injected_skill_ids)
            .await
        {
            tracing::warn!(error = %e, "skill injection skipped (stream)");
        }
        let mut trajectory_steps: Vec<TrajectoryStep> = Vec::new();
        let all_tool_defs = tool_registry.definitions();
        let tool_defs: Vec<_> = all_tool_defs
            .iter()
            .filter(|td| {
                let name = &td.function.name;
                if !memory_tool_visible_for_agent(name, &config.agent_id) {
                    return false;
                }
                if !config.behavior.tools_deny.is_empty()
                    && config.behavior.tools_deny.iter().any(|d| d == name)
                {
                    return false;
                }
                if !config.behavior.tools_allow.is_empty()
                    && !config.behavior.tools_allow.iter().any(|a| a == name)
                {
                    return false;
                }
                true
            })
            .cloned()
            .collect();
        let tools_for_llm = if tool_defs.is_empty() {
            None
        } else {
            Some(tool_defs.as_slice())
        };

        let temperature = request.temperature.unwrap_or(config.model.temperature);
        let max_tokens = request.max_tokens.or(config.model.max_tokens);
        let model = request
            .model
            .as_deref()
            .unwrap_or(&config.model.model)
            .to_string();

        let mut total_tool_calls: u32 = 0;
        let mut consecutive_errors: u32 = 0;
        let mut iteration: u32 = 0;
        let mut failure_streak_traces: Vec<ToolCallTrace> = Vec::new();
        let mut self_iter_recovery_used: u32 = 0;
        let mut error_limit_reached = false;
        let stream_start = std::time::Instant::now();
        let mut acc_prompt_tokens: u32 = 0;
        let mut acc_completion_tokens: u32 = 0;

        loop {
            if error_limit_reached {
                tracing::warn!(
                    agent_id = %config.agent_id,
                    consecutive_errors,
                    "stopping outer stream loop — consecutive error limit reached"
                );
                let _ = send_stream_event(
                    &tx,
                    StreamEvent::Error(format!(
                        "agent stopped: {} consecutive tool errors",
                        consecutive_errors
                    )),
                    false,
                )
                .await;
                self.finalize_injected_skills(&injected_skill_ids, false).await;
                return Err(anyhow::anyhow!(
                    "agent '{}' stopped: {} consecutive tool errors",
                    config.agent_id,
                    consecutive_errors
                ));
            }

            iteration += 1;

            tracing::info!(
                agent_id = %config.agent_id,
                model = %model,
                iteration,
                msg_count = messages.len(),
                "streaming LLM call"
            );

            const MAX_STREAM_RESUME_ATTEMPTS: u32 = 5;
            let mut stream_resume_attempts: u32 = 0;

            let provider = match &llm_override {
                Some(p) => p.clone(),
                None => self.resolve_provider(&config.agent_id)?,
            };

            let mut accumulated_content = String::new();
            let mut tool_call_accum: Vec<ToolCallAccumulator> = Vec::new();
            let mut stream_errored = false;

            'stream_try: loop {
                let params = CompletionParams {
                    model: &model,
                    messages: &messages,
                    temperature,
                    max_tokens,
                    tools: tools_for_llm,
                };

                let mut stream = provider.chat_completion_stream(&params).await?;

                let mut should_resume = false;
                while let Some(result) = stream.next().await {
                    let delta = match result {
                        Ok(d) => d,
                        Err(e) => {
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
                                &tx,
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
                                accumulate_tool_call(&mut tool_call_accum, tc_delta);
                            }
                        }
                    }

                    if let Some(ref u) = delta.usage {
                        acc_prompt_tokens += u.prompt_tokens;
                        acc_completion_tokens += u.completion_tokens;
                    }

                    if tool_call_accum.is_empty() {
                        let _ = send_stream_event(&tx, StreamEvent::Delta(delta), true).await;
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

            if stream_errored {
                self.finalize_injected_skills(&injected_skill_ids, false).await;
                return Err(anyhow::anyhow!(
                    "provider stream error (already sent to client)"
                ));
            }

            let has_valid_tool_calls = tool_call_accum.iter().any(|a| !a.name.is_empty());

            // Same as non-streaming: never treat finish_reason "stop" as canceling a valid tool stream.
            let build_done_usage = || -> Option<fastclaw_core::types::Usage> {
                let total = acc_prompt_tokens + acc_completion_tokens;
                if total > 0 {
                    Some(fastclaw_core::types::Usage {
                        prompt_tokens: acc_prompt_tokens,
                        completion_tokens: acc_completion_tokens,
                        total_tokens: total,
                    })
                } else {
                    None
                }
            };

            if !has_valid_tool_calls {
                let _ = send_stream_event(
                    &tx,
                    StreamEvent::Done {
                        session_id: request.session_id.clone(),
                        tool_calls_made: total_tool_calls,
                        iterations: iteration,
                        final_tool_calls: None,
                        usage: build_done_usage(),
                        elapsed_ms: stream_start.elapsed().as_millis() as u64,
                        context_tokens: None,
                        context_window: None,
                    },
                    false,
                )
                .await;
                self.finalize_injected_skills(&injected_skill_ids, true).await;
                self.record_completed_trajectory(request, config, &trajectory_steps, true)
                    .await;
                return Ok((total_tool_calls, iteration));
            }

            if iteration >= max_iterations {
                tracing::warn!(
                    agent_id = %config.agent_id,
                    max_iterations,
                    "streaming tool call limit reached"
                );
                let final_tc: Vec<ToolCall> = tool_call_accum
                    .iter()
                    .filter(|a| !a.name.is_empty())
                    .map(|a| a.to_tool_call())
                    .collect();
                let _ = send_stream_event(
                    &tx,
                    StreamEvent::Done {
                        session_id: request.session_id.clone(),
                        tool_calls_made: total_tool_calls,
                        iterations: iteration,
                        final_tool_calls: if final_tc.is_empty() { None } else { Some(final_tc) },
                        usage: build_done_usage(),
                        elapsed_ms: stream_start.elapsed().as_millis() as u64,
                        context_tokens: None,
                        context_window: None,
                    },
                    false,
                )
                .await;
                self.finalize_injected_skills(&injected_skill_ids, true).await;
                self.record_completed_trajectory(request, config, &trajectory_steps, true)
                    .await;
                return Ok((total_tool_calls, iteration));
            }

            let assembled_calls: Vec<ToolCall> = tool_call_accum
                .iter()
                .filter(|a| !a.name.is_empty())
                .map(|a| a.to_tool_call())
                .collect();

            if assembled_calls.is_empty() {
                tracing::warn!("stream tool call deltas produced no valid tool calls, stopping");
                let _ = send_stream_event(
                    &tx,
                    StreamEvent::Done {
                        session_id: request.session_id.clone(),
                        tool_calls_made: total_tool_calls,
                        iterations: iteration,
                        final_tool_calls: None,
                        usage: build_done_usage(),
                        elapsed_ms: stream_start.elapsed().as_millis() as u64,
                        context_tokens: None,
                        context_window: None,
                    },
                    false,
                )
                .await;
                self.finalize_injected_skills(&injected_skill_ids, true).await;
                self.record_completed_trajectory(request, config, &trajectory_steps, true)
                    .await;
                return Ok((total_tool_calls, iteration));
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
                    &tx,
                    StreamEvent::ToolExecuting {
                        tool_name: tc.function.name.clone(),
                        call_id: tc.id.clone(),
                        args: args_str,
                    },
                    false,
                )
                .await;
            }

            // Execute all tool calls in parallel (fork-join).
            let stream_tool_futures: Vec<_> = assembled_calls.iter().map(|tc| {
                let tool_name = tc.function.name.clone();
                let call_id = tc.id.clone();
                let arguments = tc.function.arguments.clone();
                let registry = tool_registry.clone();
                let behavior = config.behavior.clone();
                let work_dir = request.work_dir.clone();
                async move {
                    if !is_tool_allowed(&tool_name, &behavior) {
                        tracing::warn!(tool = %tool_name, "tool blocked by allow/deny policy (stream)");
                        let msg = format!("tool '{}' is not allowed by agent policy", tool_name);
                        return (tool_name, call_id, arguments, fastclaw_core::tool::ToolResult::err(msg));
                    }
                    let result = match registry.get(&tool_name) {
                        Some(tool) => {
                            let work_dir_path = work_dir.as_ref().map(std::path::PathBuf::from);
                            with_file_access_mode(
                                behavior.file_access,
                                with_work_dir(work_dir_path, tool.execute(&arguments)),
                            )
                            .await
                        }
                        None => {
                            let msg = format!("tool not found: {}", tool_name);
                            fastclaw_core::tool::ToolResult::err(msg)
                        }
                    };
                    tracing::info!(
                        tool = %tool_name, success = result.success,
                        output_len = result.output.len(), "tool result (stream)"
                    );
                    (tool_name, call_id, arguments, result)
                }
            }).collect();

            let stream_results = futures::future::join_all(stream_tool_futures).await;

            // Process results sequentially for ordering, confirmation, and state updates.
            for (tool_name, call_id, arguments, mut result) in stream_results {
                total_tool_calls += 1;

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
                        {
                            let mut guard = pending_map.lock().await;
                            guard.insert(confirm_request_id.clone(), answer_tx);
                        }

                        let _ = send_stream_event(
                            &tx,
                            StreamEvent::AskQuestion {
                                request_id: confirm_request_id.clone(),
                                question: result.output.clone(),
                                options: vec![
                                    AskQuestionOption { id: "allow".into(), label: "Allow".into() },
                                    AskQuestionOption { id: "deny".into(), label: "Deny".into() },
                                ],
                                timeout_secs: 60,
                                allow_multiple: false,
                            },
                            true,
                        )
                        .await;

                        let user_answer = tokio::time::timeout(
                            Duration::from_secs(60),
                            answer_rx,
                        )
                        .await;

                        {
                            let mut guard = pending_map.lock().await;
                            guard.remove(&confirm_request_id);
                        }

                        let approved = matches!(user_answer, Ok(Ok(ref a)) if a == "allow");

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
                                    with_work_dir(work_dir_path, tool.execute(&confirmed_args)),
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
                    consecutive_errors += 1;
                    failure_streak_traces.push(ToolCallTrace {
                        tool_name: tool_name.clone(),
                        success: false,
                        latency_ms: 0,
                        error: Some(result.output.clone()),
                    });
                } else {
                    consecutive_errors = 0;
                    failure_streak_traces.clear();
                }

                let llm_out = truncate_tool_result_output(&result.output);
                let _ = send_stream_event(
                    &tx,
                    StreamEvent::ToolResult {
                        tool_name: tool_name.clone(),
                        call_id: call_id.clone(),
                        output: result.output.clone(),
                        success: result.success,
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

                messages.push(ChatMessage {
                    role: Role::Tool,
                    content: Some(serde_json::Value::String(llm_out)),
                    name: Some(tool_name.clone()),
                    tool_calls: None,
                    tool_call_id: Some(call_id),
                });

                if self.try_self_iter_tool_recovery(
                    &mut messages,
                    config,
                    request,
                    iteration,
                    consecutive_errors,
                    max_errors,
                    &failure_streak_traces,
                    &mut self_iter_recovery_used,
                ) {
                    consecutive_errors = 0;
                    failure_streak_traces.clear();
                }

                if consecutive_errors >= max_errors {
                    tracing::warn!(
                        agent_id = %config.agent_id,
                        consecutive_errors,
                        "consecutive error limit reached (stream)"
                    );
                    error_limit_reached = true;
                    break;
                }
            }
        }
    }

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

    /// When consecutive tool errors hit the trigger threshold, run `SelfIterEngine` diagnosis
    /// and merge recovery text into the primary system prompt (Anthropic-safe single-system).
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

    async fn finalize_injected_skills(&self, injected_skill_ids: &[String], success: bool) {
        let store = match self.skill_store.read() {
            Ok(g) => g.clone(),
            Err(e) => {
                tracing::warn!(error = %e, "skill_store lock poisoned in finalize_injected_skills");
                return;
            }
        };
        let Some(store) = store else {
            return;
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
        if !run_succeeded {
            return;
        }
        let store = match self.trajectory_store.read() {
            Ok(g) => g.clone(),
            Err(e) => {
                tracing::warn!(error = %e, "trajectory_store lock poisoned");
                return;
            }
        };
        let Some(store) = store else {
            return;
        };

        let session_id = request
            .session_id
            .clone()
            .unwrap_or_else(|| "default".to_string());
        let trajectory = Trajectory {
            id: uuid::Uuid::new_v4().to_string(),
            agent_id: config.agent_id.clone(),
            session_id,
            task_type: infer_task_type(steps),
            steps: steps.to_vec(),
            outcome: TrajectoryOutcome::Success { user_rating: None },
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        if let Err(e) = store.record_trajectory(&trajectory).await {
            tracing::warn!(error = %e, "trajectory record failed");
        }
    }

    /// Loads matching **active** skills (plus a few **candidate** skills) and appends guidance.
    async fn inject_relevant_skills(
        &self,
        messages: &mut Vec<ChatMessage>,
        request: &ChatRequest,
        injected_skill_ids: &mut Vec<String>,
    ) -> anyhow::Result<()> {
        let store = match self.skill_store.read() {
            Ok(g) => g.clone(),
            Err(e) => anyhow::bail!("skill_store lock poisoned: {e}"),
        };
        let Some(store) = store else {
            return Ok(());
        };
        let task = last_user_turn_text(&request.messages);
        if task.trim().is_empty() {
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
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
        );
    }

    fn build_messages(
        &self,
        config: &AgentConfig,
        user_messages: &[ChatMessage],
    ) -> Vec<ChatMessage> {
        let mut messages = Vec::with_capacity(user_messages.len() + 1);

        let configured = config
            .system_prompt
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());

        if let Some(prompt) = configured {
            messages.push(ChatMessage {
                role: Role::System,
                content: Some(serde_json::Value::String(prompt.to_string())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            });
        } else {
            messages.push(ChatMessage {
                role: Role::System,
                content: Some(serde_json::Value::String(
                    default_runtime_system_prompt_for_agent(&config.agent_id),
                )),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            });
        }

        messages.extend_from_slice(user_messages);
        messages
    }
}

const SKILL_MANAGEMENT_GUIDANCE: &str = "\n\n\
[Skill Management]\n\
When you successfully complete a complex, multi-step task:\n\
1. Consider if the approach could be reused. If so, use `write_skill` to save it as a reusable skill.\n\
2. If an existing skill was helpful but could be improved, use `read_skill` + `write_skill` to refine it.\n\
3. Good skill candidates: tasks with 3+ tool calls, recurring patterns, domain-specific workflows.\n\
4. Keep skills concise: task pattern, key steps, tool sequence, and any gotchas.\n\
Do NOT create skills for trivial single-step tasks or pure conversation.\n";

fn last_user_turn_text(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .rev()
        .filter(|m| matches!(m.role, Role::User))
        .find_map(|m| m.text_content())
        .unwrap_or_default()
}

fn truncate_for_trajectory(s: &str) -> String {
    const MAX_CHARS: usize = 400;
    let mut iter = s.chars();
    let chunk: String = iter.by_ref().take(MAX_CHARS).collect();
    if iter.next().is_some() {
        format!("{chunk}…")
    } else {
        chunk
    }
}

/// Accumulates streaming tool call deltas into a complete tool call.
struct ToolCallAccumulator {
    id: String,
    name: String,
    arguments: String,
}

impl ToolCallAccumulator {
    fn to_tool_call(&self) -> ToolCall {
        ToolCall {
            id: self.id.clone(),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: self.name.clone(),
                arguments: self.arguments.clone(),
            },
            output: None,
            success: None,
            duration_ms: None,
        }
    }
}

fn accumulate_tool_call(
    accum: &mut Vec<ToolCallAccumulator>,
    delta: &fastclaw_core::types::StreamToolCallDelta,
) {
    let idx = delta.index as usize;

    while accum.len() <= idx {
        accum.push(ToolCallAccumulator {
            id: String::new(),
            name: String::new(),
            arguments: String::new(),
        });
    }

    let entry = &mut accum[idx];

    if let Some(ref id) = delta.id {
        if !id.is_empty() {
            entry.id = id.clone();
        }
    }

    if let Some(ref func) = delta.function {
        if let Some(ref name) = func.name {
            if !name.is_empty() {
                entry.name = name.clone();
            }
        }
        if let Some(ref args) = func.arguments {
            entry.arguments.push_str(args);
        }
    }
}

#[cfg(test)]
mod tool_result_truncation_tests {
    use super::{truncate_tool_result_output, MAX_TOOL_RESULT_CHARS};

    #[test]
    fn no_truncation_at_or_below_char_limit() {
        let s = "a".repeat(MAX_TOOL_RESULT_CHARS);
        let out = truncate_tool_result_output(&s);
        assert_eq!(out, s);
        assert!(!out.contains("truncated"));
    }

    #[test]
    fn truncates_long_output_and_suffix_reports_total_chars() {
        let total = MAX_TOOL_RESULT_CHARS + 999;
        let s = "a".repeat(total);
        let out = truncate_tool_result_output(&s);
        let expected_suffix = format!(
            "\n... (truncated, showing first {MAX_TOOL_RESULT_CHARS} of {total} chars)"
        );
        assert!(out.ends_with(&expected_suffix), "got len {}", out.len());
        assert_eq!(
            out.chars().take(MAX_TOOL_RESULT_CHARS).collect::<String>(),
            "a".repeat(MAX_TOOL_RESULT_CHARS)
        );
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
        ChatMessage, ChatRequest, DeltaContent, Role, StreamChoice, StreamDelta,
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
        let registry = ToolRegistry::new();
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
