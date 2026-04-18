use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolResult};

use crate::client::FeishuClient;

/// Tool: feishu_send_message — Send a text message to a Feishu user or group.
pub struct FeishuSendMessageTool {
    client: Arc<FeishuClient>,
}

impl FeishuSendMessageTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuSendMessageTool {
    fn name(&self) -> &str {
        "feishu_send_message"
    }

    fn description(&self) -> &str {
        "Send a text message to a Feishu (Lark) user or group chat. \
         Specify receive_id and receive_id_type (open_id, chat_id, user_id, union_id, email)."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "receive_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The ID of the recipient (user or chat)"
            }),
        );
        properties.insert(
            "receive_id_type".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["open_id", "chat_id", "user_id", "union_id", "email"],
                "description": "Type of receive_id"
            }),
        );
        properties.insert(
            "text".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Message text to send"
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties,
            required: vec![
                "receive_id".to_string(),
                "receive_id_type".to_string(),
                "text".to_string(),
            ],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("invalid arguments: {e}")),
        };

        let receive_id = args["receive_id"].as_str().unwrap_or("");
        let receive_id_type = args["receive_id_type"].as_str().unwrap_or("chat_id");
        let text = args["text"].as_str().unwrap_or("");

        if receive_id.is_empty() || text.is_empty() {
            return ToolResult::err("receive_id and text are required");
        }

        match self
            .client
            .send_message(receive_id, receive_id_type, text)
            .await
        {
            Ok(data) => ToolResult::ok(serde_json::to_string(&data).unwrap_or_default()),
            Err(e) => ToolResult::err(format!("feishu send failed: {e}")),
        }
    }
}

/// Tool: feishu_reply_message — Reply to a specific Feishu message.
pub struct FeishuReplyMessageTool {
    client: Arc<FeishuClient>,
}

impl FeishuReplyMessageTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuReplyMessageTool {
    fn name(&self) -> &str {
        "feishu_reply_message"
    }

    fn description(&self) -> &str {
        "Reply to a specific Feishu (Lark) message by message_id."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "message_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The message_id to reply to"
            }),
        );
        properties.insert(
            "text".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Reply text content"
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties,
            required: vec!["message_id".to_string(), "text".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("invalid arguments: {e}")),
        };

        let message_id = args["message_id"].as_str().unwrap_or("");
        let text = args["text"].as_str().unwrap_or("");

        if message_id.is_empty() || text.is_empty() {
            return ToolResult::err("message_id and text are required");
        }

        match self.client.reply_message(message_id, text).await {
            Ok(data) => ToolResult::ok(serde_json::to_string(&data).unwrap_or_default()),
            Err(e) => ToolResult::err(format!("feishu reply failed: {e}")),
        }
    }
}

/// Tool: feishu_get_chat_messages — Retrieve recent messages from a Feishu chat.
pub struct FeishuGetChatMessagesTool {
    client: Arc<FeishuClient>,
}

impl FeishuGetChatMessagesTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuGetChatMessagesTool {
    fn name(&self) -> &str {
        "feishu_get_chat_messages"
    }

    fn description(&self) -> &str {
        "Retrieve recent messages from a Feishu (Lark) group chat or DM by chat_id."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "chat_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The chat_id to fetch messages from"
            }),
        );
        properties.insert(
            "page_size".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Number of messages to retrieve (default 20, max 50)",
                "default": 20
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties,
            required: vec!["chat_id".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("invalid arguments: {e}")),
        };

        let chat_id = args["chat_id"].as_str().unwrap_or("");
        let page_size = args["page_size"].as_u64().unwrap_or(20).min(50);

        if chat_id.is_empty() {
            return ToolResult::err("chat_id is required");
        }

        match self
            .client
            .get_chat_messages(chat_id, page_size as u32)
            .await
        {
            Ok(data) => ToolResult::ok(serde_json::to_string(&data).unwrap_or_default()),
            Err(e) => ToolResult::err(format!("feishu get messages failed: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_tool_schema() {
        let client = Arc::new(FeishuClient::new("test", "test"));
        let tool = FeishuSendMessageTool::new(client);
        assert_eq!(tool.name(), "feishu_send_message");
        let schema = tool.parameters_schema();
        assert!(schema.required.contains(&"receive_id".to_string()));
        assert!(schema.required.contains(&"text".to_string()));
    }

    #[test]
    fn reply_tool_schema() {
        let client = Arc::new(FeishuClient::new("test", "test"));
        let tool = FeishuReplyMessageTool::new(client);
        assert_eq!(tool.name(), "feishu_reply_message");
    }

    #[test]
    fn get_messages_tool_schema() {
        let client = Arc::new(FeishuClient::new("test", "test"));
        let tool = FeishuGetChatMessagesTool::new(client);
        assert_eq!(tool.name(), "feishu_get_chat_messages");
        let schema = tool.parameters_schema();
        assert!(schema.required.contains(&"chat_id".to_string()));
    }
}
