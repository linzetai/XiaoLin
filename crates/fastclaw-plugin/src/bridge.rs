use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolRegistry, ToolResult};

use crate::manifest::PluginCapability;
use crate::registry::PluginRegistry as WasmPluginRegistry;

/// Wraps a single WASM plugin capability as a Tool for the agent's ToolRegistry.
///
/// Invocations are synchronous (WASM is single-threaded) but wrapped in
/// `spawn_blocking` by the async execute method to avoid blocking the
/// tokio runtime.
pub struct PluginTool {
    plugin_id: String,
    capability: PluginCapability,
    registry: Arc<WasmPluginRegistry>,
}

impl PluginTool {
    pub fn new(
        plugin_id: String,
        capability: PluginCapability,
        registry: Arc<WasmPluginRegistry>,
    ) -> Self {
        Self {
            plugin_id,
            capability,
            registry,
        }
    }
}

#[async_trait]
impl Tool for PluginTool {
    fn name(&self) -> &str {
        &self.capability.name
    }

    fn description(&self) -> &str {
        &self.capability.description
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        if let Some(schema) = &self.capability.parameters_schema {
            if let Ok(parsed) = serde_json::from_value::<ToolParameterSchema>(schema.clone()) {
                return parsed;
            }
        }
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: HashMap::new(),
            required: Vec::new(),
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let registry = self.registry.clone();
        let plugin_id = self.plugin_id.clone();
        let cap_name = self.capability.name.clone();
        let input = arguments.to_string();

        let result = tokio::task::spawn_blocking(move || {
            registry.invoke_by_name(&plugin_id, &cap_name, &input)
        })
        .await;

        match result {
            Ok(Ok(output)) => ToolResult::ok(output),
            Ok(Err(e)) => ToolResult::err(format!("plugin error: {e}")),
            Err(e) => ToolResult::err(format!("plugin task panicked: {e}")),
        }
    }
}

/// Register all capabilities from a WasmPluginRegistry into a ToolRegistry.
pub fn bridge_plugins(plugin_registry: &Arc<WasmPluginRegistry>, tool_registry: &mut ToolRegistry) {
    for manifest in plugin_registry.list_plugins() {
        for cap in &manifest.capabilities {
            let tool = PluginTool::new(manifest.id.clone(), cap.clone(), plugin_registry.clone());
            tool_registry.register(Arc::new(tool));
        }
    }
}
