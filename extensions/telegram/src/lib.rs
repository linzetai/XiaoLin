use std::sync::Arc;

use async_trait::async_trait;
use fastclaw_core::channel::{
    ChannelCapabilities, ChannelMeta, ChannelPlugin, InboundMessage, OutboundMessage, WebhookResult,
};
use fastclaw_core::config::ChannelConfig;
use fastclaw_core::tool::Tool;

pub struct TelegramPluginConfig {
    pub bot_token: String,
}

impl TelegramPluginConfig {
    pub fn from_channel_config(config: &ChannelConfig) -> Option<Self> {
        let token = config.app_secret.clone()?;
        Some(Self { bot_token: token })
    }
}

pub struct TelegramPlugin {
    config: TelegramPluginConfig,
    client: reqwest::Client,
    meta: ChannelMeta,
}

impl TelegramPlugin {
    pub fn new(config: TelegramPluginConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::builder()
                .user_agent("FastClaw/0.1.0")
                .build()
                .unwrap_or_default(),
            meta: ChannelMeta {
                id: "telegram".to_string(),
                name: "Telegram".to_string(),
                description: "Telegram Bot channel via Bot API".to_string(),
                aliases: vec!["tg".to_string()],
            },
        }
    }

    fn api_url(&self, method: &str) -> String {
        format!(
            "https://api.telegram.org/bot{}/{}",
            self.config.bot_token, method
        )
    }
}

#[async_trait]
impl ChannelPlugin for TelegramPlugin {
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
            streaming: false,
        }
    }

    async fn handle_webhook(&self, payload: serde_json::Value) -> anyhow::Result<WebhookResult> {
        let message = match payload
            .get("message")
            .or_else(|| payload.get("edited_message"))
        {
            Some(m) => m,
            None => return Ok(WebhookResult::Ignored),
        };

        let chat_id = message
            .get("chat")
            .and_then(|c| c.get("id"))
            .and_then(|v| v.as_i64())
            .map(|v| v.to_string())
            .unwrap_or_default();

        let chat_type = message
            .get("chat")
            .and_then(|c| c.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("private");

        let sender_id = message
            .get("from")
            .and_then(|f| f.get("id"))
            .and_then(|v| v.as_i64())
            .map(|v| v.to_string())
            .unwrap_or_default();

        let message_id = message
            .get("message_id")
            .and_then(|v| v.as_i64())
            .map(|v| v.to_string())
            .unwrap_or_default();

        let text = message
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if text.is_empty() {
            return Ok(WebhookResult::Ignored);
        }

        let bot_mentioned = text.starts_with('/')
            || message
                .get("entities")
                .and_then(|e| e.as_array())
                .map(|arr| {
                    arr.iter()
                        .any(|e| e.get("type").and_then(|v| v.as_str()) == Some("mention"))
                })
                .unwrap_or(false);

        let is_private = chat_type == "private";

        Ok(WebhookResult::Messages(vec![InboundMessage {
            channel_id: "telegram".to_string(),
            sender_id,
            chat_id,
            message_id,
            text,
            msg_type: "text".to_string(),
            chat_type: if is_private {
                "p2p".to_string()
            } else {
                "group".to_string()
            },
            bot_mentioned: bot_mentioned || is_private,
            extra: serde_json::json!({}),
        }]))
    }

    async fn send_message(&self, msg: &OutboundMessage) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .client
            .post(self.api_url("sendMessage"))
            .json(&serde_json::json!({
                "chat_id": msg.target_id,
                "text": msg.text,
                "parse_mode": "Markdown",
            }))
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if !status.is_success() {
            anyhow::bail!("Telegram API error ({}): {}", status, json);
        }
        Ok(json)
    }

    async fn reply_message(
        &self,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let msg_parts: Vec<&str> = message_id.split(':').collect();
        let (chat_id, msg_id) = if msg_parts.len() == 2 {
            (msg_parts[0], msg_parts[1])
        } else {
            anyhow::bail!("invalid message_id format, expected 'chat_id:message_id'");
        };

        let resp = self
            .client
            .post(self.api_url("sendMessage"))
            .json(&serde_json::json!({
                "chat_id": chat_id,
                "text": text,
                "reply_to_message_id": msg_id.parse::<i64>().unwrap_or(0),
                "parse_mode": "Markdown",
            }))
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;
        Ok(json)
    }

    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        Vec::new()
    }

    async fn start(
        &self,
        inbound_tx: tokio::sync::mpsc::UnboundedSender<InboundMessage>,
    ) -> anyhow::Result<()> {
        let token = self.config.bot_token.clone();
        let client = self.client.clone();

        tokio::spawn(async move {
            let mut offset: i64 = 0;
            loop {
                let url = format!(
                    "https://api.telegram.org/bot{}/getUpdates?offset={}&timeout=30",
                    token, offset
                );
                match client.get(&url).send().await {
                    Ok(resp) => {
                        if let Ok(json) = resp.json::<serde_json::Value>().await {
                            if let Some(updates) = json.get("result").and_then(|v| v.as_array()) {
                                for update in updates {
                                    if let Some(uid) =
                                        update.get("update_id").and_then(|v| v.as_i64())
                                    {
                                        offset = uid + 1;
                                    }
                                    if let Some(message) = update.get("message") {
                                        let chat_id = message
                                            .get("chat")
                                            .and_then(|c| c.get("id"))
                                            .and_then(|v| v.as_i64())
                                            .map(|v| v.to_string())
                                            .unwrap_or_default();
                                        let sender_id = message
                                            .get("from")
                                            .and_then(|f| f.get("id"))
                                            .and_then(|v| v.as_i64())
                                            .map(|v| v.to_string())
                                            .unwrap_or_default();
                                        let message_id = message
                                            .get("message_id")
                                            .and_then(|v| v.as_i64())
                                            .map(|v| v.to_string())
                                            .unwrap_or_default();
                                        let text = message
                                            .get("text")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string();
                                        let chat_type = message
                                            .get("chat")
                                            .and_then(|c| c.get("type"))
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("private");

                                        if !text.is_empty() {
                                            let _ = inbound_tx.send(InboundMessage {
                                                channel_id: "telegram".to_string(),
                                                sender_id,
                                                chat_id,
                                                message_id,
                                                text,
                                                msg_type: "text".to_string(),
                                                chat_type: if chat_type == "private" {
                                                    "p2p".to_string()
                                                } else {
                                                    "group".to_string()
                                                },
                                                bot_mentioned: chat_type == "private",
                                                extra: serde_json::json!({}),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "telegram polling error, retrying...");
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                }
            }
        });

        Ok(())
    }

    fn connection_mode(&self) -> &str {
        "long-polling"
    }
}
