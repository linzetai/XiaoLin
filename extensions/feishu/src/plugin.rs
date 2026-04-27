use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex};

use fastclaw_core::channel::{
    ChannelCapabilities, ChannelMeta, ChannelPlugin, InboundMessage, OutboundMessage, WebhookResult,
};
use fastclaw_core::tool::Tool;

use crate::client::FeishuClient;
use crate::tools::{FeishuGetChatMessagesTool, FeishuReplyMessageTool, FeishuSendMessageTool};
use crate::ws;

const DEFAULT_FEISHU_DOMAIN: &str = "https://open.feishu.cn";

/// Configuration for the Feishu channel plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuPluginConfig {
    pub app_id: String,
    pub app_secret: String,
    #[serde(default)]
    pub verification_token: Option<String>,
    #[serde(default)]
    pub encrypt_key: Option<String>,
    #[serde(default = "default_agent_id")]
    pub agent_id: String,
    #[serde(default = "default_connection_mode")]
    pub connection_mode: String,
    #[serde(default = "default_domain")]
    pub domain: String,
    /// "mention_only" (default) = reply in group only when @mentioned; "always" = reply to all.
    #[serde(default = "default_reply_mode")]
    pub reply_mode: String,
    /// User access token for user-scoped Open APIs (tasks, bitable, docx, calendar, media).
    #[serde(default)]
    pub user_access_token: Option<String>,
}

fn default_agent_id() -> String {
    "main".to_string()
}

fn default_connection_mode() -> String {
    "websocket".to_string()
}

fn default_domain() -> String {
    DEFAULT_FEISHU_DOMAIN.to_string()
}

fn default_reply_mode() -> String {
    "mention_only".to_string()
}

impl FeishuPluginConfig {
    /// Create from JSON channel config. All fields must be provided in the config file.
    pub fn from_channel_config(cfg: &fastclaw_core::config::ChannelConfig) -> Option<Self> {
        let app_id = cfg.app_id.clone()?;
        let app_secret = cfg.app_secret.clone()?;
        Some(Self {
            app_id,
            app_secret,
            verification_token: cfg.verification_token.clone(),
            encrypt_key: cfg.encrypt_key.clone(),
            agent_id: cfg.agent_id.clone().unwrap_or_else(|| "main".to_string()),
            connection_mode: cfg
                .connection_mode
                .clone()
                .unwrap_or_else(|| "websocket".to_string()),
            domain: cfg
                .domain
                .clone()
                .unwrap_or_else(|| DEFAULT_FEISHU_DOMAIN.to_string()),
            reply_mode: cfg
                .reply_mode
                .clone()
                .unwrap_or_else(|| "mention_only".to_string()),
            user_access_token: cfg.user_access_token.clone(),
        })
    }
}

/// FastClaw Feishu Channel Plugin — bridges Feishu/Lark messaging into the
/// FastClaw agent ecosystem.
///
/// Modeled after the official OpenClaw Lark plugin pattern:
/// - Registers as a channel (handles inbound webhooks + outbound messaging)
/// - Provides tools for agent use (send_message, reply_message, get_messages)
pub struct FeishuPlugin {
    meta: ChannelMeta,
    config: FeishuPluginConfig,
    client: Arc<FeishuClient>,
    /// WebSocket client for long-connection mode (None in webhook mode)
    ws_client: Arc<Mutex<Option<Arc<ws::FeishuWsClient>>>>,
}

impl FeishuPlugin {
    pub fn new(config: FeishuPluginConfig) -> Self {
        let base_url = if config.domain == DEFAULT_FEISHU_DOMAIN {
            None
        } else {
            Some(format!("{}/open-apis", config.domain.trim_end_matches('/')))
        };
        let client = Arc::new(match base_url {
            Some(ref url) => FeishuClient::with_base_url_user_token(
                &config.app_id,
                &config.app_secret,
                url,
                config.user_access_token.clone(),
            ),
            None => FeishuClient::new_with_user_token(
                &config.app_id,
                &config.app_secret,
                config.user_access_token.clone(),
            ),
        });
        Self {
            meta: ChannelMeta {
                id: "feishu".to_string(),
                name: "Feishu".to_string(),
                description: "Lark/Feishu enterprise messaging channel with IM/Doc/Calendar tools"
                    .to_string(),
                aliases: vec!["lark".to_string()],
            },
            config,
            client,
            ws_client: Arc::new(Mutex::new(None)),
        }
    }

    pub fn client(&self) -> &Arc<FeishuClient> {
        &self.client
    }

    pub fn config(&self) -> &FeishuPluginConfig {
        &self.config
    }

    pub fn reply_mode(&self) -> &str {
        &self.config.reply_mode
    }

    async fn get_bot_open_id(&self) -> Option<String> {
        match self.client.get_tenant_token().await {
            Ok(token) => {
                let url = format!(
                    "{}/bot/v3/info",
                    if self.config.domain == DEFAULT_FEISHU_DOMAIN {
                        "https://open.feishu.cn/open-apis".to_string()
                    } else {
                        format!("{}/open-apis", self.config.domain.trim_end_matches('/'))
                    }
                );
                let resp = reqwest::Client::new()
                    .get(&url)
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await
                    .ok()?
                    .json::<serde_json::Value>()
                    .await
                    .ok()?;
                let open_id = resp
                    .get("bot")
                    .and_then(|b| b.get("open_id"))
                    .and_then(|v| v.as_str())
                    .map(String::from);
                if let Some(ref id) = open_id {
                    tracing::info!(bot_open_id = %id, "feishu: resolved bot open_id");
                }
                open_id
            }
            Err(e) => {
                tracing::warn!(error = %e, "feishu: could not get bot open_id");
                None
            }
        }
    }

    fn verify_token(&self, token: &str) -> bool {
        match &self.config.verification_token {
            Some(vt) if !vt.is_empty() => vt == token,
            _ => true,
        }
    }
}

/// Infer Feishu `receive_id_type` from the target ID prefix and the generic target_type hint.
/// Feishu API expects: "open_id" for `ou_*`, "chat_id" for `oc_*`, "user_id" for enterprise IDs.
fn infer_receive_id_type<'a>(target_id: &str, target_type: &'a str) -> &'a str {
    if target_id.starts_with("oc_") {
        return "chat_id";
    }
    if target_id.starts_with("ou_") {
        return "open_id";
    }
    match target_type {
        "p2p" | "open_id" => "open_id",
        "group" | "chat_id" => "chat_id",
        "user_id" => "user_id",
        _ => "chat_id",
    }
}

#[async_trait]
impl ChannelPlugin for FeishuPlugin {
    fn meta(&self) -> &ChannelMeta {
        &self.meta
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            direct_message: true,
            group_chat: true,
            media: true,
            reactions: true,
            threads: true,
            streaming: true,
        }
    }

    async fn verify_webhook(
        &self,
        _headers: &BTreeMap<String, String>,
        raw_body: &[u8],
    ) -> anyhow::Result<()> {
        let payload: serde_json::Value = serde_json::from_slice(raw_body).unwrap_or_default();
        let token = payload
            .get("token")
            .and_then(|v| v.as_str())
            .or_else(|| {
                payload
                    .get("header")
                    .and_then(|h| h.get("token"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("");
        if !self.verify_token(token) {
            anyhow::bail!("Feishu webhook token mismatch");
        }
        Ok(())
    }

    async fn handle_webhook(&self, payload: serde_json::Value) -> anyhow::Result<WebhookResult> {
        // URL verification challenge (token already verified by verify_webhook)
        if let Some(challenge) = payload.get("challenge").and_then(|v| v.as_str()) {
            return Ok(WebhookResult::Challenge(
                serde_json::json!({ "challenge": challenge }),
            ));
        }

        let event_type = payload
            .get("header")
            .and_then(|h| h.get("event_type"))
            .and_then(|v| v.as_str())
            .or_else(|| payload.get("type").and_then(|v| v.as_str()))
            .unwrap_or("");

        if event_type != "im.message.receive_v1" {
            return Ok(WebhookResult::Ignored);
        }

        let event = match payload.get("event") {
            Some(e) => e,
            None => return Ok(WebhookResult::Ignored),
        };

        let message = match event.get("message") {
            Some(m) => m,
            None => return Ok(WebhookResult::Ignored),
        };

        let msg_type = message
            .get("message_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if msg_type != "text" {
            tracing::debug!(msg_type, "ignoring non-text Feishu message");
            return Ok(WebhookResult::Ignored);
        }

        let message_id = message
            .get("message_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let chat_id = message
            .get("chat_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let sender_id = event
            .get("sender")
            .and_then(|s| s.get("sender_id"))
            .and_then(|s| s.get("open_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let content_str = message
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("{}");
        let text = serde_json::from_str::<serde_json::Value>(content_str)
            .ok()
            .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(String::from))
            .unwrap_or_default();

        if text.is_empty() {
            return Ok(WebhookResult::Ignored);
        }

        let chat_type = message
            .get("chat_type")
            .and_then(|v| v.as_str())
            .unwrap_or("p2p")
            .to_string();

        Ok(WebhookResult::Messages(vec![InboundMessage {
            channel_id: "feishu".to_string(),
            sender_id,
            chat_id,
            message_id,
            text,
            msg_type: msg_type.to_string(),
            chat_type,
            bot_mentioned: false,
            extra: event.clone(),
        }]))
    }

    async fn send_message(&self, msg: &OutboundMessage) -> anyhow::Result<serde_json::Value> {
        let receive_id_type = infer_receive_id_type(&msg.target_id, &msg.target_type);
        self.client
            .send_message(&msg.target_id, receive_id_type, &msg.text)
            .await
    }

    async fn reply_message(
        &self,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<serde_json::Value> {
        self.client.reply_message(message_id, text).await
    }

    async fn reply_streaming_placeholder(
        &self,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<serde_json::Value> {
        self.client.reply_card_message(message_id, text).await
    }

    async fn update_message(
        &self,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<serde_json::Value> {
        self.client.update_message(message_id, text).await
    }

    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        use crate::tools::{
            FeishuBitableListRecordsTool, FeishuCalendarListEventsTool, FeishuDocCreateTool,
            FeishuDocGetContentTool, FeishuSendImageTool, FeishuReplyImageTool,
            FeishuTaskCreateTool, FeishuTaskListTool,
        };
        vec![
            Arc::new(FeishuSendMessageTool::new(self.client.clone())),
            Arc::new(FeishuReplyMessageTool::new(self.client.clone())),
            Arc::new(FeishuGetChatMessagesTool::new(self.client.clone())),
            Arc::new(FeishuSendImageTool::new(self.client.clone())),
            Arc::new(FeishuReplyImageTool::new(self.client.clone())),
            Arc::new(FeishuTaskCreateTool::new(self.client.clone())),
            Arc::new(FeishuTaskListTool::new(self.client.clone())),
            Arc::new(FeishuBitableListRecordsTool::new(self.client.clone())),
            Arc::new(FeishuDocGetContentTool::new(self.client.clone())),
            Arc::new(FeishuDocCreateTool::new(self.client.clone())),
            Arc::new(FeishuCalendarListEventsTool::new(self.client.clone())),
        ]
    }

    async fn probe(&self) -> anyhow::Result<bool> {
        match self.client.get_tenant_token().await {
            Ok(_) => Ok(true),
            Err(e) => {
                tracing::warn!(error = %e, "Feishu probe failed");
                Ok(false)
            }
        }
    }

    async fn start(&self, inbound_tx: mpsc::UnboundedSender<InboundMessage>) -> anyhow::Result<()> {
        if self.config.connection_mode != "websocket" {
            tracing::info!("feishu channel: webhook mode, no background tasks to start");
            return Ok(());
        }

        tracing::info!(
            domain = %self.config.domain,
            "feishu channel: starting WebSocket long connection"
        );

        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let ws_client = Arc::new(ws::FeishuWsClient::new(
            &self.config.app_id,
            &self.config.app_secret,
            &self.config.domain,
            event_tx,
        )?);

        // Store reference for graceful shutdown
        {
            let mut guard = self.ws_client.lock().await;
            *guard = Some(Arc::clone(&ws_client));
        }

        let ws_client_clone = Arc::clone(&ws_client);
        tokio::spawn(async move {
            tracing::info!("feishu ws: background task started, connecting...");
            if let Err(e) = ws_client_clone.start().await {
                tracing::error!(error = %e, "feishu ws client start failed");
            }
        });

        let bot_open_id = self.get_bot_open_id().await;
        let reply_mode = self.config.reply_mode.clone();
        tracing::info!(reply_mode = %reply_mode, "feishu ws: event bridge configured");
        tokio::spawn(async move {
            ws::run_event_bridge(event_rx, inbound_tx, bot_open_id, reply_mode).await;
        });

        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        if self.config.connection_mode != "websocket" {
            return Ok(());
        }
        let ws = {
            let mut guard = self.ws_client.lock().await;
            guard.take()
        };
        if let Some(ws) = ws {
            tracing::info!("feishu channel: stopping WebSocket connection");
            ws.stop();
        }
        Ok(())
    }

    fn connection_mode(&self) -> &str {
        &self.config.connection_mode
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> FeishuPluginConfig {
        FeishuPluginConfig {
            app_id: "test".into(),
            app_secret: "secret".into(),
            verification_token: None,
            encrypt_key: None,
            agent_id: "main".into(),
            connection_mode: "webhook".into(),
            domain: DEFAULT_FEISHU_DOMAIN.into(),
            reply_mode: "mention_only".into(),
            user_access_token: None,
        }
    }

    #[test]
    fn plugin_meta() {
        let plugin = FeishuPlugin::new(test_config());
        assert_eq!(plugin.meta().id, "feishu");
        assert_eq!(plugin.meta().name, "Feishu");
        assert!(plugin.meta().aliases.contains(&"lark".to_string()));
    }

    #[test]
    fn plugin_capabilities() {
        let plugin = FeishuPlugin::new(test_config());
        let caps = plugin.capabilities();
        assert!(caps.direct_message);
        assert!(caps.group_chat);
        assert!(caps.media);
    }

    #[test]
    fn plugin_tools_count() {
        let plugin = FeishuPlugin::new(test_config());
        assert_eq!(plugin.tools().len(), 11);
    }

    #[test]
    fn plugin_connection_mode() {
        let mut cfg = test_config();
        cfg.connection_mode = "websocket".into();
        let plugin = FeishuPlugin::new(cfg);
        assert_eq!(plugin.connection_mode(), "websocket");
    }

    #[tokio::test]
    async fn handle_challenge() {
        let plugin = FeishuPlugin::new(test_config());
        let payload = serde_json::json!({
            "challenge": "abc123",
            "token": "test",
            "type": "url_verification"
        });
        let result = plugin.handle_webhook(payload).await.unwrap();
        match result {
            WebhookResult::Challenge(v) => {
                assert_eq!(v["challenge"], "abc123");
            }
            _ => panic!("expected Challenge"),
        }
    }

    #[tokio::test]
    async fn handle_text_message() {
        let plugin = FeishuPlugin::new(test_config());
        let payload = serde_json::json!({
            "header": {
                "event_type": "im.message.receive_v1",
                "token": ""
            },
            "event": {
                "sender": {
                    "sender_id": { "open_id": "ou_abc" }
                },
                "message": {
                    "message_id": "om_123",
                    "chat_id": "oc_456",
                    "message_type": "text",
                    "content": "{\"text\": \"hello\"}"
                }
            }
        });
        let result = plugin.handle_webhook(payload).await.unwrap();
        match result {
            WebhookResult::Messages(msgs) => {
                assert_eq!(msgs.len(), 1);
                assert_eq!(msgs[0].text, "hello");
                assert_eq!(msgs[0].chat_id, "oc_456");
                assert_eq!(msgs[0].sender_id, "ou_abc");
            }
            _ => panic!("expected Messages"),
        }
    }

    #[tokio::test]
    async fn handle_token_mismatch() {
        let mut cfg = test_config();
        cfg.verification_token = Some("expected_token".into());
        let plugin = FeishuPlugin::new(cfg);
        let payload = serde_json::json!({
            "header": { "event_type": "im.message.receive_v1", "token": "wrong_token" },
            "event": { "message": { "message_id": "om_1", "chat_id": "oc_1", "message_type": "text", "content": "{\"text\":\"hi\"}" } }
        });
        let raw_body = serde_json::to_vec(&payload).unwrap();
        let headers = BTreeMap::new();
        let result = plugin.verify_webhook(&headers, &raw_body).await;
        assert!(result.is_err(), "wrong token should be rejected by verify_webhook");
    }
}
