use crate::client::FeishuClient;
use crate::oauth::OAuthConfig;
use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolResult};
use std::collections::HashMap;
use std::sync::Arc;

/// Tool: feishu_calendar_list_events — List upcoming calendar events.
pub struct FeishuCalendarListEventsTool {
    client: Arc<FeishuClient>,
}

impl FeishuCalendarListEventsTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuCalendarListEventsTool {
    fn name(&self) -> &str {
        "feishu_calendar_list_events"
    }
    fn description(&self) -> &str {
        "List upcoming events from the user's Feishu calendar within a time range."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "calendar_id".to_string(),
            serde_json::json!({"type": "string", "description": "Calendar ID (default: primary)"}),
        );
        properties.insert(
            "start_time".to_string(),
            serde_json::json!({"type": "string", "description": "Start time in ISO 8601 format"}),
        );
        properties.insert(
            "end_time".to_string(),
            serde_json::json!({"type": "string", "description": "End time in ISO 8601 format"}),
        );
        properties.insert("page_size".to_string(), serde_json::json!({"type": "integer", "description": "Number of events (default 20, max 50)", "default": 20}));
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec![],
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
        let calendar_id = args
            .get("calendar_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("primary");
        let page_size = args
            .get("page_size")
            .and_then(|v| v.as_u64())
            .unwrap_or(20)
            .clamp(1, 50);
        let path = format!("/calendar/v4/calendars/{calendar_id}/events");
        let mut owned: Vec<(String, String)> =
            vec![("page_size".to_string(), page_size.to_string())];
        if let Some(st) = args.get("start_time").and_then(|v| v.as_str()) {
            if !st.is_empty() {
                owned.push(("start_time".to_string(), st.to_string()));
            }
        }
        if let Some(et) = args.get("end_time").and_then(|v| v.as_str()) {
            if !et.is_empty() {
                owned.push(("end_time".to_string(), et.to_string()));
            }
        }
        let refs: Vec<(&str, &str)> = owned
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        match self.client.user_get_query(&path, &refs).await {
            Ok(v) => match serde_json::to_string(&v) {
                Ok(s) => ToolResult::ok(s),
                Err(e) => ToolResult::err(format!("feishu_calendar_list_events: serialize: {e}")),
            },
            Err(e) => ToolResult::err(format!("feishu_calendar_list_events: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_tool_name() {
        let client = Arc::new(FeishuClient::new("t", "s"));
        let tool = FeishuCalendarListEventsTool::new(client);
        assert_eq!(tool.name(), "feishu_calendar_list_events");
    }

    #[tokio::test]
    async fn calendar_without_oauth_returns_tool_error() {
        let client = Arc::new(FeishuClient::new("t", "s"));
        let tool = FeishuCalendarListEventsTool::new(client);
        let r = tool.execute("{}").await;
        assert!(!r.success);
        assert!(r.output.contains("userAccessToken") || r.output.contains("user OAuth"));
    }
}
