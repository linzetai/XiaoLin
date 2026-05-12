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
                Err(e) => ToolResult::err(format!("feishu_bitable_list_records: serialize: {e}")),
            },
            Err(e) => ToolResult::err(format!("feishu_bitable_list_records: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn require_oauth(client: &FeishuClient) -> Option<ToolResult> {
    if !client.user_oauth_configured() {
        Some(ToolResult::err(
            OAuthConfig::missing_user_token_message().to_string(),
        ))
    } else {
        None
    }
}

fn parse_args(arguments: &str) -> Result<serde_json::Value, ToolResult> {
    serde_json::from_str(arguments).map_err(|e| ToolResult::err(format!("invalid args: {e}")))
}

fn require_str<'a>(args: &'a serde_json::Value, key: &str) -> Result<&'a str, ToolResult> {
    args.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolResult::err(format!("{key} is required")))
}

fn ok_json(v: serde_json::Value) -> ToolResult {
    ToolResult::ok(serde_json::to_string(&v).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// feishu_bitable_get_meta
// ---------------------------------------------------------------------------

pub struct FeishuBitableGetMetaTool {
    client: Arc<FeishuClient>,
}
impl FeishuBitableGetMetaTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuBitableGetMetaTool {
    fn name(&self) -> &str {
        "feishu_bitable_get_meta"
    }
    fn description(&self) -> &str {
        "Get metadata of a Bitable app including table list. Use this first when given a /wiki/ or /base/ URL."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "app_token".into(),
            serde_json::json!({"type": "string", "description": "Bitable app token"}),
        );
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec!["app_token".into()],
        }
    }
    async fn execute(&self, arguments: &str) -> ToolResult {
        if let Some(e) = require_oauth(&self.client) {
            return e;
        }
        let args = match parse_args(arguments) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let app_token = match require_str(&args, "app_token") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let path = format!("/bitable/v1/apps/{app_token}");
        match self.client.user_get(&path).await {
            Ok(v) => ok_json(v),
            Err(e) => ToolResult::err(format!("feishu_bitable_get_meta: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// feishu_bitable_list_fields
// ---------------------------------------------------------------------------

pub struct FeishuBitableListFieldsTool {
    client: Arc<FeishuClient>,
}
impl FeishuBitableListFieldsTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuBitableListFieldsTool {
    fn name(&self) -> &str {
        "feishu_bitable_list_fields"
    }
    fn description(&self) -> &str {
        "List all fields (columns) in a Bitable table with their types and properties."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert("app_token".into(), serde_json::json!({"type": "string"}));
        properties.insert("table_id".into(), serde_json::json!({"type": "string"}));
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec!["app_token".into(), "table_id".into()],
        }
    }
    async fn execute(&self, arguments: &str) -> ToolResult {
        if let Some(e) = require_oauth(&self.client) {
            return e;
        }
        let args = match parse_args(arguments) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let app_token = match require_str(&args, "app_token") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let table_id = match require_str(&args, "table_id") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let path = format!("/bitable/v1/apps/{app_token}/tables/{table_id}/fields");
        match self.client.user_get(&path).await {
            Ok(v) => ok_json(v),
            Err(e) => ToolResult::err(format!("feishu_bitable_list_fields: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// feishu_bitable_get_record
// ---------------------------------------------------------------------------

pub struct FeishuBitableGetRecordTool {
    client: Arc<FeishuClient>,
}
impl FeishuBitableGetRecordTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuBitableGetRecordTool {
    fn name(&self) -> &str {
        "feishu_bitable_get_record"
    }
    fn description(&self) -> &str {
        "Get a single record from a Bitable table by record_id."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert("app_token".into(), serde_json::json!({"type": "string"}));
        properties.insert("table_id".into(), serde_json::json!({"type": "string"}));
        properties.insert("record_id".into(), serde_json::json!({"type": "string"}));
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec!["app_token".into(), "table_id".into(), "record_id".into()],
        }
    }
    async fn execute(&self, arguments: &str) -> ToolResult {
        if let Some(e) = require_oauth(&self.client) {
            return e;
        }
        let args = match parse_args(arguments) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let app_token = match require_str(&args, "app_token") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let table_id = match require_str(&args, "table_id") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let record_id = match require_str(&args, "record_id") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let path = format!("/bitable/v1/apps/{app_token}/tables/{table_id}/records/{record_id}");
        match self.client.user_get(&path).await {
            Ok(v) => ok_json(v),
            Err(e) => ToolResult::err(format!("feishu_bitable_get_record: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// feishu_bitable_create_record
// ---------------------------------------------------------------------------

pub struct FeishuBitableCreateRecordTool {
    client: Arc<FeishuClient>,
}
impl FeishuBitableCreateRecordTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuBitableCreateRecordTool {
    fn name(&self) -> &str {
        "feishu_bitable_create_record"
    }
    fn description(&self) -> &str {
        "Create a new record in a Bitable table. Provide field values as a JSON object."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert("app_token".into(), serde_json::json!({"type": "string"}));
        properties.insert("table_id".into(), serde_json::json!({"type": "string"}));
        properties.insert(
            "fields".into(),
            serde_json::json!({"type": "object", "description": "Field name to value mapping"}),
        );
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec!["app_token".into(), "table_id".into(), "fields".into()],
        }
    }
    async fn execute(&self, arguments: &str) -> ToolResult {
        if let Some(e) = require_oauth(&self.client) {
            return e;
        }
        let args = match parse_args(arguments) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let app_token = match require_str(&args, "app_token") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let table_id = match require_str(&args, "table_id") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let fields = match args.get("fields") {
            Some(v) if v.is_object() => v,
            _ => return ToolResult::err("fields must be a JSON object".to_string()),
        };
        let path = format!("/bitable/v1/apps/{app_token}/tables/{table_id}/records");
        let body = serde_json::json!({ "fields": fields });
        match self.client.user_post_json(&path, &body).await {
            Ok(v) => ok_json(v),
            Err(e) => ToolResult::err(format!("feishu_bitable_create_record: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// feishu_bitable_update_record
// ---------------------------------------------------------------------------

pub struct FeishuBitableUpdateRecordTool {
    client: Arc<FeishuClient>,
}
impl FeishuBitableUpdateRecordTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuBitableUpdateRecordTool {
    fn name(&self) -> &str {
        "feishu_bitable_update_record"
    }
    fn description(&self) -> &str {
        "Update an existing record in a Bitable table. Provide the fields to update."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert("app_token".into(), serde_json::json!({"type": "string"}));
        properties.insert("table_id".into(), serde_json::json!({"type": "string"}));
        properties.insert("record_id".into(), serde_json::json!({"type": "string"}));
        properties.insert(
            "fields".into(),
            serde_json::json!({"type": "object", "description": "Field name to new value mapping"}),
        );
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec![
                "app_token".into(),
                "table_id".into(),
                "record_id".into(),
                "fields".into(),
            ],
        }
    }
    async fn execute(&self, arguments: &str) -> ToolResult {
        if let Some(e) = require_oauth(&self.client) {
            return e;
        }
        let args = match parse_args(arguments) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let app_token = match require_str(&args, "app_token") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let table_id = match require_str(&args, "table_id") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let record_id = match require_str(&args, "record_id") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let fields = match args.get("fields") {
            Some(v) if v.is_object() => v,
            _ => return ToolResult::err("fields must be a JSON object".to_string()),
        };
        let path = format!("/bitable/v1/apps/{app_token}/tables/{table_id}/records/{record_id}");
        let body = serde_json::json!({ "fields": fields });
        match self.client.user_put_json(&path, &body).await {
            Ok(v) => ok_json(v),
            Err(e) => ToolResult::err(format!("feishu_bitable_update_record: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// feishu_bitable_create_app
// ---------------------------------------------------------------------------

pub struct FeishuBitableCreateAppTool {
    client: Arc<FeishuClient>,
}
impl FeishuBitableCreateAppTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuBitableCreateAppTool {
    fn name(&self) -> &str {
        "feishu_bitable_create_app"
    }
    fn description(&self) -> &str {
        "Create a new Bitable (multidimensional table) application."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "name".into(),
            serde_json::json!({"type": "string", "description": "App name"}),
        );
        properties.insert(
            "folder_token".into(),
            serde_json::json!({"type": "string", "description": "Parent folder token (optional)"}),
        );
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec!["name".into()],
        }
    }
    async fn execute(&self, arguments: &str) -> ToolResult {
        if let Some(e) = require_oauth(&self.client) {
            return e;
        }
        let args = match parse_args(arguments) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let name = match require_str(&args, "name") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let mut body = serde_json::json!({ "name": name });
        if let Some(ft) = args
            .get("folder_token")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            body["folder_token"] = serde_json::Value::String(ft.to_string());
        }
        match self.client.user_post_json("/bitable/v1/apps", &body).await {
            Ok(v) => ok_json(v),
            Err(e) => ToolResult::err(format!("feishu_bitable_create_app: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// feishu_bitable_create_field
// ---------------------------------------------------------------------------

pub struct FeishuBitableCreateFieldTool {
    client: Arc<FeishuClient>,
}
impl FeishuBitableCreateFieldTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuBitableCreateFieldTool {
    fn name(&self) -> &str {
        "feishu_bitable_create_field"
    }
    fn description(&self) -> &str {
        "Create a new field (column) in a Bitable table."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert("app_token".into(), serde_json::json!({"type": "string"}));
        properties.insert("table_id".into(), serde_json::json!({"type": "string"}));
        properties.insert(
            "field_name".into(),
            serde_json::json!({"type": "string", "description": "Field display name"}),
        );
        properties.insert("field_type".into(), serde_json::json!({"type": "integer", "description": "Field type number (1=Text, 2=Number, 3=SingleSelect, etc.)"}));
        properties.insert("property".into(), serde_json::json!({"type": "object", "description": "Optional field properties (e.g. select options)"}));
        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec![
                "app_token".into(),
                "table_id".into(),
                "field_name".into(),
                "field_type".into(),
            ],
        }
    }
    async fn execute(&self, arguments: &str) -> ToolResult {
        if let Some(e) = require_oauth(&self.client) {
            return e;
        }
        let args = match parse_args(arguments) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let app_token = match require_str(&args, "app_token") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let table_id = match require_str(&args, "table_id") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let field_name = match require_str(&args, "field_name") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let field_type = match args.get("field_type").and_then(|v| v.as_i64()) {
            Some(ft) => ft,
            None => return ToolResult::err("field_type (integer) is required".to_string()),
        };
        let path = format!("/bitable/v1/apps/{app_token}/tables/{table_id}/fields");
        let mut body = serde_json::json!({
            "field_name": field_name,
            "type": field_type,
        });
        if let Some(prop) = args.get("property") {
            if prop.is_object() {
                body["property"] = prop.clone();
            }
        }
        match self.client.user_post_json(&path, &body).await {
            Ok(v) => ok_json(v),
            Err(e) => ToolResult::err(format!("feishu_bitable_create_field: {e}")),
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
        let r = tool.execute(r#"{"app_token":"a","table_id":"b"}"#).await;
        assert!(!r.success);
        assert!(
            r.output.contains("userAccessToken") || r.output.contains("user OAuth"),
            "unexpected: {}",
            r.output
        );
    }

    #[test]
    fn new_bitable_tool_names() {
        let client = Arc::new(FeishuClient::new("t", "s"));
        assert_eq!(
            FeishuBitableGetMetaTool::new(client.clone()).name(),
            "feishu_bitable_get_meta"
        );
        assert_eq!(
            FeishuBitableListFieldsTool::new(client.clone()).name(),
            "feishu_bitable_list_fields"
        );
        assert_eq!(
            FeishuBitableGetRecordTool::new(client.clone()).name(),
            "feishu_bitable_get_record"
        );
        assert_eq!(
            FeishuBitableCreateRecordTool::new(client.clone()).name(),
            "feishu_bitable_create_record"
        );
        assert_eq!(
            FeishuBitableUpdateRecordTool::new(client.clone()).name(),
            "feishu_bitable_update_record"
        );
        assert_eq!(
            FeishuBitableCreateAppTool::new(client.clone()).name(),
            "feishu_bitable_create_app"
        );
        assert_eq!(
            FeishuBitableCreateFieldTool::new(client).name(),
            "feishu_bitable_create_field"
        );
    }
}
