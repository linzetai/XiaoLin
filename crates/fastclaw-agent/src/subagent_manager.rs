use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;

use fastclaw_core::agent_config::{AgentConfig, SubAgentPolicy};
use fastclaw_core::tool::ToolRegistry;
use fastclaw_core::types::{SubAgentRun, SubAgentStatus, SubAgentType, Usage};
use fastclaw_protocol::{AgentEvent, TokenUsage, TurnId};

use crate::llm::LlmProvider;
use crate::runtime::AgentRuntime;

/// Manages the lifecycle of all sub-agent runs: spawn, cancel, track, query.
pub struct SubAgentManager {
    runtime: Arc<AgentRuntime>,
    available_agents: Vec<AgentConfig>,
    runs: Arc<DashMap<String, SubAgentRun>>,
    cancel_tokens: Arc<DashMap<String, CancellationToken>>,
    concurrency: Arc<Semaphore>,
    #[allow(dead_code)]
    default_policy: SubAgentPolicy,
}

impl SubAgentManager {
    pub fn new(
        runtime: Arc<AgentRuntime>,
        available_agents: Vec<AgentConfig>,
        default_policy: SubAgentPolicy,
    ) -> Self {
        let max_parallel = default_policy.max_parallel as usize;
        Self {
            runtime,
            available_agents,
            runs: Arc::new(DashMap::new()),
            cancel_tokens: Arc::new(DashMap::new()),
            concurrency: Arc::new(Semaphore::new(max_parallel)),
            default_policy,
        }
    }

    /// Update the available agents list (e.g. after config reload).
    pub fn set_available_agents(&mut self, agents: Vec<AgentConfig>) {
        self.available_agents = agents;
    }

    pub fn available_agents(&self) -> &[AgentConfig] {
        &self.available_agents
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

        let runs = self.runs.clone();
        let cancel_tokens = self.cancel_tokens.clone();
        let runtime = self.runtime.clone();
        let concurrency = self.concurrency.clone();
        let timeout = Duration::from_secs(policy.timeout_seconds);
        let max_depth = policy.max_depth;
        let rid = run_id.clone();
        let forward_turn_id = turn_id.clone();

        tokio::spawn(async move {
            let _permit = match concurrency.acquire().await {
                Ok(p) => p,
                Err(_) => {
                    Self::fail_run(&runs, &cancel_tokens, &rid, "concurrency semaphore closed");
                    return;
                }
            };

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
                ) => {
                    res
                }
            };

            let elapsed_ms = t0.elapsed().as_millis() as u64;

            match result {
                Ok((response_text, tool_calls_made, iterations, usage)) => {
                    let _ = parent_tx
                        .send(AgentEvent::SubAgentComplete {
                            turn_id: complete_turn_id.clone(),
                            run_id: rid.clone(),
                            status: "completed".into(),
                            result: Some(response_text.clone()),
                            tool_calls_made,
                            iterations,
                            usage: usage.clone().map(|u| TokenUsage {
                                prompt_tokens: u.prompt_tokens,
                                completion_tokens: u.completion_tokens,
                                total_tokens: u.total_tokens,
                            }),
                            elapsed_ms,
                        })
                        .await;

                    if let Some(mut r) = runs.get_mut(&rid) {
                        r.status = SubAgentStatus::Completed;
                        r.completed_at = Some(Self::now_ms());
                        r.result = Some(response_text);
                        r.tool_calls_made = tool_calls_made;
                        r.iterations = iterations;
                        r.token_usage = usage;
                        r.elapsed_ms = Some(elapsed_ms);
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

                    if let Some(mut r) = runs.get_mut(&rid) {
                        if msg.contains("cancelled") {
                            r.status = SubAgentStatus::Cancelled;
                        } else {
                            r.status = SubAgentStatus::Failed(msg);
                        }
                        r.completed_at = Some(Self::now_ms());
                        r.elapsed_ms = Some(elapsed_ms);
                    }
                }
            }

            cancel_tokens.remove(&rid);
        });

        Ok(run_id)
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
    ) -> anyhow::Result<(String, u32, u32, Option<Usage>)> {
        use fastclaw_core::types::{ChatMessage, ChatRequest, Role};

        let mut messages = Vec::new();
        if let Some(ctx) = context {
            messages.push(ChatMessage {
                role: Role::System,
                content: Some(serde_json::Value::String(format!(
                    "Context from parent agent:\n{ctx}"
                ))),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
                compact_metadata: None,
            });
        }
        messages.push(ChatMessage {
            role: Role::User,
            content: Some(serde_json::Value::String(task.to_string())),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
            compact_metadata: None,
        });

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
            work_dir: None,
        };

        let (child_tx, mut child_rx) = mpsc::channel::<AgentEvent>(256);

        let run_id_owned = run_id.to_string();
        let parent_tx_clone = parent_tx.clone();
        let forward_turn_id = turn_id.clone();

        let forwarder = tokio::spawn(async move {
            let mut accumulated_text = String::new();
            let mut final_usage: Option<Usage> = None;

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
                    AgentEvent::ToolExecuting {
                        tool_name,
                        call_id,
                        args,
                        ..
                    } => {
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
                            });
                        }
                    }
                    _ => {}
                }
            }

            (accumulated_text, final_usage)
        });

        let stream_result = runtime
            .execute_stream(config, &request, tool_registry, child_tx, llm_override)
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastclaw_core::agent_config::AgentConfig;

    fn make_manager(agents: Vec<AgentConfig>) -> SubAgentManager {
        let runtime = Arc::new(crate::AgentRuntime::new(Arc::from(
            crate::OpenAiProvider::new("http://example.com", "fake"),
        )));
        SubAgentManager::new(runtime, agents, SubAgentPolicy::default())
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

    #[test]
    fn resolve_agent_known_and_unknown() {
        let mgr = make_manager(vec![test_agent("alpha"), test_agent("beta")]);
        assert!(mgr.resolve_agent("alpha").is_some());
        assert!(mgr.resolve_agent("beta").is_some());
        assert!(mgr.resolve_agent("gamma").is_none());
    }

    #[test]
    fn agent_descriptions_lists_all() {
        let mgr = make_manager(vec![test_agent("a"), test_agent("b")]);
        let descs = mgr.agent_descriptions();
        assert_eq!(descs.len(), 2);
        assert!(descs.iter().any(|(id, _)| id == "a"));
        assert!(descs.iter().any(|(id, _)| id == "b"));
    }

    #[test]
    fn set_available_agents_replaces_list() {
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
            )
            .await;
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("depth limit"));
    }

    #[test]
    fn list_runs_filters_by_session() {
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

    #[test]
    fn gc_removes_old_terminal_runs() {
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

    #[test]
    fn cancel_nonexistent_run_returns_false() {
        let mgr = make_manager(vec![]);
        assert!(!mgr.cancel("nonexistent"));
    }
}
