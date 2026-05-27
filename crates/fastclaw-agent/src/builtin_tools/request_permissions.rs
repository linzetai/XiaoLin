//! `request_permissions` tool: allows the agent to request expanded file system
//! or network access from the user at runtime.

use std::collections::HashMap;

use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolGroup, ToolKind, ToolParameterSchema, ToolResult};

pub struct RequestPermissionsTool;

#[async_trait]
impl Tool for RequestPermissionsTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }

    fn name(&self) -> &str {
        "request_permissions"
    }

    fn description(&self) -> &str {
        "Request additional permissions from the user. Use when the current file access \
         or network policy blocks an operation you need to perform. Provide a clear reason \
         and the specific permissions needed. The user will be prompted to approve or deny."
    }

    fn group(&self) -> ToolGroup {
        ToolGroup::Utility
    }

    fn is_deferred(&self) -> bool {
        true
    }

    fn search_hint(&self) -> &str {
        "permission access grant allow file network elevate"
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "reason".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Clear explanation of why additional permissions are needed."
            }),
        );
        props.insert(
            "permissions".to_string(),
            serde_json::json!({
                "type": "object",
                "description": "Permissions to request.",
                "properties": {
                    "network": {
                        "type": "boolean",
                        "description": "Request unrestricted network access."
                    },
                    "file_system": {
                        "type": "object",
                        "description": "File system access expansion.",
                        "properties": {
                            "read": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Additional paths to request read access for."
                            },
                            "write": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Additional paths to request write access for."
                            }
                        }
                    }
                }
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["reason".to_string(), "permissions".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("Invalid JSON: {e}")),
        };

        let reason = args
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("No reason provided");

        let permissions = args.get("permissions").cloned().unwrap_or_default();

        // Format the permission request for user presentation.
        // Actual approval is handled by the orchestrator / interaction handle;
        // this tool's execute just formats the request summary.
        let mut summary = format!("Permission request: {reason}\n\nRequested permissions:\n");

        if let Some(true) = permissions.get("network").and_then(|v| v.as_bool()) {
            summary.push_str("  - Network: unrestricted access\n");
        }

        if let Some(fs) = permissions.get("file_system") {
            if let Some(read_paths) = fs.get("read").and_then(|v| v.as_array()) {
                for p in read_paths {
                    if let Some(path) = p.as_str() {
                        summary.push_str(&format!("  - File read: {path}\n"));
                    }
                }
            }
            if let Some(write_paths) = fs.get("write").and_then(|v| v.as_array()) {
                for p in write_paths {
                    if let Some(path) = p.as_str() {
                        summary.push_str(&format!("  - File write: {path}\n"));
                    }
                }
            }
        }

        summary.push_str(
            "\nNote: This request requires user approval via the interaction handle. \
             The orchestrator will present this to the user for confirmation.",
        );

        ToolResult::ok(summary)
    }
}
