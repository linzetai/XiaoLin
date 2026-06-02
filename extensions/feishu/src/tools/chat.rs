use crate::client::FeishuClient;
use crate::oauth::OAuthConfig;
use async_trait::async_trait;
use xiaolin_core::tool::{Tool, ToolParameterSchema, ToolResult};
use std::collections::HashMap;
use std::sync::Arc;

/// Action-based chat management tool matching OpenClaw's `feishu_chat`.
pub struct FeishuChatTool {
    client: Arc<FeishuClient>,
}

impl FeishuChatTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuChatTool {
    fn name(&self) -> &str {
        "feishu_chat"
    }
    fn description(&self) -> &str {
        "Feishu group chat management. Actions: info (get chat info), members (list chat members), \
         member_info (get a single member's info by user_id)."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "action".into(),
            serde_json::json!({"type": "string", "enum": ["info", "members", "member_info"]}),
        );
        properties.insert(
            "chat_id".into(),
            serde_json::json!({"type": "string", "description": "Chat ID"}),
        );
        properties.insert(
            "user_id".into(),
            serde_json::json!({"type": "string", "description": "User open_id (for member_info)"}),
        );
        properties.insert(
            "page_size".into(),
            serde_json::json!({"type": "integer", "description": "Page size for members (1-100)"}),
        );
        properties.insert(
            "page_token".into(),
            serde_json::json!({"type": "string", "description": "Pagination token"}),
        );
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec!["action".into(), "chat_id".into()],
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
        let chat_id = match args.get("chat_id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return ToolResult::err("chat_id is required".to_string()),
        };

        let result = match action {
            "info" => {
                let path = format!("/im/v1/chats/{chat_id}");
                self.client.user_get(&path).await
            }
            "members" => {
                let ps = args
                    .get("page_size")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(20)
                    .clamp(1, 100);
                let mut path =
                    format!("/im/v1/chats/{chat_id}/members?member_id_type=open_id&page_size={ps}");
                if let Some(pt) = args
                    .get("page_token")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    path.push_str(&format!("&page_token={pt}"));
                }
                self.client.user_get(&path).await
            }
            "member_info" => {
                let user_id = match args.get("user_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("user_id is required for member_info".to_string()),
                };
                let path = format!("/contact/v3/users/{user_id}?user_id_type=open_id");
                self.client.user_get(&path).await
            }
            _ => return ToolResult::err(format!("unknown action: {action}")),
        };
        match result {
            Ok(v) => ToolResult::ok(serde_json::to_string(&v).unwrap_or_default()),
            Err(e) => ToolResult::err(format!("feishu_chat {action}: {e}")),
        }
    }
}
