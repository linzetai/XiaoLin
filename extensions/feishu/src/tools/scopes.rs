use crate::client::FeishuClient;
use crate::oauth::OAuthConfig;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use xiaolin_core::tool::{Tool, ToolParameterSchema, ToolResult};

/// List the app's authorized scopes (permissions). Helps the LLM
/// understand which APIs the bot has access to.
pub struct FeishuAppScopesTool {
    client: Arc<FeishuClient>,
}

impl FeishuAppScopesTool {
    pub fn new(client: Arc<FeishuClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for FeishuAppScopesTool {
    fn name(&self) -> &str {
        "feishu_app_scopes"
    }
    fn description(&self) -> &str {
        "List the scopes (permissions) that the current Feishu app has been authorized. \
         Useful when the LLM needs to check whether a particular API is available."
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        ToolParameterSchema {
            schema_type: "object".into(),
            properties: HashMap::new(),
            required: vec![],
        }
    }
    async fn execute(&self, _arguments: &str) -> ToolResult {
        if !self.client.user_oauth_configured() {
            return ToolResult::err(OAuthConfig::missing_user_token_message().to_string());
        }
        match self
            .client
            .user_get("/application/v6/applications/underauditlist?lang=zh_cn&page_size=1")
            .await
        {
            Ok(v) => ToolResult::ok(serde_json::to_string(&v).unwrap_or_default()),
            Err(e) => ToolResult::err(format!("feishu_app_scopes: {e}")),
        }
    }
}
