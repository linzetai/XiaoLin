use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use fastclaw_core::agent_config::AgentConfig;
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolRegistry, ToolResult};
use fastclaw_core::types::{ChatMessage, ChatRequest, Role};

use crate::AgentRuntime;

/// A tool that spawns a child agent to handle a delegated task.
/// The child runs in its own session with its own system prompt,
/// but shares the same LLM provider and tool registry.
pub struct SubAgentTool {
    runtime: Arc<AgentRuntime>,
    tool_registry: Arc<ToolRegistry>,
    available_agents: Vec<AgentConfig>,
    max_depth: u32,
    current_depth: u32,
}

impl SubAgentTool {
    pub fn new(
        runtime: Arc<AgentRuntime>,
        tool_registry: Arc<ToolRegistry>,
        available_agents: Vec<AgentConfig>,
    ) -> Self {
        Self {
            runtime,
            tool_registry,
            available_agents,
            max_depth: 3,
            current_depth: 0,
        }
    }

    pub fn with_depth(mut self, current: u32, max: u32) -> Self {
        self.current_depth = current;
        self.max_depth = max;
        self
    }

    fn resolve_agent(&self, agent_id: &str) -> Option<&AgentConfig> {
        self.available_agents
            .iter()
            .find(|a| a.agent_id == agent_id)
    }
}

#[derive(Deserialize)]
struct SpawnParams {
    task: String,
    #[serde(default = "default_agent")]
    agent_id: String,
    #[serde(default)]
    context: Option<String>,
}

fn default_agent() -> String {
    "default".to_string()
}

#[derive(Serialize)]
struct SpawnResult {
    agent_id: String,
    task: String,
    response: String,
    tool_calls_made: u32,
    iterations: u32,
}

#[async_trait]
impl Tool for SubAgentTool {
    fn name(&self) -> &str {
        "spawn_subagent"
    }

    fn description(&self) -> &str {
        "Spawn a sub-agent to handle a delegated task. The sub-agent runs independently \
         with its own system prompt and returns its response. Use this to break complex \
         tasks into smaller pieces handled by specialized agents."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "task".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Clear description of the task for the sub-agent to perform"
            }),
        );

        let agent_ids: Vec<&str> = self
            .available_agents
            .iter()
            .map(|a| a.agent_id.as_str())
            .collect();
        props.insert(
            "agent_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": format!("ID of the agent to use. Available: {:?}", agent_ids),
                "default": "default"
            }),
        );
        props.insert(
            "context".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional context/data to pass to the sub-agent"
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

        if self.current_depth >= self.max_depth {
            return ToolResult::err(format!(
                "sub-agent depth limit reached ({}/{}). Cannot spawn deeper.",
                self.current_depth, self.max_depth
            ));
        }

        let agent_config = match self.resolve_agent(&params.agent_id) {
            Some(c) => c.clone(),
            None => {
                let available: Vec<&str> = self
                    .available_agents
                    .iter()
                    .map(|a| a.agent_id.as_str())
                    .collect();
                return ToolResult::err(format!(
                    "agent '{}' not found. Available: {:?}",
                    params.agent_id, available
                ));
            }
        };

        let mut messages = Vec::new();
        if let Some(ctx) = &params.context {
            messages.push(ChatMessage {
                role: Role::System,
                content: Some(serde_json::Value::String(format!(
                    "Context from parent agent:\n{ctx}"
                ))),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            });
        }
        messages.push(ChatMessage {
            role: Role::User,
            content: Some(serde_json::Value::String(params.task.clone())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        });

        let request = ChatRequest {
            messages,
            stream: false,
            model: None,
            temperature: None,
            max_tokens: None,
            agent_id: Some(params.agent_id.clone()),
            session_id: None,
            tools: None,
            slash_intent: None,
            work_dir: None,
        };

        // Build a child tool registry without the subagent tool itself
        // to prevent infinite recursion while still allowing other tools.
        let mut child_registry = ToolRegistry::new();
        for def in self.tool_registry.definitions() {
            if def.function.name != "spawn_subagent" {
                if let Some(tool) = self.tool_registry.get(&def.function.name) {
                    child_registry.register(tool.clone());
                }
            }
        }

        // If max depth allows, add a depth-incremented subagent tool
        if self.current_depth + 1 < self.max_depth {
            let child_subagent = SubAgentTool::new(
                self.runtime.clone(),
                self.tool_registry.clone(),
                self.available_agents.clone(),
            )
            .with_depth(self.current_depth + 1, self.max_depth);
            child_registry.register(Arc::new(child_subagent));
        }

        tracing::info!(
            parent_depth = self.current_depth,
            child_agent = %params.agent_id,
            task_len = params.task.len(),
            "spawning sub-agent"
        );

        match self
            .runtime
            .execute(&agent_config, &request, &child_registry, None)
            .await
        {
            Ok(result) => {
                let response_text = result
                    .response
                    .choices
                    .first()
                    .and_then(|c| c.message.text_content())
                    .unwrap_or_else(|| "(no response)".to_string());

                let out = SpawnResult {
                    agent_id: params.agent_id,
                    task: params.task,
                    response: response_text,
                    tool_calls_made: result.tool_calls_made,
                    iterations: result.iterations,
                };

                match serde_json::to_string(&out) {
                    Ok(json) => ToolResult::ok(json),
                    Err(e) => ToolResult::err(format!("serialization error: {e}")),
                }
            }
            Err(e) => ToolResult::err(format!("sub-agent execution failed: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subagent_tool_definition() {
        let runtime = Arc::new(AgentRuntime::new(Arc::from(crate::OpenAiProvider::new(
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

        let tool = SubAgentTool::new(runtime, tool_reg, agents);
        let def = tool.to_definition();
        assert_eq!(def.function.name, "spawn_subagent");
        assert!(def.function.description.contains("sub-agent"));
    }
}
