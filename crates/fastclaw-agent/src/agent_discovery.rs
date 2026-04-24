use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolResult};

use crate::subagent_manager::SubAgentManager;

// ---------------------------------------------------------------------------
// ListAgentsTool
// ---------------------------------------------------------------------------

/// Returns a summary of all available agents for delegation.
pub struct ListAgentsTool {
    manager: Arc<SubAgentManager>,
}

impl ListAgentsTool {
    pub fn new(manager: Arc<SubAgentManager>) -> Self {
        Self { manager }
    }
}

#[derive(Serialize)]
struct AgentSummary {
    id: String,
    name: Option<String>,
    description: Option<String>,
    model: String,
    tool_count: usize,
    has_system_prompt: bool,
    subagent_enabled: bool,
}

#[async_trait]
impl Tool for ListAgentsTool {
    fn name(&self) -> &str {
        "list_agents"
    }

    fn description(&self) -> &str {
        "List all available agents that can be delegated to via spawn_subagent. \
         Returns each agent's ID, name, description, model, tool count, and capabilities. \
         Use this before spawn_subagent to discover which agent is best suited for a task."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: HashMap::new(),
            required: vec![],
        }
    }

    async fn execute(&self, _arguments: &str) -> ToolResult {
        let agents = self.manager.available_agents();
        let summaries: Vec<AgentSummary> = agents
            .iter()
            .map(|a| AgentSummary {
                id: a.agent_id.to_string(),
                name: a.name.clone(),
                description: a.description.clone(),
                model: format!("{}/{}", a.model.provider, a.model.model),
                tool_count: a.tools.len(),
                has_system_prompt: a.system_prompt.is_some(),
                subagent_enabled: a.behavior.subagent.enabled,
            })
            .collect();

        match serde_json::to_string_pretty(&summaries) {
            Ok(json) => ToolResult::ok(json),
            Err(e) => ToolResult::err(format!("serialization error: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// GetAgentInfoTool
// ---------------------------------------------------------------------------

/// Returns detailed configuration for a specific agent by ID.
pub struct GetAgentInfoTool {
    manager: Arc<SubAgentManager>,
}

impl GetAgentInfoTool {
    pub fn new(manager: Arc<SubAgentManager>) -> Self {
        Self { manager }
    }
}

#[derive(Deserialize)]
struct InfoParams {
    agent_id: String,
}

#[derive(Serialize)]
struct AgentInfo {
    id: String,
    name: Option<String>,
    description: Option<String>,
    model: ModelInfo,
    tools: Vec<ToolInfo>,
    behavior: BehaviorInfo,
}

#[derive(Serialize)]
struct ModelInfo {
    provider: String,
    model: String,
    temperature: f32,
    max_tokens: Option<u32>,
    context_window: Option<u32>,
}

#[derive(Serialize)]
struct ToolInfo {
    id: String,
    name: Option<String>,
    description: Option<String>,
    enabled: bool,
}

#[derive(Serialize)]
struct BehaviorInfo {
    max_tool_calls_per_turn: u32,
    tools_allow: Vec<String>,
    tools_deny: Vec<String>,
    subagent_enabled: bool,
    subagent_max_depth: u32,
    subagent_max_parallel: u32,
}

#[async_trait]
impl Tool for GetAgentInfoTool {
    fn name(&self) -> &str {
        "get_agent_info"
    }

    fn description(&self) -> &str {
        "Get detailed configuration for a specific agent by ID. Returns the agent's model, \
         tools, behavior settings, and delegation policy. Use after list_agents to inspect \
         a specific agent before delegating a task to it."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        let agent_descs = self.manager.agent_descriptions();
        let ids: Vec<&str> = agent_descs.iter().map(|(id, _)| id.as_str()).collect();

        props.insert(
            "agent_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": format!("The agent ID to look up. Available: {}", ids.join(", "))
            }),
        );

        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["agent_id".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let params: InfoParams = match serde_json::from_str(arguments) {
            Ok(p) => p,
            Err(e) => return ToolResult::err(format!("invalid arguments: {e}")),
        };

        let agent = match self.manager.resolve_agent(&params.agent_id) {
            Some(a) => a,
            None => {
                let available: Vec<String> = self
                    .manager
                    .agent_descriptions()
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect();
                return ToolResult::err(format!(
                    "agent '{}' not found. Available: {:?}",
                    params.agent_id, available
                ));
            }
        };

        let info = AgentInfo {
            id: agent.agent_id.to_string(),
            name: agent.name.clone(),
            description: agent.description.clone(),
            model: ModelInfo {
                provider: agent.model.provider.clone(),
                model: agent.model.model.clone(),
                temperature: agent.model.temperature,
                max_tokens: agent.model.max_tokens,
                context_window: agent.model.context_window,
            },
            tools: agent
                .tools
                .iter()
                .map(|t| ToolInfo {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    description: t.description.clone(),
                    enabled: t.enabled,
                })
                .collect(),
            behavior: BehaviorInfo {
                max_tool_calls_per_turn: agent.behavior.max_tool_calls_per_turn,
                tools_allow: agent.behavior.tools_allow.clone(),
                tools_deny: agent.behavior.tools_deny.clone(),
                subagent_enabled: agent.behavior.subagent.enabled,
                subagent_max_depth: agent.behavior.subagent.max_depth,
                subagent_max_parallel: agent.behavior.subagent.max_parallel,
            },
        };

        match serde_json::to_string_pretty(&info) {
            Ok(json) => ToolResult::ok(json),
            Err(e) => ToolResult::err(format!("serialization error: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastclaw_core::agent_config::{AgentConfig, SubAgentPolicy};

    fn make_manager() -> Arc<SubAgentManager> {
        let runtime = Arc::new(crate::AgentRuntime::new(Arc::from(
            crate::OpenAiProvider::new("http://example.com", "fake"),
        )));
        let agents = vec![
            AgentConfig {
                agent_id: "main".into(),
                name: Some("Main Agent".into()),
                description: Some("General-purpose assistant".into()),
                model: Default::default(),
                system_prompt: Some("You are helpful.".into()),
                tools: vec![],
                behavior: Default::default(),
                mcp_servers: vec![],
                min_tier: None,
                max_tier: None,
                avatar: None,
                channels: HashMap::new(),
            },
            AgentConfig {
                agent_id: "code".into(),
                name: Some("Code Agent".into()),
                description: Some("Specialized for coding tasks".into()),
                model: Default::default(),
                system_prompt: None,
                tools: vec![],
                behavior: Default::default(),
                mcp_servers: vec![],
                min_tier: None,
                max_tier: None,
                avatar: None,
                channels: HashMap::new(),
            },
        ];
        Arc::new(SubAgentManager::new(runtime, agents, SubAgentPolicy::default()))
    }

    #[tokio::test]
    async fn list_agents_returns_all() {
        let mgr = make_manager();
        let tool = ListAgentsTool::new(mgr);
        let result = tool.execute("{}").await;
        assert!(result.success);
        let summaries: Vec<serde_json::Value> =
            serde_json::from_str(&result.output).unwrap();
        assert_eq!(summaries.len(), 2);
        assert!(summaries.iter().any(|s| s["id"] == "main"));
        assert!(summaries.iter().any(|s| s["id"] == "code"));
    }

    #[tokio::test]
    async fn get_agent_info_returns_details() {
        let mgr = make_manager();
        let tool = GetAgentInfoTool::new(mgr);
        let result = tool
            .execute(r#"{"agent_id": "main"}"#)
            .await;
        assert!(result.success);
        let info: serde_json::Value =
            serde_json::from_str(&result.output).unwrap();
        assert_eq!(info["id"], "main");
        assert_eq!(info["name"], "Main Agent");
        assert_eq!(info["description"], "General-purpose assistant");
    }

    #[tokio::test]
    async fn get_agent_info_unknown_returns_error() {
        let mgr = make_manager();
        let tool = GetAgentInfoTool::new(mgr);
        let result = tool
            .execute(r#"{"agent_id": "nonexistent"}"#)
            .await;
        assert!(!result.success);
        assert!(result.output.contains("not found"));
    }
}
