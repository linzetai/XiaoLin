use crate::client::FeishuClient;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use xiaolin_core::tool::{Tool, ToolParameterSchema, ToolResult};

// ---------------------------------------------------------------------------
// feishu_send_rich_text
// ---------------------------------------------------------------------------

pub struct FeishuSendRichTextTool {
    client: Arc<FeishuClient>,
}

impl FeishuSendRichTextTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuSendRichTextTool {
    fn name(&self) -> &str {
        "feishu_send_rich_text"
    }
    fn description(&self) -> &str {
        "Send a rich text (post) message to Feishu with structured content including titles, \
         paragraphs, at-mentions, links, and inline images. The content should be a JSON object \
         with locale keys (e.g. zh_cn) containing title and content arrays."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "receive_id".into(),
            serde_json::json!({"type": "string", "description": "Recipient ID (user or chat)"}),
        );
        properties.insert(
            "receive_id_type".into(),
            serde_json::json!({"type": "string", "enum": ["open_id", "chat_id", "user_id", "union_id", "email"], "description": "Type of receive_id"}),
        );
        properties.insert(
            "content".into(),
            serde_json::json!({"type": "object", "description": "Rich text content object with locale keys (zh_cn/en_us), each containing title and content array"}),
        );
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec![
                "receive_id".into(),
                "receive_id_type".into(),
                "content".into(),
            ],
        }
    }
    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("invalid args: {e}")),
        };
        let receive_id = match args.get("receive_id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return ToolResult::err("receive_id is required".to_string()),
        };
        let receive_id_type = args
            .get("receive_id_type")
            .and_then(|v| v.as_str())
            .unwrap_or("chat_id");
        let content = match args.get("content") {
            Some(v) if v.is_object() => v,
            _ => return ToolResult::err("content must be a JSON object".to_string()),
        };
        match self
            .client
            .send_rich_text(receive_id, receive_id_type, content)
            .await
        {
            Ok(v) => ToolResult::ok(serde_json::to_string(&v).unwrap_or_default()),
            Err(e) => ToolResult::err(format!("feishu_send_rich_text: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// feishu_send_file
// ---------------------------------------------------------------------------

pub struct FeishuSendFileTool {
    client: Arc<FeishuClient>,
}

impl FeishuSendFileTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuSendFileTool {
    fn name(&self) -> &str {
        "feishu_send_file"
    }
    fn description(&self) -> &str {
        "Upload and send a file to a Feishu user or group chat. Provide file_url to download \
         and send. Supported file types: opus, mp4, pdf, doc, xls, ppt, stream. Max 30MB."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "receive_id".into(),
            serde_json::json!({"type": "string", "description": "Recipient ID"}),
        );
        properties.insert(
            "receive_id_type".into(),
            serde_json::json!({"type": "string", "enum": ["open_id", "chat_id", "user_id", "union_id", "email"]}),
        );
        properties.insert(
            "file_url".into(),
            serde_json::json!({"type": "string", "description": "URL of the file to download and send"}),
        );
        properties.insert(
            "file_name".into(),
            serde_json::json!({"type": "string", "description": "File name with extension (e.g. report.pdf)"}),
        );
        properties.insert(
            "file_type".into(),
            serde_json::json!({"type": "string", "enum": ["opus", "mp4", "pdf", "doc", "xls", "ppt", "stream"], "description": "File type category"}),
        );
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec![
                "receive_id".into(),
                "receive_id_type".into(),
                "file_url".into(),
                "file_name".into(),
                "file_type".into(),
            ],
        }
    }
    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("invalid args: {e}")),
        };
        let receive_id = match args.get("receive_id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return ToolResult::err("receive_id is required".to_string()),
        };
        let receive_id_type = args
            .get("receive_id_type")
            .and_then(|v| v.as_str())
            .unwrap_or("chat_id");
        let file_url = match args.get("file_url").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return ToolResult::err("file_url is required".to_string()),
        };
        let file_name = match args.get("file_name").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return ToolResult::err("file_name is required".to_string()),
        };
        let file_type = match args.get("file_type").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return ToolResult::err("file_type is required".to_string()),
        };

        let data = match reqwest::get(file_url).await {
            Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                Ok(b) => b.to_vec(),
                Err(e) => return ToolResult::err(format!("failed to read file bytes: {e}")),
            },
            Ok(resp) => {
                return ToolResult::err(format!("file download failed: HTTP {}", resp.status()))
            }
            Err(e) => return ToolResult::err(format!("failed to download file: {e}")),
        };

        match self.client.upload_file(file_type, &data, file_name).await {
            Ok(file_key) => match self
                .client
                .send_file(receive_id, receive_id_type, &file_key)
                .await
            {
                Ok(result) => ToolResult::ok(
                    serde_json::json!({
                        "success": true,
                        "file_key": file_key,
                        "result": result,
                    })
                    .to_string(),
                ),
                Err(e) => ToolResult::err(format!("send file failed: {e}")),
            },
            Err(e) => ToolResult::err(format!("upload failed: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// feishu_edit_message
// ---------------------------------------------------------------------------

pub struct FeishuEditMessageTool {
    client: Arc<FeishuClient>,
}

impl FeishuEditMessageTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuEditMessageTool {
    fn name(&self) -> &str {
        "feishu_edit_message"
    }
    fn description(&self) -> &str {
        "Edit an existing Feishu message in-place. Only text and post messages can be edited."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "message_id".into(),
            serde_json::json!({"type": "string", "description": "Message ID to edit"}),
        );
        properties.insert(
            "content".into(),
            serde_json::json!({"type": "string", "description": "New message content (JSON string matching msg_type format)"}),
        );
        properties.insert(
            "msg_type".into(),
            serde_json::json!({"type": "string", "enum": ["text", "post"], "description": "Message type (default: text)"}),
        );
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec!["message_id".into(), "content".into()],
        }
    }
    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("invalid args: {e}")),
        };
        let message_id = match args.get("message_id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return ToolResult::err("message_id is required".to_string()),
        };
        let content = match args.get("content").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return ToolResult::err("content is required".to_string()),
        };
        let msg_type = args
            .get("msg_type")
            .and_then(|v| v.as_str())
            .unwrap_or("text");
        match self
            .client
            .edit_message(message_id, msg_type, content)
            .await
        {
            Ok(v) => ToolResult::ok(serde_json::to_string(&v).unwrap_or_default()),
            Err(e) => ToolResult::err(format!("feishu_edit_message: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// feishu_get_message
// ---------------------------------------------------------------------------

pub struct FeishuGetMessageTool {
    client: Arc<FeishuClient>,
}

impl FeishuGetMessageTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuGetMessageTool {
    fn name(&self) -> &str {
        "feishu_get_message"
    }
    fn description(&self) -> &str {
        "Get a single Feishu message by its message_id. Useful for fetching quoted or replied message content."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "message_id".into(),
            serde_json::json!({"type": "string", "description": "Message ID to fetch"}),
        );
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec!["message_id".into()],
        }
    }
    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("invalid args: {e}")),
        };
        let message_id = match args.get("message_id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return ToolResult::err("message_id is required".to_string()),
        };
        match self.client.get_message(message_id).await {
            Ok(v) => ToolResult::ok(serde_json::to_string(&v).unwrap_or_default()),
            Err(e) => ToolResult::err(format!("feishu_get_message: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// feishu_forward_message
// ---------------------------------------------------------------------------

pub struct FeishuForwardMessageTool {
    client: Arc<FeishuClient>,
}

impl FeishuForwardMessageTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuForwardMessageTool {
    fn name(&self) -> &str {
        "feishu_forward_message"
    }
    fn description(&self) -> &str {
        "Forward a Feishu message to another user or chat."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "message_id".into(),
            serde_json::json!({"type": "string", "description": "Message ID to forward"}),
        );
        properties.insert(
            "receive_id".into(),
            serde_json::json!({"type": "string", "description": "Target recipient ID"}),
        );
        properties.insert(
            "receive_id_type".into(),
            serde_json::json!({"type": "string", "enum": ["open_id", "chat_id", "user_id", "union_id", "email"]}),
        );
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec![
                "message_id".into(),
                "receive_id".into(),
                "receive_id_type".into(),
            ],
        }
    }
    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("invalid args: {e}")),
        };
        let message_id = match args.get("message_id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return ToolResult::err("message_id is required".to_string()),
        };
        let receive_id = match args.get("receive_id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return ToolResult::err("receive_id is required".to_string()),
        };
        let receive_id_type = args
            .get("receive_id_type")
            .and_then(|v| v.as_str())
            .unwrap_or("chat_id");
        match self
            .client
            .forward_message(message_id, receive_id, receive_id_type)
            .await
        {
            Ok(v) => ToolResult::ok(serde_json::to_string(&v).unwrap_or_default()),
            Err(e) => ToolResult::err(format!("feishu_forward_message: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// feishu_delete_message
// ---------------------------------------------------------------------------

pub struct FeishuDeleteMessageTool {
    client: Arc<FeishuClient>,
}

impl FeishuDeleteMessageTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuDeleteMessageTool {
    fn name(&self) -> &str {
        "feishu_delete_message"
    }
    fn description(&self) -> &str {
        "Delete (recall) a Feishu message. Only messages sent by the bot can be deleted."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "message_id".into(),
            serde_json::json!({"type": "string", "description": "Message ID to delete"}),
        );
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec!["message_id".into()],
        }
    }
    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("invalid args: {e}")),
        };
        let message_id = match args.get("message_id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return ToolResult::err("message_id is required".to_string()),
        };
        match self.client.delete_message(message_id).await {
            Ok(_) => ToolResult::ok(r#"{"success": true}"#.to_string()),
            Err(e) => ToolResult::err(format!("feishu_delete_message: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// feishu_reaction (action-based: add, remove, list)
// ---------------------------------------------------------------------------

pub struct FeishuReactionTool {
    client: Arc<FeishuClient>,
}

impl FeishuReactionTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuReactionTool {
    fn name(&self) -> &str {
        "feishu_reaction"
    }
    fn description(&self) -> &str {
        "Manage emoji reactions on Feishu messages. Actions: add, remove, list."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "action".into(),
            serde_json::json!({"type": "string", "enum": ["add", "remove", "list"], "description": "Action to perform"}),
        );
        properties.insert(
            "message_id".into(),
            serde_json::json!({"type": "string", "description": "Message ID"}),
        );
        properties.insert(
            "emoji_type".into(),
            serde_json::json!({"type": "string", "description": "Emoji type (e.g. SMILE, THUMBSUP, HEART). Required for add, optional filter for list."}),
        );
        properties.insert(
            "reaction_id".into(),
            serde_json::json!({"type": "string", "description": "Reaction ID (required for remove)"}),
        );
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec!["action".into(), "message_id".into()],
        }
    }
    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("invalid args: {e}")),
        };
        let action = match args.get("action").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolResult::err("action is required".to_string()),
        };
        let message_id = match args.get("message_id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return ToolResult::err("message_id is required".to_string()),
        };

        match action {
            "add" => {
                let emoji_type = match args.get("emoji_type").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("emoji_type is required for add".to_string()),
                };
                match self.client.add_reaction(message_id, emoji_type).await {
                    Ok(v) => ToolResult::ok(serde_json::to_string(&v).unwrap_or_default()),
                    Err(e) => ToolResult::err(format!("feishu_reaction add: {e}")),
                }
            }
            "remove" => {
                let reaction_id = match args.get("reaction_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("reaction_id is required for remove".to_string()),
                };
                match self.client.remove_reaction(message_id, reaction_id).await {
                    Ok(v) => ToolResult::ok(serde_json::to_string(&v).unwrap_or_default()),
                    Err(e) => ToolResult::err(format!("feishu_reaction remove: {e}")),
                }
            }
            "list" => {
                let emoji_type = args.get("emoji_type").and_then(|v| v.as_str());
                match self.client.list_reactions(message_id, emoji_type).await {
                    Ok(v) => ToolResult::ok(serde_json::to_string(&v).unwrap_or_default()),
                    Err(e) => ToolResult::err(format!("feishu_reaction list: {e}")),
                }
            }
            _ => ToolResult::err(format!("unknown action: {action}")),
        }
    }
}

// ---------------------------------------------------------------------------
// feishu_pin (action-based: create, remove, list)
// ---------------------------------------------------------------------------

pub struct FeishuPinTool {
    client: Arc<FeishuClient>,
}

impl FeishuPinTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuPinTool {
    fn name(&self) -> &str {
        "feishu_pin"
    }
    fn description(&self) -> &str {
        "Manage pinned messages in Feishu chats. Actions: create (pin a message), \
         remove (unpin), list (list pinned messages in a chat)."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "action".into(),
            serde_json::json!({"type": "string", "enum": ["create", "remove", "list"], "description": "Action to perform"}),
        );
        properties.insert(
            "message_id".into(),
            serde_json::json!({"type": "string", "description": "Message ID (required for create/remove)"}),
        );
        properties.insert(
            "chat_id".into(),
            serde_json::json!({"type": "string", "description": "Chat ID (required for list)"}),
        );
        properties.insert(
            "page_size".into(),
            serde_json::json!({"type": "integer", "description": "Page size for list (1-100, default 20)"}),
        );
        properties.insert(
            "page_token".into(),
            serde_json::json!({"type": "string", "description": "Pagination token for list"}),
        );
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec!["action".into()],
        }
    }
    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("invalid args: {e}")),
        };
        let action = match args.get("action").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolResult::err("action is required".to_string()),
        };

        match action {
            "create" => {
                let message_id = match args.get("message_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("message_id is required for create".to_string()),
                };
                match self.client.create_pin(message_id).await {
                    Ok(v) => ToolResult::ok(serde_json::to_string(&v).unwrap_or_default()),
                    Err(e) => ToolResult::err(format!("feishu_pin create: {e}")),
                }
            }
            "remove" => {
                let message_id = match args.get("message_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("message_id is required for remove".to_string()),
                };
                match self.client.remove_pin(message_id).await {
                    Ok(_) => ToolResult::ok(r#"{"success": true}"#.to_string()),
                    Err(e) => ToolResult::err(format!("feishu_pin remove: {e}")),
                }
            }
            "list" => {
                let chat_id = match args.get("chat_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("chat_id is required for list".to_string()),
                };
                let page_size = args
                    .get("page_size")
                    .and_then(|v| v.as_u64())
                    .map(|v| v.clamp(1, 100) as u32);
                let page_token = args.get("page_token").and_then(|v| v.as_str());
                match self.client.list_pins(chat_id, page_size, page_token).await {
                    Ok(v) => ToolResult::ok(serde_json::to_string(&v).unwrap_or_default()),
                    Err(e) => ToolResult::err(format!("feishu_pin list: {e}")),
                }
            }
            _ => ToolResult::err(format!("unknown action: {action}")),
        }
    }
}
