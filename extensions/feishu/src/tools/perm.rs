use crate::client::FeishuClient;
use crate::oauth::OAuthConfig;
use async_trait::async_trait;
use xiaolin_core::tool::{Tool, ToolParameterSchema, ToolResult};
use std::collections::HashMap;
use std::sync::Arc;

/// Action-based permission management tool matching OpenClaw's `feishu_perm`.
pub struct FeishuPermTool {
    client: Arc<FeishuClient>,
}

impl FeishuPermTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuPermTool {
    fn name(&self) -> &str {
        "feishu_perm"
    }
    fn description(&self) -> &str {
        "Manage permissions on Feishu documents. Actions: list (list collaborators), \
         add (grant access), remove (revoke access)."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "action".into(),
            serde_json::json!({"type": "string", "enum": ["list", "add", "remove"]}),
        );
        properties.insert("type".into(), serde_json::json!({"type": "string", "enum": ["doc", "docx", "sheet", "bitable", "folder", "file", "wiki"], "description": "Document type"}));
        properties.insert(
            "token".into(),
            serde_json::json!({"type": "string", "description": "Document/folder token"}),
        );
        properties.insert("member_type".into(), serde_json::json!({"type": "string", "enum": ["email", "openid", "userid", "unionid", "openchat", "opendepartmentid", "departmentid"], "description": "Member identifier type"}));
        properties.insert(
            "member_id".into(),
            serde_json::json!({"type": "string", "description": "Member identifier value"}),
        );
        properties.insert("perm".into(), serde_json::json!({"type": "string", "enum": ["view", "edit", "full_access"], "description": "Permission level (for add)"}));
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec!["action".into(), "type".into(), "token".into()],
        }
    }
    async fn execute(&self, arguments: &str) -> ToolResult {
        if !self.client.user_oauth_configured() {
            return ToolResult::err(OAuthConfig::missing_user_token_message().to_string());
        }
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("invalid args: {e}")),
        };
        let action = match args.get("action").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolResult::err("action is required".to_string()),
        };
        let doc_type = match args.get("type").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return ToolResult::err("type is required".to_string()),
        };
        let token = match args.get("token").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return ToolResult::err("token is required".to_string()),
        };

        let result = match action {
            "list" => {
                let path = format!("/drive/v1/permissions/{token}/members?type={doc_type}");
                self.client.user_get(&path).await
            }
            "add" => {
                let member_type = match args.get("member_type").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("member_type is required for add".to_string()),
                };
                let member_id = match args.get("member_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("member_id is required for add".to_string()),
                };
                let perm = args.get("perm").and_then(|v| v.as_str()).unwrap_or("view");
                let body = serde_json::json!({
                    "member_type": member_type,
                    "member_id": member_id,
                    "perm": perm,
                });
                self.client
                    .user_post_json(
                        &format!("/drive/v1/permissions/{token}/members?type={doc_type}"),
                        &body,
                    )
                    .await
            }
            "remove" => {
                let member_type = match args.get("member_type").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("member_type is required for remove".to_string()),
                };
                let member_id = match args.get("member_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("member_id is required for remove".to_string()),
                };
                let path = format!(
                    "/drive/v1/permissions/{token}/members/{member_id}?type={doc_type}&member_type={member_type}"
                );
                self.client.user_delete(&path).await
            }
            _ => return ToolResult::err(format!("unknown action: {action}")),
        };
        match result {
            Ok(v) => ToolResult::ok(serde_json::to_string(&v).unwrap_or_default()),
            Err(e) => ToolResult::err(format!("feishu_perm {action}: {e}")),
        }
    }
}
