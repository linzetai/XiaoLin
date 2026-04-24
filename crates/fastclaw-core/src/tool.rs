use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::error::{FastClawError, FastClawResult};

/// JSON Schema describing a tool's parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolParameterSchema {
    #[serde(rename = "type")]
    pub schema_type: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub properties: HashMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<String>,
}

/// OpenAI-compatible tool definition sent to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: ToolParameterSchema,
}

/// Result of a tool execution.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
    /// When `true`, the runtime should pause and ask the user for confirmation
    /// before retrying this tool call. Used by the dangerous-ops-policy `confirm` mode.
    pub needs_confirmation: bool,
}

impl ToolResult {
    pub fn ok(output: impl Into<String>) -> Self {
        Self {
            success: true,
            output: output.into(),
            needs_confirmation: false,
        }
    }

    pub fn err(error: impl Into<String>) -> Self {
        Self {
            success: false,
            output: error.into(),
            needs_confirmation: false,
        }
    }

    /// A dangerous operation was detected and requires user confirmation before proceeding.
    /// The runtime will automatically present a confirmation dialog to the user.
    /// If approved, the tool is re-executed with `"confirmed": true` injected.
    pub fn needs_confirm(description: impl Into<String>) -> Self {
        let desc = description.into();
        Self {
            success: false,
            output: format!("⚠️ Dangerous operation — awaiting user confirmation.\n{desc}"),
            needs_confirmation: true,
        }
    }
}

/// Trait all tools must implement.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> ToolParameterSchema;

    async fn execute(&self, arguments: &str) -> ToolResult;

    fn to_definition(&self) -> ToolDefinition {
        ToolDefinition {
            tool_type: "function".to_string(),
            function: FunctionDefinition {
                name: self.name().to_string(),
                description: self.description().to_string(),
                parameters: self.parameters_schema(),
            },
        }
    }
}

/// Registry holding all available tools.
///
/// Uses interior `RwLock` so tools can be dynamically registered/unregistered
/// through a shared `Arc<ToolRegistry>` without external mutability.
pub struct ToolRegistry {
    tools: std::sync::RwLock<HashMap<String, Arc<dyn Tool>>>,
}

impl Clone for ToolRegistry {
    fn clone(&self) -> Self {
        let guard = self.tools.read().expect("ToolRegistry poisoned");
        Self {
            tools: std::sync::RwLock::new(guard.clone()),
        }
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: std::sync::RwLock::new(HashMap::new()),
        }
    }

    pub fn register(&self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        let mut guard = self.tools.write().expect("ToolRegistry poisoned");
        if guard.contains_key(&name) {
            tracing::warn!(tool = %name, "duplicate tool name – overwriting previous registration");
        }
        guard.insert(name, tool);
    }

    /// Remove all tools whose name starts with `prefix`. Returns the number removed.
    pub fn unregister_by_prefix(&self, prefix: &str) -> usize {
        let mut guard = self.tools.write().expect("ToolRegistry poisoned");
        let before = guard.len();
        guard.retain(|name, _| !name.starts_with(prefix));
        before - guard.len()
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        let guard = self.tools.read().expect("ToolRegistry poisoned");
        guard.get(name).cloned()
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let guard = self.tools.read().expect("ToolRegistry poisoned");
        guard.values().map(|t| t.to_definition()).collect()
    }

    pub fn is_empty(&self) -> bool {
        let guard = self.tools.read().expect("ToolRegistry poisoned");
        guard.is_empty()
    }

    pub fn len(&self) -> usize {
        let guard = self.tools.read().expect("ToolRegistry poisoned");
        guard.len()
    }

    /// Execute a registered tool by name.
    ///
    /// Returns [`FastClawError::ToolNotFound`] when the name is missing.
    pub async fn execute_named(&self, name: &str, arguments: &str) -> FastClawResult<ToolResult> {
        let tool = {
            let guard = self.tools.read().expect("ToolRegistry poisoned");
            guard
                .get(name)
                .cloned()
                .ok_or_else(|| FastClawError::ToolNotFound(name.to_string()))?
        };
        Ok(tool.execute(arguments).await)
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
