use std::sync::Arc;

use async_trait::async_trait;
use fastclaw_core::channel::{
    ChannelCapabilities, ChannelMeta, ChannelPlugin, InboundMessage, OutboundMessage, WebhookResult,
};
use fastclaw_core::config::ChannelConfig;
use fastclaw_core::tool::Tool;
use tokio::sync::Mutex;
use url::Url;

pub struct MatrixPluginConfig {
    pub homeserver_url: String,
    pub access_token: String,
    pub user_id: String,
}

impl MatrixPluginConfig {
    pub fn from_channel_config(config: &ChannelConfig) -> Option<Self> {
        let mut homeserver_url = config.domain.clone()?;
        homeserver_url = homeserver_url.trim().to_string();
        if !homeserver_url.starts_with("http://") && !homeserver_url.starts_with("https://") {
            homeserver_url = format!("https://{}", homeserver_url);
        }
        homeserver_url = homeserver_url.trim_end_matches('/').to_string();
        let access_token = config.app_secret.clone()?;
        let user_id = config.app_id.clone()?;
        Some(Self {
            homeserver_url,
            access_token,
            user_id,
        })
    }
}

pub struct MatrixPlugin {
    config: MatrixPluginConfig,
    client: reqwest::Client,
    meta: ChannelMeta,
    sync_since: Arc<Mutex<Option<String>>>,
}

impl MatrixPlugin {
    pub fn new(config: MatrixPluginConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::builder()
                .user_agent("FastClaw/0.1.0")
                .build()
                .unwrap_or_default(),
            meta: ChannelMeta {
                id: "matrix".to_string(),
                name: "Matrix".to_string(),
                description: "Matrix client sync / appservice".to_string(),
                aliases: vec![],
            },
            sync_since: Arc::new(Mutex::new(None)),
        }
    }

    fn send_event_url(&self, room_id: &str, txn_id: &str) -> anyhow::Result<String> {
        let mut u = Url::parse(&self.config.homeserver_url)?;
        u.path_segments_mut()
            .map_err(|()| anyhow::anyhow!("invalid homeserver url"))?
            .push("_matrix")
            .push("client")
            .push("v3")
            .push("rooms")
            .push(room_id)
            .push("send")
            .push("m.room.message")
            .push(txn_id);
        Ok(u.into())
    }
}

#[async_trait]
impl ChannelPlugin for MatrixPlugin {
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
        let events = match payload.get("events").and_then(|e| e.as_array()) {
            Some(e) => e,
            None => return Ok(WebhookResult::Ignored),
        };

        let mut out = Vec::new();
        for ev in events {
            if ev.get("type").and_then(|v| v.as_str()) != Some("m.room.message") {
                continue;
            }
            let room_id = ev
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let event_id = ev
                .get("event_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let sender = ev
                .get("sender")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if sender == self.config.user_id {
                continue;
            }
            let content = ev.get("content").cloned().unwrap_or(serde_json::json!({}));
            match content.get("msgtype").and_then(|v| v.as_str()) {
                Some("m.text") | None => {}
                Some(_) => continue,
            }
            let body = content
                .get("body")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if body.is_empty() {
                continue;
            }
            out.push(InboundMessage {
                channel_id: "matrix".to_string(),
                sender_id: sender,
                chat_id: room_id.clone(),
                message_id: format!("{}:{}", room_id, event_id),
                text: body,
                msg_type: "text".to_string(),
                chat_type: "group".to_string(),
                bot_mentioned: true,
                extra: content,
            });
        }

        if out.is_empty() {
            Ok(WebhookResult::Ignored)
        } else {
            Ok(WebhookResult::Messages(out))
        }
    }

    async fn send_message(&self, msg: &OutboundMessage) -> anyhow::Result<serde_json::Value> {
        let txn_id = uuid::Uuid::new_v4().simple().to_string();
        let url = self.send_event_url(&msg.target_id, &txn_id)?;
        let body = serde_json::json!({
            "msgtype": "m.text",
            "body": msg.text,
        });
        let resp = self
            .client
            .put(url)
            .header(
                "Authorization",
                format!("Bearer {}", self.config.access_token),
            )
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;
        if !status.is_success() {
            anyhow::bail!("Matrix API error ({}): {}", status, json);
        }
        Ok(json)
    }

    async fn reply_message(
        &self,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let (room_id, event_id) = parse_matrix_event_id(message_id)?;
        let txn_id = uuid::Uuid::new_v4().simple().to_string();
        let url = self.send_event_url(room_id, &txn_id)?;
        let body = serde_json::json!({
            "msgtype": "m.text",
            "body": text,
            "m.relates_to": {
                "m.in_reply_to": { "event_id": event_id },
            },
        });
        let resp = self
            .client
            .put(url)
            .header(
                "Authorization",
                format!("Bearer {}", self.config.access_token),
            )
            .json(&body)
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
        let client = self.client.clone();
        let hs = self.config.homeserver_url.clone();
        let token = self.config.access_token.clone();
        let user_id = self.config.user_id.clone();
        let since = self.sync_since.clone();

        tokio::spawn(async move {
            loop {
                let since_guard = since.lock().await.clone();
                let since_param = since_guard.as_deref();
                let mut u = match Url::parse(&format!("{}/_matrix/client/v3/sync", hs)) {
                    Ok(u) => u,
                    Err(e) => {
                        tracing::error!(error = %e, "matrix invalid homeserver");
                        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                        continue;
                    }
                };
                {
                    let mut q = u.query_pairs_mut();
                    q.append_pair("timeout", "30000");
                    if let Some(s) = since_param {
                        if !s.is_empty() {
                            q.append_pair("since", s);
                        }
                    }
                }
                let url_s: String = u.into();
                match client
                    .get(&url_s)
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await
                {
                    Ok(resp) => {
                        if let Ok(json) = resp.json::<serde_json::Value>().await {
                            if let Some(nb) = json.get("next_batch").and_then(|v| v.as_str()) {
                                *since.lock().await = Some(nb.to_string());
                            }
                            if let Some(rooms) = json.get("rooms").and_then(|r| r.get("join")) {
                                if let Some(map) = rooms.as_object() {
                                    for (room_id, room_data) in map {
                                        let timeline = room_data
                                            .get("timeline")
                                            .and_then(|t| t.get("events"))
                                            .and_then(|e| e.as_array());
                                        let Some(events) = timeline else { continue };
                                        for ev in events {
                                            if ev.get("type").and_then(|v| v.as_str())
                                                != Some("m.room.message")
                                            {
                                                continue;
                                            }
                                            let sender = ev
                                                .get("sender")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("");
                                            if sender == user_id.as_str() {
                                                continue;
                                            }
                                            let event_id = ev
                                                .get("event_id")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("");
                                            let body = ev
                                                .get("content")
                                                .and_then(|c| c.get("body"))
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("");
                                            if body.is_empty() {
                                                continue;
                                            }
                                            let _ = inbound_tx.send(InboundMessage {
                                                channel_id: "matrix".to_string(),
                                                sender_id: sender.to_string(),
                                                chat_id: room_id.clone(),
                                                message_id: format!("{}:{}", room_id, event_id),
                                                text: body.to_string(),
                                                msg_type: "text".to_string(),
                                                chat_type: "group".to_string(),
                                                bot_mentioned: true,
                                                extra: ev
                                                    .get("content")
                                                    .cloned()
                                                    .unwrap_or(serde_json::json!({})),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "matrix sync error, retrying");
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                }
            }
        });

        Ok(())
    }

    fn connection_mode(&self) -> &str {
        "polling"
    }
}

fn parse_matrix_event_id(message_id: &str) -> anyhow::Result<(&str, &str)> {
    let idx = message_id
        .rfind(':')
        .ok_or_else(|| anyhow::anyhow!("invalid matrix message_id"))?;
    let (room, rest) = message_id.split_at(idx);
    let event_id = rest.trim_start_matches(':');
    if room.is_empty() || event_id.is_empty() {
        anyhow::bail!("invalid matrix message_id");
    }
    Ok((room, event_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_from_channel() {
        let mut c = ChannelConfig::default();
        assert!(MatrixPluginConfig::from_channel_config(&c).is_none());
        c.domain = Some("matrix.example.com".into());
        c.app_secret = Some("tok".into());
        c.app_id = Some("@bot:example.com".into());
        let m = MatrixPluginConfig::from_channel_config(&c).unwrap();
        assert_eq!(m.homeserver_url, "https://matrix.example.com");
        assert_eq!(m.access_token, "tok");
        assert_eq!(m.user_id, "@bot:example.com");
    }

    #[tokio::test]
    async fn webhook_appservice_events() {
        let cfg = MatrixPluginConfig {
            homeserver_url: "https://h".into(),
            access_token: "t".into(),
            user_id: "@bot:h".into(),
        };
        let p = MatrixPlugin::new(cfg);
        let payload = serde_json::json!({
            "events": [{
                "type": "m.room.message",
                "room_id": "!r:example.com",
                "sender": "@u:example.com",
                "event_id": "$abc",
                "content": { "msgtype": "m.text", "body": "hi" }
            }]
        });
        let r = p.handle_webhook(payload).await.unwrap();
        match r {
            WebhookResult::Messages(msgs) => {
                assert_eq!(msgs.len(), 1);
                assert_eq!(msgs[0].text, "hi");
                assert_eq!(msgs[0].message_id, "!r:example.com:$abc");
            }
            _ => panic!("expected messages"),
        }
    }

    #[test]
    fn parse_event_id_split() {
        let mid = "!room:server.com:$evid";
        let (r, e) = parse_matrix_event_id(mid).unwrap();
        assert_eq!(r, "!room:server.com");
        assert_eq!(e, "$evid");
    }

    #[tokio::test]
    async fn matrix_webhook_empty_events_yields_ignored() {
        let cfg = MatrixPluginConfig {
            homeserver_url: "https://h".into(),
            access_token: "t".into(),
            user_id: "@bot:h".into(),
        };
        let p = MatrixPlugin::new(cfg);
        let r = p
            .handle_webhook(serde_json::json!({ "events": [] }))
            .await
            .unwrap();
        assert!(matches!(r, WebhookResult::Ignored));
    }

    #[tokio::test]
    async fn matrix_webhook_invalid_message_type_skipped() {
        let cfg = MatrixPluginConfig {
            homeserver_url: "https://h".into(),
            access_token: "t".into(),
            user_id: "@bot:h".into(),
        };
        let p = MatrixPlugin::new(cfg);
        let payload = serde_json::json!({
            "events": [{
                "type": "m.room.message",
                "room_id": "!r:example.com",
                "sender": "@u:example.com",
                "event_id": "$img",
                "content": { "msgtype": "m.image", "body": "pic.png", "url": "mxc://x/y" }
            }]
        });
        let r = p.handle_webhook(payload).await.unwrap();
        assert!(matches!(r, WebhookResult::Ignored));
    }
}

#[cfg(test)]
mod outbound_http_tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::extract::State;
    use axum::http::Request;
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

    async fn capture_put(
        State(cap): State<Capture>,
        req: Request<axum::body::Body>,
    ) -> Json<serde_json::Value> {
        assert_eq!(*req.method(), axum::http::Method::PUT);
        let path = req.uri().path().to_string();
        let body = to_bytes(req.into_body(), usize::MAX).await.unwrap_or_default();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or(json!({}));
        cap.log.lock().await.push(("PUT".into(), path, v));
        Json(json!({ "event_id": "$mock" }))
    }

    #[tokio::test]
    async fn send_message_put_room_send_endpoint_and_payload() {
        let cap = Capture::default();
        let app = Router::new()
            .fallback(capture_put)
            .with_state(cap.clone());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;

        let base = format!("http://{}", addr);
        let plugin = MatrixPlugin::new(MatrixPluginConfig {
            homeserver_url: base,
            access_token: "matrix-token".into(),
            user_id: "@bot:mock".into(),
        });

        let room = "!outbound:mock.hs";
        let msg = OutboundMessage {
            target_id: room.into(),
            target_type: "room".into(),
            text: "matrix line".into(),
            reply_to: None,
            image_key: None,
        };
        plugin.send_message(&msg).await.unwrap();

        let log = cap.log.lock().await;
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].0, "PUT");
        let path = &log[0].1;
        assert!(
            path.contains("/_matrix/client/v3/rooms/"),
            "unexpected path: {path}"
        );
        assert!(path.contains("/send/m.room.message/"), "unexpected path: {path}");
        assert!(
            path.contains("%21outbound%3Amock.hs") || path.contains("!outbound:mock.hs"),
            "room id should appear in path: {path}"
        );
        assert_eq!(log[0].2["msgtype"], "m.text");
        assert_eq!(log[0].2["body"], "matrix line");
    }
}
