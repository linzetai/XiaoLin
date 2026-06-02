use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::mpsc;

use xiaolin_core::agent_config::SubAgentPolicy;
use xiaolin_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolRegistry, ToolResult};
use xiaolin_core::types::SubAgentType;
use xiaolin_protocol::AgentEvent;

use crate::subagent_manager::SubAgentManager;

/// A tool that spawns a child agent to handle a delegated task.
///
/// Backed by [`SubAgentManager`] for lifecycle management, concurrency control,
/// and streaming. Each child agent gets a type-appropriate tool registry.
pub struct SubAgentTool {
    manager: Arc<SubAgentManager>,
    parent_tool_registry: Arc<ToolRegistry>,
    policy: SubAgentPolicy,
    current_depth: u32,
    parent_tx: Option<mpsc::Sender<AgentEvent>>,
    parent_session_id: String,
}

impl SubAgentTool {
    pub fn new(
        manager: Arc<SubAgentManager>,
        parent_tool_registry: Arc<ToolRegistry>,
        policy: SubAgentPolicy,
    ) -> Self {
        Self {
            manager,
            parent_tool_registry,
            policy,
            current_depth: 0,
            parent_tx: None,
            parent_session_id: String::new(),
        }
    }

    pub fn with_depth(mut self, current: u32) -> Self {
        self.current_depth = current;
        self
    }

    pub fn with_parent_tx(mut self, tx: mpsc::Sender<AgentEvent>) -> Self {
        self.parent_tx = Some(tx);
        self
    }

    pub fn with_parent_session(mut self, session_id: String) -> Self {
        self.parent_session_id = session_id;
        self
    }
}

#[derive(Deserialize)]
struct SpawnParams {
    task: String,
    /// Sub-agent type ID (maps to a SubAgentDef). Legacy `agent_id` is accepted
    /// but treated as an alias for `type` in the new model.
    #[serde(default, alias = "agent_id")]
    r#type: Option<String>,
    /// Legacy field — still accepted for backward compatibility.
    #[serde(default)]
    subagent_type: Option<String>,
    #[serde(default)]
    context: Option<String>,
    /// Override the def's background setting for this invocation.
    #[serde(default)]
    background: Option<bool>,
}

fn parse_subagent_type(s: Option<&str>) -> SubAgentType {
    match s {
        Some("explore") => SubAgentType::Explore,
        Some("shell") => SubAgentType::Shell,
        Some("browser") => SubAgentType::Browser,
        Some("general") | None => SubAgentType::General,
        Some(other) => SubAgentType::Custom(other.to_string()),
    }
}

/// Build a child tool registry filtered by sub-agent type.
///
/// - `General`: inherits all parent tools except `spawn_subagent` (added back if depth allows)
/// - `Explore`: read-only tools only
/// - `Shell`: shell + file tools
/// - `Browser`: browser + web tools
/// - `Custom`: same as General (custom filtering is done via agent config `tools_allow`/`tools_deny`)
pub fn build_child_registry(
    parent_registry: &ToolRegistry,
    subagent_type: &SubAgentType,
) -> ToolRegistry {
    let child = ToolRegistry::new();

    let allowed: Box<dyn Fn(&str) -> bool> = match subagent_type {
        SubAgentType::Explore => Box::new(|name: &str| {
            matches!(
                name,
                "read_file"
                    | "file_read"
                    | "search_in_files"
                    | "file_search"
                    | "list_directory"
                    | "workspace_symbols"
                    | "go_to_definition"
                    | "find_references"
                    | "web_search"
                    | "web_fetch"
                    | "http_fetch"
                    | "memory_search"
                    | "get_current_time"
                    | "calculator"
                    | "list_skills"
                    | "read_skill"
            ) || name.starts_with("mcp_")
        }),
        SubAgentType::Shell => Box::new(|name: &str| {
            matches!(
                name,
                "shell_exec"
                    | "shell"
                    | "read_file"
                    | "file_read"
                    | "write_file"
                    | "file_write"
                    | "edit_file"
                    | "list_directory"
                    | "search_in_files"
                    | "file_search"
                    | "multi_edit"
                    | "get_current_time"
            )
        }),
        SubAgentType::Browser => Box::new(|name: &str| {
            name.starts_with("browser")
                || matches!(
                    name,
                    "web_fetch" | "http_fetch" | "web_search" | "get_current_time"
                )
        }),
        SubAgentType::General | SubAgentType::Custom(_) => {
            Box::new(|name: &str| name != "spawn_subagent")
        }
    };

    for def in parent_registry.definitions().iter() {
        let name = &def.function.name;
        if allowed(name) {
            if let Some(tool) = parent_registry.get(name) {
                child.register(tool.clone());
            }
        }
    }

    child
}

#[async_trait]
impl Tool for SubAgentTool {
    fn name(&self) -> &str {
        "spawn_subagent"
    }

    fn description(&self) -> &str {
        "Spawn a sub-agent to handle a delegated task. Use the `type` parameter to select \
         a sub-agent type (e.g. 'explore', 'code', 'shell', 'research'). Use list_agents \
         to discover available types. By default, runs synchronously and returns the result \
         directly. Set background=true for async execution."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "task".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Clear, self-contained description of the task. Include all necessary context — the sub-agent cannot see your conversation."
            }),
        );

        let def_descs = self.manager.subagent_def_descriptions();
        let type_list: Vec<String> = def_descs
            .iter()
            .map(|(id, desc)| {
                if let Some(d) = desc {
                    format!("{id} ({d})")
                } else {
                    id.clone()
                }
            })
            .collect();
        props.insert(
            "type".to_string(),
            serde_json::json!({
                "type": "string",
                "description": format!(
                    "Sub-agent type to spawn. Available: {}. \
                     Each type has a specific tool set and system prompt.",
                    type_list.join(", ")
                ),
                "default": "code"
            }),
        );
        props.insert(
            "context".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional context or data to pass to the sub-agent that it cannot discover on its own"
            }),
        );
        props.insert(
            "background".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "Run in background (async). Default depends on the sub-agent type definition. When false, blocks until completion and returns the result directly."
            }),
        );

        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["task".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let params: SpawnParams = match serde_json::from_str(arguments) {
            Ok(p) => p,
            Err(e) => return ToolResult::err(format!("invalid arguments: {e}")),
        };

        if !self.policy.enabled {
            return ToolResult::err("sub-agent delegation is disabled for this agent".to_string());
        }

        if self.current_depth >= self.policy.max_depth {
            return ToolResult::err(format!(
                "sub-agent depth limit reached ({}/{}). Cannot spawn deeper.",
                self.current_depth, self.policy.max_depth
            ));
        }

        let type_id = params
            .r#type
            .as_deref()
            .or(params.subagent_type.as_deref())
            .unwrap_or("code");

        if !self.policy.allowed_types.is_empty() && !self.policy.allowed_types.contains(&type_id.to_string()) {
            return ToolResult::err(format!(
                "sub-agent type '{}' not allowed. Allowed: {:?}",
                type_id, self.policy.allowed_types
            ));
        }

        let def = self.manager.resolve_subagent_def(type_id);
        let subagent_type = parse_subagent_type(Some(type_id));

        let (child_registry, use_background) = if let Some(ref def) = def {
            let registry = SubAgentManager::build_child_registry_from_def(
                &self.parent_tool_registry,
                def,
            );
            let bg = params.background.unwrap_or(def.background);
            (registry, bg)
        } else {
            let registry = build_child_registry(&self.parent_tool_registry, &subagent_type);
            let bg = params.background.unwrap_or(true);
            (registry, bg)
        };

        if self.current_depth + 1 < self.policy.max_depth {
            let child_subagent = SubAgentTool::new(
                self.manager.clone(),
                self.parent_tool_registry.clone(),
                self.policy.clone(),
            )
            .with_depth(self.current_depth + 1);
            child_registry.register(Arc::new(child_subagent));
        }

        let child_registry = Arc::new(child_registry);

        let agent_config = match self.manager.resolve_agent("main") {
            Some(mut c) => {
                if let Some(ref def) = def {
                    if let Some(ref prompt) = def.system_prompt {
                        c.system_prompt = Some(prompt.clone());
                    }
                }
                c
            }
            None => {
                let agents = self.manager.available_agents();
                match agents.first() {
                    Some(c) => {
                        let mut c = c.clone();
                        if let Some(ref def) = def {
                            if let Some(ref prompt) = def.system_prompt {
                                c.system_prompt = Some(prompt.clone());
                            }
                        }
                        c
                    }
                    None => return ToolResult::err("no agent config available".to_string()),
                }
            }
        };

        let concurrency_safe = def.as_ref().is_some_and(|d| d.concurrency_safe);

        tracing::info!(
            parent_depth = self.current_depth,
            def_type = %type_id,
            background = use_background,
            concurrency_safe,
            task_len = params.task.len(),
            "spawning sub-agent"
        );

        let parent_tx = match &self.parent_tx {
            Some(tx) => tx.clone(),
            None => {
                let (tx, _rx) = mpsc::channel(16);
                tx
            }
        };

        if use_background {
            let run_id = match self
                .manager
                .spawn(
                    agent_config,
                    subagent_type.clone(),
                    params.task.clone(),
                    params.context.clone(),
                    self.parent_session_id.clone(),
                    String::new(),
                    self.current_depth,
                    &self.policy,
                    child_registry,
                    parent_tx,
                    None,
                    concurrency_safe,
                )
                .await
            {
                Ok(id) => id,
                Err(e) => return ToolResult::err(format!("failed to spawn sub-agent: {e}")),
            };

            ToolResult::ok(serde_json::json!({
                "run_id": run_id,
                "type": type_id,
                "status": "running",
                "message": "Sub-agent spawned in background. Use subagent_get with this run_id to check results."
            }).to_string())
        } else {
            #[allow(deprecated)]
            match self
                .manager
                .spawn_sync(
                    agent_config,
                    subagent_type.clone(),
                    params.task.clone(),
                    params.context.clone(),
                    self.parent_session_id.clone(),
                    String::new(),
                    self.current_depth,
                    &self.policy,
                    child_registry,
                    parent_tx,
                    None,
                    concurrency_safe,
                )
                .await
            {
                Ok((result, run_id)) => {
                    ToolResult::ok(serde_json::json!({
                        "run_id": run_id,
                        "type": type_id,
                        "status": "completed",
                        "result": result,
                    }).to_string())
                }
                Err(e) => ToolResult::err(format!("sub-agent failed: {e}")),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SubAgentGetTool — query a specific run by ID (non-blocking)
// ---------------------------------------------------------------------------

pub struct SubAgentGetTool {
    manager: Arc<SubAgentManager>,
}

impl SubAgentGetTool {
    pub fn new(manager: Arc<SubAgentManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for SubAgentGetTool {
    fn name(&self) -> &str {
        "subagent_get"
    }

    fn description(&self) -> &str {
        "Check the status and result of a previously spawned sub-agent by its run_id. Returns the current status (running/completed/failed/cancelled) and, if finished, the sub-agent's response."
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "run_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The run_id returned by spawn_subagent."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["run_id".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        #[derive(Deserialize)]
        struct Params {
            run_id: String,
        }
        let params: Params = match serde_json::from_str(arguments) {
            Ok(p) => p,
            Err(e) => return ToolResult::err(format!("invalid arguments: {e}")),
        };

        match self.manager.get_run(&params.run_id) {
            Some(run) => {
                let json = serde_json::json!({
                    "run_id": run.run_id,
                    "agent_id": run.agent_id.to_string(),
                    "subagent_type": run.subagent_type.to_string(),
                    "task": run.task,
                    "status": format!("{:?}", run.status),
                    "result": run.result,
                    "tool_calls_made": run.tool_calls_made,
                    "iterations": run.iterations,
                    "elapsed_ms": run.completed_at.map(|c| c.saturating_sub(run.created_at)),
                });
                ToolResult::ok(json.to_string())
            }
            None => ToolResult::err(format!(
                "no sub-agent run found with id '{}'",
                params.run_id
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// SubAgentListTool — list all sub-agent runs for the session
// ---------------------------------------------------------------------------

pub struct SubAgentListTool {
    manager: Arc<SubAgentManager>,
}

impl SubAgentListTool {
    pub fn new(manager: Arc<SubAgentManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for SubAgentListTool {
    fn name(&self) -> &str {
        "subagent_list"
    }

    fn description(&self) -> &str {
        "List all sub-agent runs in the current session with their status and summary."
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: HashMap::new(),
            required: vec![],
        }
    }

    async fn execute(&self, _arguments: &str) -> ToolResult {
        let runs = self.manager.list_runs(None);
        let summaries: Vec<serde_json::Value> = runs
            .iter()
            .map(|r| {
                serde_json::json!({
                    "run_id": r.run_id,
                    "agent_id": r.agent_id.to_string(),
                    "subagent_type": r.subagent_type.to_string(),
                    "status": format!("{:?}", r.status),
                    "task": if r.task.len() > 100 { let end = r.task.floor_char_boundary(100); format!("{}…", &r.task[..end]) } else { r.task.clone() },
                    "has_result": r.result.is_some(),
                })
            })
            .collect();
        ToolResult::ok(
            serde_json::json!({
                "total": runs.len(),
                "runs": summaries,
            })
            .to_string(),
        )
    }
}

// ---------------------------------------------------------------------------
// WaitAgentTool — wait for sub-agent(s) to complete
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct WaitParams {
    run_ids: Vec<String>,
    #[serde(default = "default_wait_mode")]
    mode: String,
    timeout_seconds: Option<u64>,
}

fn default_wait_mode() -> String {
    "all".to_string()
}

pub struct WaitAgentTool {
    manager: Arc<SubAgentManager>,
}

impl WaitAgentTool {
    pub fn new(manager: Arc<SubAgentManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for WaitAgentTool {
    fn name(&self) -> &str {
        "wait_agent"
    }

    fn description(&self) -> &str {
        "Wait for one or more sub-agent runs to complete. Use mode='all' to wait for all, or mode='any' to return as soon as the first one finishes."
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "run_ids".to_string(),
            serde_json::json!({
                "type": "array",
                "items": { "type": "string" },
                "description": "List of sub-agent run IDs to wait for."
            }),
        );
        props.insert(
            "mode".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["all", "any"],
                "description": "Wait strategy: 'all' waits for every run to complete; 'any' returns on the first completion. Default: 'all'."
            }),
        );
        props.insert(
            "timeout_seconds".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Maximum seconds to wait. Default: 300."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["run_ids".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        use crate::spawn_controller::SlotEvent;
        use xiaolin_core::types::SubAgentStatus;

        let params: WaitParams = match serde_json::from_str(arguments) {
            Ok(p) => p,
            Err(e) => return ToolResult::err(format!("invalid arguments: {e}")),
        };

        if params.run_ids.is_empty() {
            return ToolResult::err("run_ids must not be empty".to_string());
        }

        let wait_all = params.mode == "all";
        let timeout = std::time::Duration::from_secs(params.timeout_seconds.unwrap_or(300));

        for rid in &params.run_ids {
            if self.manager.get_run(rid).is_none() {
                return ToolResult::err(format!("unknown run_id: {rid}"));
            }
        }

        let mut results: HashMap<String, serde_json::Value> = HashMap::new();
        let mut pending: std::collections::HashSet<String> =
            params.run_ids.iter().cloned().collect();

        for rid in &params.run_ids {
            if let Some(run) = self.manager.get_run(rid) {
                if run.status.is_terminal() {
                    let entry = match &run.status {
                        SubAgentStatus::Completed => serde_json::json!({
                            "status": "completed",
                            "result": run.result
                        }),
                        SubAgentStatus::Failed(msg) => serde_json::json!({
                            "status": "failed",
                            "error": msg
                        }),
                        SubAgentStatus::Cancelled => serde_json::json!({
                            "status": "cancelled"
                        }),
                        _ => unreachable!(),
                    };
                    results.insert(rid.clone(), entry);
                    pending.remove(rid);
                }
            }
        }

        if !wait_all && !results.is_empty() {
            return ToolResult::ok(
                serde_json::json!({
                    "results": results,
                    "timed_out": false
                })
                .to_string(),
            );
        }

        if pending.is_empty() {
            return ToolResult::ok(
                serde_json::json!({
                    "results": results,
                    "timed_out": false
                })
                .to_string(),
            );
        }

        let controller = self.manager.controller();
        let mut receivers: Vec<tokio::sync::broadcast::Receiver<SlotEvent>> = Vec::new();
        for (_, pool) in controller.snapshot().sessions.iter().map(|s| {
            (
                s.session_id.clone(),
                controller.get_or_create_session_pool(&s.session_id),
            )
        }) {
            receivers.push(pool.subscribe_events());
        }
        if receivers.is_empty() {
            receivers.push(controller.get_or_create_session_pool("__wait__").subscribe_events());
        }

        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return ToolResult::ok(
                    serde_json::json!({
                        "results": results,
                        "timed_out": true
                    })
                    .to_string(),
                );
            }

            tokio::select! {
                _ = tokio::time::sleep(remaining) => {
                    return ToolResult::ok(serde_json::json!({
                        "results": results,
                        "timed_out": true
                    }).to_string());
                }
                _ = async {
                    if let Some(rx) = receivers.first_mut() {
                        let _ = rx.recv().await;
                    } else {
                        tokio::time::sleep(remaining).await;
                    }
                } => {}
            }

            let mut newly_done = Vec::new();
            for rid in &pending {
                if let Some(run) = self.manager.get_run(rid) {
                    if run.status.is_terminal() {
                        let entry = match &run.status {
                            SubAgentStatus::Completed => serde_json::json!({
                                "status": "completed",
                                "result": run.result
                            }),
                            SubAgentStatus::Failed(msg) => serde_json::json!({
                                "status": "failed",
                                "error": msg
                            }),
                            SubAgentStatus::Cancelled => serde_json::json!({
                                "status": "cancelled"
                            }),
                            _ => unreachable!(),
                        };
                        results.insert(rid.clone(), entry);
                        newly_done.push(rid.clone());
                    }
                }
            }

            for rid in &newly_done {
                pending.remove(rid);
            }

            if !wait_all && !newly_done.is_empty() {
                return ToolResult::ok(
                    serde_json::json!({
                        "results": results,
                        "timed_out": false
                    })
                    .to_string(),
                );
            }

            if pending.is_empty() {
                return ToolResult::ok(
                    serde_json::json!({
                        "results": results,
                        "timed_out": false
                    })
                    .to_string(),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaolin_core::agent_config::{AgentConfig, SubAgentPolicy, builtin_subagent_defs};

    #[tokio::test]
    async fn subagent_tool_definition() {
        let runtime = Arc::new(crate::AgentRuntime::new(Arc::from(
            crate::OpenAiProvider::new("http://example.com", "fake"),
        )));
        let tool_reg = Arc::new(ToolRegistry::new());
        let agents = vec![AgentConfig {
            agent_id: "main".into(),
            name: Some("Main Agent".into()),
            description: None,
            model: Default::default(),
            system_prompt: None,
            tools: vec![],
            behavior: Default::default(),
            mcp_servers: vec![],
            min_tier: None,
            max_tier: None,
            avatar: None,
            channels: std::collections::HashMap::new(),
        }];

        let controller = Arc::new(crate::spawn_controller::SpawnController::new(
            crate::spawn_controller::SpawnConfig::default(),
        ));
        let manager = Arc::new(SubAgentManager::new(
            runtime,
            agents,
            SubAgentPolicy::default(),
            controller,
        ));
        manager.set_subagent_defs(builtin_subagent_defs());
        let tool = SubAgentTool::new(manager, tool_reg, SubAgentPolicy::default());
        let def = tool.to_definition();
        assert_eq!(def.function.name, "spawn_subagent");
        assert!(def.function.description.contains("sub-agent"));
    }

    #[test]
    fn parse_subagent_types() {
        assert_eq!(parse_subagent_type(None), SubAgentType::General);
        assert_eq!(parse_subagent_type(Some("general")), SubAgentType::General);
        assert_eq!(parse_subagent_type(Some("explore")), SubAgentType::Explore);
        assert_eq!(parse_subagent_type(Some("shell")), SubAgentType::Shell);
        assert_eq!(parse_subagent_type(Some("browser")), SubAgentType::Browser);
        assert_eq!(
            parse_subagent_type(Some("custom_thing")),
            SubAgentType::Custom("custom_thing".into())
        );
    }

    #[test]
    fn build_explore_registry_is_readonly() {
        let parent = ToolRegistry::new();
        let child = build_child_registry(&parent, &SubAgentType::Explore);
        for def in child.definitions().iter() {
            assert!(
                !matches!(
                    def.function.name.as_str(),
                    "write_file" | "file_write" | "shell_exec" | "shell" | "edit_file"
                ),
                "explore registry should not contain write tool: {}",
                def.function.name
            );
        }
    }

    // ===== WaitAgentTool tests =====

    use xiaolin_core::types::SubAgentRun;

    fn make_wait_manager() -> Arc<SubAgentManager> {
        let runtime = Arc::new(crate::AgentRuntime::new(Arc::from(
            crate::OpenAiProvider::new("http://example.com", "fake"),
        )));
        let controller = Arc::new(crate::spawn_controller::SpawnController::new(
            crate::spawn_controller::SpawnConfig::default(),
        ));
        Arc::new(SubAgentManager::new(
            runtime,
            vec![],
            SubAgentPolicy::default(),
            controller,
        ))
    }

    fn completed_run(run_id: &str, result: &str) -> SubAgentRun {
        SubAgentRun {
            run_id: run_id.into(),
            agent_id: "a".into(),
            subagent_type: SubAgentType::General,
            task: "t".into(),
            status: xiaolin_core::types::SubAgentStatus::Completed,
            parent_session_id: "s1".into(),
            parent_message_id: "m1".into(),
            depth: 0,
            result: Some(result.into()),
            tool_calls_made: 0,
            iterations: 0,
            created_at: 0,
            completed_at: Some(1),
            token_usage: None,
            elapsed_ms: None,
        }
    }

    fn running_run(run_id: &str) -> SubAgentRun {
        SubAgentRun {
            run_id: run_id.into(),
            agent_id: "a".into(),
            subagent_type: SubAgentType::General,
            task: "t".into(),
            status: xiaolin_core::types::SubAgentStatus::Running,
            parent_session_id: "s1".into(),
            parent_message_id: "m1".into(),
            depth: 0,
            result: None,
            tool_calls_made: 0,
            iterations: 0,
            created_at: 0,
            completed_at: None,
            token_usage: None,
            elapsed_ms: None,
        }
    }

    // --- 8.1 wait-all returns when all complete ---
    #[tokio::test]
    async fn wait_all_returns_when_all_complete() {
        let mgr = make_wait_manager();
        mgr.insert_run(completed_run("r1", "res1"));
        mgr.insert_run(completed_run("r2", "res2"));
        let tool = WaitAgentTool::new(mgr);

        let result = tool
            .execute(r#"{"run_ids":["r1","r2"],"mode":"all"}"#)
            .await;
        assert!(result.success);
        let v: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["timed_out"], false);
        assert!(v["results"]["r1"]["status"] == "completed");
        assert!(v["results"]["r2"]["status"] == "completed");
    }

    // --- 8.2 wait-any returns on first completion ---
    #[tokio::test]
    async fn wait_any_returns_on_first_completion() {
        let mgr = make_wait_manager();
        mgr.insert_run(completed_run("r1", "first"));
        mgr.insert_run(running_run("r2"));
        let tool = WaitAgentTool::new(mgr);

        let result = tool
            .execute(r#"{"run_ids":["r1","r2"],"mode":"any"}"#)
            .await;
        assert!(result.success);
        let v: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["timed_out"], false);
        assert!(v["results"]["r1"]["status"] == "completed");
        assert!(v["results"]["r2"].is_null());
    }

    // --- 8.3 wait timeout returns partial ---
    #[tokio::test]
    async fn wait_timeout_returns_partial() {
        let mgr = make_wait_manager();
        mgr.insert_run(completed_run("r1", "done"));
        mgr.insert_run(running_run("r2"));
        let tool = WaitAgentTool::new(mgr);

        let result = tool
            .execute(r#"{"run_ids":["r1","r2"],"mode":"all","timeout_seconds":1}"#)
            .await;
        assert!(result.success);
        let v: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["timed_out"], true);
        assert!(v["results"]["r1"]["status"] == "completed");
    }

    // --- 8.4 wait already completed returns immediately ---
    #[tokio::test]
    async fn wait_already_completed_returns_immediately() {
        let mgr = make_wait_manager();
        mgr.insert_run(completed_run("r1", "instant"));
        let tool = WaitAgentTool::new(mgr);

        let t0 = tokio::time::Instant::now();
        let result = tool
            .execute(r#"{"run_ids":["r1"],"mode":"all"}"#)
            .await;
        let elapsed = t0.elapsed();
        assert!(result.success);
        assert!(elapsed < std::time::Duration::from_millis(50));
    }

    // --- 8.5 unknown run_id returns error ---
    #[tokio::test]
    async fn wait_unknown_run_id_returns_error() {
        let mgr = make_wait_manager();
        let tool = WaitAgentTool::new(mgr);

        let result = tool
            .execute(r#"{"run_ids":["unknown"],"mode":"all"}"#)
            .await;
        assert!(!result.success);
        assert!(result.output.contains("unknown run_id"));
    }
}
