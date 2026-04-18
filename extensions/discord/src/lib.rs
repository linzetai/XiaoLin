use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use fastclaw_core::channel::{
    ChannelCapabilities, ChannelMeta, ChannelPlugin, InboundMessage, OutboundMessage, WebhookResult,
};
use fastclaw_core::config::ChannelConfig;
use fastclaw_core::tool::Tool;

const DISCORD_API: &str = "https://discord.com/api/v10";
const DISCORD_GATEWAY: &str = "wss://gateway.discord.gg/?v=10&encoding=json";

pub struct DiscordPluginConfig {
    pub bot_token: String,
    pub application_id: String,
}

impl DiscordPluginConfig {
    pub fn from_channel_config(config: &ChannelConfig) -> Option<Self> {
        let token = config.app_secret.clone()?;
        let app_id = config.app_id.clone()?;
        Some(Self {
            bot_token: token,
            application_id: app_id,
        })
    }
}

pub struct DiscordPlugin {
    config: DiscordPluginConfig,
    client: reqwest::Client,
    meta: ChannelMeta,
}

impl DiscordPlugin {
    pub fn new(config: DiscordPluginConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::builder()
                .user_agent("DiscordBot (FastClaw, 0.1.0)")
                .build()
                .unwrap_or_default(),
            meta: ChannelMeta {
                id: "discord".to_string(),
                name: "Discord".to_string(),
                description: "Discord bot channel via Gateway WebSocket".to_string(),
                aliases: vec![],
            },
        }
    }
}

#[async_trait]
impl ChannelPlugin for DiscordPlugin {
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
        let interaction_type = payload.get("type").and_then(|v| v.as_u64()).unwrap_or(0);

        if interaction_type == 1 {
            return Ok(WebhookResult::Challenge(serde_json::json!({ "type": 1 })));
        }

        Ok(WebhookResult::Ignored)
    }

    async fn send_message(&self, msg: &OutboundMessage) -> anyhow::Result<serde_json::Value> {
        let url = format!("{}/channels/{}/messages", DISCORD_API, msg.target_id);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bot {}", self.config.bot_token))
            .json(&serde_json::json!({
                "content": msg.text,
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Discord API error: {status} — {text}");
        }

        let json: serde_json::Value = resp.json().await?;
        Ok(json)
    }

    async fn reply_message(
        &self,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let parts: Vec<&str> = message_id.split(':').collect();
        let channel_id = parts.first().unwrap_or(&"");
        let msg_id = parts.get(1).unwrap_or(&"");

        let url = format!("{}/channels/{}/messages", DISCORD_API, channel_id);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bot {}", self.config.bot_token))
            .json(&serde_json::json!({
                "content": text,
                "message_reference": {
                    "message_id": msg_id,
                },
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
        use futures::{SinkExt, StreamExt};
        use tokio_tungstenite::connect_async;

        let token = self.config.bot_token.clone();

        tokio::spawn(async move {
            loop {
                match connect_async(DISCORD_GATEWAY).await {
                    Ok((ws, _)) => {
                        tracing::info!("discord gateway connected");

                        let identify = serde_json::json!({
                            "op": 2,
                            "d": {
                                "token": token,
                                "intents": 33281, // GUILDS | GUILD_MESSAGES | DIRECT_MESSAGES | MESSAGE_CONTENT
                                "properties": {
                                    "os": "linux",
                                    "browser": "fastclaw",
                                    "device": "fastclaw"
                                }
                            }
                        });

                        use tokio_tungstenite::tungstenite::Message as WsMsg;
                        let (mut write, mut read) = ws.split();
                        if let Err(e) = write.send(WsMsg::Text(identify.to_string())).await {
                            tracing::error!(error = %e, "failed to send identify");
                            continue;
                        }

                        let write_mtx = Arc::new(tokio::sync::Mutex::new(write));
                        let last_seq = Arc::new(AtomicI64::new(-1));
                        let mut heartbeat: Option<tokio::task::JoinHandle<()>> = None;

                        while let Some(msg) = read.next().await {
                            let data = match msg {
                                Ok(WsMsg::Text(t)) => t,
                                Ok(WsMsg::Close(_)) => break,
                                Err(e) => {
                                    tracing::warn!(error = %e, "discord ws error");
                                    break;
                                }
                                _ => continue,
                            };

                            let payload: serde_json::Value = match serde_json::from_str(&data) {
                                Ok(v) => v,
                                Err(_) => continue,
                            };

                            if let Some(sv) = payload.get("s") {
                                if let Some(n) = sv.as_i64() {
                                    last_seq.store(n, Ordering::Relaxed);
                                } else if let Some(n) = sv.as_u64() {
                                    last_seq.store(n as i64, Ordering::Relaxed);
                                }
                            }

                            let op = payload.get("op").and_then(|v| v.as_u64()).unwrap_or(0);
                            let event_name =
                                payload.get("t").and_then(|v| v.as_str()).unwrap_or("");

                            if op == 11 {
                                tracing::debug!("discord gateway heartbeat ack");
                                continue;
                            }

                            if op == 10 {
                                let interval = payload
                                    .get("d")
                                    .and_then(|d| d.get("heartbeat_interval"))
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(41250);

                                if let Some(h) = heartbeat.take() {
                                    h.abort();
                                }

                                let wm = Arc::clone(&write_mtx);
                                let seq = Arc::clone(&last_seq);
                                heartbeat = Some(tokio::spawn(async move {
                                    loop {
                                        tokio::time::sleep(Duration::from_millis(interval)).await;
                                        let d = match seq.load(Ordering::Relaxed) {
                                            -1 => serde_json::Value::Null,
                                            n => serde_json::json!(n),
                                        };
                                        let body =
                                            serde_json::json!({ "op": 1, "d": d }).to_string();
                                        let mut w = wm.lock().await;
                                        if w.send(WsMsg::Text(body)).await.is_err() {
                                            break;
                                        }
                                    }
                                }));
                                continue;
                            }

                            if event_name == "MESSAGE_CREATE" {
                                if let Some(d) = payload.get("d") {
                                    let is_bot = d
                                        .get("author")
                                        .and_then(|a| a.get("bot"))
                                        .and_then(|v| v.as_bool())
                                        .unwrap_or(false);
                                    if is_bot {
                                        continue;
                                    }

                                    let channel_id = d
                                        .get("channel_id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let sender_id = d
                                        .get("author")
                                        .and_then(|a| a.get("id"))
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let message_id = d
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let text = d
                                        .get("content")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();

                                    if !text.is_empty() {
                                        let _ = inbound_tx.send(InboundMessage {
                                            channel_id: "discord".to_string(),
                                            sender_id,
                                            chat_id: channel_id.clone(),
                                            message_id: format!("{}:{}", channel_id, message_id),
                                            text,
                                            msg_type: "text".to_string(),
                                            chat_type: "group".to_string(),
                                            bot_mentioned: false,
                                            extra: serde_json::json!({}),
                                        });
                                    }
                                }
                            }
                        }

                        if let Some(h) = heartbeat {
                            h.abort();
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "discord gateway connection failed");
                    }
                }
                tracing::info!("discord gateway reconnecting in 5s...");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        });

        Ok(())
    }

    fn connection_mode(&self) -> &str {
        "websocket"
    }
}
