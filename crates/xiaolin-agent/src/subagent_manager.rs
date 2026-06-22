use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use xiaolin_core::agent_config::{AgentConfig, FileAccessMode, SubAgentDef, SubAgentPolicy};
use xiaolin_core::tool::ToolRegistry;
use xiaolin_core::types::{ChatMessage, SubAgentRun, SubAgentStatus, SubAgentType, Usage};
use xiaolin_protocol::{AgentEvent, CompletionSummary, TokenUsage, TurnId};

use crate::message_queue::MessageQueue;

/// Context inherited from the parent session to ensure subagents run
/// with the same file access permissions and work directory.
#[derive(Clone, Debug)]
pub struct SubAgentInheritedContext {
    pub work_dir: Option<String>,
    pub file_access: FileAccessMode,
    pub additional_allowed_paths: Vec<String>,
}

use crate::llm::LlmProvider;
use crate::runtime::AgentRuntime;
use crate::runtime::orchestrator::ToolOrchestrator;
use crate::spawn_controller::{SlotEvent, SpawnController};

/// Manages the lifecycle of all sub-agent runs: spawn, cancel, track, query.
pub struct SubAgentManager {
    runtime: Arc<AgentRuntime>,
    available_agents: Vec<AgentConfig>,
    /// Registry of sub-agent type definitions (builtin + custom).
    subagent_defs: Arc<std::sync::RwLock<Vec<SubAgentDef>>>,
    runs: Arc<DashMap<String, SubAgentRun>>,
    cancel_tokens: Arc<DashMap<String, CancellationToken>>,
    controller: Arc<SpawnController>,
    orchestrator: Arc<ToolOrchestrator>,
    /// Per-session completion broadcast channels for the reactive loop.
    completion_channels: Arc<DashMap<String, broadcast::Sender<CompletionSummary>>>,
    /// Per-session event senders for streaming SubAgent events to the frontend.
    /// Registered by session_bridge at turn start, unregistered at turn end.
    session_event_senders: Arc<DashMap<String, mpsc::Sender<AgentEvent>>>,
    /// Per-run message queues for steering running sub-agents.
    run_queues: Arc<DashMap<String, Arc<MessageQueue>>>,
    /// Shared port for bubble approval requests from sub-agents.
    bubble_port: Arc<xiaolin_core::tool_runtime::BubbleApprovalPort>,
}

impl SubAgentManager {
    pub fn new(
        runtime: Arc<AgentRuntime>,
        available_agents: Vec<AgentConfig>,
        _default_policy: SubAgentPolicy,
        controller: Arc<SpawnController>,
    ) -> Self {
        Self {
            runtime,
            available_agents,
            subagent_defs: Arc::new(std::sync::RwLock::new(Vec::new())),
            runs: Arc::new(DashMap::new()),
            cancel_tokens: Arc::new(DashMap::new()),
            controller,
            orchestrator: Arc::new(ToolOrchestrator::new()),
            completion_channels: Arc::new(DashMap::new()),
            session_event_senders: Arc::new(DashMap::new()),
            run_queues: Arc::new(DashMap::new()),
            bubble_port: Arc::new(xiaolin_core::tool_runtime::BubbleApprovalPort::new()),
        }
    }

    pub fn controller(&self) -> &Arc<SpawnController> {
        &self.controller
    }

    /// Register a session's event sender so SubAgentTool can forward streaming events.
    pub fn register_session_tx(&self, session_id: &str, tx: mpsc::Sender<AgentEvent>) {
        self.session_event_senders.insert(session_id.to_string(), tx);
    }

    /// Unregister a session's event sender (called when a turn ends).
    pub fn unregister_session_tx(&self, session_id: &str) {
        self.session_event_senders.remove(session_id);
    }

    /// Get the event sender for a session (used by SubAgentTool).
    pub fn get_session_tx(&self, session_id: &str) -> Option<mpsc::Sender<AgentEvent>> {
        self.session_event_senders.get(session_id).map(|r| r.value().clone())
    }

    /// Create a message queue for a run and return it.
    pub fn create_run_queue(&self, run_id: &str) -> Arc<MessageQueue> {
        let q = Arc::new(MessageQueue::new());
        self.run_queues.insert(run_id.to_string(), Arc::clone(&q));
        q
    }

    /// Get the message queue for a running sub-agent.
    pub fn get_run_queue(&self, run_id: &str) -> Option<Arc<MessageQueue>> {
        self.run_queues.get(run_id).map(|r| Arc::clone(r.value()))
    }

    /// Remove the message queue when a run completes.
    pub fn remove_run_queue(&self, run_id: &str) {
        self.run_queues.remove(run_id);
    }

    /// Get the shared bubble approval port for resolving sub-agent approval requests.
    pub fn bubble_port(&self) -> &Arc<xiaolin_core::tool_runtime::BubbleApprovalPort> {
        &self.bubble_port
    }

    /// Update the available agents list (e.g. after config reload).
    pub fn set_available_agents(&mut self, agents: Vec<AgentConfig>) {
        self.available_agents = agents;
    }

    pub fn available_agents(&self) -> &[AgentConfig] {
        &self.available_agents
    }

    /// Replace the sub-agent definition registry.
    pub fn set_subagent_defs(&self, defs: Vec<SubAgentDef>) {
        let mut lock = self.subagent_defs.write().expect("subagent_defs poisoned");
        *lock = defs;
    }

    /// Get a snapshot of all sub-agent definitions.
    pub fn subagent_defs(&self) -> Vec<SubAgentDef> {
        self.subagent_defs
            .read()
            .expect("subagent_defs poisoned")
            .clone()
    }

    /// Look up a sub-agent definition by ID.
    pub fn resolve_subagent_def(&self, def_id: &str) -> Option<SubAgentDef> {
        self.subagent_defs
            .read()
            .expect("subagent_defs poisoned")
            .iter()
            .find(|d| d.id == def_id)
            .cloned()
    }

    /// Build a child tool registry filtered according to a `SubAgentDef`.
    pub fn build_child_registry_from_def(
        parent_registry: &ToolRegistry,
        def: &SubAgentDef,
    ) -> ToolRegistry {
        let child = ToolRegistry::new();
        for name in parent_registry.tool_names() {
            if def.tools.is_tool_allowed(&name) {
                if let Some(tool) = parent_registry.get(&name) {
                    child.register(tool);
                }
            }
        }
        child
    }

    /// Build descriptions for sub-agent defs (for prompt injection / tool schemas).
    pub fn subagent_def_descriptions(&self) -> Vec<(String, Option<String>)> {
        self.subagent_defs
            .read()
            .expect("subagent_defs poisoned")
            .iter()
            .map(|d| (d.id.clone(), d.description.clone()))
            .collect()
    }

    /// Subscribe to completion notifications for a given session.
    /// The returned receiver will get a `CompletionSummary` each time a sub-agent
    /// in that session finishes (success, failure, or cancel).
    pub fn subscribe_completions(&self, session_id: &str) -> broadcast::Receiver<CompletionSummary> {
        let entry = self
            .completion_channels
            .entry(session_id.to_string())
            .or_insert_with(|| broadcast::channel(64).0);
        entry.subscribe()
    }

    /// List all currently active (Pending or Running) runs for a session.
    pub fn active_runs(&self, session_id: &str) -> Vec<SubAgentRun> {
        self.runs
            .iter()
            .filter(|r| r.parent_session_id == session_id && !r.status.is_terminal())
            .map(|r| r.value().clone())
            .collect()
    }

    /// Build a `CompletionSummary` from a finished run.
    pub fn get_completion_summary(&self, run_id: &str) -> Option<CompletionSummary> {
        self.runs.get(run_id).and_then(|r| {
            if !r.status.is_terminal() {
                return None;
            }
            let (status_str, error) = match &r.status {
                SubAgentStatus::Completed => ("completed".to_string(), None),
                SubAgentStatus::Failed(msg) => ("failed".to_string(), Some(msg.clone())),
                SubAgentStatus::Cancelled => ("cancelled".to_string(), None),
                _ => return None,
            };
            let result_preview = r.result.as_ref().map(|text| {
                if text.len() > 2000 {
                    let end = text.floor_char_boundary(2000);
                    format!("{}…", &text[..end])
                } else {
                    text.clone()
                }
            });
            Some(CompletionSummary {
                run_id: r.run_id.clone(),
                agent_id: r.agent_id.to_string(),
                subagent_type: r.subagent_type.to_string(),
                task: r.task.clone(),
                status: status_str,
                elapsed_ms: r.elapsed_ms.unwrap_or(0),
                tool_call_count: r.tool_calls_made,
                result_preview,
                error,
            })
        })
    }

    /// Broadcast a completion event for a run to the session's reactive loop subscriber.
    #[allow(dead_code)]
    fn broadcast_completion(&self, session_id: &str, summary: CompletionSummary) {
        if let Some(tx) = self.completion_channels.get(session_id) {
            let _ = tx.send(summary);
        }
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    fn generate_run_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    /// Resolve an agent config by ID.
    pub fn resolve_agent(&self, agent_id: &str) -> Option<AgentConfig> {
        self.available_agents
            .iter()
            .find(|a| *a.agent_id == *agent_id)
            .cloned()
    }

    /// Build the list of (agent_id, description) pairs for prompt injection.
    pub fn agent_descriptions(&self) -> Vec<(String, Option<String>)> {
        self.available_agents
            .iter()
            .map(|a| (a.agent_id.to_string(), a.description.clone()))
            .collect()
    }

    /// Spawn a sub-agent run. Returns the run_id immediately.
    ///
    /// The sub-agent executes asynchronously; progress streams to `parent_tx`
    /// as `AgentEvent::SubAgent*` variants. The caller (usually `SubAgentTool`)
    /// collects the final result from the `SubAgentComplete` event or queries
    /// `get_run()`.
    #[allow(clippy::too_many_arguments)]
    pub async fn spawn(
        &self,
        agent_config: AgentConfig,
        subagent_type: SubAgentType,
        task: String,
        context: Option<String>,
        parent_session_id: String,
        parent_message_id: String,
        current_depth: u32,
        policy: &SubAgentPolicy,
        tool_registry: Arc<ToolRegistry>,
        parent_tx: mpsc::Sender<AgentEvent>,
        llm_override: Option<Arc<dyn LlmProvider>>,
        concurrency_safe: bool,
        inherited_context: Option<SubAgentInheritedContext>,
        initial_messages: Option<Vec<ChatMessage>>,
        permission_mode: xiaolin_core::agent_config::PermissionMode,
        parent_queue: Option<Arc<MessageQueue>>,
        max_result_chars: Option<usize>,
    ) -> anyhow::Result<String> {
        if !policy.enabled {
            anyhow::bail!("sub-agent delegation is disabled for this agent");
        }
        if current_depth >= policy.max_depth {
            anyhow::bail!(
                "sub-agent depth limit reached ({}/{})",
                current_depth,
                policy.max_depth
            );
        }

        let run_id = Self::generate_run_id();
        let run = SubAgentRun {
            run_id: run_id.clone(),
            parent_session_id: parent_session_id.clone(),
            parent_message_id: parent_message_id.clone(),
            agent_id: agent_config.agent_id.clone(),
            subagent_type: subagent_type.clone(),
            task: task.clone(),
            status: SubAgentStatus::Pending,
            created_at: Self::now_ms(),
            completed_at: None,
            result: None,
            tool_calls_made: 0,
            iterations: 0,
            token_usage: None,
            depth: current_depth + 1,
            elapsed_ms: None,
        };
        self.runs.insert(run_id.clone(), run);

        let cancel_token = CancellationToken::new();
        self.cancel_tokens
            .insert(run_id.clone(), cancel_token.clone());

        let turn_id = TurnId::generate();

        let _ = parent_tx
            .send(AgentEvent::SubAgentStart {
                turn_id: turn_id.clone(),
                run_id: run_id.clone(),
                agent_id: agent_config.agent_id.to_string(),
                subagent_type: subagent_type.to_string(),
                task: task.clone(),
                depth: current_depth + 1,
            })
            .await;

        let mq = self.create_run_queue(&run_id);
        let run_queues_ref = self.run_queues.clone();

        let approval_strat = match permission_mode {
            xiaolin_core::agent_config::PermissionMode::AutoApprove => {
                xiaolin_core::tool_runtime::ApprovalStrategy::AutoApprove
            }
            xiaolin_core::agent_config::PermissionMode::Bubble => {
                xiaolin_core::tool_runtime::ApprovalStrategy::Bubble(self.bubble_port.clone())
            }
            xiaolin_core::agent_config::PermissionMode::Deny => {
                xiaolin_core::tool_runtime::ApprovalStrategy::DenyAll
            }
        };

        let runs = self.runs.clone();
        let cancel_tokens = self.cancel_tokens.clone();
        let runtime = self.runtime.clone();
        let controller = self.controller.clone();
        let orchestrator = self.orchestrator.clone();
        let completion_channels = self.completion_channels.clone();
        let result_char_limit = max_result_chars
            .unwrap_or(crate::sidechain::MAX_RESULT_CHARS);

        let timeout = Duration::from_secs(policy.timeout_seconds);
        let slot_timeout = controller.config().slot_acquire_timeout;
        // Hard wall-clock limit: covers both slot reservation + execution + buffer
        let wall_clock_limit = timeout + slot_timeout + Duration::from_secs(30);
        let max_depth = policy.max_depth;
        let rid = run_id.clone();
        let forward_turn_id = turn_id.clone();
        let session_id_owned = parent_session_id.clone();
        let initial_msgs = initial_messages;
        let coordinator_queue = parent_queue;

        tokio::spawn(async move {
            // Clone variables needed in both inner async block and outer timeout handler
            let rid_outer = rid.clone();
            let runs_outer = runs.clone();
            let forward_turn_id_outer = forward_turn_id.clone();
            let parent_tx_outer = parent_tx.clone();
            let completion_channels_outer = completion_channels.clone();
            let session_id_outer = session_id_owned.clone();
            let agent_id_outer = agent_config.agent_id.to_string();
            let subagent_type_outer = subagent_type.to_string();
            let task_outer = task.clone();
            let cancel_tokens_outer = cancel_tokens.clone();
            let run_queues_outer = run_queues_ref.clone();

            // Hard wall-clock timeout: ensures the entire spawned task (slot wait + execution)
            // cannot exceed wall_clock_limit, preventing indefinite hangs.
            let wall_clock_result = tokio::time::timeout(wall_clock_limit, async move {
            let reservation = match controller
                .reserve(&session_id_owned, &rid, concurrency_safe, slot_timeout)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    let error_msg = format!("slot acquisition failed: {e}");
                    Self::fail_run(
                        &runs,
                        &cancel_tokens,
                        &rid,
                        &error_msg,
                    );
                    // Notify parent and completion channels so reactive loop doesn't hang
                    let _ = parent_tx
                        .send(AgentEvent::SubAgentComplete {
                            turn_id: forward_turn_id.clone(),
                            run_id: rid.clone(),
                            status: "failed".into(),
                            result: None,
                            tool_calls_made: 0,
                            iterations: 0,
                            usage: None,
                            elapsed_ms: 0,
                        })
                        .await;
                    if let Some(tx) = completion_channels.get(&session_id_owned) {
                        let _ = tx.send(CompletionSummary {
                            run_id: rid.clone(),
                            agent_id: agent_config.agent_id.to_string(),
                            subagent_type: subagent_type.to_string(),
                            task: task.clone(),
                            status: "failed".into(),
                            elapsed_ms: 0,
                            tool_call_count: 0,
                            result_preview: None,
                            error: Some(error_msg),
                        });
                    }
                    return;
                }
            };

            reservation.session_pool().broadcast(SlotEvent::Acquired {
                run_id: rid.clone(),
                concurrency_safe,
                def_id: String::new(),
            });

            if let Some(mut r) = runs.get_mut(&rid) {
                r.status = SubAgentStatus::Running;
            }

            let t0 = std::time::Instant::now();
            let complete_turn_id = forward_turn_id.clone();

            let result: anyhow::Result<(String, u32, u32, Option<Usage>)> = tokio::select! {
                _ = cancel_token.cancelled() => {
                    Err(anyhow::anyhow!("cancelled"))
                }
                _ = tokio::time::sleep(timeout) => {
                    Err(anyhow::anyhow!("timeout after {}s", timeout.as_secs()))
                }
                res = Self::run_subagent(
                    &runtime,
                    &agent_config,
                    &task,
                    context.as_deref(),
                    &subagent_type,
                    current_depth + 1,
                    max_depth,
                    &tool_registry,
                    parent_tx.clone(),
                    &rid,
                    forward_turn_id,
                    llm_override,
                    orchestrator.clone(),
                    inherited_context.as_ref(),
                    &session_id_owned,
                    initial_msgs.as_deref(),
                    Some(mq.clone()),
                    approval_strat,
                ) => {
                    res
                }
            };

            let elapsed_ms = t0.elapsed().as_millis() as u64;

            match result {
                Ok((response_text, tool_calls_made, iterations, usage)) => {
                    let truncated_result =
                        crate::sidechain::truncate_result(&response_text, result_char_limit);
                    let _ = parent_tx
                        .send(AgentEvent::SubAgentComplete {
                            turn_id: complete_turn_id.clone(),
                            run_id: rid.clone(),
                            status: "completed".into(),
                            result: Some(truncated_result),
                            tool_calls_made,
                            iterations,
                            usage: usage.clone().map(|u| TokenUsage {
                                prompt_tokens: u.prompt_tokens,
                                completion_tokens: u.completion_tokens,
                                total_tokens: u.total_tokens,
                                cached_input_tokens: u.effective_cache_read_tokens(),
                            }),
                            elapsed_ms,
                        })
                        .await;

                    reservation.session_pool().broadcast(SlotEvent::Completed {
                        run_id: rid.clone(),
                        result: Some(response_text.clone()),
                    });

                    if let Some(mut r) = runs.get_mut(&rid) {
                        r.status = SubAgentStatus::Completed;
                        r.completed_at = Some(Self::now_ms());
                        r.result = Some(response_text.clone());
                        r.tool_calls_made = tool_calls_made;
                        r.iterations = iterations;
                        r.token_usage = usage;
                        r.elapsed_ms = Some(elapsed_ms);
                    }

                    let result_preview = if response_text.len() > 2000 {
                        let end = response_text.floor_char_boundary(2000);
                        Some(format!("{}…", &response_text[..end]))
                    } else {
                        Some(response_text)
                    };
                    if let Some(tx) = completion_channels.get(&session_id_owned) {
                        let _ = tx.send(CompletionSummary {
                            run_id: rid.clone(),
                            agent_id: agent_config.agent_id.to_string(),
                            subagent_type: subagent_type.to_string(),
                            task: task.clone(),
                            status: "completed".into(),
                            elapsed_ms,
                            tool_call_count: tool_calls_made,
                            result_preview: result_preview.clone(),
                            error: None,
                        });
                    }

                    // Push completion notification to coordinator's queue if present
                    if let Some(ref cq) = coordinator_queue {
                        let notification = format!(
                            "[Worker Completed] run_id={}, type={}, task=\"{}\", \
                             elapsed={}ms, tools_used={}\nResult: {}",
                            rid,
                            subagent_type,
                            if task.len() > 100 {
                                let end = task.floor_char_boundary(100);
                                format!("{}…", &task[..end])
                            } else {
                                task.clone()
                            },
                            elapsed_ms,
                            tool_calls_made,
                            result_preview.as_deref().unwrap_or("(no result)"),
                        );
                        cq.push(
                            crate::message_queue::Priority::High,
                            "system",
                            &notification,
                        );
                    }
                }
                Err(e) => {
                    let msg = e.to_string();
                    let status_str = if msg.contains("cancelled") {
                        "cancelled"
                    } else {
                        "failed"
                    };

                    let _ = parent_tx
                        .send(AgentEvent::SubAgentComplete {
                            turn_id: complete_turn_id.clone(),
                            run_id: rid.clone(),
                            status: status_str.into(),
                            result: None,
                            tool_calls_made: 0,
                            iterations: 0,
                            usage: None,
                            elapsed_ms,
                        })
                        .await;

                    reservation.session_pool().broadcast(SlotEvent::Failed {
                        run_id: rid.clone(),
                        error: msg.clone(),
                    });

                    if let Some(mut r) = runs.get_mut(&rid) {
                        if msg.contains("cancelled") {
                            r.status = SubAgentStatus::Cancelled;
                        } else {
                            r.status = SubAgentStatus::Failed(msg.clone());
                        }
                        r.completed_at = Some(Self::now_ms());
                        r.elapsed_ms = Some(elapsed_ms);
                    }

                    if let Some(tx) = completion_channels.get(&session_id_owned) {
                        let _ = tx.send(CompletionSummary {
                            run_id: rid.clone(),
                            agent_id: agent_config.agent_id.to_string(),
                            subagent_type: subagent_type.to_string(),
                            task: task.clone(),
                            status: status_str.into(),
                            elapsed_ms,
                            tool_call_count: 0,
                            result_preview: None,
                            error: Some(msg.clone()),
                        });
                    }

                    // Push failure notification to coordinator's queue if present
                    if let Some(ref cq) = coordinator_queue {
                        let notification = format!(
                            "[Worker {}] run_id={}, type={}, task=\"{}\", \
                             elapsed={}ms\nError: {}",
                            status_str,
                            rid,
                            subagent_type,
                            if task.len() > 100 {
                                let end = task.floor_char_boundary(100);
                                format!("{}…", &task[..end])
                            } else {
                                task.clone()
                            },
                            elapsed_ms,
                            msg,
                        );
                        cq.push(
                            crate::message_queue::Priority::High,
                            "system",
                            &notification,
                        );
                    }
                }
            }

            drop(reservation);
            }).await; // end of wall_clock_result async block

            // Handle wall-clock timeout (the entire task exceeded the limit)
            if wall_clock_result.is_err() {
                tracing::error!(
                    run_id = %rid_outer,
                    wall_clock_limit_secs = wall_clock_limit.as_secs(),
                    "subagent exceeded wall-clock limit, force-terminating"
                );
                if let Some(mut r) = runs_outer.get_mut(&rid_outer) {
                    if !r.status.is_terminal() {
                        r.status = SubAgentStatus::Failed(format!(
                            "wall-clock timeout after {}s",
                            wall_clock_limit.as_secs()
                        ));
                        r.completed_at = Some(Self::now_ms());
                    }
                }
                let _ = parent_tx_outer
                    .send(AgentEvent::SubAgentComplete {
                        turn_id: forward_turn_id_outer.clone(),
                        run_id: rid_outer.clone(),
                        status: "failed".into(),
                        result: None,
                        tool_calls_made: 0,
                        iterations: 0,
                        usage: None,
                        elapsed_ms: wall_clock_limit.as_millis() as u64,
                    })
                    .await;
                if let Some(tx) = completion_channels_outer.get(&session_id_outer) {
                    let _ = tx.send(CompletionSummary {
                        run_id: rid_outer.clone(),
                        agent_id: agent_id_outer.clone(),
                        subagent_type: subagent_type_outer.clone(),
                        task: task_outer.clone(),
                        status: "failed".into(),
                        elapsed_ms: wall_clock_limit.as_millis() as u64,
                        tool_call_count: 0,
                        result_preview: None,
                        error: Some(format!(
                            "wall-clock timeout after {}s",
                            wall_clock_limit.as_secs()
                        )),
                    });
                }
            }

            cancel_tokens_outer.remove(&rid_outer);
            run_queues_outer.remove(&rid_outer);

            // Auto-GC: remove the run from the map after a short retention period.
            // This prevents unbounded growth of the `runs` DashMap.
            let runs_for_gc = runs_outer;
            let rid_for_gc = rid_outer;
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(300)).await;
                runs_for_gc.remove(&rid_for_gc);
            });
        });

        Ok(run_id)
    }

    /// Spawn a sub-agent and wait for completion using broadcast events.
    ///
    /// Returns `(result_text, run_id)` on success. Uses event-driven notification
    /// instead of polling for minimal latency.
    #[allow(clippy::too_many_arguments)]
    pub async fn spawn_and_wait(
        &self,
        agent_config: AgentConfig,
        subagent_type: SubAgentType,
        task: String,
        context: Option<String>,
        parent_session_id: String,
        parent_message_id: String,
        current_depth: u32,
        policy: &SubAgentPolicy,
        tool_registry: Arc<ToolRegistry>,
        parent_tx: mpsc::Sender<AgentEvent>,
        llm_override: Option<Arc<dyn LlmProvider>>,
        concurrency_safe: bool,
        inherited_context: Option<SubAgentInheritedContext>,
        initial_messages: Option<Vec<ChatMessage>>,
        permission_mode: xiaolin_core::agent_config::PermissionMode,
        parent_queue: Option<Arc<MessageQueue>>,
        max_result_chars: Option<usize>,
    ) -> anyhow::Result<(String, String)> {
        let session_pool = self
            .controller
            .get_or_create_session_pool(&parent_session_id);
        let mut events_rx = session_pool.subscribe_events();

        let run_id = self
            .spawn(
                agent_config,
                subagent_type,
                task,
                context,
                parent_session_id,
                parent_message_id,
                current_depth,
                policy,
                tool_registry,
                parent_tx,
                llm_override,
                concurrency_safe,
                inherited_context,
                initial_messages,
                permission_mode,
                parent_queue,
                max_result_chars,
            )
            .await?;

        let timeout = Duration::from_secs(policy.timeout_seconds);

        tokio::select! {
            _ = tokio::time::sleep(timeout) => {
                self.cancel(&run_id);
                anyhow::bail!("sync sub-agent timed out after {}s", policy.timeout_seconds);
            }
            result = async {
                loop {
                    if let Some(run) = self.get_run(&run_id) {
                        match &run.status {
                            SubAgentStatus::Completed => {
                                return Ok((run.result.unwrap_or_default(), run_id.clone()));
                            }
                            SubAgentStatus::Failed(msg) => {
                                return Err(anyhow::anyhow!("sub-agent failed: {msg}"));
                            }
                            SubAgentStatus::Cancelled => {
                                return Err(anyhow::anyhow!("sub-agent was cancelled"));
                            }
                            SubAgentStatus::Pending | SubAgentStatus::Running => {}
                        }
                    } else {
                        return Err(anyhow::anyhow!("sub-agent run {run_id} disappeared"));
                    }

                    match events_rx.recv().await {
                        Ok(SlotEvent::Completed { run_id: rid, .. })
                        | Ok(SlotEvent::Failed { run_id: rid, .. })
                        | Ok(SlotEvent::Released { run_id: rid })
                            if rid == run_id => {}
                        Ok(_) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            return Err(anyhow::anyhow!("event channel closed"));
                        }
                    }
                }
            } => {
                result
            }
        }
    }

    /// Spawn a sub-agent and block until it completes (backward-compatible alias).
    #[deprecated(note = "use spawn_and_wait() instead")]
    #[allow(clippy::too_many_arguments)]
    pub async fn spawn_sync(
        &self,
        agent_config: AgentConfig,
        subagent_type: SubAgentType,
        task: String,
        context: Option<String>,
        parent_session_id: String,
        parent_message_id: String,
        current_depth: u32,
        policy: &SubAgentPolicy,
        tool_registry: Arc<ToolRegistry>,
        parent_tx: mpsc::Sender<AgentEvent>,
        llm_override: Option<Arc<dyn LlmProvider>>,
        concurrency_safe: bool,
        inherited_context: Option<SubAgentInheritedContext>,
        initial_messages: Option<Vec<ChatMessage>>,
        permission_mode: xiaolin_core::agent_config::PermissionMode,
        parent_queue: Option<Arc<MessageQueue>>,
        max_result_chars: Option<usize>,
    ) -> anyhow::Result<(String, String)> {
        self.spawn_and_wait(
            agent_config,
            subagent_type,
            task,
            context,
            parent_session_id,
            parent_message_id,
            current_depth,
            policy,
            tool_registry,
            parent_tx,
            llm_override,
            concurrency_safe,
            inherited_context,
            initial_messages,
            permission_mode,
            parent_queue,
            max_result_chars,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_subagent(
        runtime: &AgentRuntime,
        config: &AgentConfig,
        task: &str,
        context: Option<&str>,
        _subagent_type: &SubAgentType,
        _depth: u32,
        _max_depth: u32,
        tool_registry: &Arc<ToolRegistry>,
        parent_tx: mpsc::Sender<AgentEvent>,
        run_id: &str,
        turn_id: TurnId,
        llm_override: Option<Arc<dyn LlmProvider>>,
        orchestrator: Arc<ToolOrchestrator>,
        inherited_context: Option<&SubAgentInheritedContext>,
        parent_session_id: &str,
        initial_messages: Option<&[ChatMessage]>,
        message_queue: Option<Arc<MessageQueue>>,
        approval_strategy: xiaolin_core::tool_runtime::ApprovalStrategy,
    ) -> anyhow::Result<(String, u32, u32, Option<Usage>)> {
        use crate::sidechain::{SidechainMessage, SidechainMeta, SidechainWriter};
        use xiaolin_core::types::{ChatMessage, ChatRequest, Role};

        let mut messages = Vec::new();

        // Prepend inherited parent context messages (Fork Agent)
        if let Some(inherited) = initial_messages {
            messages.extend_from_slice(inherited);
        }

        if let Some(ctx) = context {
            messages.push(ChatMessage {
                role: Role::System,
                content: Some(serde_json::Value::String(format!(
                    "Context from parent agent:\n{ctx}"
                ))),
            ..Default::default()
            });
        }
        messages.push(ChatMessage {
            role: Role::User,
            content: Some(serde_json::Value::String(task.to_string())),
        ..Default::default()
        });

        let work_dir = inherited_context.and_then(|ic| ic.work_dir.clone());

        let request = ChatRequest {
            messages,
            stream: true,
            model: None,
            temperature: None,
            max_tokens: None,
            agent_id: Some(config.agent_id.clone()),
            session_id: None,
            tools: None,
            slash_intent: None,
            work_dir,
            response_language: None,
            goal_mode: None,
        };

        // Initialize sidechain writer for transcript persistence
        let sidechain_writer = match SidechainWriter::new(
            parent_session_id,
            run_id,
            SidechainMeta {
                _meta: true,
                run_id: run_id.to_string(),
                agent_id: config.agent_id.to_string(),
                parent_session_id: parent_session_id.to_string(),
                task: task.to_string(),
                started_at: Self::now_ms(),
            },
        )
        .await
        {
            Ok(w) => Some(w),
            Err(e) => {
                tracing::warn!(run_id, error = %e, "failed to create sidechain writer");
                None
            }
        };

        // Write initial user message to sidechain
        if let Some(ref _writer) = sidechain_writer {
            // We'll write it inside the forwarder after moving writer ownership
        }

        let (child_tx, mut child_rx) = mpsc::channel::<AgentEvent>(256);

        let run_id_owned = run_id.to_string();
        let parent_tx_clone = parent_tx.clone();
        let forward_turn_id = turn_id.clone();
        let agent_id_for_sidechain = config.agent_id.to_string();
        let task_for_sidechain = task.to_string();

        let forwarder = tokio::spawn(async move {
            let mut accumulated_text = String::new();
            let mut final_usage: Option<Usage> = None;
            let mut writer = sidechain_writer;

            // Write initial user message to sidechain
            if let Some(ref mut w) = writer {
                let user_msg = SidechainMessage {
                    role: "user".to_string(),
                    content: task_for_sidechain,
                    tool_calls_json: None,
                    tool_call_id: None,
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64,
                    agent_id: agent_id_for_sidechain.clone(),
                };
                if let Err(e) = w.append(&user_msg).await {
                    tracing::warn!(error = %e, "sidechain: failed to write user message");
                }
            }

            while let Some(event) = child_rx.recv().await {
                match &event {
                    AgentEvent::ContentDelta { delta, .. } => {
                        if let Some(content) = delta
                            .get("choices")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("delta"))
                            .and_then(|d| d.get("content"))
                            .and_then(|c| c.as_str())
                        {
                            accumulated_text.push_str(content);
                            let _ = parent_tx_clone
                                .send(AgentEvent::SubAgentDelta {
                                    turn_id: forward_turn_id.clone(),
                                    run_id: run_id_owned.clone(),
                                    content: content.to_string(),
                                })
                                .await;
                        }
                    }
                    AgentEvent::ReasoningDelta { content, .. } => {
                        accumulated_text.push_str(content);
                        let _ = parent_tx_clone
                            .send(AgentEvent::SubAgentDelta {
                                turn_id: forward_turn_id.clone(),
                                run_id: run_id_owned.clone(),
                                content: content.clone(),
                            })
                            .await;
                    }
                    AgentEvent::ToolExecuting {
                        tool_name,
                        call_id,
                        args,
                        ..
                    } => {
                        // Write tool call to sidechain
                        if let Some(ref mut w) = writer {
                            let tc_json = serde_json::json!([{
                                "id": call_id,
                                "function": {
                                    "name": tool_name,
                                    "arguments": args
                                }
                            }]);
                            let msg = SidechainMessage {
                                role: "assistant".to_string(),
                                content: String::new(),
                                tool_calls_json: Some(tc_json.to_string()),
                                tool_call_id: None,
                                timestamp: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_millis() as u64,
                                agent_id: agent_id_for_sidechain.clone(),
                            };
                            if let Err(e) = w.append(&msg).await {
                                tracing::warn!(error = %e, "sidechain: failed to write tool call");
                            }
                        }

                        let _ = parent_tx_clone
                            .send(AgentEvent::SubAgentToolExecuting {
                                turn_id: forward_turn_id.clone(),
                                run_id: run_id_owned.clone(),
                                tool_name: tool_name.clone(),
                                call_id: call_id.clone(),
                                args: args.clone(),
                            })
                            .await;
                    }
                    AgentEvent::ToolResult {
                        tool_name,
                        call_id,
                        output,
                        display_output,
                        success,
                        ..
                    } => {
                        // Write tool result to sidechain
                        if let Some(ref mut w) = writer {
                            let result_text = display_output.as_ref().unwrap_or(output);
                            let msg = SidechainMessage {
                                role: "tool".to_string(),
                                content: result_text.clone(),
                                tool_calls_json: None,
                                tool_call_id: Some(call_id.clone()),
                                timestamp: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_millis() as u64,
                                agent_id: tool_name.clone(),
                            };
                            if let Err(e) = w.append(&msg).await {
                                tracing::warn!(error = %e, "sidechain: failed to write tool result");
                            }
                        }

                        let ui_out = display_output.as_ref().unwrap_or(output);
                        let _ = parent_tx_clone
                            .send(AgentEvent::SubAgentToolResult {
                                turn_id: forward_turn_id.clone(),
                                run_id: run_id_owned.clone(),
                                tool_name: tool_name.clone(),
                                call_id: call_id.clone(),
                                output: ui_out.clone(),
                                success: *success,
                            })
                            .await;
                    }
                    AgentEvent::TurnEnd { summary, .. } => {
                        if let Some(u) = &summary.usage {
                            final_usage = Some(Usage {
                                prompt_tokens: u.prompt_tokens,
                                completion_tokens: u.completion_tokens,
                                total_tokens: u.total_tokens,
                                ..Default::default()
                            });
                        }
                    }
                    AgentEvent::FileArtifact { .. } => {
                        let _ = parent_tx_clone.send(event).await;
                    }
                    _ => {}
                }
            }

            // Write final assistant message to sidechain (full accumulated text)
            if let Some(ref mut w) = writer {
                if !accumulated_text.is_empty() {
                    let msg = SidechainMessage {
                        role: "assistant".to_string(),
                        content: accumulated_text.clone(),
                        tool_calls_json: None,
                        tool_call_id: None,
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                        agent_id: agent_id_for_sidechain.clone(),
                    };
                    if let Err(e) = w.append(&msg).await {
                        tracing::warn!(error = %e, "sidechain: failed to write assistant message");
                    }
                }
            }

            (accumulated_text, final_usage)
        });

        let effective_config;
        let config_ref = if let Some(ic) = inherited_context {
            let mut cfg = config.clone();
            cfg.behavior.file_access = ic.file_access;
            if !ic.additional_allowed_paths.is_empty() {
                cfg.behavior.additional_allowed_paths = ic.additional_allowed_paths.clone();
            }
            effective_config = cfg;
            &effective_config
        } else {
            config
        };

        let stream_result = runtime
            .execute_unified_with_cost_store(
                config_ref,
                &request,
                tool_registry,
                child_tx,
                approval_strategy,
                llm_override,
                orchestrator,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                message_queue,
            )
            .await;

        let (accumulated_text, final_usage) = forwarder
            .await
            .map_err(|e| anyhow::anyhow!("forwarder task panicked: {e}"))?;

        match stream_result {
            Ok(summary) => Ok((
                accumulated_text,
                summary.tool_calls_made,
                summary.iterations,
                final_usage,
            )),
            Err(e) => Err(e),
        }
    }

    fn fail_run(
        runs: &DashMap<String, SubAgentRun>,
        cancel_tokens: &DashMap<String, CancellationToken>,
        run_id: &str,
        reason: &str,
    ) {
        if let Some(mut r) = runs.get_mut(run_id) {
            r.status = SubAgentStatus::Failed(reason.to_string());
            r.completed_at = Some(Self::now_ms());
        }
        cancel_tokens.remove(run_id);
    }

    /// Cancel a running sub-agent.
    pub fn cancel(&self, run_id: &str) -> bool {
        if let Some((_, token)) = self.cancel_tokens.remove(run_id) {
            token.cancel();
            true
        } else {
            false
        }
    }

    /// Get a snapshot of a sub-agent run.
    pub fn get_run(&self, run_id: &str) -> Option<SubAgentRun> {
        self.runs.get(run_id).map(|r| r.clone())
    }

    /// List all runs, optionally filtered by parent session.
    pub fn list_runs(&self, parent_session_id: Option<&str>) -> Vec<SubAgentRun> {
        self.runs
            .iter()
            .filter(|r| parent_session_id.is_none_or(|sid| r.parent_session_id == sid))
            .map(|r| r.value().clone())
            .collect()
    }

    /// Insert a run directly (for testing).
    #[cfg(test)]
    pub(crate) fn insert_run(&self, run: SubAgentRun) {
        self.runs.insert(run.run_id.clone(), run);
    }

    /// Remove completed/failed/cancelled runs older than `max_age`.
    pub fn gc(&self, max_age: Duration) {
        let cutoff = Self::now_ms().saturating_sub(max_age.as_millis() as u64);
        self.runs.retain(|_, r| {
            if r.status.is_terminal() {
                r.completed_at.is_none_or(|t| t > cutoff)
            } else {
                true
            }
        });

        // Also prune completion channels whose broadcast senders have no receivers
        // and no active runs referencing that session.
        self.completion_channels.retain(|session_id, tx| {
            if tx.receiver_count() > 0 {
                return true;
            }
            // Keep the channel if there are still active runs for this session
            self.runs
                .iter()
                .any(|r| r.parent_session_id == *session_id && !r.status.is_terminal())
        });
    }

    /// Clean up all resources associated with a session.
    /// Should be called when a session is destroyed or the session actor is dropped.
    pub fn cleanup_session(&self, session_id: &str) {
        self.completion_channels.remove(session_id);
        self.session_event_senders.remove(session_id);
        self.runs
            .retain(|_, r| r.parent_session_id != session_id || !r.status.is_terminal());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spawn_controller::{SpawnConfig, SpawnController};
    use xiaolin_core::agent_config::AgentConfig;

    fn make_manager(agents: Vec<AgentConfig>) -> SubAgentManager {
        let runtime = Arc::new(crate::AgentRuntime::new(Arc::from(
            crate::OpenAiProvider::new("http://example.com", "fake"),
        )));
        runtime.init_self_arc();
        let controller = Arc::new(SpawnController::new(SpawnConfig::default()));
        SubAgentManager::new(runtime, agents, SubAgentPolicy::default(), controller)
    }

    fn test_agent(id: &str) -> AgentConfig {
        AgentConfig {
            agent_id: id.into(),
            name: Some(format!("{id} agent")),
            description: Some(format!("Test agent {id}")),
            model: Default::default(),
            system_prompt: None,
            tools: vec![],
            behavior: Default::default(),
            mcp_servers: vec![],
            min_tier: None,
            max_tier: None,
            avatar: None,
            channels: std::collections::HashMap::new(),
        }
    }

    #[tokio::test]
    async fn resolve_agent_known_and_unknown() {
        let mgr = make_manager(vec![test_agent("alpha"), test_agent("beta")]);
        assert!(mgr.resolve_agent("alpha").is_some());
        assert!(mgr.resolve_agent("beta").is_some());
        assert!(mgr.resolve_agent("gamma").is_none());
    }

    #[tokio::test]
    async fn agent_descriptions_lists_all() {
        let mgr = make_manager(vec![test_agent("a"), test_agent("b")]);
        let descs = mgr.agent_descriptions();
        assert_eq!(descs.len(), 2);
        assert!(descs.iter().any(|(id, _)| id == "a"));
        assert!(descs.iter().any(|(id, _)| id == "b"));
    }

    #[tokio::test]
    async fn set_available_agents_replaces_list() {
        let mut mgr = make_manager(vec![test_agent("old")]);
        assert!(mgr.resolve_agent("old").is_some());
        mgr.set_available_agents(vec![test_agent("new")]);
        assert!(mgr.resolve_agent("old").is_none());
        assert!(mgr.resolve_agent("new").is_some());
    }

    #[tokio::test]
    async fn spawn_rejects_when_disabled() {
        let mgr = make_manager(vec![test_agent("x")]);
        let agent_config = mgr.resolve_agent("x").unwrap();
        let (tx, _rx) = mpsc::channel(16);
        let policy = SubAgentPolicy {
            enabled: false,
            ..Default::default()
        };

        let err = mgr
            .spawn(
                agent_config,
                SubAgentType::General,
                "test task".into(),
                None,
                "session1".into(),
                "msg1".into(),
                0,
                &policy,
                Arc::new(ToolRegistry::new()),
                tx,
                None,
                false,
                None,
                None,
                xiaolin_core::agent_config::PermissionMode::AutoApprove,
                None,
                None,
            )
            .await;
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("disabled"));
    }

    #[tokio::test]
    async fn spawn_rejects_at_depth_limit() {
        let mgr = make_manager(vec![test_agent("x")]);
        let agent_config = mgr.resolve_agent("x").unwrap();
        let (tx, _rx) = mpsc::channel(16);
        let policy = SubAgentPolicy {
            max_depth: 2,
            ..Default::default()
        };

        let err = mgr
            .spawn(
                agent_config,
                SubAgentType::General,
                "test task".into(),
                None,
                "session1".into(),
                "msg1".into(),
                2,
                &policy,
                Arc::new(ToolRegistry::new()),
                tx,
                None,
                false,
                None,
                None,
                xiaolin_core::agent_config::PermissionMode::AutoApprove,
                None,
                None,
            )
            .await;
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("depth limit"));
    }

    #[tokio::test]
    async fn list_runs_filters_by_session() {
        let mgr = make_manager(vec![]);
        let run1 = SubAgentRun {
            run_id: "r1".into(),
            agent_id: "a".into(),
            subagent_type: SubAgentType::General,
            task: "t1".into(),
            status: SubAgentStatus::Running,
            parent_session_id: "s1".into(),
            parent_message_id: "m1".into(),
            depth: 0,
            result: None,
            tool_calls_made: 0,
            iterations: 0,
            created_at: 100,
            completed_at: None,
            token_usage: None,
            elapsed_ms: None,
        };
        let mut run2 = run1.clone();
        run2.run_id = "r2".into();
        run2.parent_session_id = "s2".into();
        mgr.runs.insert("r1".into(), run1);
        mgr.runs.insert("r2".into(), run2);

        assert_eq!(mgr.list_runs(None).len(), 2);
        assert_eq!(mgr.list_runs(Some("s1")).len(), 1);
        assert_eq!(mgr.list_runs(Some("s1"))[0].run_id, "r1");
        assert_eq!(mgr.list_runs(Some("s999")).len(), 0);
    }

    #[tokio::test]
    async fn gc_removes_old_terminal_runs() {
        let mgr = make_manager(vec![]);
        let old_run = SubAgentRun {
            run_id: "old".into(),
            agent_id: "a".into(),
            subagent_type: SubAgentType::General,
            task: "t".into(),
            status: SubAgentStatus::Completed,
            parent_session_id: "s".into(),
            parent_message_id: "m".into(),
            depth: 0,
            result: Some("done".into()),
            tool_calls_made: 1,
            iterations: 1,
            created_at: 0,
            completed_at: Some(1),
            token_usage: None,
            elapsed_ms: None,
        };
        let mut running = old_run.clone();
        running.run_id = "active".into();
        running.status = SubAgentStatus::Running;
        running.completed_at = None;
        mgr.runs.insert("old".into(), old_run);
        mgr.runs.insert("active".into(), running);

        mgr.gc(Duration::from_secs(1));
        assert!(
            mgr.get_run("old").is_none(),
            "old completed run should be GC'd"
        );
        assert!(
            mgr.get_run("active").is_some(),
            "running run should survive GC"
        );
    }

    #[tokio::test]
    async fn cancel_nonexistent_run_returns_false() {
        let mgr = make_manager(vec![]);
        assert!(!mgr.cancel("nonexistent"));
    }

    #[tokio::test]
    async fn cleanup_session_removes_event_senders() {
        let mgr = make_manager(vec![]);
        let (tx, _rx) = mpsc::channel(16);
        mgr.register_session_tx("s1", tx.clone());
        mgr.register_session_tx("s2", tx);
        assert!(mgr.get_session_tx("s1").is_some());
        assert!(mgr.get_session_tx("s2").is_some());

        mgr.cleanup_session("s1");
        assert!(mgr.get_session_tx("s1").is_none(), "s1 sender should be removed");
        assert!(mgr.get_session_tx("s2").is_some(), "s2 sender should remain");
    }
}
