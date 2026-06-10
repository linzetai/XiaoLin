use std::sync::Arc;

use xiaolin_core::types::{ChatMessage, ChatRequest, Role};

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
/// XiaoLin agent pipeline, and replies via Feishu API.
pub struct FeishuChannel {
    pub client: Arc<FeishuClient>,
    pub config: FeishuChannelConfig,
    runtime: Arc<xiaolin_agent::AgentRuntime>,
    router: Arc<std::sync::RwLock<xiaolin_core::Router>>,
    tool_registry: Arc<xiaolin_core::tool::ToolRegistry>,
    session_store: Arc<xiaolin_session::SessionStore>,
    event_log: Arc<xiaolin_session::EventLog>,
    tool_orchestrator: Arc<xiaolin_agent::runtime::orchestrator::ToolOrchestrator>,
}

impl FeishuChannel {
    pub fn new(
        config: FeishuChannelConfig,
        runtime: Arc<xiaolin_agent::AgentRuntime>,
        router: Arc<std::sync::RwLock<xiaolin_core::Router>>,
        tool_registry: Arc<xiaolin_core::tool::ToolRegistry>,
        session_store: Arc<xiaolin_session::SessionStore>,
    ) -> Self {
        let event_log = Arc::new(xiaolin_session::EventLog::new(session_store.pool()));
        let client = Arc::new(FeishuClient::new(&config.app_id, &config.app_secret));
        let tool_orchestrator = Arc::new(
            xiaolin_agent::runtime::orchestrator::ToolOrchestrator::new(),
        );
        Self {
            client,
            config,
            runtime,
            router,
            tool_registry,
            session_store,
            event_log,
            tool_orchestrator,
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

        let user_msg = ChatMessage {
            role: Role::User,
            content: Some(serde_json::Value::String(text.to_string())),
        ..Default::default()
        };
        self.session_store
            .append_message(&session.id, &user_msg)
            .await?;
        {
            let turn_id = xiaolin_protocol::TurnId::generate();
            let items =
                xiaolin_core::history_compat::chat_message_to_history(&user_msg, turn_id);
            if let Err(e) = self
                .session_store
                .append_history_items(&session.id, &items)
                .await
            {
                tracing::warn!(session = %session.id, error = %e, "failed to dual-write user history items");
            }
        }

        let history_items = self.session_store.load_history(&session.id).await?;
        let messages: Vec<xiaolin_core::types::ChatMessage> = if history_items.is_empty() {
            let arc = self.session_store.load_chat_messages(&session.id).await?;
            std::sync::Arc::try_unwrap(arc).unwrap_or_else(|a| (*a).clone())
        } else {
            xiaolin_core::history_compat::history_items_to_chat_messages(&history_items)
        };

        let request = ChatRequest {
            messages,
            stream: false,
            model: None,
            temperature: None,
            max_tokens: None,
            agent_id: Some(self.config.agent_id.clone().into()),
            session_id: Some(session.id.clone().into()),
            tools: None,
            slash_intent: None,
            work_dir: None,
            response_language: None,
        };

        let agent_config = self
            .router
            .read()
            .map_err(|_| anyhow::anyhow!("router lock poisoned"))?
            .resolve(&request)
            .cloned()
            .map_err(|e| anyhow::anyhow!("agent resolve failed: {e}"))?;

        let (tx, mut rx) = tokio::sync::mpsc::channel::<xiaolin_protocol::AgentEvent>(64);
        let summary = self
            .runtime
            .execute_unified(
                &agent_config,
                &request,
                &self.tool_registry,
                tx,
                xiaolin_core::tool_runtime::ApprovalStrategy::PolicyBased,
                None,
                self.tool_orchestrator.clone(),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await?;

        // Collect the final assistant content from events.
        let mut reply = String::new();
        rx.close();
        while let Some(event) = rx.recv().await {
            if let xiaolin_protocol::AgentEvent::ContentDelta { delta, .. } = event {
                if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
                    reply.push_str(text);
                }
            }
        }
        if reply.is_empty() {
            reply = "(no response)".to_string();
        }

        let assistant_msg = ChatMessage {
            role: Role::Assistant,
            content: Some(serde_json::Value::String(reply.clone())),
        ..Default::default()
        };
        self.session_store
            .append_message(&session.id, &assistant_msg)
            .await?;
        {
            let turn_id = xiaolin_protocol::TurnId::generate();
            let items =
                xiaolin_core::history_compat::chat_message_to_history(&assistant_msg, turn_id);
            if let Err(e) = self
                .session_store
                .append_history_items(&session.id, &items)
                .await
            {
                tracing::warn!(session = %session.id, error = %e, "failed to dual-write assistant history items");
            }
        }

        // Synthesize events and write to event_log for audit trail
        {
            use xiaolin_protocol::AgentEvent;
            let turn_id = xiaolin_protocol::TurnId::generate();
            let turn_start = AgentEvent::TurnStart {
                turn_id: turn_id.clone(),
                session_id: Some(session.id.clone()),
            };
            let turn_end = AgentEvent::TurnEnd {
                turn_id: turn_id.clone(),
                summary: xiaolin_protocol::TurnSummary {
                    turn_id,
                    tool_calls_made: summary.tool_calls_made,
                    iterations: summary.iterations,
                    usage: None,
                    elapsed_ms: 0,
                    context_tokens: None,
                    context_window: None,
                },
                session_id: Some(session.id.clone()),
                final_tool_calls: None,
            };
            for event in [turn_start, turn_end] {
                self.event_log.append(&session.id, &event);
            }
        }

        tracing::info!(
            session = %session.id,
            chat_id,
            tool_calls = summary.tool_calls_made,
            iterations = summary.iterations,
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
