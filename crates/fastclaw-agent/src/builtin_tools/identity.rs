use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolResult};
use fastclaw_core::workspace::AgentWorkspace;
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
        "Read the agent workspace identity Markdown: SOUL.md (voice, persona), USER.md (user profile to mirror), AGENTS.md (rules and guardrails). \
         Call get_identity before set_identity or before broad behavior shifts so you merge from current text instead of clobbering it. \
         file must be soul | user | agents | all; all returns all three bodies in one JSON object. Unknown strings fall back like all—still pass the documented enum to avoid surprises. \
         Missing files surface as \"(empty)\"—that is normal for new workspaces, not a failure. \
         For arbitrary repo paths or non-identity files, use read_file; this tool only reads those three canonical names under the workspace root. \
         Example: {\"file\": \"soul\"}; {\"file\": \"all\"} when you need persona + user + rules together."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "file".to_string(),
            json!({
                "type": "string",
                "enum": ["soul", "user", "agents", "all"],
                "description": "soul|user|agents loads one file; all returns JSON keys soul, user, agents. Lowercase strings only. Typos still return all three—prefer exact enum tokens."
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
                 Pass e.g. {{\"file\": \"all\"}} or {{\"file\": \"soul\"}} with a string enum, then retry."
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
            "soul" => {
                result.insert("soul".into(), json!(read("SOUL.md")));
            }
            "user" => {
                result.insert("user".into(), json!(read("USER.md")));
            }
            "agents" => {
                result.insert("agents".into(), json!(read("AGENTS.md")));
            }
            _ => {
                result.insert("soul".into(), json!(read("SOUL.md")));
                result.insert("user".into(), json!(read("USER.md")));
                result.insert("agents".into(), json!(read("AGENTS.md")));
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
        "Overwrite one identity Markdown in the workspace: soul→SOUL.md, user→USER.md, agents→AGENTS.md (persona, user mirror, operating rules). \
         Full-file replace—read with get_identity, merge offline mentally, then set_identity with the entire new document (same safety model as write_file). \
         Effects apply on later turns; they cannot alter messages already delivered. \
         Avoid embedding raw secrets unless the user demanded it—reference secret stores instead. \
         For general source files, use write_file; set_identity is only for the identity trio. \
         Anti-pattern: partial Markdown with ellipses implying unchanged sections—those sections are deleted. \
         Example: {\"file\": \"agents\", \"content\": \"# Rules\\n- Confirm before rm -rf.\\n\"}."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "file".to_string(),
            json!({
                "type": "string",
                "enum": ["soul", "user", "agents"],
                "description": "Exactly one of soul, user, agents (maps to SOUL.md, USER.md, AGENTS.md). No 'all' on write—issue separate calls per file. Use get_identity first if unsure which to edit."
            }),
        );
        props.insert(
            "content".to_string(),
            json!({
                "type": "string",
                "description": "Full replacement Markdown as one JSON string (use \\n for newlines). Example: \"# User\\n- Prefers concise answers\\n\". Include every section you intend to keep."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["file".to_string(), "content".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "set_identity: arguments are not valid JSON: {e}. \
                 Pass {{\"file\": \"soul\"|\"user\"|\"agents\", \"content\": \"...\"}} with double-quoted keys, then retry."
            )),
        };

        let file = match args.get("file").and_then(|v| v.as_str()) {
            Some(f) => f,
            None => return ToolResult::err(
                "set_identity is missing required string field 'file'. \
                 Example: {\"file\": \"user\", \"content\": \"# Profile\\n...\"}. \
                 Allowed values: soul, user, agents."
                    .to_string(),
            ),
        };
        let content = match args.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolResult::err(
                "set_identity is missing required string field 'content'. \
                 Send the full Markdown document as one JSON string after merging with get_identity output."
                    .to_string(),
            ),
        };

        let filename = match file {
            "soul" => "SOUL.md",
            "user" => "USER.md",
            "agents" => "AGENTS.md",
            other => {
                return ToolResult::err(format!(
                    "set_identity: unknown file value '{other}'. \
                     Use exactly 'soul', 'user', or 'agents' (lowercase). \
                     To read all three, use get_identity with file 'all'—set_identity does not support 'all'."
                ))
            }
        };

        match self.workspace.write_file(filename, content) {
            Ok(()) => {
                tracing::info!(
                    agent_id = %self.workspace.agent_id,
                    file = filename,
                    size = content.len(),
                    "identity file updated via tool"
                );
                ToolResult::ok(format!(
                    "{filename} updated ({} bytes). Changes take effect on the next message.",
                    content.len()
                ))
            }
            Err(e) => ToolResult::err(format!(
                "set_identity could not write '{filename}' to the workspace: {e}. \
                 What to do next: verify the workspace root is writable, disk is not full, and the path is not read-only; retry after fixing permissions or choosing a different workspace."
            )),
        }
    }
}
