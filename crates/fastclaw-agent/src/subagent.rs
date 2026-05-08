use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use fastclaw_core::agent_config::SubAgentPolicy;
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolRegistry, ToolResult};
use fastclaw_core::types::{StreamEvent, SubAgentType};

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
    parent_tx: Option<mpsc::Sender<StreamEvent>>,
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

    pub fn with_parent_tx(mut self, tx: mpsc::Sender<StreamEvent>) -> Self {
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
    #[serde(default = "default_agent")]
    agent_id: String,
    #[serde(default)]
    subagent_type: Option<String>,
    #[serde(default)]
    context: Option<String>,
}

fn default_agent() -> String {
    "default".to_string()
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

#[derive(Serialize)]
struct SpawnResult {
    run_id: String,
    agent_id: String,
    subagent_type: String,
    task: String,
    response: String,
    tool_calls_made: u32,
    iterations: u32,
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
                "read_file" | "file_read" | "search_in_files" | "file_search"
                    | "list_directory" | "workspace_symbols" | "go_to_definition"
                    | "find_references" | "web_search" | "web_fetch" | "http_fetch"
                    | "memory_search" | "get_current_time" | "calculator"
                    | "list_skills" | "read_skill"
            ) || name.starts_with("mcp_")
        }),
        SubAgentType::Shell => Box::new(|name: &str| {
            matches!(
                name,
                "shell_exec" | "shell" | "read_file" | "file_read"
                    | "write_file" | "file_write" | "edit_file"
                    | "list_directory" | "search_in_files" | "file_search"
                    | "multi_edit" | "get_current_time"
            )
        }),
        SubAgentType::Browser => Box::new(|name: &str| {
            name.starts_with("browser") || matches!(
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
        "Spawn a sub-agent to handle a delegated task. The sub-agent runs independently \
         with its own context and tools. agent_id selects the model/behavior config; \
         subagent_type controls the tool set (e.g. agent_id='main' + subagent_type='explore' \
         = main model with read-only tools). Types: general (full), explore (read-only), \
         shell (commands only), browser (web automation)."
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

        let agent_descs = self.manager.agent_descriptions();
        let first_agent_id = agent_descs.first()
            .map(|(id, _)| id.clone())
            .unwrap_or_else(|| "default".to_string());
        let agent_list: Vec<String> = agent_descs.iter()
            .map(|(id, desc)| {
                if let Some(d) = desc {
                    format!("{id} ({d})")
                } else {
                    id.clone()
                }
            })
            .collect();
        props.insert(
            "agent_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": format!(
                    "Agent identity — determines the model and behavior config used. Available: {}. \
                     This is independent of subagent_type (tool set).",
                    agent_list.join(", ")
                ),
                "default": first_agent_id
            }),
        );
        props.insert(
            "subagent_type".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["general", "explore", "shell", "browser"],
                "description": "Tool set scope for this sub-agent (independent of agent_id). \
                 'general': full tool set. \
                 'explore': read-only (read_file, search_in_files, glob, web_search, web_fetch). \
                 'shell': only shell_exec for build/test. \
                 'browser': only browser automation tools. \
                 Example: agent_id='main' + subagent_type='explore' = main model with read-only tools.",
                "default": "general"
            }),
        );
        props.insert(
            "context".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional context or data to pass to the sub-agent that it cannot discover on its own"
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

        let subagent_type = parse_subagent_type(params.subagent_type.as_deref());

        if !self.policy.allowed_types.is_empty() {
            let type_str = subagent_type.to_string();
            if !self.policy.allowed_types.contains(&type_str) {
                return ToolResult::err(format!(
                    "sub-agent type '{}' not allowed. Allowed: {:?}",
                    type_str, self.policy.allowed_types
                ));
            }
        }

        if !self.policy.allowed_agents.is_empty()
            && !self.policy.allowed_agents.contains(&params.agent_id)
        {
            return ToolResult::err(format!(
                "agent '{}' not in allowed delegation targets: {:?}",
                params.agent_id, self.policy.allowed_agents
            ));
        }

        let agent_config = match self.manager.resolve_agent(&params.agent_id) {
            Some(c) => c,
            None if params.agent_id == "default" => {
                let descs = self.manager.agent_descriptions();
                if let Some((first_id, _)) = descs.first() {
                    tracing::info!(
                        fallback_from = "default",
                        fallback_to = %first_id,
                        "agent 'default' not found, falling back to first available agent"
                    );
                    match self.manager.resolve_agent(first_id) {
                        Some(c) => c,
                        None => return ToolResult::err("no agents available".to_string()),
                    }
                } else {
                    return ToolResult::err("no agents available for delegation".to_string());
                }
            }
            None => {
                let available: Vec<String> = self.manager.agent_descriptions()
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect();
                return ToolResult::err(format!(
                    "agent '{}' not found. Available: {:?}",
                    params.agent_id, available
                ));
            }
        };

        let child_registry = build_child_registry(&self.parent_tool_registry, &subagent_type);

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

        tracing::info!(
            parent_depth = self.current_depth,
            child_agent = %params.agent_id,
            subagent_type = %subagent_type,
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

        let run_id = match self.manager.spawn(
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
        ).await {
            Ok(id) => id,
            Err(e) => return ToolResult::err(format!("failed to spawn sub-agent: {e}")),
        };

        let poll_interval = std::time::Duration::from_millis(200);
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_secs(self.policy.timeout_seconds);

        loop {
            tokio::time::sleep(poll_interval).await;

            if std::time::Instant::now() > deadline {
                self.manager.cancel(&run_id);
                return ToolResult::err("sub-agent timed out while waiting for result".to_string());
            }

            if let Some(run) = self.manager.get_run(&run_id) {
                if run.status.is_terminal() {
                    let response = run.result.unwrap_or_else(|| "(no response)".to_string());
                    let out = SpawnResult {
                        run_id: run.run_id,
                        agent_id: params.agent_id,
                        subagent_type: subagent_type.to_string(),
                        task: params.task,
                        response,
                        tool_calls_made: run.tool_calls_made,
                        iterations: run.iterations,
                    };
                    return match serde_json::to_string(&out) {
                        Ok(json) => ToolResult::ok(json),
                        Err(e) => ToolResult::err(format!("serialization error: {e}")),
                    };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastclaw_core::agent_config::{AgentConfig, SubAgentPolicy};

    #[test]
    fn subagent_tool_definition() {
        let runtime = Arc::new(crate::AgentRuntime::new(Arc::from(crate::OpenAiProvider::new(
            "http://example.com",
            "fake",
        ))));
        let tool_reg = Arc::new(ToolRegistry::new());
        let agents = vec![AgentConfig {
            agent_id: "test".into(),
            name: Some("Test Agent".into()),
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

        let manager = Arc::new(SubAgentManager::new(
            runtime,
            agents,
            SubAgentPolicy::default(),
        ));
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
}
