use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use xiaolin_core::channel::{ChannelPlugin, WebhookResult};

use crate::client::FeishuClient;
use crate::messaging::inbound::{
    extract_inbound_text, parse_im_mentions_from_message, MessageDedup,
};
use crate::plugin::FeishuPlugin;
use crate::webhook_security::{parse_webhook_payload, verify_lark_webhook_headers};

#[derive(Clone)]
pub struct FeishuWebhookConfig {
    pub verification_token: String,
    pub encrypt_key: Option<String>,
}

/// Feishu event callback payload (simplified)
#[derive(Debug, Deserialize)]
struct EventCallback {
    #[serde(rename = "type")]
    #[serde(default)]
    event_type: Option<String>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    header: Option<EventHeader>,
    #[serde(default)]
    event: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct EventHeader {
    #[serde(default)]
    event_type: Option<String>,
    #[serde(default)]
    token: Option<String>,
}

#[derive(Debug, Serialize)]
struct ChallengeResponse {
    challenge: String,
}

pub struct FeishuWebhookState {
    pub client: Arc<FeishuClient>,
    pub config: FeishuWebhookConfig,
    pub message_handler: Arc<dyn FeishuMessageHandler>,
    /// When set, delegates verify/parse/dedup/mention filtering to the unified plugin pipeline.
    pub unified_plugin: Option<Arc<FeishuPlugin>>,
    dedup: Arc<Mutex<MessageDedup>>,
    reply_mode: String,
    bot_open_id: Option<String>,
}

impl FeishuWebhookState {
    pub fn new(
        client: Arc<FeishuClient>,
        config: FeishuWebhookConfig,
        message_handler: Arc<dyn FeishuMessageHandler>,
    ) -> Self {
        Self {
            client,
            config,
            message_handler,
            unified_plugin: None,
            dedup: Arc::new(Mutex::new(MessageDedup::new(Duration::from_secs(300)))),
            reply_mode: "mention_only".into(),
            bot_open_id: None,
        }
    }

    pub fn with_unified_plugin(mut self, plugin: Arc<FeishuPlugin>) -> Self {
        self.unified_plugin = Some(plugin);
        self
    }

    pub fn with_reply_mode(mut self, reply_mode: impl Into<String>) -> Self {
        self.reply_mode = reply_mode.into();
        self
    }

    pub fn with_bot_open_id(mut self, bot_open_id: impl Into<String>) -> Self {
        self.bot_open_id = Some(bot_open_id.into());
        self
    }
}

#[async_trait::async_trait]
pub trait FeishuMessageHandler: Send + Sync {
    async fn handle_message(
        &self,
        sender_id: &str,
        message_id: &str,
        chat_id: &str,
        text: &str,
    ) -> anyhow::Result<String>;
}

fn headers_to_map(headers: &HeaderMap) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for (name, value) in headers.iter() {
        if let Ok(v) = value.to_str() {
            map.entry(name.as_str().to_string())
                .or_insert_with(|| v.to_string());
        }
    }
    map
}

/// Axum handler for Feishu event webhook endpoint.
///
/// Mount at: `POST /webhook/feishu`
#[deprecated(note = "Use FeishuPlugin as ChannelPlugin instead")]
pub async fn feishu_webhook_handler(
    State(state): State<Arc<FeishuWebhookState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(plugin) = state.unified_plugin.clone() {
        return handle_via_unified_plugin(state, &plugin, headers, body).await;
    }

    let header_map = headers_to_map(&headers);

    if let Err(e) = verify_lark_webhook_headers(
        &header_map,
        state.config.encrypt_key.as_deref(),
        &body,
    ) {
        tracing::warn!(error = %e, "Feishu webhook signature verification failed");
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"code": -1, "msg": "signature verification failed"})),
        )
            .into_response();
    }

    let payload = match parse_webhook_payload(state.config.encrypt_key.as_deref(), &body) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "failed to parse Feishu webhook payload");
            return Json(serde_json::json!({"code": -1, "msg": "invalid payload"})).into_response();
        }
    };

    let callback: EventCallback = match serde_json::from_value(payload.clone()) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "failed to parse Feishu event");
            return Json(serde_json::json!({"code": -1, "msg": "invalid payload"})).into_response();
        }
    };

    // Token verification (auxiliary; signature verified above when encrypt_key is set)
    let token = callback
        .token
        .as_deref()
        .or_else(|| callback.header.as_ref().and_then(|h| h.token.as_deref()))
        .unwrap_or("");
    if state.config.verification_token.is_empty() {
        tracing::warn!("Feishu webhook verification_token not configured, rejecting");
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"code": -1, "msg": "token not configured"})),
        )
            .into_response();
    }
    if token != state.config.verification_token {
        tracing::warn!("Feishu webhook token mismatch");
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"code": -1, "msg": "token mismatch"})),
        )
            .into_response();
    }

    // URL verification challenge (after token verification)
    if let Some(challenge) = payload.get("challenge").and_then(|v| v.as_str()) {
        return Json(ChallengeResponse {
            challenge: challenge.to_string(),
        })
        .into_response();
    }

    // Determine event type
    let event_type = callback
        .header
        .as_ref()
        .and_then(|h| h.event_type.as_deref())
        .or(callback.event_type.as_deref())
        .unwrap_or("");

    if event_type == "im.message.receive_v1" {
        if let Some(event) = callback.event {
            tokio::spawn(handle_im_message(state.clone(), event));
        }
    }

    Json(serde_json::json!({"code": 0})).into_response()
}

async fn handle_via_unified_plugin(
    state: Arc<FeishuWebhookState>,
    plugin: &FeishuPlugin,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let header_map = headers_to_map(&headers);
    if let Err(e) = plugin.verify_webhook(&header_map, &body).await {
        tracing::warn!(error = %e, "Feishu webhook verification failed (unified plugin)");
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"code": -1, "msg": "verification failed"})),
        )
            .into_response();
    }

    let payload = match plugin.parse_webhook_payload(&body) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "failed to parse Feishu webhook payload (unified plugin)");
            return Json(serde_json::json!({"code": -1, "msg": "invalid payload"})).into_response();
        }
    };

    match plugin.handle_webhook(payload).await {
        Ok(WebhookResult::Challenge(v)) => Json(v).into_response(),
        Ok(WebhookResult::Ignored) => Json(serde_json::json!({"code": 0})).into_response(),
        Ok(WebhookResult::Messages(msgs)) => {
            for msg in msgs {
                let state = state.clone();
                tokio::spawn(async move {
                    dispatch_inbound_message(state, msg).await;
                });
            }
            Json(serde_json::json!({"code": 0})).into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "Feishu unified webhook handler failed");
            Json(serde_json::json!({"code": -1, "msg": "processing failed"})).into_response()
        }
    }
}

async fn dispatch_inbound_message(
    state: Arc<FeishuWebhookState>,
    msg: xiaolin_core::channel::InboundMessage,
) {
    if msg.text.is_empty() {
        return;
    }
    match state
        .message_handler
        .handle_message(&msg.sender_id, &msg.message_id, &msg.chat_id, &msg.text)
        .await
    {
        Ok(reply) => {
            if !reply.is_empty() {
                if let Err(e) = state.client.reply_message(&msg.message_id, &reply).await {
                    tracing::error!(error = %e, message_id = %msg.message_id, "failed to reply on Feishu");
                }
            }
        }
        Err(e) => {
            tracing::error!(error = %e, message_id = %msg.message_id, "Feishu message handler error");
            let _ = state
                .client
                .reply_message(&msg.message_id, "Sorry, something went wrong processing your message.")
                .await;
        }
    }
}

async fn handle_im_message(state: Arc<FeishuWebhookState>, event: serde_json::Value) {
    let message = match event.get("message") {
        Some(m) => m,
        None => return,
    };

    let message_id = message
        .get("message_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if message_id.is_empty() {
        return;
    }

    {
        let mut dedup = state.dedup.lock().await;
        if !dedup.check(message_id) {
            tracing::debug!(message_id, "Feishu webhook: duplicate message skipped");
            return;
        }
    }

    let msg_type = message
        .get("message_type")
        .and_then(|v| v.as_str())
        .unwrap_or("text");
    let chat_id = message
        .get("chat_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let chat_type = message
        .get("chat_type")
        .and_then(|v| v.as_str())
        .unwrap_or("p2p");
    let sender_id = event
        .get("sender")
        .and_then(|s| s.get("sender_id"))
        .and_then(|s| s.get("open_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let content_str = message
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("{}");
    let mut text = extract_inbound_text(msg_type, content_str);
    let (bot_mentioned, stripped) =
        parse_im_mentions_from_message(message, text, state.bot_open_id.as_deref());
    text = stripped;

    if text.is_empty() {
        return;
    }

    if chat_type == "group" && state.reply_mode == "mention_only" && !bot_mentioned {
        tracing::debug!(
            chat_id,
            message_id,
            "Feishu webhook: group message without @mention, skipped"
        );
        return;
    }

    tracing::info!(
        sender_id,
        chat_id,
        message_id,
        text_len = text.len(),
        bot_mentioned,
        "Feishu message received"
    );

    match state
        .message_handler
        .handle_message(sender_id, message_id, chat_id, &text)
        .await
    {
        Ok(reply) => {
            if !reply.is_empty() {
                if let Err(e) = state.client.reply_message(message_id, &reply).await {
                    tracing::error!(error = %e, message_id, "failed to reply on Feishu");
                }
            }
        }
        Err(e) => {
            tracing::error!(error = %e, message_id, "Feishu message handler error");
            let _ = state
                .client
                .reply_message(
                    message_id,
                    "Sorry, something went wrong processing your message.",
                )
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messaging::inbound::parse_message_event;

    #[test]
    fn url_verification_challenge_response() {
        let payload =
            serde_json::json!({"challenge": "abc123", "token": "test", "type": "url_verification"});
        let challenge = payload.get("challenge").and_then(|v| v.as_str()).unwrap();
        let resp = ChallengeResponse {
            challenge: challenge.to_string(),
        };
        assert_eq!(resp.challenge, "abc123");
    }

    #[test]
    fn shared_parse_extracts_text() {
        let event = serde_json::json!({
            "sender": {"sender_id": {"open_id": "ou_abc"}},
            "message": {
                "message_id": "om_123",
                "chat_id": "oc_456",
                "chat_type": "group",
                "message_type": "text",
                "content": "{\"text\": \"hello world\"}"
            }
        });
        let ctx = parse_message_event(&event).unwrap();
        assert_eq!(ctx.text, "hello world");
    }
}
