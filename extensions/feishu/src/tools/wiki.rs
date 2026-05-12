use crate::client::FeishuClient;
use crate::oauth::OAuthConfig;
use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolResult};
use std::collections::HashMap;
use std::sync::Arc;

/// Single action-based tool matching OpenClaw's `feishu_wiki`.
pub struct FeishuWikiTool {
    client: Arc<FeishuClient>,
}

impl FeishuWikiTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuWikiTool {
    fn name(&self) -> &str {
        "feishu_wiki"
    }
    fn description(&self) -> &str {
        "Feishu knowledge base operations. Actions: spaces (list spaces), nodes (list child nodes), \
         get (get node by token), search (search docs), create (create node), move (move node), \
         rename (rename node)."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert("action".into(), serde_json::json!({"type": "string", "enum": ["spaces", "nodes", "get", "search", "create", "move", "rename"]}));
        properties.insert(
            "space_id".into(),
            serde_json::json!({"type": "string", "description": "Knowledge space ID"}),
        );
        properties.insert("token".into(), serde_json::json!({"type": "string", "description": "Wiki node token (for get action)"}));
        properties.insert(
            "node_token".into(),
            serde_json::json!({"type": "string", "description": "Node token (for move/rename)"}),
        );
        properties.insert(
            "parent_node_token".into(),
            serde_json::json!({"type": "string", "description": "Parent node token (optional)"}),
        );
        properties.insert("query".into(), serde_json::json!({"type": "string", "description": "Search query (for search action)"}));
        properties.insert(
            "title".into(),
            serde_json::json!({"type": "string", "description": "Node title (for create/rename)"}),
        );
        properties.insert("obj_type".into(), serde_json::json!({"type": "string", "enum": ["docx", "sheet", "bitable"], "description": "Object type for create (default: docx)"}));
        properties.insert(
            "target_space_id".into(),
            serde_json::json!({"type": "string", "description": "Target space ID for move"}),
        );
        properties.insert("target_parent_token".into(), serde_json::json!({"type": "string", "description": "Target parent node token for move"}));
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
            "spaces" => self.client.user_get("/wiki/v2/spaces").await,
            "nodes" => {
                let space_id = match args.get("space_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("space_id is required for nodes".to_string()),
                };
                let mut path = format!("/wiki/v2/spaces/{space_id}/nodes");
                if let Some(pt) = args
                    .get("parent_node_token")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    path.push_str(&format!("?parent_node_token={pt}"));
                }
                self.client.user_get(&path).await
            }
            "get" => {
                let token = match args.get("token").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("token is required for get".to_string()),
                };
                self.client
                    .user_get_query("/wiki/v2/spaces/get_node", &[("token", token)])
                    .await
            }
            "search" => {
                let query = match args.get("query").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("query is required for search".to_string()),
                };
                let mut body = serde_json::json!({ "search_key": query, "count": 20, "offset": 0 });
                if let Some(sid) = args
                    .get("space_id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    body["docs_type"] = serde_json::json!([]);
                    body["owner_ids"] = serde_json::json!([]);
                    body["space_id"] = serde_json::Value::String(sid.to_string());
                }
                self.client
                    .user_post_json("/suite/docs-api/search/object", &body)
                    .await
            }
            "create" => {
                let space_id = match args.get("space_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("space_id is required for create".to_string()),
                };
                let title = match args.get("title").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("title is required for create".to_string()),
                };
                let obj_type = args
                    .get("obj_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("docx");
                let mut body = serde_json::json!({ "obj_type": obj_type, "title": title });
                if let Some(pt) = args
                    .get("parent_node_token")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    body["parent_node_token"] = serde_json::Value::String(pt.to_string());
                }
                self.client
                    .user_post_json(&format!("/wiki/v2/spaces/{space_id}/nodes"), &body)
                    .await
            }
            "move" => {
                let space_id = match args.get("space_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("space_id is required for move".to_string()),
                };
                let node_token = match args.get("node_token").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("node_token is required for move".to_string()),
                };
                let mut body = serde_json::json!({});
                if let Some(ts) = args
                    .get("target_space_id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    body["target_space_id"] = serde_json::Value::String(ts.to_string());
                }
                if let Some(tp) = args
                    .get("target_parent_token")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    body["target_parent_token"] = serde_json::Value::String(tp.to_string());
                }
                self.client
                    .user_post_json(
                        &format!("/wiki/v2/spaces/{space_id}/nodes/{node_token}/move"),
                        &body,
                    )
                    .await
            }
            "rename" => {
                let space_id = match args.get("space_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("space_id is required for rename".to_string()),
                };
                let node_token = match args.get("node_token").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("node_token is required for rename".to_string()),
                };
                let title = match args.get("title").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::err("title is required for rename".to_string()),
                };
                let body = serde_json::json!({ "title": title });
                self.client
                    .user_put_json(
                        &format!("/wiki/v2/spaces/{space_id}/nodes/{node_token}"),
                        &body,
                    )
                    .await
            }
            _ => return ToolResult::err(format!("unknown action: {action}")),
        };
        match result {
            Ok(v) => ToolResult::ok(serde_json::to_string(&v).unwrap_or_default()),
            Err(e) => ToolResult::err(format!("feishu_wiki {action}: {e}")),
        }
    }
}
