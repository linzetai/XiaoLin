use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use xiaolin_core::agent_config::SubAgentDefSource;
use xiaolin_core::tool::{Tool, ToolParameterSchema, ToolResult};

use crate::subagent_manager::SubAgentManager;

fn format_source(source: &SubAgentDefSource) -> String {
    match source {
        SubAgentDefSource::Builtin => "builtin".to_string(),
        SubAgentDefSource::JsonFile(p) => format!("json:{}", p.display()),
        SubAgentDefSource::MarkdownFile(p) => format!("markdown:{}", p.display()),
    }
}

// ---------------------------------------------------------------------------
// ListAgentsTool
// ---------------------------------------------------------------------------

/// Returns a summary of all available sub-agent definitions that can be spawned.
pub struct ListAgentsTool {
    manager: Arc<SubAgentManager>,
}

impl ListAgentsTool {
    pub fn new(manager: Arc<SubAgentManager>) -> Self {
        Self { manager }
    }
}

#[derive(Serialize)]
struct SubAgentDefSummary {
    id: String,
    name: Option<String>,
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    background: bool,
    concurrency_safe: bool,
    has_system_prompt: bool,
    tool_filter: ToolFilterSummary,
    source: String,
}

#[derive(Serialize)]
struct ToolFilterSummary {
    allowed_count: usize,
    denied_count: usize,
}

#[async_trait]
impl Tool for ListAgentsTool {
    fn name(&self) -> &str {
        "list_agents"
    }

    fn description(&self) -> &str {
        "List all available sub-agent types that can be spawned via spawn_subagent. \
         Returns each type's ID, name, description, and capabilities. \
         Use this to discover which sub-agent type is best suited for a task."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: HashMap::new(),
            required: vec![],
        }
    }

    async fn execute(&self, _arguments: &str) -> ToolResult {
        let defs = self.manager.subagent_defs();
        let summaries: Vec<SubAgentDefSummary> = defs
            .iter()
            .map(|d| SubAgentDefSummary {
                id: d.id.clone(),
                name: d.name.clone(),
                description: d.description.clone(),
                model: d
                    .model
                    .as_ref()
                    .map(|m| format!("{}/{}", m.provider, m.model)),
                background: d.background,
                concurrency_safe: d.concurrency_safe,
                has_system_prompt: d.system_prompt.is_some(),
                tool_filter: ToolFilterSummary {
                    allowed_count: d.tools.allowed.len(),
                    denied_count: d.tools.denied.len(),
                },
                source: format_source(&d.source),
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

/// Returns detailed configuration for a specific sub-agent type by ID.
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
struct SubAgentDefInfo {
    id: String,
    name: Option<String>,
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    system_prompt_preview: Option<String>,
    background: bool,
    concurrency_safe: bool,
    tools_allowed: Vec<String>,
    tools_denied: Vec<String>,
}

#[async_trait]
impl Tool for GetAgentInfoTool {
    fn name(&self) -> &str {
        "get_agent_info"
    }

    fn description(&self) -> &str {
        "Get detailed configuration for a specific sub-agent type by ID. Returns the type's \
         model override, tool filters, system prompt preview, and concurrency settings. \
         Use after list_agents to inspect a specific type before spawning it."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "agent_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The sub-agent type ID to look up (e.g. 'explore', 'code', 'shell', 'research')"
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

        let def = match self.manager.resolve_subagent_def(&params.agent_id) {
            Some(d) => d,
            None => {
                let available: Vec<String> = self
                    .manager
                    .subagent_def_descriptions()
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect();
                return ToolResult::err(format!(
                    "sub-agent type '{}' not found. Available: {:?}",
                    params.agent_id, available
                ));
            }
        };

        let info = SubAgentDefInfo {
            id: def.id.clone(),
            name: def.name.clone(),
            description: def.description.clone(),
            model: def
                .model
                .as_ref()
                .map(|m| format!("{}/{}", m.provider, m.model)),
            system_prompt_preview: def.system_prompt.as_ref().map(|p| {
                let preview: String = p.chars().take(200).collect();
                if p.len() > 200 {
                    format!("{preview}...")
                } else {
                    preview
                }
            }),
            background: def.background,
            concurrency_safe: def.concurrency_safe,
            tools_allowed: def.tools.allowed.clone(),
            tools_denied: def.tools.denied.clone(),
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
    use xiaolin_core::agent_config::{builtin_subagent_defs, SubAgentPolicy};

    fn make_manager() -> Arc<SubAgentManager> {
        let runtime = Arc::new(crate::AgentRuntime::new(Arc::from(
            crate::OpenAiProvider::new("http://example.com", "fake"),
        )));
        runtime.init_self_arc();
        let controller = Arc::new(crate::spawn_controller::SpawnController::new(
            crate::spawn_controller::SpawnConfig::default(),
        ));
        let mgr = Arc::new(SubAgentManager::new(
            runtime,
            vec![],
            SubAgentPolicy::default(),
            controller,
        ));
        mgr.set_subagent_defs(builtin_subagent_defs());
        mgr
    }

    #[tokio::test]
    async fn list_agents_returns_builtin_defs() {
        let mgr = make_manager();
        let tool = ListAgentsTool::new(mgr);
        let result = tool.execute("{}").await;
        assert!(result.success);
        let summaries: Vec<serde_json::Value> = serde_json::from_str(&result.output).unwrap();
        assert!(summaries.len() >= 4);
        assert!(summaries.iter().any(|s| s["id"] == "explore"));
        assert!(summaries.iter().any(|s| s["id"] == "code"));
        assert!(summaries.iter().any(|s| s["id"] == "shell"));
        assert!(summaries.iter().any(|s| s["id"] == "research"));
    }

    #[tokio::test]
    async fn get_agent_info_returns_def_details() {
        let mgr = make_manager();
        let tool = GetAgentInfoTool::new(mgr);
        let result = tool.execute(r#"{"agent_id": "explore"}"#).await;
        assert!(result.success);
        let info: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(info["id"], "explore");
        assert_eq!(info["name"], "Explorer");
        assert_eq!(info["concurrency_safe"], true);
        assert_eq!(info["background"], false);
    }

    #[tokio::test]
    async fn get_agent_info_unknown_returns_error() {
        let mgr = make_manager();
        let tool = GetAgentInfoTool::new(mgr);
        let result = tool.execute(r#"{"agent_id": "nonexistent"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("not found"));
    }
}
