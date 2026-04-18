use crate::client::FeishuClient;
use crate::oauth::OAuthConfig;
use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolResult};
use std::collections::HashMap;
use std::sync::Arc;

/// Tool: feishu_bitable_list_records — List records from a Feishu Bitable.
pub struct FeishuBitableListRecordsTool {
    client: Arc<FeishuClient>,
}

impl FeishuBitableListRecordsTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuBitableListRecordsTool {
    fn name(&self) -> &str {
        "feishu_bitable_list_records"
    }
    fn description(&self) -> &str {
        "List records from a Feishu Bitable (multi-dimensional table). Supports filtering and pagination."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "app_token".to_string(),
            serde_json::json!({"type": "string", "description": "Bitable app token"}),
        );
        properties.insert(
            "table_id".to_string(),
            serde_json::json!({"type": "string", "description": "Table ID within the bitable"}),
        );
        properties.insert(
            "page_size".to_string(),
            serde_json::json!({"type": "integer", "default": 20}),
        );
        properties.insert(
            "filter".to_string(),
            serde_json::json!({"type": "string", "description": "Filter expression"}),
        );
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec!["app_token".into(), "table_id".into()],
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
        let app_token = match args.get("app_token").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return ToolResult::err("app_token is required".to_string()),
        };
        let table_id = match args.get("table_id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return ToolResult::err("table_id is required".to_string()),
        };
        let page_size = args
            .get("page_size")
            .and_then(|v| v.as_u64())
            .unwrap_or(20)
            .clamp(1, 500);
        let path = format!("/bitable/v1/apps/{app_token}/tables/{table_id}/records");
        let mut owned: Vec<(String, String)> =
            vec![("page_size".to_string(), page_size.to_string())];
        if let Some(f) = args.get("filter").and_then(|v| v.as_str()) {
            if !f.is_empty() {
                owned.push(("filter".to_string(), f.to_string()));
            }
        }
        let refs: Vec<(&str, &str)> = owned
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        match self.client.user_get_query(&path, &refs).await {
            Ok(v) => match serde_json::to_string(&v) {
                Ok(s) => ToolResult::ok(s),
                Err(e) => {
                    ToolResult::err(format!("feishu_bitable_list_records: serialize: {e}"))
                }
            },
            Err(e) => ToolResult::err(format!("feishu_bitable_list_records: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitable_tool_name() {
        let client = Arc::new(FeishuClient::new("t", "s"));
        let tool = FeishuBitableListRecordsTool::new(client);
        assert_eq!(tool.name(), "feishu_bitable_list_records");
    }

    #[tokio::test]
    async fn bitable_without_oauth_returns_tool_error() {
        let client = Arc::new(FeishuClient::new("t", "s"));
        let tool = FeishuBitableListRecordsTool::new(client);
        let r = tool
            .execute(r#"{"app_token":"a","table_id":"b"}"#)
            .await;
        assert!(!r.success);
        assert!(
            r.output.contains("userAccessToken") || r.output.contains("user OAuth"),
            "unexpected: {}",
            r.output
        );
    }
}
