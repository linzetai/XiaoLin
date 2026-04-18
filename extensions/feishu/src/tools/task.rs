use crate::client::FeishuClient;
use crate::oauth::OAuthConfig;
use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolResult};
use std::collections::HashMap;
use std::sync::Arc;

fn missing_oauth_tool_result() -> ToolResult {
    ToolResult::err(OAuthConfig::missing_user_token_message().to_string())
}

/// Tool: feishu_task_create — Create a Feishu task.
pub struct FeishuTaskCreateTool {
    client: Arc<FeishuClient>,
}

impl FeishuTaskCreateTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuTaskCreateTool {
    fn name(&self) -> &str {
        "feishu_task_create"
    }
    fn description(&self) -> &str {
        "Create a new task in Feishu Tasks with summary, due date, and assignees."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "summary".to_string(),
            serde_json::json!({"type": "string", "description": "Task summary/title"}),
        );
        properties.insert(
            "description".to_string(),
            serde_json::json!({"type": "string", "description": "Optional task description"}),
        );
        properties.insert(
            "due".to_string(),
            serde_json::json!({"type": "string", "description": "Due date in ISO 8601 format"}),
        );
        properties.insert("assignees".to_string(), serde_json::json!({"type": "array", "items": {"type": "string"}, "description": "Assignee open_ids"}));
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec!["summary".into()],
        }
    }
    async fn execute(&self, arguments: &str) -> ToolResult {
        if !self.client.user_oauth_configured() {
            return missing_oauth_tool_result();
        }
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("invalid args: {e}")),
        };
        let summary = match args.get("summary").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return ToolResult::err("summary is required".to_string()),
        };
        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let mut task = serde_json::json!({
            "summary": summary,
            "description": description,
        });
        if let Some(due) = args.get("due").and_then(|v| v.as_str()) {
            if !due.is_empty() {
                task["due"] = serde_json::json!({
                    "timestamp": due,
                });
            }
        }
        if let Some(arr) = args.get("assignees").and_then(|v| v.as_array()) {
            if !arr.is_empty() {
                task["assignees"] = serde_json::Value::Array(arr.clone());
            }
        }
        let body = serde_json::json!({ "task": task });
        match self.client.user_post_json("/task/v2/tasks", &body).await {
            Ok(v) => match serde_json::to_string(&v) {
                Ok(s) => ToolResult::ok(s),
                Err(e) => ToolResult::err(format!("feishu_task_create: serialize response: {e}")),
            },
            Err(e) => ToolResult::err(format!("feishu_task_create: {e}")),
        }
    }
}

/// Tool: feishu_task_list — List tasks.
pub struct FeishuTaskListTool {
    client: Arc<FeishuClient>,
}

impl FeishuTaskListTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuTaskListTool {
    fn name(&self) -> &str {
        "feishu_task_list"
    }
    fn description(&self) -> &str {
        "List tasks from Feishu Tasks with optional filters."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "page_size".to_string(),
            serde_json::json!({"type": "integer", "default": 20}),
        );
        properties.insert(
            "completed".to_string(),
            serde_json::json!({"type": "boolean", "description": "Filter by completion status"}),
        );
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec![],
        }
    }
    async fn execute(&self, arguments: &str) -> ToolResult {
        if !self.client.user_oauth_configured() {
            return missing_oauth_tool_result();
        }
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("invalid args: {e}")),
        };
        let page_size = args
            .get("page_size")
            .and_then(|v| v.as_u64())
            .unwrap_or(20)
            .clamp(1, 100);
        let mut body = serde_json::json!({ "page_size": page_size });
        if let Some(done) = args.get("completed").and_then(|v| v.as_bool()) {
            body["completed"] = serde_json::Value::Bool(done);
        }
        match self.client.user_post_json("/task/v2/tasks/query", &body).await {
            Ok(v) => match serde_json::to_string(&v) {
                Ok(s) => ToolResult::ok(s),
                Err(e) => ToolResult::err(format!("feishu_task_list: serialize response: {e}")),
            },
            Err(e) => ToolResult::err(format!("feishu_task_list: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_tool_names() {
        let client = Arc::new(FeishuClient::new("t", "s"));
        assert_eq!(
            FeishuTaskCreateTool::new(client.clone()).name(),
            "feishu_task_create"
        );
        assert_eq!(FeishuTaskListTool::new(client).name(), "feishu_task_list");
    }

    #[tokio::test]
    async fn task_create_without_oauth_is_tool_error_not_bail() {
        let client = Arc::new(FeishuClient::new("t", "s"));
        let tool = FeishuTaskCreateTool::new(client);
        let r = tool
            .execute(r#"{"summary":"x","description":"y"}"#)
            .await;
        assert!(!r.success);
        let err = r.output;
        assert!(
            err.contains("userAccessToken") || err.contains("user OAuth"),
            "unexpected: {err}"
        );
    }

    #[tokio::test]
    async fn task_list_without_oauth_is_tool_error() {
        let client = Arc::new(FeishuClient::new("t", "s"));
        let tool = FeishuTaskListTool::new(client);
        let r = tool.execute("{}").await;
        assert!(!r.success);
        let err = r.output;
        assert!(
            err.contains("userAccessToken") || err.contains("user OAuth"),
            "unexpected: {err}"
        );
    }
}
