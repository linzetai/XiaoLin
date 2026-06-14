use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use xiaolin_core::tool::{Tool, ToolParameterSchema, ToolResult};
use xiaolin_core::workspace::AgentWorkspace;
use serde_json::json;

pub struct GetIdentityTool {
    workspace: Arc<AgentWorkspace>,
}

impl GetIdentityTool {
    pub fn new(workspace: Arc<AgentWorkspace>) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for GetIdentityTool {
    fn name(&self) -> &str {
        "get_identity"
    }

    fn description(&self) -> &str {
        "Read agent workspace identity files: IDENTITY.md (persona, name, style, operating rules) \
         and USER.md (user profile). \
         file must be identity | user | all; all returns both in one JSON object. \
         Missing files surface as \"(empty)\" — normal for new workspaces. \
         Example: {\"file\": \"identity\"}; {\"file\": \"all\"}"
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "file".to_string(),
            json!({
                "type": "string",
                "enum": ["identity", "user", "all"],
                "description": "identity|user loads one file; all returns JSON with both."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["file".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "get_identity: arguments are not valid JSON: {e}. \
                 Pass e.g. {{\"file\": \"all\"}} or {{\"file\": \"identity\"}}, then retry."
            )),
        };
        let file = args.get("file").and_then(|v| v.as_str()).unwrap_or("all");

        let read = |filename: &str| -> String {
            let path = self.workspace.root.join(filename);
            std::fs::read_to_string(&path)
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "(empty)".to_string())
        };

        let mut result = serde_json::Map::new();
        match file {
            "identity" => {
                result.insert("identity".into(), json!(read("IDENTITY.md")));
            }
            "user" => {
                result.insert("user".into(), json!(read("USER.md")));
            }
            // Backward compatibility: old file names map to identity
            "soul" | "agents" => {
                result.insert("identity".into(), json!(read("IDENTITY.md")));
            }
            _ => {
                result.insert("identity".into(), json!(read("IDENTITY.md")));
                result.insert("user".into(), json!(read("USER.md")));
            }
        }

        ToolResult::ok(serde_json::to_string(&result).unwrap_or_default())
    }
}

pub struct SetIdentityTool {
    workspace: Arc<AgentWorkspace>,
}

impl SetIdentityTool {
    pub fn new(workspace: Arc<AgentWorkspace>) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for SetIdentityTool {
    fn name(&self) -> &str {
        "set_identity"
    }

    fn description(&self) -> &str {
        "Overwrite one identity Markdown in the workspace (identity or user)."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "file".to_string(),
            json!({"type": "string", "enum": ["identity", "user"]}),
        );
        props.insert("content".to_string(), json!({"type": "string"}));
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["file".to_string(), "content".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("set_identity: invalid JSON: {e}")),
        };

        let file = match args.get("file").and_then(|v| v.as_str()) {
            Some(f) => f,
            None => {
                return ToolResult::err(
                    "set_identity requires 'file': identity or user.".to_string(),
                )
            }
        };
        let content = match args.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolResult::err("set_identity requires 'content'.".to_string()),
        };

        let filename = match file {
            "identity" | "soul" | "agents" => "IDENTITY.md",
            "user" => "USER.md",
            other => {
                return ToolResult::err(format!(
                    "Unknown file '{other}'. Use identity or user."
                ))
            }
        };

        match self.workspace.write_file(filename, content) {
            Ok(()) => {
                tracing::info!(agent_id = %self.workspace.agent_id, file = filename, size = content.len(), "identity updated");
                ToolResult::ok(format!("{filename} updated ({} bytes).", content.len()))
            }
            Err(e) => ToolResult::err(format!("Could not write '{filename}': {e}")),
        }
    }
}

// --- Unified Identity Tool ---

pub struct UnifiedIdentityTool {
    get: GetIdentityTool,
    set: SetIdentityTool,
}

impl UnifiedIdentityTool {
    pub fn new(workspace: Arc<AgentWorkspace>) -> Self {
        Self {
            get: GetIdentityTool::new(workspace.clone()),
            set: SetIdentityTool::new(workspace),
        }
    }
}

#[async_trait]
impl Tool for UnifiedIdentityTool {
    fn name(&self) -> &str {
        "identity"
    }

    fn description(&self) -> &str {
        "Read or write agent identity files (IDENTITY.md, USER.md). \
         action 'get': read identity files (file: identity|user|all). \
         action 'set': overwrite one identity file (file + content required). \
         Always get before set to avoid clobbering existing content. \
         NOTE: Identity files are already injected into your context as <user_provided_context> at session start. \
         Do NOT call get just to answer 'who are you' — the content is already available. \
         MUST use set when the user changes your name, personality, vibe, or any identity attribute."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "action".to_string(),
            json!({
                "type": "string",
                "enum": ["get", "set"],
                "description": "get: read identity files; set: overwrite one identity file."
            }),
        );
        props.insert(
            "file".to_string(),
            json!({
                "type": "string",
                "enum": ["identity", "user", "all"],
                "description": "Which file. 'all' only valid for get."
            }),
        );
        props.insert(
            "content".to_string(),
            json!({
                "type": "string",
                "description": "Full replacement Markdown (required for set)."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["action".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("identity: invalid JSON: {e}")),
        };

        let action = match args.get("action").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => {
                return ToolResult::err("identity requires 'action': 'get' or 'set'.".to_string())
            }
        };

        match action {
            "get" => {
                let inner =
                    json!({"file": args.get("file").and_then(|v| v.as_str()).unwrap_or("all")})
                        .to_string();
                self.get.execute(&inner).await
            }
            "set" => self.set.execute(arguments).await,
            other => ToolResult::err(format!(
                "identity: unknown action '{other}'. Use 'get' or 'set'."
            )),
        }
    }
}
