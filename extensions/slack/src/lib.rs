use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use fastclaw_core::channel::{
    ChannelCapabilities, ChannelMeta, ChannelPlugin, InboundMessage, OutboundMessage, WebhookResult,
};
use fastclaw_core::config::ChannelConfig;
use fastclaw_core::tool::Tool;

const SLACK_API: &str = "https://slack.com/api";

pub struct SlackPluginConfig {
    pub bot_token: String,
    pub signing_secret: String,
    pub app_id: Option<String>,
}

impl SlackPluginConfig {
    pub fn from_channel_config(config: &ChannelConfig) -> Option<Self> {
        let token = config.app_secret.clone()?;
        let signing = config.verification_token.clone().unwrap_or_default();
        Some(Self {
            bot_token: token,
            signing_secret: signing,
            app_id: config.app_id.clone(),
        })
    }
}

pub struct SlackPlugin {
    config: SlackPluginConfig,
    client: reqwest::Client,
    meta: ChannelMeta,
}

impl SlackPlugin {
    pub fn new(config: SlackPluginConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::builder()
                .user_agent("FastClaw/0.1.0")
                .build()
                .unwrap_or_default(),
            meta: ChannelMeta {
                id: "slack".to_string(),
                name: "Slack".to_string(),
                description: "Slack workspace bot via Events API".to_string(),
                aliases: vec![],
            },
        }
    }

    /// Verify Slack request signature using signing secret.
    pub fn verify_signature(&self, timestamp: &str, body: &str, signature: &str) -> bool {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let sig_basestring = format!("v0:{}:{}", timestamp, body);
        let mut mac = match Hmac::<Sha256>::new_from_slice(self.config.signing_secret.as_bytes()) {
            Ok(m) => m,
            Err(_) => return false,
        };
        mac.update(sig_basestring.as_bytes());
        let expected = format!("v0={}", hex::encode(mac.finalize().into_bytes()));
        expected == signature
    }
}

#[async_trait]
impl ChannelPlugin for SlackPlugin {
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
        if self.config.signing_secret.is_empty() {
            return Ok(());
        }
        let timestamp = headers
            .get("x-slack-request-timestamp")
            .ok_or_else(|| anyhow::anyhow!("missing X-Slack-Request-Timestamp header"))?;
        let signature = headers
            .get("x-slack-signature")
            .ok_or_else(|| anyhow::anyhow!("missing X-Slack-Signature header"))?;
        let body_str = std::str::from_utf8(raw_body).unwrap_or("");
        if !self.verify_signature(timestamp, body_str, signature) {
            anyhow::bail!("invalid Slack request signature");
        }
        Ok(())
    }

    async fn handle_webhook(&self, payload: serde_json::Value) -> anyhow::Result<WebhookResult> {
        // URL verification challenge
        if let Some(challenge) = payload.get("challenge").and_then(|v| v.as_str()) {
            return Ok(WebhookResult::Challenge(serde_json::json!({
                "challenge": challenge,
            })));
        }

        let event = match payload.get("event") {
            Some(e) => e,
            None => return Ok(WebhookResult::Ignored),
        };

        let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if event_type != "message" && event_type != "app_mention" {
            return Ok(WebhookResult::Ignored);
        }

        // Skip bot messages and message changes
        if event.get("bot_id").is_some() || event.get("subtype").is_some() {
            return Ok(WebhookResult::Ignored);
        }

        let channel = event
            .get("channel")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let user = event
            .get("user")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let text = event
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let ts = event
            .get("ts")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let thread_ts = event
            .get("thread_ts")
            .and_then(|v| v.as_str())
            .map(String::from);

        if text.is_empty() {
            return Ok(WebhookResult::Ignored);
        }

        let is_dm = channel.starts_with('D');

        Ok(WebhookResult::Messages(vec![InboundMessage {
            channel_id: "slack".to_string(),
            sender_id: user,
            chat_id: channel.clone(),
            message_id: format!("{}:{}", channel, ts),
            text,
            msg_type: "text".to_string(),
            chat_type: if is_dm {
                "p2p".to_string()
            } else {
                "group".to_string()
            },
            bot_mentioned: event_type == "app_mention" || is_dm,
            extra: serde_json::json!({
                "thread_ts": thread_ts,
            }),
        }]))
    }

    async fn send_message(&self, msg: &OutboundMessage) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .client
            .post(format!("{}/chat.postMessage", SLACK_API))
            .header("Authorization", format!("Bearer {}", self.config.bot_token))
            .json(&serde_json::json!({
                "channel": msg.target_id,
                "text": msg.text,
            }))
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;
        if json.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let err = json
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            anyhow::bail!("Slack API error: {err}");
        }
        Ok(json)
    }

    async fn reply_message(
        &self,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let parts: Vec<&str> = message_id.split(':').collect();
        let channel = parts.first().unwrap_or(&"");
        let ts = parts.get(1).unwrap_or(&"");

        let resp = self
            .client
            .post(format!("{}/chat.postMessage", SLACK_API))
            .header("Authorization", format!("Bearer {}", self.config.bot_token))
            .json(&serde_json::json!({
                "channel": channel,
                "text": text,
                "thread_ts": ts,
            }))
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;
        Ok(json)
    }

    async fn update_message(
        &self,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let parts: Vec<&str> = message_id.split(':').collect();
        let channel = parts.first().unwrap_or(&"");
        let ts = parts.get(1).unwrap_or(&"");

        let resp = self
            .client
            .post(format!("{}/chat.update", SLACK_API))
            .header("Authorization", format!("Bearer {}", self.config.bot_token))
            .json(&serde_json::json!({
                "channel": channel,
                "ts": ts,
                "text": text,
            }))
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;
        Ok(json)
    }

    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        Vec::new()
    }

    fn connection_mode(&self) -> &str {
        "webhook"
    }
}
