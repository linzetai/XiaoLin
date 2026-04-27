use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use fastclaw_core::channel::{
    ChannelCapabilities, ChannelMeta, ChannelPlugin, InboundMessage, OutboundMessage, WebhookResult,
};
use fastclaw_core::config::ChannelConfig;
use fastclaw_core::tool::Tool;

const GRAPH_MESSAGES: &str = "https://graph.facebook.com/v19.0";

pub struct WhatsAppPluginConfig {
    pub phone_number_id: String,
    pub access_token: String,
    pub verify_token: String,
    /// Meta App Secret used to verify X-Hub-Signature-256 on inbound webhooks.
    /// When empty, webhook signature verification is skipped.
    pub app_secret: String,
    /// When set (e.g. to a local mock server origin), outbound calls use this instead of the
    /// default Graph API base URL.
    pub graph_api_base: Option<String>,
}

impl WhatsAppPluginConfig {
    pub fn from_channel_config(config: &ChannelConfig) -> Option<Self> {
        let phone_number_id = config.app_id.clone()?;
        let access_token = config.app_secret.clone()?;
        let verify_token = config.verification_token.clone()?;
        let app_secret = config.encrypt_key.clone().unwrap_or_default();
        Some(Self {
            phone_number_id,
            access_token,
            verify_token,
            app_secret,
            graph_api_base: None,
        })
    }
}

pub struct WhatsAppPlugin {
    config: WhatsAppPluginConfig,
    client: reqwest::Client,
    meta: ChannelMeta,
}

impl WhatsAppPlugin {
    pub fn new(config: WhatsAppPluginConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::builder()
                .user_agent("FastClaw/0.1.0")
                .build()
                .unwrap_or_default(),
            meta: ChannelMeta {
                id: "whatsapp".to_string(),
                name: "WhatsApp".to_string(),
                description: "WhatsApp Business Cloud API".to_string(),
                aliases: vec!["wa".to_string()],
            },
        }
    }

    fn graph_base(&self) -> &str {
        self.config
            .graph_api_base
            .as_deref()
            .unwrap_or(GRAPH_MESSAGES)
    }

    fn messages_url(&self) -> String {
        format!(
            "{}/{}/messages",
            self.graph_base(),
            self.config.phone_number_id
        )
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.config.access_token)
    }
}

#[async_trait]
impl ChannelPlugin for WhatsAppPlugin {
    fn meta(&self) -> &ChannelMeta {
        &self.meta
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            direct_message: true,
            group_chat: true,
            media: true,
            reactions: false,
            threads: true,
            streaming: false,
        }
    }

    async fn verify_webhook(
        &self,
        headers: &BTreeMap<String, String>,
        raw_body: &[u8],
    ) -> anyhow::Result<()> {
        if self.config.app_secret.is_empty() {
            return Ok(());
        }
        let sig_header = match headers.get("x-hub-signature-256") {
            Some(s) => s,
            None => return Ok(()),
        };
        let expected_prefix = "sha256=";
        let hex_sig = sig_header
            .strip_prefix(expected_prefix)
            .ok_or_else(|| anyhow::anyhow!("malformed X-Hub-Signature-256 header"))?;

        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let mut mac = Hmac::<Sha256>::new_from_slice(self.config.app_secret.as_bytes())
            .map_err(|_| anyhow::anyhow!("invalid HMAC key"))?;
        mac.update(raw_body);
        let computed = hex::encode(mac.finalize().into_bytes());
        if !constant_time_eq::constant_time_eq(computed.as_bytes(), hex_sig.as_bytes()) {
            anyhow::bail!("invalid X-Hub-Signature-256");
        }
        Ok(())
    }

    async fn handle_webhook(&self, payload: serde_json::Value) -> anyhow::Result<WebhookResult> {
        if let Some(hub) = payload.get("hub").and_then(|v| v.as_object()) {
            if let (Some(mode), Some(challenge), Some(vt)) = (
                hub.get("mode").and_then(|v| v.as_str()),
                hub.get("challenge").and_then(|v| v.as_str()),
                hub.get("verify_token").and_then(|v| v.as_str()),
            ) {
                if mode == "subscribe" && vt == self.config.verify_token.as_str() {
                    return Ok(WebhookResult::Challenge(serde_json::json!({
                        "challenge": challenge,
                    })));
                }
            }
        }

        if payload
            .get("object")
            .and_then(|v| v.as_str())
            != Some("whatsapp_business_account")
        {
            return Ok(WebhookResult::Ignored);
        }

        let mut out = Vec::new();
        let entries = match payload.get("entry").and_then(|e| e.as_array()) {
            Some(e) => e,
            None => return Ok(WebhookResult::Ignored),
        };

        for entry in entries {
            let changes = match entry.get("changes").and_then(|c| c.as_array()) {
                Some(c) => c,
                None => continue,
            };
            for ch in changes {
                let value = match ch.get("value").and_then(|v| v.as_object()) {
                    Some(v) => v,
                    None => continue,
                };
                let messages = match value.get("messages").and_then(|m| m.as_array()) {
                    Some(m) => m,
                    None => continue,
                };
                for m in messages {
                    let from = m
                        .get("from")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let wamid = m
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let msg_type = m
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("text")
                        .to_string();
                    let text = if msg_type == "text" {
                        m.get("text")
                            .and_then(|t| t.get("body"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string()
                    } else if msg_type == "button" {
                        m.get("button")
                            .and_then(|b| b.get("text"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string()
                    } else {
                        String::new()
                    };
                    if text.is_empty() {
                        continue;
                    }
                    let phone_number_id = value
                        .get("metadata")
                        .and_then(|md| md.get("phone_number_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or(&self.config.phone_number_id)
                        .to_string();
                    let sender_id = from.clone();
                    let chat_id = from.clone();
                    let message_id = format!("{chat_id}:{wamid}");
                    out.push(InboundMessage {
                        channel_id: "whatsapp".to_string(),
                        sender_id,
                        chat_id,
                        message_id,
                        text,
                        msg_type: "text".to_string(),
                        chat_type: "p2p".to_string(),
                        bot_mentioned: true,
                        extra: serde_json::json!({
                            "phone_number_id": phone_number_id,
                            "whatsapp_message_id": wamid,
                        }),
                    });
                }
            }
        }

        if out.is_empty() {
            Ok(WebhookResult::Ignored)
        } else {
            Ok(WebhookResult::Messages(out))
        }
    }

    async fn send_message(&self, msg: &OutboundMessage) -> anyhow::Result<serde_json::Value> {
        let body = serde_json::json!({
            "messaging_product": "whatsapp",
            "to": msg.target_id,
            "type": "text",
            "text": { "body": msg.text },
        });
        let resp = self
            .client
            .post(self.messages_url())
            .header("Authorization", self.auth_header())
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if !status.is_success() {
            anyhow::bail!("WhatsApp API error ({}): {}", status, json);
        }
        Ok(json)
    }

    async fn reply_message(
        &self,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let (to, wamid) = parse_whatsapp_message_id(message_id)?;
        let body = serde_json::json!({
            "messaging_product": "whatsapp",
            "to": to,
            "type": "text",
            "text": { "body": text },
            "context": { "message_id": wamid },
        });
        let resp = self
            .client
            .post(self.messages_url())
            .header("Authorization", self.auth_header())
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if !status.is_success() {
            anyhow::bail!("WhatsApp API error ({}): {}", status, json);
        }
        Ok(json)
    }

    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        Vec::new()
    }

    async fn start(
        &self,
        _inbound_tx: tokio::sync::mpsc::UnboundedSender<InboundMessage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn connection_mode(&self) -> &str {
        "webhook"
    }
}

fn parse_whatsapp_message_id(message_id: &str) -> anyhow::Result<(&str, &str)> {
    let idx = message_id
        .rfind(':')
        .ok_or_else(|| anyhow::anyhow!("invalid message_id, expected 'E164:wamid'"))?;
    let (to, wamid) = message_id.split_at(idx);
    let wamid = wamid.trim_start_matches(':');
    if to.is_empty() || wamid.is_empty() {
        anyhow::bail!("invalid message_id");
    }
    Ok((to, wamid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_from_channel() {
        let mut c = ChannelConfig::default();
        assert!(WhatsAppPluginConfig::from_channel_config(&c).is_none());
        c.app_id = Some("123".into());
        c.app_secret = Some("token".into());
        c.verification_token = Some("vt".into());
        let w = WhatsAppPluginConfig::from_channel_config(&c).unwrap();
        assert_eq!(w.phone_number_id, "123");
        assert_eq!(w.access_token, "token");
        assert_eq!(w.verify_token, "vt");
        assert!(w.graph_api_base.is_none());
    }

    #[tokio::test]
    async fn webhook_hub_challenge() {
        let cfg = WhatsAppPluginConfig {
            phone_number_id: "1".into(),
            access_token: "t".into(),
            verify_token: "secret".into(),
            app_secret: String::new(),
            graph_api_base: None,
        };
        let p = WhatsAppPlugin::new(cfg);
        let payload = serde_json::json!({
            "hub": {
                "mode": "subscribe",
                "challenge": "abc123",
                "verify_token": "secret"
            }
        });
        let r = p.handle_webhook(payload).await.unwrap();
        match r {
            WebhookResult::Challenge(v) => {
                assert_eq!(v["challenge"], "abc123");
            }
            _ => panic!("expected challenge"),
        }
    }

    #[tokio::test]
    async fn webhook_message_event() {
        let cfg = WhatsAppPluginConfig {
            phone_number_id: "pnid".into(),
            access_token: "t".into(),
            verify_token: "v".into(),
            app_secret: String::new(),
            graph_api_base: None,
        };
        let p = WhatsAppPlugin::new(cfg);
        let payload = serde_json::json!({
            "object": "whatsapp_business_account",
            "entry": [{
                "changes": [{
                    "field": "messages",
                    "value": {
                        "messaging_product": "whatsapp",
                        "metadata": { "phone_number_id": "pnid" },
                        "messages": [{
                            "from": "15550001111",
                            "id": "wamid.xxx",
                            "timestamp": "1",
                            "type": "text",
                            "text": { "body": "hello" }
                        }]
                    }
                }]
            }]
        });
        let r = p.handle_webhook(payload).await.unwrap();
        match r {
            WebhookResult::Messages(msgs) => {
                assert_eq!(msgs.len(), 1);
                assert_eq!(msgs[0].text, "hello");
                assert_eq!(msgs[0].sender_id, "15550001111");
                assert_eq!(msgs[0].message_id, "15550001111:wamid.xxx");
            }
            _ => panic!("expected messages"),
        }
    }

    #[test]
    fn parse_msg_id() {
        let mid = "15550001111:wamid.HBgM";
        let (a, b) = parse_whatsapp_message_id(mid).unwrap();
        assert_eq!(a, "15550001111");
        assert_eq!(b, "wamid.HBgM");
    }

    #[tokio::test]
    async fn whatsapp_webhook_rejects_bad_verify_token() {
        let cfg = WhatsAppPluginConfig {
            phone_number_id: "1".into(),
            access_token: "t".into(),
            verify_token: "correct".into(),
            app_secret: String::new(),
            graph_api_base: None,
        };
        let p = WhatsAppPlugin::new(cfg);
        let payload = serde_json::json!({
            "hub": {
                "mode": "subscribe",
                "challenge": "should-not-return",
                "verify_token": "wrong"
            }
        });
        let r = p.handle_webhook(payload).await.unwrap();
        assert!(
            matches!(r, WebhookResult::Ignored),
            "wrong verify_token must not yield Challenge, got {r:?}"
        );
    }

    #[tokio::test]
    async fn whatsapp_webhook_malformed_entry_errors_gracefully() {
        let cfg = WhatsAppPluginConfig {
            phone_number_id: "pnid".into(),
            access_token: "t".into(),
            verify_token: "v".into(),
            app_secret: String::new(),
            graph_api_base: None,
        };
        let p = WhatsAppPlugin::new(cfg);

        let empty_entry = serde_json::json!({
            "object": "whatsapp_business_account",
            "entry": []
        });
        assert!(matches!(
            p.handle_webhook(empty_entry).await.unwrap(),
            WebhookResult::Ignored
        ));

        let no_entry = serde_json::json!({
            "object": "whatsapp_business_account"
        });
        assert!(matches!(
            p.handle_webhook(no_entry).await.unwrap(),
            WebhookResult::Ignored
        ));

        let entry_not_array = serde_json::json!({
            "object": "whatsapp_business_account",
            "entry": "not-an-array"
        });
        assert!(matches!(
            p.handle_webhook(entry_not_array).await.unwrap(),
            WebhookResult::Ignored
        ));

        let sparse = serde_json::json!({
            "object": "whatsapp_business_account",
            "entry": [{ "changes": [] }, { "id": "only" }]
        });
        assert!(matches!(
            p.handle_webhook(sparse).await.unwrap(),
            WebhookResult::Ignored
        ));
    }
}

#[cfg(test)]
mod outbound_http_tests {
    use super::*;
    use axum::body::Bytes;
    use axum::extract::{Path, State};
    use axum::routing::post;
    use axum::{Json, Router};
    use fastclaw_core::channel::{ChannelPlugin, OutboundMessage};
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    type RequestLog = Arc<Mutex<Vec<(String, String, serde_json::Value)>>>;

    #[derive(Clone)]
    struct Capture {
        log: RequestLog,
    }

    impl Default for Capture {
        fn default() -> Self {
            Self {
                log: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    async fn capture_messages_post(
        State(cap): State<Capture>,
        Path(phone_id): Path<String>,
        body: Bytes,
    ) -> Json<serde_json::Value> {
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or(json!({}));
        cap.log
            .lock()
            .await
            .push(("POST".into(), format!("/{phone_id}/messages"), v));
        Json(json!({}))
    }

    #[tokio::test]
    async fn send_message_posts_graph_messages_url_and_payload() {
        let cap = Capture::default();
        let app = Router::new()
            .route("/:phone_id/messages", post(capture_messages_post))
            .with_state(cap.clone());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;

        let base = format!("http://{}", addr);
        let plugin = WhatsAppPlugin::new(WhatsAppPluginConfig {
            phone_number_id: "9876543210".into(),
            access_token: "test-access".into(),
            verify_token: "vt".into(),
            app_secret: String::new(),
            graph_api_base: Some(base),
        });

        let msg = OutboundMessage {
            target_id: "+15550001111".into(),
            target_type: "user".into(),
            text: "outbound body".into(),
            reply_to: None,
            image_key: None,
        };
        plugin.send_message(&msg).await.unwrap();

        let log = cap.log.lock().await;
        assert_eq!(log.len(), 1, "expected one outbound HTTP request");
        assert_eq!(log[0].0, "POST");
        assert_eq!(log[0].1, "/9876543210/messages");
        assert_eq!(log[0].2["messaging_product"], "whatsapp");
        assert_eq!(log[0].2["to"], "+15550001111");
        assert_eq!(log[0].2["type"], "text");
        assert_eq!(log[0].2["text"]["body"], "outbound body");
    }
}
