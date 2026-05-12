use crate::client::FeishuClient;
use crate::oauth::OAuthConfig;
use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolResult};
use std::collections::HashMap;
use std::sync::Arc;

/// Single action-based tool matching OpenClaw's `feishu_drive`.
pub struct FeishuDriveTool {
    client: Arc<FeishuClient>,
}

impl FeishuDriveTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuDriveTool {
    fn name(&self) -> &str {
        "feishu_drive"
    }
    fn description(&self) -> &str {
        "Feishu cloud storage operations. Actions: list (list files in folder), info (get file metadata), \
         create_folder, move (move file), delete, list_comments, add_comment, reply_comment."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert("action".into(), serde_json::json!({"type": "string", "enum": ["list", "info", "create_folder", "move", "delete", "list_comments", "add_comment", "reply_comment"]}));
        properties.insert("folder_token".into(), serde_json::json!({"type": "string", "description": "Folder token (for list, create_folder, move)"}));
        properties.insert(
            "file_token".into(),
            serde_json::json!({"type": "string", "description": "File or folder token"}),
        );
        properties.insert("type".into(), serde_json::json!({"type": "string", "enum": ["doc", "docx", "sheet", "bitable", "folder", "file", "mindnote", "shortcut"], "description": "File type"}));
        properties.insert(
            "name".into(),
            serde_json::json!({"type": "string", "description": "Folder name (for create_folder)"}),
        );
        properties.insert(
            "comment_id".into(),
            serde_json::json!({"type": "string", "description": "Comment ID (for reply_comment)"}),
        );
        properties.insert(
            "content".into(),
            serde_json::json!({"type": "string", "description": "Comment text content"}),
        );
        properties.insert(
            "page_size".into(),
            serde_json::json!({"type": "integer", "description": "Page size (1-100)"}),
        );
        properties.insert(
            "page_token".into(),
            serde_json::json!({"type": "string", "description": "Pagination token"}),
        );
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec!["action".into()],
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

        let result = match action {
            "list" => {
                let mut query = vec![];
                if let Some(ft) = args
                    .get("folder_token")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    query.push(("folder_token", ft));
                }
                self.client.user_get_query("/drive/v1/files", &query).await
            }
            "info" => {
                let file_token = match args.get("file_token").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("file_token is required for info".to_string()),
                };
                let file_type = match args.get("type").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("type is required for info".to_string()),
                };
                let body = serde_json::json!({
                    "request_docs": [{ "doc_token": file_token, "doc_type": file_type }]
                });
                self.client
                    .user_post_json("/drive/v1/metas/batch_query", &body)
                    .await
            }
            "create_folder" => {
                let name = match args.get("name").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("name is required for create_folder".to_string()),
                };
                let mut body = serde_json::json!({ "name": name });
                if let Some(ft) = args
                    .get("folder_token")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    body["folder_token"] = serde_json::Value::String(ft.to_string());
                }
                self.client
                    .user_post_json("/drive/v1/files/create_folder", &body)
                    .await
            }
            "move" => {
                let file_token = match args.get("file_token").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("file_token is required for move".to_string()),
                };
                let folder_token = match args.get("folder_token").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => {
                        return ToolResult::err(
                            "folder_token (target) is required for move".to_string(),
                        )
                    }
                };
                let file_type = args.get("type").and_then(|v| v.as_str()).unwrap_or("file");
                let body = serde_json::json!({ "type": file_type, "folder_token": folder_token });
                self.client
                    .user_post_json(&format!("/drive/v1/files/{file_token}/move"), &body)
                    .await
            }
            "delete" => {
                let file_token = match args.get("file_token").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("file_token is required for delete".to_string()),
                };
                let file_type = args.get("type").and_then(|v| v.as_str()).unwrap_or("file");
                let path = format!("/drive/v1/files/{file_token}?type={file_type}");
                self.client.user_delete(&path).await
            }
            "list_comments" => {
                let file_token = match args.get("file_token").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => {
                        return ToolResult::err(
                            "file_token is required for list_comments".to_string(),
                        )
                    }
                };
                let mut pairs: Vec<(&str, &str)> = vec![];
                let ps_str;
                if let Some(ps) = args.get("page_size").and_then(|v| v.as_u64()) {
                    ps_str = ps.clamp(1, 100).to_string();
                    pairs.push(("page_size", &ps_str));
                }
                if let Some(pt) = args.get("page_token").and_then(|v| v.as_str()) {
                    pairs.push(("page_token", pt));
                }
                self.client
                    .user_get_query(&format!("/drive/v1/files/{file_token}/comments"), &pairs)
                    .await
            }
            "add_comment" => {
                let file_token = match args.get("file_token").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => {
                        return ToolResult::err(
                            "file_token is required for add_comment".to_string(),
                        )
                    }
                };
                let content = match args.get("content").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("content is required for add_comment".to_string()),
                };
                let body = serde_json::json!({
                    "reply_list": { "replies": [{ "content": { "elements": [{ "type": "textRun", "textRun": { "text": content } }] } }] }
                });
                self.client
                    .user_post_json(&format!("/drive/v1/files/{file_token}/comments"), &body)
                    .await
            }
            "reply_comment" => {
                let file_token = match args.get("file_token").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => {
                        return ToolResult::err(
                            "file_token is required for reply_comment".to_string(),
                        )
                    }
                };
                let comment_id = match args.get("comment_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => {
                        return ToolResult::err(
                            "comment_id is required for reply_comment".to_string(),
                        )
                    }
                };
                let content = match args.get("content").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => {
                        return ToolResult::err("content is required for reply_comment".to_string())
                    }
                };
                let body = serde_json::json!({
                    "content": { "elements": [{ "type": "textRun", "textRun": { "text": content } }] }
                });
                self.client
                    .user_post_json(
                        &format!("/drive/v1/files/{file_token}/comments/{comment_id}/replies"),
                        &body,
                    )
                    .await
            }
            _ => return ToolResult::err(format!("unknown action: {action}")),
        };
        match result {
            Ok(v) => ToolResult::ok(serde_json::to_string(&v).unwrap_or_default()),
            Err(e) => ToolResult::err(format!("feishu_drive {action}: {e}")),
        }
    }
}
