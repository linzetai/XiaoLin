use crate::client::FeishuClient;
use crate::oauth::OAuthConfig;
use async_trait::async_trait;
use xiaolin_core::tool::{Tool, ToolParameterSchema, ToolResult};
use std::collections::HashMap;
use std::sync::Arc;

/// Tool: feishu_doc_get_content — Get the content of a Feishu document.
pub struct FeishuDocGetContentTool {
    client: Arc<FeishuClient>,
}

impl FeishuDocGetContentTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuDocGetContentTool {
    fn name(&self) -> &str {
        "feishu_doc_get_content"
    }
    fn description(&self) -> &str {
        "Get the content of a Feishu document by document token."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "document_id".to_string(),
            serde_json::json!({"type": "string", "description": "Document token"}),
        );
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec!["document_id".into()],
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
        let document_id = match args.get("document_id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return ToolResult::err("document_id is required".to_string()),
        };
        let path = format!("/docx/v1/documents/{document_id}/raw_content");
        match self.client.user_get(&path).await {
            Ok(v) => match serde_json::to_string(&v) {
                Ok(s) => ToolResult::ok(s),
                Err(e) => ToolResult::err(format!("feishu_doc_get_content: serialize: {e}")),
            },
            Err(e) => ToolResult::err(format!("feishu_doc_get_content: {e}")),
        }
    }
}

/// Tool: feishu_doc_create — Create a new Feishu document.
pub struct FeishuDocCreateTool {
    client: Arc<FeishuClient>,
}

impl FeishuDocCreateTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuDocCreateTool {
    fn name(&self) -> &str {
        "feishu_doc_create"
    }
    fn description(&self) -> &str {
        "Create a new Feishu document with specified title and content."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "title".to_string(),
            serde_json::json!({"type": "string", "description": "Document title"}),
        );
        properties.insert(
            "folder_token".to_string(),
            serde_json::json!({"type": "string", "description": "Parent folder token (optional)"}),
        );
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec!["title".into()],
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
        let title = match args.get("title").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return ToolResult::err("title is required".to_string()),
        };
        let mut body = serde_json::json!({ "title": title });
        if let Some(ft) = args.get("folder_token").and_then(|v| v.as_str()) {
            if !ft.is_empty() {
                body["folder_token"] = serde_json::Value::String(ft.to_string());
            }
        }
        match self
            .client
            .user_post_json("/docx/v1/documents", &body)
            .await
        {
            Ok(v) => match serde_json::to_string(&v) {
                Ok(s) => ToolResult::ok(s),
                Err(e) => ToolResult::err(format!("feishu_doc_create: serialize: {e}")),
            },
            Err(e) => ToolResult::err(format!("feishu_doc_create: {e}")),
        }
    }
}

/// Unified action-based document tool combining read/create/write/block operations.
pub struct FeishuDocTool {
    client: Arc<FeishuClient>,
}

impl FeishuDocTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuDocTool {
    fn name(&self) -> &str {
        "feishu_doc"
    }
    fn description(&self) -> &str {
        "Feishu document operations. Actions: read (get document content), create (create new doc), \
         write (batch update blocks), list_blocks (list blocks in doc), get_block (get block by id), \
         update_block (update a block), delete_block (delete blocks)."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert("action".into(), serde_json::json!({"type": "string", "enum": ["read", "create", "write", "list_blocks", "get_block", "update_block", "delete_block"]}));
        properties.insert(
            "document_id".into(),
            serde_json::json!({"type": "string", "description": "Document token"}),
        );
        properties.insert(
            "title".into(),
            serde_json::json!({"type": "string", "description": "Document title (for create)"}),
        );
        properties.insert("folder_token".into(), serde_json::json!({"type": "string", "description": "Parent folder token (for create)"}));
        properties.insert(
            "block_id".into(),
            serde_json::json!({"type": "string", "description": "Block ID"}),
        );
        properties.insert("body".into(), serde_json::json!({"type": "object", "description": "Request body for write/update_block operations"}));
        properties.insert(
            "start_index".into(),
            serde_json::json!({"type": "integer", "description": "Start index for delete_block"}),
        );
        properties.insert(
            "end_index".into(),
            serde_json::json!({"type": "integer", "description": "End index for delete_block"}),
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
            "read" => {
                let document_id = match args.get("document_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("document_id is required for read".to_string()),
                };
                self.client
                    .user_get(&format!("/docx/v1/documents/{document_id}/raw_content"))
                    .await
            }
            "create" => {
                let title = match args.get("title").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("title is required for create".to_string()),
                };
                let mut body = serde_json::json!({ "title": title });
                if let Some(ft) = args
                    .get("folder_token")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    body["folder_token"] = serde_json::Value::String(ft.to_string());
                }
                self.client
                    .user_post_json("/docx/v1/documents", &body)
                    .await
            }
            "write" => {
                let document_id = match args.get("document_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("document_id is required for write".to_string()),
                };
                let body = match args.get("body") {
                    Some(v) if v.is_object() || v.is_array() => v,
                    _ => return ToolResult::err("body is required for write".to_string()),
                };
                self.client
                    .user_post_json(
                        &format!("/docx/v1/documents/{document_id}/blocks/batch_update"),
                        body,
                    )
                    .await
            }
            "list_blocks" => {
                let document_id = match args.get("document_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => {
                        return ToolResult::err(
                            "document_id is required for list_blocks".to_string(),
                        )
                    }
                };
                self.client
                    .user_get(&format!("/docx/v1/documents/{document_id}/blocks"))
                    .await
            }
            "get_block" => {
                let document_id = match args.get("document_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => {
                        return ToolResult::err("document_id is required for get_block".to_string())
                    }
                };
                let block_id = match args.get("block_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("block_id is required for get_block".to_string()),
                };
                self.client
                    .user_get(&format!(
                        "/docx/v1/documents/{document_id}/blocks/{block_id}"
                    ))
                    .await
            }
            "update_block" => {
                let document_id = match args.get("document_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => {
                        return ToolResult::err(
                            "document_id is required for update_block".to_string(),
                        )
                    }
                };
                let block_id = match args.get("block_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => {
                        return ToolResult::err("block_id is required for update_block".to_string())
                    }
                };
                let body = match args.get("body") {
                    Some(v) if v.is_object() => v,
                    _ => return ToolResult::err("body is required for update_block".to_string()),
                };
                self.client
                    .user_patch_json(
                        &format!("/docx/v1/documents/{document_id}/blocks/{block_id}"),
                        body,
                    )
                    .await
            }
            "delete_block" => {
                let document_id = match args.get("document_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => {
                        return ToolResult::err(
                            "document_id is required for delete_block".to_string(),
                        )
                    }
                };
                let start_index = match args.get("start_index").and_then(|v| v.as_i64()) {
                    Some(i) => i,
                    None => {
                        return ToolResult::err(
                            "start_index is required for delete_block".to_string(),
                        )
                    }
                };
                let end_index = match args.get("end_index").and_then(|v| v.as_i64()) {
                    Some(i) => i,
                    None => {
                        return ToolResult::err(
                            "end_index is required for delete_block".to_string(),
                        )
                    }
                };
                let body = serde_json::json!({
                    "start_index": start_index,
                    "end_index": end_index,
                });
                self.client
                    .user_delete_with_body(
                        &format!("/docx/v1/documents/{document_id}/blocks/batch_delete"),
                        &body,
                    )
                    .await
            }
            _ => return ToolResult::err(format!("unknown action: {action}")),
        };
        match result {
            Ok(v) => ToolResult::ok(serde_json::to_string(&v).unwrap_or_default()),
            Err(e) => ToolResult::err(format!("feishu_doc {action}: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_tool_names() {
        let client = Arc::new(FeishuClient::new("t", "s"));
        assert_eq!(
            FeishuDocGetContentTool::new(client.clone()).name(),
            "feishu_doc_get_content"
        );
        assert_eq!(
            FeishuDocCreateTool::new(client.clone()).name(),
            "feishu_doc_create"
        );
        assert_eq!(FeishuDocTool::new(client).name(), "feishu_doc");
    }

    #[tokio::test]
    async fn doc_get_without_oauth_returns_tool_error() {
        let client = Arc::new(FeishuClient::new("t", "s"));
        let tool = FeishuDocGetContentTool::new(client);
        let r = tool.execute(r#"{"document_id":"doccnxxx"}"#).await;
        assert!(!r.success);
        assert!(r.output.contains("userAccessToken") || r.output.contains("user OAuth"));
    }

    #[tokio::test]
    async fn doc_create_without_oauth_returns_tool_error() {
        let client = Arc::new(FeishuClient::new("t", "s"));
        let tool = FeishuDocCreateTool::new(client);
        let r = tool.execute(r#"{"title":"Hello"}"#).await;
        assert!(!r.success);
        assert!(r.output.contains("userAccessToken") || r.output.contains("user OAuth"));
    }

    #[tokio::test]
    async fn unified_doc_without_oauth_returns_error() {
        let client = Arc::new(FeishuClient::new("t", "s"));
        let tool = FeishuDocTool::new(client);
        let r = tool
            .execute(r#"{"action":"read","document_id":"doccnxxx"}"#)
            .await;
        assert!(!r.success);
    }
}
