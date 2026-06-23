use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::client::FeishuClient;
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
pub async fn feishu_webhook_handler(
    State(state): State<Arc<FeishuWebhookState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
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

async fn handle_im_message(state: Arc<FeishuWebhookState>, event: serde_json::Value) {
    let message = match event.get("message") {
        Some(m) => m,
        None => return,
    };

    let msg_type = message
        .get("message_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if msg_type != "text" {
        tracing::debug!(msg_type, "ignoring non-text message");
        return;
    }

    let message_id = message
        .get("message_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let chat_id = message
        .get("chat_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
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
    let text = serde_json::from_str::<serde_json::Value>(content_str)
        .ok()
        .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(String::from))
        .unwrap_or_default();

    if text.is_empty() {
        return;
    }

    tracing::info!(
        sender_id,
        chat_id,
        message_id,
        text_len = text.len(),
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
            tracing::error!(error = %e, "Feishu message handler error");
            let _ = state
                .client
                .reply_message(message_id, &format!("Error: {}", e))
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
