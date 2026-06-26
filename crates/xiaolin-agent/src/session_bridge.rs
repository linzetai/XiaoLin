//! Bridge between `xiaolin-session-actor`'s `TurnExecutor` trait and
//! `AgentRuntime`'s `execute_unified`.
//!
//! With Phase A, the relay tasks have been removed. `InteractionHandle` is
//! passed directly to `ToolOrchestrator` (via `AgentContext`) and to builtin
//! tools (via task-local), so approvals and answers resolve without polling.

use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use dashmap::DashMap;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use xiaolin_core::agent_config::AgentConfig;
use xiaolin_core::tool::ToolRegistry;
use xiaolin_core::types::ChatRequest;
use xiaolin_protocol::{AgentEvent, AgentId, SessionId, TurnId};

use xiaolin_session_actor::{InteractionHandle, TurnError, TurnExecutor, TurnParams, TurnResult};

use crate::llm::LlmProvider;
use crate::reactive_loop;
use crate::AgentRuntime;

fn derive_approval_strategy(
    behavior: &xiaolin_core::agent_config::BehaviorConfig,
    goal_auto_approve: bool,
) -> xiaolin_core::tool_runtime::ApprovalStrategy {
    if goal_auto_approve {
        return xiaolin_core::tool_runtime::ApprovalStrategy::AutoApprove;
    }
    if let Some(ref strategy) = behavior.approval_strategy {
        if strategy.eq_ignore_ascii_case("auto_approve")
            || strategy.eq_ignore_ascii_case("autoapprove")
        {
            return xiaolin_core::tool_runtime::ApprovalStrategy::AutoApprove;
        }
    }
    xiaolin_core::tool_runtime::ApprovalStrategy::Interactive
}

async fn session_has_actionable_goal(
    session_store: Option<&Arc<xiaolin_session::SessionStore>>,
    session_id: &str,
) -> bool {
    match session_store {
        Some(store) => store
            .get_actionable_goal(session_id)
            .await
            .ok()
            .flatten()
            .is_some(),
        None => false,
    }
}

/// Adapter implementing `TurnExecutor` by delegating to `AgentRuntime`.
///
/// `InteractionHandle` is threaded into two places:
/// 1. `AgentContext.interaction_handle` — used by `ToolOrchestrator` for
///    approval resolution.
/// 2. Task-local `TASK_INTERACTION_HANDLE` — used by builtin tools
///    (`ask_question`, `confirm`) for answer resolution.
pub struct RuntimeTurnExecutor {
    pub runtime: Arc<AgentRuntime>,
    pub config: AgentConfig,
    pub tool_registry: Arc<ToolRegistry>,
    pub llm_override: Option<Arc<dyn LlmProvider>>,
    pub session_store: Option<Arc<xiaolin_session::SessionStore>>,
    pub mode_registry: Option<crate::builtin_tools::SessionModeRegistry>,
    pub todo_store: Option<crate::builtin_tools::TodoStore>,
    pub goal_store: Option<Arc<crate::builtin_tools::GoalStore>>,
    pub plan_file_store: Option<crate::builtin_tools::PlanFileStore>,
    pub stream_event_tx: Option<Arc<DashMap<String, mpsc::Sender<AgentEvent>>>>,
    pub subagent_manager: Option<Arc<crate::SubAgentManager>>,
    pub tool_orchestrator: Option<Arc<crate::runtime::orchestrator::ToolOrchestrator>>,
    /// Per-session BehaviorConfig overrides (set via permission presets).
    /// Key is session_id, value is the resolved BehaviorConfig for that session.
    pub behavior_overrides:
        Option<Arc<DashMap<String, xiaolin_core::agent_config::BehaviorConfig>>>,
    /// Lock-free access to the latest hot-reloaded agent configs.
    /// Falls back to `self.config` if None or empty.
    pub live_agents: Option<Arc<ArcSwap<Vec<AgentConfig>>>>,
    /// Persistent cost store for SQLite-backed analytics.
    pub cost_store: Option<Arc<xiaolin_session::CostStore>>,
    /// Persistent runtime-quality store for per-turn diagnostics.
    pub runtime_quality_store: Option<Arc<xiaolin_session::RuntimeQualityStore>>,
    /// File artifact store for tracking agent file operations.
    pub artifact_store: Option<Arc<dyn xiaolin_session::ArtifactStore>>,
}

impl RuntimeTurnExecutor {
    /// Resolve the effective BehaviorConfig for a session.
    /// Priority: per-session override > live hot-reloaded config > startup snapshot.
    fn effective_behavior(&self, session_id: &str) -> xiaolin_core::agent_config::BehaviorConfig {
        if let Some(ref overrides) = self.behavior_overrides {
            if let Some(entry) = overrides.get(session_id) {
                return entry.value().clone();
            }
        }
        if let Some(ref live) = self.live_agents {
            let agents = live.load();
            if let Some(agent) = agents.first() {
                return agent.behavior.clone();
            }
        }
        self.config.behavior.clone()
    }

    /// Run explicit context compaction (manual `/compact` or `SessionOp::Compact`).
    async fn execute_compact(
        &self,
        params: &TurnParams,
        tx: &mpsc::Sender<AgentEvent>,
    ) -> Result<TurnResult, TurnError> {
        let session_id = params.session_id.to_string();
        let Some(ref store) = self.session_store else {
            return Err(TurnError::Runtime {
                message: "session store not available for compaction".into(),
                code: xiaolin_protocol::event::ErrorCode::Other,
            });
        };

        let mut messages =
            std::sync::Arc::try_unwrap(store.load_chat_messages(&session_id).await.map_err(
                |e| TurnError::Runtime {
                    message: format!("failed to load messages: {e}"),
                    code: xiaolin_protocol::event::ErrorCode::Other,
                },
            )?)
            .unwrap_or_else(|arc| (*arc).clone());

        let pre_count = messages.len();
        let pre_tokens = xiaolin_context::compressor::estimate_messages_tokens(&messages);

        let context_window = self.config.model.context_window.unwrap_or(128_000);

        xiaolin_context::ContextEngine::fit_to_context_window(
            &mut messages,
            context_window,
            self.config.model.max_tokens,
        );

        let post_tokens = xiaolin_context::compressor::estimate_messages_tokens(&messages);
        let post_count = messages.len();
        let removed = pre_count.saturating_sub(post_count);

        if removed > 0 {
            if let Err(e) = store.replace_messages(&session_id, &messages).await {
                tracing::warn!(error = %e, "failed to persist compacted messages");
            }
        }

        let _ = tx
            .send(AgentEvent::CompactBoundary {
                turn_id: params.turn_id.clone(),
                trigger: xiaolin_protocol::CompactTrigger::Manual,
                pre_compact_tokens: pre_tokens,
                post_compact_tokens: post_tokens,
                messages_removed: removed,
            })
            .await;

        tracing::info!(
            session_id = %session_id,
            pre_tokens,
            post_tokens,
            messages_removed = removed,
            "manual compaction complete"
        );

        Ok(TurnResult {
            tool_calls_made: 0,
            iterations: 0,
            usage: None,
        })
    }

    /// Auto-compact if the current session's messages exceed a token threshold.
    async fn maybe_auto_compact(&self, params: &TurnParams, tx: &mpsc::Sender<AgentEvent>) {
        let compact_t0 = std::time::Instant::now();
        let Some(ref store) = self.session_store else {
            tracing::info!(
                elapsed_ms = compact_t0.elapsed().as_millis() as u64,
                "perf: auto_compact_check"
            );
            return;
        };
        let context_window = self.config.model.context_window.unwrap_or(128_000);

        // Trigger auto-compact at 85% of context window.
        let threshold = (context_window as f64 * 0.85) as usize;
        let session_id = params.session_id.to_string();

        let messages_arc = match store.load_chat_messages(&session_id).await {
            Ok(m) => m,
            Err(_) => {
                tracing::info!(
                    elapsed_ms = compact_t0.elapsed().as_millis() as u64,
                    "perf: auto_compact_check"
                );
                return;
            }
        };

        let estimated = xiaolin_context::compressor::estimate_messages_tokens(&messages_arc);
        if estimated <= threshold {
            tracing::info!(
                elapsed_ms = compact_t0.elapsed().as_millis() as u64,
                "perf: auto_compact_check"
            );
            return;
        }

        let mut messages =
            std::sync::Arc::try_unwrap(messages_arc).unwrap_or_else(|arc| (*arc).clone());

        tracing::info!(
            session_id = %session_id,
            estimated,
            threshold,
            context_window,
            "auto-compacting: token estimate exceeds threshold"
        );

        let pre_tokens = estimated;
        let pre_count = messages.len();

        xiaolin_context::ContextEngine::fit_to_context_window(
            &mut messages,
            context_window,
            self.config.model.max_tokens,
        );

        let post_tokens = xiaolin_context::compressor::estimate_messages_tokens(&messages);
        let post_count = messages.len();
        let removed = pre_count.saturating_sub(post_count);

        if removed > 0 {
            if let Err(e) = store.replace_messages(&session_id, &messages).await {
                tracing::warn!(error = %e, "failed to persist auto-compacted messages");
            }

            let _ = tx
                .send(AgentEvent::CompactBoundary {
                    turn_id: params.turn_id.clone(),
                    trigger: xiaolin_protocol::CompactTrigger::Auto,
                    pre_compact_tokens: pre_tokens,
                    post_compact_tokens: post_tokens,
                    messages_removed: removed,
                })
                .await;
        }

        tracing::info!(
            elapsed_ms = compact_t0.elapsed().as_millis() as u64,
            "perf: auto_compact_check"
        );
    }
}

impl RuntimeTurnExecutor {
    fn request_from_extra(params: &xiaolin_session_actor::turn::TurnParams) -> ChatRequest {
        params
            .extra
            .get("_enriched_request")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_else(|| ChatRequest {
                messages: serde_json::from_value(params.messages.clone()).unwrap_or_default(),
                session_id: Some(SessionId::new(params.session_id.to_string())),
                agent_id: Some(AgentId::new(params.agent_id.clone())),
                model: params.model.clone(),
                max_tokens: None,
                temperature: None,
                stream: true,
                tools: None,
                work_dir: params.work_dir.clone(),
                slash_intent: None,
                response_language: None,
                goal_mode: None,
            })
    }

    fn config_from_extra(
        params: &xiaolin_session_actor::turn::TurnParams,
        default: &AgentConfig,
    ) -> AgentConfig {
        params
            .extra
            .get("_agent_config")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_else(|| default.clone())
    }

    /// Build the per-turn active sub-agent status block for injection into the
    /// last user message (NOT the system prompt). Recomputed each turn / re-prompt
    /// with fresh `elapsed_ms` so the cacheable system prefix stays byte-stable.
    fn build_active_runs_context(&self, session_id: &str) -> Option<String> {
        let mgr = self.subagent_manager.as_ref()?;
        let active = mgr.active_runs(session_id);
        // Running runs only persist `elapsed_ms` at completion, so derive a live
        // elapsed from `created_at` for in-flight workers (else they always show 0s).
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let summaries: Vec<crate::runtime::ActiveRunSummary> = active
            .iter()
            .map(|r| crate::runtime::ActiveRunSummary {
                run_id: r.run_id.clone(),
                subagent_type: r.subagent_type.to_string(),
                task: r.task.clone(),
                elapsed_ms: r
                    .elapsed_ms
                    .unwrap_or_else(|| now_ms.saturating_sub(r.created_at)),
                tool_calls_made: r.tool_calls_made,
                current_tool: r.current_tool.clone(),
            })
            .collect();
        crate::build_active_runs_context(&summaries)
    }

    /// Reactive loop: after initial execute_unified, if sub-agents are active,
    /// wait for completions → inject notification → re-prompt until all done.
    #[allow(clippy::too_many_arguments)]
    async fn run_reactive_loop(
        &self,
        first_result: anyhow::Result<xiaolin_protocol::TurnSummary>,
        config: &AgentConfig,
        request: &ChatRequest,
        session_id: &str,
        tx: &mpsc::Sender<AgentEvent>,
        orchestrator: &Arc<crate::runtime::orchestrator::ToolOrchestrator>,
        llm: &Option<Arc<dyn LlmProvider>>,
        interaction: &InteractionHandle,
        subagent_prompt: &Option<String>,
        mode_state: &Option<crate::builtin_tools::ExecutionModeState>,
        plan_ctx: &Option<crate::builtin_tools::PlanContext>,
        session_store: &Option<Arc<xiaolin_session::SessionStore>>,
        todo_store: &Option<crate::builtin_tools::TodoStore>,
        stream_context_key: &str,
        cancel: &CancellationToken,
        approval_strategy: xiaolin_core::tool_runtime::ApprovalStrategy,
    ) -> anyhow::Result<xiaolin_protocol::TurnSummary> {
        let mgr = self
            .subagent_manager
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("subagent manager required for reactive loop"))?;
        let policy = &config.behavior.subagent;
        let batch_window = Duration::from_millis(policy.batch_window_ms);
        let max_reprompts = policy.max_reprompts_per_turn;
        let suppress_ack = policy.suppress_intermediate_ack;

        let mut accumulated_result = first_result?;
        let mut reprompt_count: u32 = 0;

        loop {
            let active = mgr.active_runs(session_id);
            if active.is_empty() {
                tracing::debug!(
                    session_id,
                    reprompt_count,
                    "reactive loop: no active sub-agents, turn complete"
                );
                break;
            }

            if reprompt_count >= max_reprompts {
                tracing::warn!(
                    session_id,
                    reprompt_count,
                    max_reprompts,
                    "reactive loop: max reprompts reached, ending turn with active sub-agents"
                );
                break;
            }

            tracing::info!(
                session_id,
                active_count = active.len(),
                reprompt_count,
                "reactive loop: waiting for sub-agent completions"
            );

            // Subscribe and wait for completions with batch window.
            let mut completion_rx = mgr.subscribe_completions(session_id);

            let completions = tokio::select! {
                c = reactive_loop::wait_for_completions(&mut completion_rx, batch_window) => c,
                () = cancel.cancelled() => {
                    return Err(anyhow::anyhow!("cancelled by session actor"));
                }
            };

            if completions.is_empty() {
                tracing::debug!(session_id, "reactive loop: channel closed, ending");
                break;
            }

            let remaining_active = mgr.active_runs(session_id).len() as u32;
            let turn_id = TurnId::generate();

            // Emit notification event for frontend.
            reactive_loop::emit_notification_event(tx, &turn_id, &completions, remaining_active)
                .await;

            // Build notification message and inject into conversation.
            let notification_text = reactive_loop::build_completion_notification(
                &completions,
                remaining_active as usize,
            );

            // Build a lightweight reprompt request: only clone the notification
            // message instead of the entire message history, and load from the
            // session store which already caches messages in memory.
            let reprompt_messages = if let Some(ref store) = session_store {
                if let Some(ref sid) = request.session_id {
                    match store.load_chat_messages(&sid.to_string()).await {
                        Ok(arc) => {
                            let mut messages =
                                std::sync::Arc::try_unwrap(arc).unwrap_or_else(|a| (*a).clone());
                            messages.push(reactive_loop::notification_as_system_message(
                                &notification_text,
                            ));
                            messages
                        }
                        Err(_) => {
                            let mut msgs = request.messages.clone();
                            msgs.push(reactive_loop::notification_as_system_message(
                                &notification_text,
                            ));
                            msgs
                        }
                    }
                } else {
                    let mut msgs = request.messages.clone();
                    msgs.push(reactive_loop::notification_as_system_message(
                        &notification_text,
                    ));
                    msgs
                }
            } else {
                let mut msgs = request.messages.clone();
                msgs.push(reactive_loop::notification_as_system_message(
                    &notification_text,
                ));
                msgs
            };

            let reprompt_request = ChatRequest {
                messages: reprompt_messages,
                session_id: request.session_id.clone(),
                agent_id: request.agent_id.clone(),
                model: request.model.clone(),
                max_tokens: request.max_tokens,
                temperature: request.temperature,
                stream: request.stream,
                tools: None,
                work_dir: request.work_dir.clone(),
                slash_intent: None,
                response_language: None,
                goal_mode: request.goal_mode,
            };

            // Re-prompt LLM.
            let runtime = self.runtime.clone();
            let tool_registry = self.tool_registry.clone();
            let ih_for_tools = interaction.clone();
            let stream_ctx_key_inner = stream_context_key.to_string();
            let mode_state_c = mode_state.clone();
            let plan_ctx_c = plan_ctx.clone();

            // Recompute active sub-agent status with fresh `elapsed_ms` for this
            // re-prompt; injected into the user message, not the system prompt.
            let active_runs_context = self.build_active_runs_context(session_id);

            let runtime_fut: std::pin::Pin<
                Box<
                    dyn std::future::Future<Output = anyhow::Result<xiaolin_protocol::TurnSummary>>
                        + Send,
                >,
            > = Box::pin(runtime.execute_unified_with_cost_store(
                config,
                &reprompt_request,
                &tool_registry,
                tx.clone(),
                approval_strategy.clone(),
                llm.clone(),
                orchestrator.clone(),
                Some(interaction.clone()),
                subagent_prompt.clone(),
                mode_state.clone(),
                session_store.clone(),
                todo_store.clone(),
                self.goal_store.clone(),
                self.cost_store.clone(),
                self.runtime_quality_store.clone(),
                self.artifact_store.clone(),
                self.behavior_overrides.clone(),
                None,
                active_runs_context,
            ));

            let reprompt_result = {
                let session_id_for_scope = session_id.to_string();
                let wrapped_fut = async move {
                    let runtime_with_ih =
                        crate::builtin_tools::with_interaction_handle(ih_for_tools, runtime_fut);
                    let runtime_with_session =
                        crate::with_subagent_session_id(session_id_for_scope, runtime_with_ih);
                    if let Some(ms) = mode_state_c {
                        crate::builtin_tools::with_stream_context(
                            stream_ctx_key_inner,
                            crate::builtin_tools::with_session_mode(
                                ms,
                                plan_ctx_c,
                                runtime_with_session,
                            ),
                        )
                        .await
                    } else {
                        crate::builtin_tools::with_stream_context(
                            stream_ctx_key_inner,
                            runtime_with_session,
                        )
                        .await
                    }
                };

                tokio::select! {
                    r = wrapped_fut => r,
                    () = cancel.cancelled() => Err(anyhow::anyhow!("cancelled by session actor")),
                }
            };

            match reprompt_result {
                Ok(summary) => {
                    // If suppress_ack is enabled and the LLM just acknowledged
                    // without doing anything meaningful, don't count it as progress.
                    if suppress_ack
                        && reactive_loop::is_intermediate_ack(&summary)
                        && !mgr.active_runs(session_id).is_empty()
                    {
                        tracing::debug!(session_id, "reactive loop: suppressing intermediate ack");
                    }

                    accumulated_result.tool_calls_made += summary.tool_calls_made;
                    accumulated_result.iterations += summary.iterations;
                    reprompt_count += 1;
                }
                Err(e) => {
                    tracing::error!(
                        session_id,
                        error = %e,
                        "reactive loop: re-prompt failed, ending loop"
                    );
                    return Err(e);
                }
            }
        }

        Ok(accumulated_result)
    }
}

#[async_trait::async_trait]
impl TurnExecutor for RuntimeTurnExecutor {
    async fn execute(
        &self,
        params: TurnParams,
        interaction: InteractionHandle,
        tx: mpsc::Sender<AgentEvent>,
        cancel: CancellationToken,
    ) -> Result<TurnResult, TurnError> {
        let execute_t0 = std::time::Instant::now();
        let is_compact = params
            .extra
            .get("_compact")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if is_compact {
            let result = self.execute_compact(&params, &tx).await;
            tracing::info!(
                elapsed_ms = execute_t0.elapsed().as_millis() as u64,
                "perf: bridge_execute_total"
            );
            return result;
        }

        // Auto-compact: check message token count before starting the turn.
        // If exceeding threshold, run inline compaction.
        self.maybe_auto_compact(&params, &tx).await;

        let (request, config, per_request_llm) = if let Some(ref td) = params.typed_data {
            if let Some(typed) = xiaolin_core::typed_turn_data::TypedTurnData::extract(td) {
                let llm: Option<Arc<dyn LlmProvider>> =
                    typed.llm_override.as_ref().and_then(|any| {
                        let result = any.downcast_ref::<Arc<dyn LlmProvider>>().cloned();
                        if result.is_none() {
                            tracing::warn!(
                                type_id = ?std::any::Any::type_id(any.as_ref()),
                                expected_type_id = ?std::any::TypeId::of::<Arc<dyn LlmProvider>>(),
                                "llm_override downcast failed — provider override will be lost"
                            );
                        }
                        result
                    });
                tracing::info!(
                    has_llm_override = typed.llm_override.is_some(),
                    downcast_ok = llm.is_some(),
                    request_model = ?typed.enriched_request.model,
                    "session_bridge: extracted TypedTurnData"
                );
                (
                    typed.enriched_request.clone(),
                    typed.agent_config.clone(),
                    llm,
                )
            } else {
                (
                    Self::request_from_extra(&params),
                    Self::config_from_extra(&params, &self.config),
                    None,
                )
            }
        } else {
            (
                Self::request_from_extra(&params),
                Self::config_from_extra(&params, &self.config),
                None,
            )
        };

        // Apply per-session permission preset overrides if any.
        let mut config = config;
        let sid = params.session_id.to_string();
        let has_override = self
            .behavior_overrides
            .as_ref()
            .map_or(false, |m| m.contains_key(&sid));
        let effective_behavior = self.effective_behavior(&sid);
        let goal_mode_flag = params
            .extra
            .get("goalMode")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || request.goal_mode.unwrap_or(false);
        let has_actionable_goal =
            session_has_actionable_goal(self.session_store.as_ref(), &sid).await;
        let has_approved_plan = self
            .plan_file_store
            .as_ref()
            .map(|store| store.plan_path(&sid).exists())
            .unwrap_or(false);
        let goal_auto_approve = goal_mode_flag || has_actionable_goal || has_approved_plan;
        let approval_strategy = derive_approval_strategy(&effective_behavior, goal_auto_approve);
        tracing::debug!(
            session_id = %sid,
            has_override,
            goal_mode_flag,
            has_actionable_goal,
            has_approved_plan,
            goal_auto_approve,
            tools_deny = ?effective_behavior.tools_deny,
            approval_strategy = ?effective_behavior.approval_strategy,
            derived_strategy = ?approval_strategy,
            "permission preset applied"
        );
        config.behavior = effective_behavior;

        // Fallback: if the request has no work_dir, try to load from session store.
        let mut request = request;
        if request.work_dir.is_none() {
            if let Some(ref store) = self.session_store {
                if let Ok(Some(session)) = store.get_session(&sid).await {
                    if session.work_dir.is_some() {
                        request.work_dir = session.work_dir;
                    }
                }
            }
        }

        let orchestrator = self
            .tool_orchestrator
            .clone()
            .unwrap_or_else(|| Arc::new(crate::runtime::orchestrator::ToolOrchestrator::new()));

        // Wrap the outbound tx to inject session_id into interaction events.
        let session_id_str = params.session_id.to_string();
        let (inner_tx, mut inner_rx) = mpsc::channel::<AgentEvent>(128);

        let stream_context_key = params
            .extra
            .get("_stream_context_key")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        if let Some(ref map) = self.stream_event_tx {
            map.insert(stream_context_key.clone(), inner_tx.clone());
        }

        let mode_state = self
            .mode_registry
            .as_ref()
            .map(|r| r.get_or_create(&params.session_id.to_string()));

        let plan_ctx =
            self.plan_file_store
                .as_ref()
                .map(|store| crate::builtin_tools::PlanContext {
                    session_id: params.session_id.to_string(),
                    store: store.clone(),
                });

        if let Some(ref mgr) = self.subagent_manager {
            mgr.register_session_tx(&session_id_str, inner_tx.clone());
        }

        let subagent_prompt = self.subagent_manager.as_ref().and_then(|mgr| {
            let policy = &config.behavior.subagent;
            let available = mgr.agent_descriptions();
            let ctx = crate::SubAgentPromptContext {
                policy,
                available_agents: &available,
                current_depth: 0,
            };
            crate::build_subagent_prompt_block(&ctx)
        });

        // Active sub-agent status is injected per-turn into the last user message
        // (not the system prompt) to keep the cacheable system prefix byte-stable
        // even as `elapsed_ms` changes (prompt-cache D1/D3).
        let active_runs_context = self.build_active_runs_context(&session_id_str);

        let injector = {
            let outer_tx = tx.clone();
            let sid = session_id_str.clone();
            tokio::spawn(async move {
                while let Some(mut event) = inner_rx.recv().await {
                    match &mut event {
                        AgentEvent::ApprovalRequired { session_id, .. }
                        | AgentEvent::AskQuestion { session_id, .. } => {
                            if session_id.is_none() {
                                *session_id = Some(sid.clone());
                            }
                        }
                        _ => {}
                    }
                    if outer_tx.send(event).await.is_err() {
                        break;
                    }
                }
            })
        };

        let steer_inbox: crate::builtin_tools::SteerInbox =
            std::sync::Arc::new(tokio::sync::Mutex::new(params.steer_rx));

        let result = {
            let runtime = self.runtime.clone();
            let tool_registry = self.tool_registry.clone();
            let llm = per_request_llm
                .clone()
                .or_else(|| self.llm_override.clone());
            let session_store = self.session_store.clone();
            let todo_store = self.todo_store.clone();
            let goal_store = self.goal_store.clone();
            let stream_ctx_key_inner = stream_context_key.clone();
            let ih_for_tools = interaction.clone();

            // Clone for the reactive loop (the first closure moves these).
            let mode_state_for_loop = mode_state.clone();
            let plan_ctx_for_loop = plan_ctx.clone();

            let cost_store_for_runtime = self.cost_store.clone();
            let runtime_quality_store_for_runtime = self.runtime_quality_store.clone();
            let artifact_store_for_runtime = self.artifact_store.clone();
            let behavior_overrides_for_runtime = self.behavior_overrides.clone();
            let runtime_fut: std::pin::Pin<
                Box<
                    dyn std::future::Future<Output = anyhow::Result<xiaolin_protocol::TurnSummary>>
                        + Send,
                >,
            > = Box::pin(runtime.execute_unified_with_cost_store(
                &config,
                &request,
                &tool_registry,
                inner_tx.clone(),
                approval_strategy.clone(),
                llm.clone(),
                orchestrator.clone(),
                Some(interaction.clone()),
                subagent_prompt.clone(),
                mode_state.clone(),
                session_store.clone(),
                todo_store.clone(),
                goal_store,
                cost_store_for_runtime,
                runtime_quality_store_for_runtime,
                artifact_store_for_runtime,
                behavior_overrides_for_runtime,
                None,
                active_runs_context.clone(),
            ));

            let steer_inbox_inner = steer_inbox.clone();
            let session_id_for_scope = session_id_str.clone();
            let llm_for_subagents = per_request_llm.clone(); // captured before .or_else() consumes it
            let wrapped_fut = async move {
                let runtime_with_ih =
                    crate::builtin_tools::with_interaction_handle(ih_for_tools, runtime_fut);
                let runtime_with_steer =
                    crate::builtin_tools::with_steer_inbox(steer_inbox_inner, runtime_with_ih);
                let runtime_with_session =
                    crate::with_subagent_session_id(session_id_for_scope, runtime_with_steer);
                let runtime_with_llm = crate::subagent::CURRENT_LLM_OVERRIDE
                    .scope(llm_for_subagents, runtime_with_session);
                if let Some(ms) = mode_state {
                    crate::builtin_tools::with_stream_context(
                        stream_ctx_key_inner,
                        crate::builtin_tools::with_session_mode(ms, plan_ctx, runtime_with_llm),
                    )
                    .await
                } else {
                    crate::builtin_tools::with_stream_context(
                        stream_ctx_key_inner,
                        runtime_with_llm,
                    )
                    .await
                }
            };

            let first_result = tokio::select! {
                r = wrapped_fut => r,
                () = cancel.cancelled() => Err(anyhow::anyhow!("cancelled by session actor")),
            };

            // ── Reactive Loop: check for active sub-agents and re-prompt ──
            let policy = &config.behavior.subagent;
            let reactive_enabled = policy.reactive_loop_enabled && self.subagent_manager.is_some();

            if reactive_enabled {
                self.run_reactive_loop(
                    first_result,
                    &config,
                    &request,
                    &session_id_str,
                    &inner_tx,
                    &orchestrator,
                    &llm,
                    &interaction,
                    &subagent_prompt,
                    &mode_state_for_loop,
                    &plan_ctx_for_loop,
                    &session_store,
                    &todo_store,
                    &stream_context_key,
                    &cancel,
                    approval_strategy,
                )
                .await
            } else {
                first_result
            }
        };

        // Cancel active sub-agents BEFORE draining the injector, so events can still flow.
        if let Some(ref mgr) = self.subagent_manager {
            let active = mgr.active_runs(&session_id_str);
            if !active.is_empty() {
                tracing::info!(
                    session_id = %session_id_str,
                    count = active.len(),
                    "cancelling active sub-agents on session end"
                );
                for run in &active {
                    mgr.cancel(&run.run_id);
                    let _ = tx.try_send(AgentEvent::SubAgentComplete {
                        turn_id: Default::default(),
                        run_id: run.run_id.clone(),
                        status: "cancelled".into(),
                        result: None,
                        tool_calls_made: 0,
                        iterations: 0,
                        usage: None,
                        elapsed_ms: run.elapsed_ms.unwrap_or(0),
                    });
                }
            }
        }

        // Remove inner_tx clones from shared maps so the channel can close.
        if let Some(ref map) = self.stream_event_tx {
            map.remove(&stream_context_key);
        }
        if let Some(ref mgr) = self.subagent_manager {
            mgr.unregister_session_tx(&session_id_str);
        }

        // Drop the last inner_tx so injector's recv() returns None.
        drop(inner_tx);
        // Wait for the injector to drain all buffered events before proceeding.
        // Without this, events (e.g. ContentDelta) buffered in the channel can be
        // lost if the injector task hasn't been scheduled yet (single-threaded runtime).
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), injector).await;

        drop(steer_inbox);

        tracing::info!(
            elapsed_ms = execute_t0.elapsed().as_millis() as u64,
            "perf: bridge_execute_total"
        );
        match result {
            Ok(summary) => Ok(TurnResult {
                tool_calls_made: summary.tool_calls_made,
                iterations: summary.iterations,
                usage: summary.usage,
            }),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("cancelled by session actor") {
                    if let Some(ref gs) = self.goal_store {
                        if let Some(goal) = gs.get_active().await {
                            tracing::info!(goal_id = %goal.id, "pausing active goal due to user interrupt");
                            if let Some(updated) = gs
                                .update_status(
                                    &goal.id,
                                    crate::builtin_tools::GoalStatus::Paused,
                                    Some("user_interrupt"),
                                )
                                .await
                            {
                                let _ = tx
                                    .send(AgentEvent::GoalUpdated {
                                        turn_id: Default::default(),
                                        goal: updated.to_goal_data(),
                                    })
                                    .await;
                            }
                        }
                    }
                    Err(TurnError::Cancelled)
                } else {
                    let code = xiaolin_protocol::event::ErrorCode::classify(&msg);
                    tracing::error!(error = %msg, code = ?code, "turn execution failed");
                    Err(TurnError::Runtime { message: msg, code })
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::derive_approval_strategy;
    use crate::ToolOrchestrator;
    use xiaolin_core::agent_config::{BehaviorConfig, FileAccessMode};
    use xiaolin_core::tool_runtime::ApprovalStrategy;

    #[test]
    fn default_orchestrator_construction() {
        let orch = ToolOrchestrator::new();
        let _default: ToolOrchestrator = Default::default();
        drop(orch);
    }

    #[test]
    fn explicit_auto_approve_returns_auto_approve() {
        let behavior = BehaviorConfig {
            approval_strategy: Some("auto_approve".to_string()),
            ..Default::default()
        };
        assert!(matches!(
            derive_approval_strategy(&behavior, false),
            ApprovalStrategy::AutoApprove
        ));
    }

    #[test]
    fn no_explicit_strategy_defaults_to_interactive() {
        let behavior = BehaviorConfig {
            tools_ask: vec![],
            require_confirmation_for: vec![],
            file_access: FileAccessMode::Full,
            ..Default::default()
        };
        assert!(matches!(
            derive_approval_strategy(&behavior, false),
            ApprovalStrategy::Interactive
        ));
    }

    #[test]
    fn goal_mode_forces_auto_approve() {
        let behavior = BehaviorConfig::default();
        assert!(matches!(
            derive_approval_strategy(&behavior, true),
            ApprovalStrategy::AutoApprove
        ));
    }

    #[test]
    fn default_mode_returns_interactive() {
        let behavior = BehaviorConfig {
            tools_ask: vec!["write_file".into(), "edit_file".into(), "shell_exec".into()],
            require_confirmation_for: vec![
                "write_file".into(),
                "edit_file".into(),
                "shell_exec".into(),
            ],
            file_access: FileAccessMode::Workspace,
            ..Default::default()
        };
        assert!(matches!(
            derive_approval_strategy(&behavior, false),
            ApprovalStrategy::Interactive
        ));
    }

    #[test]
    fn workspace_file_access_alone_returns_interactive() {
        let behavior = BehaviorConfig {
            tools_ask: vec![],
            require_confirmation_for: vec![],
            file_access: FileAccessMode::Workspace,
            ..Default::default()
        };
        assert!(matches!(
            derive_approval_strategy(&behavior, false),
            ApprovalStrategy::Interactive
        ));
    }

    #[test]
    fn tools_ask_with_full_access_returns_interactive() {
        let behavior = BehaviorConfig {
            tools_ask: vec!["shell_exec".into()],
            require_confirmation_for: vec![],
            file_access: FileAccessMode::Full,
            ..Default::default()
        };
        assert!(matches!(
            derive_approval_strategy(&behavior, false),
            ApprovalStrategy::Interactive
        ));
    }
}
