use std::sync::Arc;

use async_trait::async_trait;
use fastclaw_core::channel::{
    ChannelCapabilities, ChannelMeta, ChannelPlugin, InboundMessage, OutboundMessage, WebhookResult,
};
use fastclaw_core::config::ChannelConfig;
use fastclaw_core::tool::Tool;
use serde::{Deserialize, Serialize};
use url::Url;

pub struct TeamsPluginConfig {
    pub app_id: String,
    pub app_password: String,
    pub tenant_id: String,
    pub default_service_url: Option<String>,
    /// When set, used as the full OAuth2 token endpoint URL instead of
    /// `https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token` (for integration tests).
    pub oauth_token_url: Option<String>,
}

impl TeamsPluginConfig {
    pub fn from_channel_config(config: &ChannelConfig) -> Option<Self> {
        let app_id = config.app_id.clone()?;
        let app_password = config.app_secret.clone()?;
        let tenant_id = config.verification_token.clone()?;
        let default_service_url = config
            .domain
            .as_ref()
            .map(|s| s.trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty());
        Some(Self {
            app_id,
            app_password,
            tenant_id,
            default_service_url,
            oauth_token_url: None,
        })
    }
}

pub struct TeamsPlugin {
    config: TeamsPluginConfig,
    client: reqwest::Client,
    meta: ChannelMeta,
}

impl TeamsPlugin {
    pub fn new(config: TeamsPluginConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::builder()
                .user_agent("FastClaw/0.1.0")
                .build()
                .unwrap_or_default(),
            meta: ChannelMeta {
                id: "msteams".to_string(),
                name: "Microsoft Teams".to_string(),
                description: "Microsoft Teams via Bot Framework".to_string(),
                aliases: vec!["teams".to_string()],
            },
        }
    }

    async fn fetch_token(&self) -> anyhow::Result<String> {
        let token_url = self
            .config
            .oauth_token_url
            .clone()
            .unwrap_or_else(|| {
                format!(
                    "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
                    self.config.tenant_id
                )
            });
        let resp = self
            .client
            .post(&token_url)
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", self.config.app_id.as_str()),
                ("client_secret", self.config.app_password.as_str()),
                (
                    "scope",
                    "https://api.botframework.com/.default",
                ),
            ])
            .send()
            .await?;
        let json: serde_json::Value = resp.json().await?;
        json.get("access_token")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| anyhow::anyhow!("teams token response missing access_token"))
    }

    async fn post_activity(
        &self,
        service_url: &str,
        conversation_id: &str,
        body: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.fetch_token().await?;
        let base = Url::parse(service_url.trim_end_matches('/'))?;
        let mut u = base.clone();
        u.path_segments_mut()
            .map_err(|()| anyhow::anyhow!("invalid serviceUrl"))?
            .push("v3")
            .push("conversations")
            .push(conversation_id)
            .push("activities");
        let resp = self
            .client
            .post(u.as_str())
            .header("Authorization", format!("Bearer {}", token))
            .json(&body)
            .send()
            .await?;
        let json: serde_json::Value = resp.json().await?;
        Ok(json)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct TeamsMessageRef {
    service_url: String,
    conversation_id: String,
    activity_id: String,
}

fn encode_message_ref(r: &TeamsMessageRef) -> anyhow::Result<String> {
    let s = serde_json::to_string(r)?;
    Ok(hex::encode(s.as_bytes()))
}

fn decode_message_ref(s: &str) -> anyhow::Result<TeamsMessageRef> {
    let bytes = hex::decode(s.trim())?;
    let t = String::from_utf8(bytes)?;
    Ok(serde_json::from_str(&t)?)
}

#[derive(Debug, Deserialize)]
struct TeamsSendTarget {
    service_url: String,
    conversation_id: String,
}

#[async_trait]
impl ChannelPlugin for TeamsPlugin {
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
        let activity_type = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if activity_type != "message" {
            return Ok(WebhookResult::Ignored);
        }

        let text = payload
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if text.is_empty() {
            return Ok(WebhookResult::Ignored);
        }

        let service_url = payload
            .get("serviceUrl")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let conversation_id = payload
            .get("conversation")
            .and_then(|c| c.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let activity_id = payload
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let sender_id = payload
            .get("from")
            .and_then(|f| f.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if service_url.is_empty() || conversation_id.is_empty() || activity_id.is_empty() {
            return Ok(WebhookResult::Ignored);
        }

        let msg_ref = TeamsMessageRef {
            service_url: service_url.clone(),
            conversation_id: conversation_id.clone(),
            activity_id: activity_id.clone(),
        };
        let message_id = encode_message_ref(&msg_ref)?;

        let conv_type = payload
            .get("conversation")
            .and_then(|c| c.get("conversationType"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let is_dm = conv_type.eq_ignore_ascii_case("personal");

        Ok(WebhookResult::Messages(vec![InboundMessage {
            channel_id: "msteams".to_string(),
            sender_id,
            chat_id: conversation_id,
            message_id,
            text,
            msg_type: "text".to_string(),
            chat_type: if is_dm {
                "p2p".to_string()
            } else {
                "group".to_string()
            },
            bot_mentioned: true,
            extra: serde_json::json!({ "serviceUrl": service_url }),
        }]))
    }

    async fn send_message(&self, msg: &OutboundMessage) -> anyhow::Result<serde_json::Value> {
        let (service_url, conversation_id) = if let Ok(t) =
            serde_json::from_str::<TeamsSendTarget>(&msg.target_id)
        {
            (t.service_url, t.conversation_id)
        } else if let Some(ref base) = self.config.default_service_url {
            (base.clone(), msg.target_id.clone())
        } else {
            anyhow::bail!(
                "teams send_message: target_id must be JSON {{\"serviceUrl\",\"conversationId\"}} or set domain as default service URL"
            );
        };

        let body = serde_json::json!({
            "type": "message",
            "text": msg.text,
        });
        self.post_activity(&service_url, &conversation_id, body)
            .await
    }

    async fn reply_message(
        &self,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let msg_ref = decode_message_ref(message_id)?;
        let body = serde_json::json!({
            "type": "message",
            "text": text,
            "replyToId": msg_ref.activity_id,
        });
        self.post_activity(&msg_ref.service_url, &msg_ref.conversation_id, body)
            .await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_from_channel() {
        let mut c = ChannelConfig::default();
        assert!(TeamsPluginConfig::from_channel_config(&c).is_none());
        c.app_id = Some("app".into());
        c.app_secret = Some("sec".into());
        c.verification_token = Some("tenant-guid".into());
        c.domain = Some("https://smba.example/v3/".into());
        let t = TeamsPluginConfig::from_channel_config(&c).unwrap();
        assert_eq!(t.app_id, "app");
        assert_eq!(t.app_password, "sec");
        assert_eq!(t.tenant_id, "tenant-guid");
        assert_eq!(t.default_service_url.as_deref(), Some("https://smba.example/v3"));
        assert!(t.oauth_token_url.is_none());
    }

    #[tokio::test]
    async fn webhook_activity() {
        let cfg = TeamsPluginConfig {
            app_id: "a".into(),
            app_password: "b".into(),
            tenant_id: "t".into(),
            default_service_url: None,
            oauth_token_url: None,
        };
        let p = TeamsPlugin::new(cfg);
        let payload = serde_json::json!({
            "type": "message",
            "id": "act1",
            "timestamp": "2024-01-01T00:00:00Z",
            "serviceUrl": "https://smba.trafficmanager.net/amer/",
            "channelId": "msteams",
            "from": { "id": "user1", "name": "U" },
            "conversation": { "id": "conv1" },
            "recipient": { "id": "bot1" },
            "text": "hello teams"
        });
        let r = p.handle_webhook(payload).await.unwrap();
        match r {
            WebhookResult::Messages(msgs) => {
                assert_eq!(msgs.len(), 1);
                assert_eq!(msgs[0].text, "hello teams");
                let dec = decode_message_ref(&msgs[0].message_id).unwrap();
                assert_eq!(dec.activity_id, "act1");
                assert_eq!(dec.conversation_id, "conv1");
            }
            _ => panic!("expected messages"),
        }
    }

    #[test]
    fn roundtrip_message_ref() {
        let r = TeamsMessageRef {
            service_url: "https://x/".into(),
            conversation_id: "c".into(),
            activity_id: "a".into(),
        };
        let enc = encode_message_ref(&r).unwrap();
        let d = decode_message_ref(&enc).unwrap();
        assert_eq!(d.service_url, r.service_url);
        assert_eq!(d.conversation_id, r.conversation_id);
        assert_eq!(d.activity_id, r.activity_id);
    }

    #[tokio::test]
    async fn msteams_webhook_missing_text_or_conversation() {
        let cfg = TeamsPluginConfig {
            app_id: "a".into(),
            app_password: "b".into(),
            tenant_id: "t".into(),
            default_service_url: None,
            oauth_token_url: None,
        };
        let p = TeamsPlugin::new(cfg);

        let no_text = serde_json::json!({
            "type": "message",
            "id": "act1",
            "serviceUrl": "https://smba.example/",
            "from": { "id": "user1" },
            "conversation": { "id": "conv1" },
            "text": ""
        });
        assert!(matches!(
            p.handle_webhook(no_text).await.unwrap(),
            WebhookResult::Ignored
        ));

        let no_conversation = serde_json::json!({
            "type": "message",
            "id": "act1",
            "serviceUrl": "https://smba.example/",
            "from": { "id": "user1" },
            "text": "hello"
        });
        assert!(matches!(
            p.handle_webhook(no_conversation).await.unwrap(),
            WebhookResult::Ignored
        ));

        let conversation_no_id = serde_json::json!({
            "type": "message",
            "id": "act1",
            "serviceUrl": "https://smba.example/",
            "from": { "id": "user1" },
            "conversation": {},
            "text": "hello"
        });
        assert!(matches!(
            p.handle_webhook(conversation_no_id).await.unwrap(),
            WebhookResult::Ignored
        ));

        let no_service_url = serde_json::json!({
            "type": "message",
            "id": "act1",
            "from": { "id": "user1" },
            "conversation": { "id": "conv1" },
            "text": "hello"
        });
        assert!(matches!(
            p.handle_webhook(no_service_url).await.unwrap(),
            WebhookResult::Ignored
        ));
    }
}

#[cfg(test)]
mod outbound_http_tests {
    use super::*;
    use axum::body::Bytes;
    use axum::extract::State;
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

    async fn oauth_token_stub(
        State(cap): State<Capture>,
        body: Bytes,
    ) -> Json<serde_json::Value> {
        let form = String::from_utf8_lossy(&body).to_string();
        cap.log
            .lock()
            .await
            .push(("POST".into(), "/token".into(), json!({ "form": form })));
        Json(json!({
            "access_token": "mock-access-token",
            "token_type": "Bearer",
            "expires_in": 3600
        }))
    }

    async fn post_activity_stub(
        State(cap): State<Capture>,
        uri: axum::http::Uri,
        body: Bytes,
    ) -> Json<serde_json::Value> {
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or(json!({}));
        cap.log.lock().await.push((
            "POST".into(),
            uri.path().to_string(),
            v,
        ));
        Json(json!({ "id": "mock-activity" }))
    }

    #[tokio::test]
    async fn send_message_posts_token_then_activity_json() {
        let cap = Capture::default();
        let app = Router::new()
            .route("/token", post(oauth_token_stub))
            .route("/v3/conversations/:conversation_id/activities", post(post_activity_stub))
            .with_state(cap.clone());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;

        let origin = format!("http://{}", addr);
        let plugin = TeamsPlugin::new(TeamsPluginConfig {
            app_id: "app-id".into(),
            app_password: "secret".into(),
            tenant_id: "tenant".into(),
            default_service_url: Some(origin.clone()),
            oauth_token_url: Some(format!("{origin}/token")),
        });

        let msg = OutboundMessage {
            target_id: "conversation-xyz".into(),
            target_type: "channel".into(),
            text: "teams outbound".into(),
            reply_to: None,
        };
        plugin.send_message(&msg).await.unwrap();

        let log = cap.log.lock().await;
        assert_eq!(log.len(), 2, "token + activity requests");
        assert_eq!(log[0].0, "POST");
        assert_eq!(log[0].1, "/token");
        let form = log[0].2["form"].as_str().unwrap();
        assert!(form.contains("grant_type=client_credentials"));
        assert!(form.contains("client_id=app-id"));
        assert!(form.contains("client_secret=secret"));

        assert_eq!(log[1].0, "POST");
        assert_eq!(log[1].1, "/v3/conversations/conversation-xyz/activities");
        assert_eq!(log[1].2["type"], "message");
        assert_eq!(log[1].2["text"], "teams outbound");
    }
}
