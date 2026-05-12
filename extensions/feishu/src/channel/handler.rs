use std::sync::Arc;

use fastclaw_core::types::{ChatMessage, ChatRequest, Role};

use crate::client::FeishuClient;
use crate::webhook::FeishuMessageHandler;

#[derive(Clone)]
pub struct FeishuChannelConfig {
    pub app_id: String,
    pub app_secret: String,
    pub verification_token: String,
    pub encrypt_key: Option<String>,
    pub agent_id: String,
}

/// Feishu messaging channel: receives messages via webhook, processes through
/// FastClaw agent pipeline, and replies via Feishu API.
pub struct FeishuChannel {
    pub client: Arc<FeishuClient>,
    pub config: FeishuChannelConfig,
    runtime: Arc<fastclaw_agent::AgentRuntime>,
    router: Arc<std::sync::RwLock<fastclaw_core::Router>>,
    tool_registry: Arc<fastclaw_core::tool::ToolRegistry>,
    session_store: Arc<fastclaw_session::SessionStore>,
}

impl FeishuChannel {
    pub fn new(
        config: FeishuChannelConfig,
        runtime: Arc<fastclaw_agent::AgentRuntime>,
        router: Arc<std::sync::RwLock<fastclaw_core::Router>>,
        tool_registry: Arc<fastclaw_core::tool::ToolRegistry>,
        session_store: Arc<fastclaw_session::SessionStore>,
    ) -> Self {
        let client = Arc::new(FeishuClient::new(&config.app_id, &config.app_secret));
        Self {
            client,
            config,
            runtime,
            router,
            tool_registry,
            session_store,
        }
    }

    fn session_key(&self, chat_id: &str) -> String {
        format!("feishu:{}", chat_id)
    }
}

#[async_trait::async_trait]
impl FeishuMessageHandler for FeishuChannel {
    async fn handle_message(
        &self,
        _sender_id: &str,
        _message_id: &str,
        chat_id: &str,
        text: &str,
    ) -> anyhow::Result<String> {
        let session_key = self.session_key(chat_id);

        if self
            .session_store
            .get_session(&session_key)
            .await?
            .is_none()
        {
            self.session_store
                .create_session_full(
                    &session_key,
                    &self.config.agent_id,
                    None,
                    None,
                    Some("feishu"),
                )
                .await?;
        }
        let session = self
            .session_store
            .get_session(&session_key)
            .await?
            .ok_or_else(|| anyhow::anyhow!("failed to create session"))?;

        self.session_store
            .append_message(
                &session.id,
                &ChatMessage {
                    role: Role::User,
                    content: Some(serde_json::Value::String(text.to_string())),
                    reasoning_content: None,
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            )
            .await?;

        let messages = self.session_store.load_chat_messages(&session.id).await?;

        let request = ChatRequest {
            messages,
            stream: false,
            model: None,
            temperature: None,
            max_tokens: None,
            agent_id: Some(self.config.agent_id.clone().into()),
            session_id: Some(session.id.clone()),
            tools: None,
            slash_intent: None,
            work_dir: None,
        };

        let agent_config = self
            .router
            .read()
            .map_err(|_| anyhow::anyhow!("router lock poisoned"))?
            .resolve(&request)
            .cloned()
            .map_err(|e| anyhow::anyhow!("agent resolve failed: {e}"))?;

        let result = self
            .runtime
            .execute(&agent_config, &request, &self.tool_registry, None)
            .await?;

        let reply = result
            .response
            .choices
            .first()
            .and_then(|c| c.message.text_content())
            .unwrap_or_else(|| "(no response)".to_string());

        self.session_store
            .append_message(
                &session.id,
                &ChatMessage {
                    role: Role::Assistant,
                    content: Some(serde_json::Value::String(reply.clone())),
                    reasoning_content: None,
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            )
            .await?;

        tracing::info!(
            session = %session.id,
            chat_id,
            tool_calls = result.tool_calls_made,
            iterations = result.iterations,
            "Feishu message processed"
        );

        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn session_key_format() {
        let key = format!("feishu:{}", "oc_abc");
        assert_eq!(key, "feishu:oc_abc");
    }

    #[test]
    fn config_fields() {
        let config = super::FeishuChannelConfig {
            app_id: "test".into(),
            app_secret: "secret".into(),
            verification_token: "vt".into(),
            encrypt_key: None,
            agent_id: "main".into(),
        };
        assert_eq!(config.agent_id, "main");
    }
}
