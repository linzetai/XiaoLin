use crate::client::FeishuClient;
use crate::oauth::OAuthConfig;
use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolResult};
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
        match self.client.user_post_json("/docx/v1/documents", &body).await {
            Ok(v) => match serde_json::to_string(&v) {
                Ok(s) => ToolResult::ok(s),
                Err(e) => ToolResult::err(format!("feishu_doc_create: serialize: {e}")),
            },
            Err(e) => ToolResult::err(format!("feishu_doc_create: {e}")),
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
        assert_eq!(FeishuDocCreateTool::new(client).name(), "feishu_doc_create");
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
}
