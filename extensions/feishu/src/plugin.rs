use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex, OnceCell};
use tokio_util::sync::CancellationToken;

use xiaolin_core::channel::{
    ChannelCapabilities, ChannelMeta, ChannelPlugin, InboundMessage, OutboundMessage, WebhookResult,
};
use xiaolin_core::tool::Tool;

use crate::client::FeishuClient;
use crate::messaging::inbound::{MessageDedup, parse_im_mentions_from_message};
use crate::messaging::inbound::parse::extract_inbound_text;
#[cfg(test)]
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
    /// When true, allow webhooks without a configured verification_token (dev/test only).
    #[serde(default)]
    pub allow_insecure_webhook: bool,
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
    fn decrypt_secret_field(value: &str, field: &str) -> String {
        match xiaolin_core::credential_crypto::decrypt_credential(value) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(field, error = %e, "failed to decrypt feishu channel secret");
                String::new()
            }
        }
    }

    fn decrypt_optional_secret(value: &Option<String>, field: &str) -> Option<String> {
        value
            .as_ref()
            .map(|s| Self::decrypt_secret_field(s, field))
            .filter(|s| !s.is_empty())
    }

    /// Create from JSON channel config. All fields must be provided in the config file.
    pub fn from_channel_config(cfg: &xiaolin_core::config::ChannelConfig) -> Option<Self> {
        let app_id = cfg.app_id.clone()?;
        let app_secret = cfg
            .app_secret
            .as_ref()
            .map(|s| Self::decrypt_secret_field(s, "app_secret"))?;
        Some(Self {
            app_id,
            app_secret,
            verification_token: Self::decrypt_optional_secret(&cfg.verification_token, "verification_token"),
            encrypt_key: Self::decrypt_optional_secret(&cfg.encrypt_key, "encrypt_key"),
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
            user_access_token: Self::decrypt_optional_secret(&cfg.user_access_token, "user_access_token"),
            allow_insecure_webhook: cfg.allow_insecure_webhook.unwrap_or(false),
        })
    }
}

/// XiaoLin Feishu Channel Plugin — bridges Feishu/Lark messaging into the
/// XiaoLin agent ecosystem.
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
    /// Cached bot open_id resolved once at startup (OnceCell coalesces concurrent fetches).
    bot_open_id_cache: Arc<OnceCell<String>>,
    /// Cancels the event bridge task on stop
    ws_bridge_cancel: Arc<Mutex<Option<CancellationToken>>>,
    dedup: Arc<Mutex<MessageDedup>>,
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
            bot_open_id_cache: Arc::new(OnceCell::new()),
            ws_bridge_cancel: Arc::new(Mutex::new(None)),
            dedup: Arc::new(Mutex::new(MessageDedup::new(Duration::from_secs(300)))),
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

    async fn accept_inbound_message(&self, message_id: &str) -> bool {
        let mut dedup = self.dedup.lock().await;
        if dedup.check(message_id) {
            true
        } else {
            tracing::debug!(message_id, "feishu: duplicate message skipped");
            false
        }
    }

    /// IM core tools used internally by the channel plugin (send/reply/image).
    /// These are NOT exposed to the LLM as the gateway handles messaging.
    #[cfg(test)]
    fn im_core_tools(&self) -> Vec<Arc<dyn Tool>> {
        use crate::tools::{FeishuReplyImageTool, FeishuSendImageTool};
        vec![
            Arc::new(FeishuSendMessageTool::new(self.client.clone())),
            Arc::new(FeishuReplyMessageTool::new(self.client.clone())),
            Arc::new(FeishuGetChatMessagesTool::new(self.client.clone())),
            Arc::new(FeishuSendImageTool::new(self.client.clone())),
            Arc::new(FeishuReplyImageTool::new(self.client.clone())),
        ]
    }

    /// Extension tools to expose to the LLM. Matches OpenClaw's pattern:
    /// doc, chat, wiki, drive, perm, bitable, scopes, calendar, task,
    /// plus IM-enhanced tools (rich text, file, edit, forward, delete, reaction, pin).
    pub fn llm_tools(&self) -> Vec<Arc<dyn Tool>> {
        use crate::tools::{
            FeishuAppScopesTool, FeishuBitableCreateAppTool, FeishuBitableCreateFieldTool,
            FeishuBitableCreateRecordTool, FeishuBitableGetMetaTool, FeishuBitableGetRecordTool,
            FeishuBitableListFieldsTool, FeishuBitableListRecordsTool,
            FeishuBitableUpdateRecordTool, FeishuCalendarListEventsTool, FeishuChatTool,
            FeishuDeleteMessageTool, FeishuDocCreateTool, FeishuDocGetContentTool, FeishuDocTool,
            FeishuDriveTool, FeishuEditMessageTool, FeishuForwardMessageTool, FeishuGetMessageTool,
            FeishuPermTool, FeishuPinTool, FeishuReactionTool, FeishuSendFileTool,
            FeishuSendRichTextTool, FeishuTaskCreateTool, FeishuTaskListTool, FeishuWikiTool,
        };
        vec![
            // IM enhanced (proactive operations the LLM can trigger)
            Arc::new(FeishuSendRichTextTool::new(self.client.clone())),
            Arc::new(FeishuSendFileTool::new(self.client.clone())),
            Arc::new(FeishuEditMessageTool::new(self.client.clone())),
            Arc::new(FeishuGetMessageTool::new(self.client.clone())),
            Arc::new(FeishuForwardMessageTool::new(self.client.clone())),
            Arc::new(FeishuDeleteMessageTool::new(self.client.clone())),
            Arc::new(FeishuReactionTool::new(self.client.clone())),
            Arc::new(FeishuPinTool::new(self.client.clone())),
            // Productivity
            Arc::new(FeishuTaskCreateTool::new(self.client.clone())),
            Arc::new(FeishuTaskListTool::new(self.client.clone())),
            // Bitable
            Arc::new(FeishuBitableGetMetaTool::new(self.client.clone())),
            Arc::new(FeishuBitableListFieldsTool::new(self.client.clone())),
            Arc::new(FeishuBitableListRecordsTool::new(self.client.clone())),
            Arc::new(FeishuBitableGetRecordTool::new(self.client.clone())),
            Arc::new(FeishuBitableCreateRecordTool::new(self.client.clone())),
            Arc::new(FeishuBitableUpdateRecordTool::new(self.client.clone())),
            Arc::new(FeishuBitableCreateAppTool::new(self.client.clone())),
            Arc::new(FeishuBitableCreateFieldTool::new(self.client.clone())),
            // Document (legacy + unified)
            Arc::new(FeishuDocGetContentTool::new(self.client.clone())),
            Arc::new(FeishuDocCreateTool::new(self.client.clone())),
            Arc::new(FeishuDocTool::new(self.client.clone())),
            // Wiki / Drive / Perm / Chat / Scopes
            Arc::new(FeishuWikiTool::new(self.client.clone())),
            Arc::new(FeishuDriveTool::new(self.client.clone())),
            Arc::new(FeishuPermTool::new(self.client.clone())),
            Arc::new(FeishuChatTool::new(self.client.clone())),
            Arc::new(FeishuAppScopesTool::new(self.client.clone())),
            // Calendar
            Arc::new(FeishuCalendarListEventsTool::new(self.client.clone())),
        ]
    }

    async fn get_bot_open_id(&self) -> Option<String> {
        if let Some(id) = self.bot_open_id_cache.get() {
            return Some(id.clone());
        }
        match self
            .bot_open_id_cache
            .get_or_try_init(|| async { self.client.get_bot_open_id().await.ok_or(()) })
            .await
        {
            Ok(id) => Some(id.clone()),
            Err(()) => None,
        }
    }

    fn verify_token(&self, token: &str) -> bool {
        match &self.config.verification_token {
            Some(vt) if !vt.is_empty() => vt == token,
            _ => {
                if self.config.allow_insecure_webhook {
                    tracing::warn!(
                        "feishu: verification_token not configured, allowing webhook (allow_insecure_webhook=true)"
                    );
                    true
                } else {
                    tracing::warn!(
                        "feishu: verification_token not configured, rejecting webhook (fail-closed)"
                    );
                    false
                }
            }
        }
    }

    /// Parse a `card.action.trigger` webhook event into a card_action InboundMessage.
    async fn handle_card_action(
        &self,
        payload: &serde_json::Value,
    ) -> anyhow::Result<WebhookResult> {
        let event = payload.get("event").unwrap_or(payload);

        let action = event.get("action").unwrap_or(&serde_json::Value::Null);
        let value = action.get("value").unwrap_or(&serde_json::Value::Null);

        let request_id = value
            .get("message_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let option_id = value
            .get("option_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let action_type = value
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if request_id.is_empty() {
            tracing::debug!("card action without request_id (message_id), ignoring");
            return Ok(WebhookResult::Ignored);
        }
        if !self.accept_inbound_message(&request_id).await {
            return Ok(WebhookResult::Ignored);
        }

        let operator_id = event
            .get("operator")
            .and_then(|o| o.get("open_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        tracing::info!(
            request_id = %request_id,
            option_id = %option_id,
            action_type = %action_type,
            operator = %operator_id,
            "feishu: card action callback received"
        );

        let extra = serde_json::json!({
            "_card_action": true,
            "request_id": request_id,
            "option_id": option_id,
            "action_type": action_type,
        });

        Ok(WebhookResult::Messages(vec![InboundMessage {
            channel_id: "feishu".to_string(),
            account_id: None,
            sender_id: operator_id,
            chat_id: String::new(),
            message_id: request_id,
            text: option_id,
            msg_type: "card_action".to_string(),
            chat_type: String::new(),
            bot_mentioned: false,
            extra,
            attachments: vec![],
        }]))
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
        headers: &BTreeMap<String, String>,
        raw_body: &[u8],
    ) -> anyhow::Result<()> {
        use crate::webhook_security::{parse_webhook_payload, verify_lark_webhook_headers};

        verify_lark_webhook_headers(headers, self.config.encrypt_key.as_deref(), raw_body)?;

        let payload = parse_webhook_payload(self.config.encrypt_key.as_deref(), raw_body)?;
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

        // Handle interactive card callbacks (ask_question button clicks)
        if event_type == "card.action.trigger" {
            return self.handle_card_action(&payload).await;
        }

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
        let message_id = message
            .get("message_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if message_id.is_empty() {
            return Ok(WebhookResult::Ignored);
        }
        if !self.accept_inbound_message(&message_id).await {
            return Ok(WebhookResult::Ignored);
        }

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
        let mut text = extract_inbound_text(msg_type, content_str);
        let bot_open_id = self.get_bot_open_id().await;
        let (bot_mentioned, stripped_text) =
            parse_im_mentions_from_message(message, text, bot_open_id.as_deref());
        text = stripped_text;

        if text.is_empty() {
            return Ok(WebhookResult::Ignored);
        }

        let chat_type = message
            .get("chat_type")
            .and_then(|v| v.as_str())
            .unwrap_or("p2p")
            .to_string();

        if chat_type == "group"
            && self.config.reply_mode == "mention_only"
            && !bot_mentioned
        {
            tracing::debug!(
                chat_id = %chat_id,
                message_id = %message_id,
                "feishu webhook: group message without @mention, skipped"
            );
            return Ok(WebhookResult::Ignored);
        }

        Ok(WebhookResult::Messages(vec![InboundMessage {
            channel_id: "feishu".to_string(),
            account_id: None,
            sender_id,
            chat_id,
            message_id,
            text,
            msg_type: msg_type.to_string(),
            chat_type,
            bot_mentioned,
            extra: event.clone(),
            attachments: vec![],
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
        self.llm_tools()
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
        let connect_cancel = ws_client.cancellation_token();
        tokio::spawn(async move {
            let mut delay = Duration::from_secs(1);
            const MAX_DELAY: Duration = Duration::from_secs(60);
            loop {
                if connect_cancel.is_cancelled() {
                    break;
                }
                tracing::info!("feishu ws: background task started, connecting...");
                match Arc::clone(&ws_client_clone).start().await {
                    Ok(()) => break,
                    Err(e) => {
                        tracing::error!(
                            error = %e,
                            retry_in_secs = delay.as_secs(),
                            "feishu ws client start failed"
                        );
                        tokio::time::sleep(delay).await;
                        delay = (delay * 2).min(MAX_DELAY);
                    }
                }
            }
        });

        let bot_open_id = self.get_bot_open_id().await;
        let reply_mode = self.config.reply_mode.clone();
        let dedup = Arc::clone(&self.dedup);
        let bridge_cancel = CancellationToken::new();
        {
            let mut guard = self.ws_bridge_cancel.lock().await;
            *guard = Some(bridge_cancel.clone());
        }
        tracing::info!(reply_mode = %reply_mode, "feishu ws: event bridge configured");
        tokio::spawn(async move {
            ws::run_event_bridge(
                event_rx,
                inbound_tx,
                bot_open_id,
                reply_mode,
                dedup,
                bridge_cancel,
            )
            .await;
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
        if let Some(cancel) = self.ws_bridge_cancel.lock().await.take() {
            cancel.cancel();
        }
        Ok(())
    }

    fn connection_mode(&self) -> &str {
        &self.config.connection_mode
    }

    async fn send_interactive_card(
        &self,
        target_id: &str,
        target_type: &str,
        card: &serde_json::Value,
    ) -> anyhow::Result<String> {
        let receive_id_type = infer_receive_id_type(target_id, target_type);
        let result = self
            .client
            .send_card(target_id, receive_id_type, card)
            .await?;

        let message_id = result
            .get("message_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                tracing::warn!("No message_id in card response, using placeholder");
                format!("card_{}", uuid::Uuid::new_v4())
            });

        Ok(message_id)
    }

    async fn update_interactive_card(
        &self,
        message_id: &str,
        card: &serde_json::Value,
    ) -> anyhow::Result<()> {
        self.client.update_card_message(message_id, card).await?;
        Ok(())
    }

    fn supports_interactive_questions(&self) -> bool {
        true
    }

    fn parse_webhook_payload(&self, raw_body: &[u8]) -> anyhow::Result<serde_json::Value> {
        crate::webhook_security::parse_webhook_payload(
            self.config.encrypt_key.as_deref(),
            raw_body,
        )
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
            allow_insecure_webhook: false,
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
        assert_eq!(plugin.tools().len(), 27);
        assert_eq!(plugin.im_core_tools().len(), 5);
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
    async fn handle_image_message() {
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
                    "message_id": "om_img",
                    "chat_id": "oc_456",
                    "message_type": "image",
                    "content": "{\"image_key\":\"img_123\"}"
                }
            }
        });
        let result = plugin.handle_webhook(payload).await.unwrap();
        match result {
            WebhookResult::Messages(msgs) => {
                assert_eq!(msgs.len(), 1);
                assert_eq!(msgs[0].text, "[图片]");
                assert_eq!(msgs[0].msg_type, "image");
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
        assert!(
            result.is_err(),
            "wrong token should be rejected by verify_webhook"
        );
    }
}
