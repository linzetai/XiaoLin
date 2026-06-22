//! Bridges Feishu WebSocket events into the XiaoLin channel pipeline.
//!
//! Reads `WsEvent` from the WS client, parses `im.message.receive_v1` payloads
//! into `InboundMessage`, and forwards them via a channel sender.
//! Applies reply-mode filtering: in group chats with `mention_only` mode,
//! messages without @mention are silently dropped.

use std::sync::Arc;

use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use xiaolin_core::channel::InboundMessage;

use crate::messaging::inbound::{MessageDedup, parse_im_mentions_from_message};

use super::client::{EventReceiver, WsEvent};

/// Runs the event→InboundMessage bridge until the receiver is closed or cancelled.
///
/// - `bot_open_id`: the bot's open_id for precise @mention detection.
/// - `reply_mode`: "mention_only" (skip non-mentioned group msgs) or "always".
pub async fn run_event_bridge(
    mut event_rx: EventReceiver,
    inbound_tx: mpsc::UnboundedSender<InboundMessage>,
    bot_open_id: Option<String>,
    reply_mode: String,
    dedup: Arc<Mutex<MessageDedup>>,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            evt = event_rx.recv() => {
                let Some(evt) = evt else {
                    tracing::info!("feishu ws: event channel closed");
                    break;
                };

                // Try parsing as a card action first
                if let Some(card_msg) = parse_card_action_payload(&evt) {
                    {
                        let mut guard = dedup.lock().await;
                        if !guard.check(&card_msg.message_id) {
                            tracing::debug!(
                                request_id = %card_msg.message_id,
                                "feishu ws: duplicate card action skipped"
                            );
                            continue;
                        }
                    }
                    tracing::info!(
                        request_id = %card_msg.message_id,
                        option = %card_msg.text,
                        "feishu ws: dispatching card action callback"
                    );
                    if inbound_tx.send(card_msg).is_err() {
                        tracing::warn!("feishu ws: inbound channel closed");
                        break;
                    }
                    continue;
                }

                match parse_event_payload(&evt, bot_open_id.as_deref()) {
                    Some(msg) => {
                        {
                            let mut guard = dedup.lock().await;
                            if !guard.check(&msg.message_id) {
                                tracing::debug!(
                                    msg_id = %msg.message_id,
                                    "feishu ws: duplicate message skipped"
                                );
                                continue;
                            }
                        }

                        if msg.chat_type == "group" && reply_mode == "mention_only" && !msg.bot_mentioned {
                            tracing::debug!(
                                chat_id = %msg.chat_id,
                                msg_id = %msg.message_id,
                                "feishu ws: group message without @mention, skipped"
                            );
                            continue;
                        }

                        tracing::info!(
                            chat_id = %msg.chat_id,
                            chat_type = %msg.chat_type,
                            sender_id = %msg.sender_id,
                            msg_id = %msg.message_id,
                            bot_mentioned = msg.bot_mentioned,
                            text_len = msg.text.len(),
                            "feishu ws: dispatching inbound message"
                        );
                        if inbound_tx.send(msg).is_err() {
                            tracing::warn!("feishu ws: inbound channel closed");
                            break;
                        }
                    }
                    None => {
                        tracing::debug!(
                            msg_type = %evt.message_type,
                            msg_id = %evt.message_id,
                            "feishu ws: event ignored (not a parseable IM message)"
                        );
                    }
                }
            }
            _ = cancel.cancelled() => {
                tracing::info!("feishu ws: event bridge cancelled");
                break;
            }
        }
    }
}

/// Parse a WsEvent as a card action callback (card.action.trigger).
/// Returns an InboundMessage with `msg_type = "card_action"` if this is a card callback,
/// None otherwise.
fn parse_card_action_payload(evt: &WsEvent) -> Option<InboundMessage> {
    let payload: serde_json::Value = serde_json::from_slice(&evt.payload).ok()?;

    let event_type = payload
        .get("header")
        .and_then(|h| h.get("event_type"))
        .and_then(|v| v.as_str())
        .or_else(|| payload.get("type").and_then(|v| v.as_str()))
        .unwrap_or("");

    if event_type != "card.action.trigger" {
        return None;
    }

    let event = payload.get("event").unwrap_or(&payload);
    let action = event.get("action")?;
    let value = action.get("value")?;

    let request_id = value
        .get("message_id")
        .and_then(|v| v.as_str())?
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
    let operator_id = event
        .get("operator")
        .and_then(|o| o.get("open_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let extra = serde_json::json!({
        "_card_action": true,
        "request_id": request_id,
        "option_id": option_id,
        "action_type": action_type,
    });

    Some(InboundMessage {
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
    })
}

/// Parse a WsEvent payload (JSON envelope from Feishu long-connection) into an
/// InboundMessage. Returns None if the event is not an IM text message.
fn parse_event_payload(evt: &WsEvent, bot_open_id: Option<&str>) -> Option<InboundMessage> {
    let payload: serde_json::Value = serde_json::from_slice(&evt.payload).ok()?;

    let (header, event) = if payload.get("header").is_some() && payload.get("event").is_some() {
        (payload.get("header"), payload.get("event")?)
    } else {
        (None, &payload)
    };

    let event_type = header
        .and_then(|h| h.get("event_type"))
        .and_then(|v| v.as_str())
        .or_else(|| event.get("type").and_then(|v| v.as_str()))
        .unwrap_or("");

    if event_type != "im.message.receive_v1" {
        return None;
    }

    let message = event.get("message")?;
    let msg_type = message.get("message_type")?.as_str()?;
    let message_id = message.get("message_id")?.as_str()?.to_string();
    let chat_id = message
        .get("chat_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let chat_type = message
        .get("chat_type")
        .and_then(|v| v.as_str())
        .unwrap_or("p2p")
        .to_string();
    let sender_id = event
        .get("sender")
        .and_then(|s| s.get("sender_id"))
        .and_then(|s| s.get("open_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let content_str = message.get("content").and_then(|v| v.as_str()).unwrap_or("{}");
    let text = match msg_type {
        "text" => {
            let parsed = serde_json::from_str::<serde_json::Value>(content_str).ok()?;
            parsed.get("text")?.as_str()?.to_string()
        }
        "image" => "[收到一张图片]".to_string(),
        "file" => {
            let file_name = serde_json::from_str::<serde_json::Value>(content_str)
                .ok()
                .and_then(|v| v.get("file_name").and_then(|n| n.as_str()).map(String::from))
                .unwrap_or_else(|| "unknown".to_string());
            format!("[收到一个文件: {file_name}]")
        }
        "audio" => "[收到一条语音消息]".to_string(),
        "media" | "video" => "[收到一条媒体消息]".to_string(),
        "sticker" => "[收到一张表情]".to_string(),
        other => format!("[收到一条{other}消息]"),
    };

    let (bot_mentioned, text) =
        parse_im_mentions_from_message(message, text, bot_open_id);
    if text.is_empty() {
        return None;
    }

    Some(InboundMessage {
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_v2_event() {
        let payload = serde_json::json!({
            "header": {
                "event_type": "im.message.receive_v1",
                "token": "abc"
            },
            "event": {
                "sender": {
                    "sender_id": { "open_id": "ou_test" }
                },
                "message": {
                    "message_id": "om_ws_001",
                    "chat_id": "oc_ws_001",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": "{\"text\":\"hello from ws\"}"
                }
            }
        });
        let evt = WsEvent {
            message_type: "event".into(),
            message_id: "msg_001".into(),
            trace_id: "trace_001".into(),
            payload: serde_json::to_vec(&payload).unwrap(),
        };
        let msg = parse_event_payload(&evt, None).unwrap();
        assert_eq!(msg.text, "hello from ws");
        assert_eq!(msg.chat_id, "oc_ws_001");
        assert_eq!(msg.chat_type, "group");
        assert_eq!(msg.sender_id, "ou_test");
        assert!(!msg.bot_mentioned);
    }

    #[test]
    fn parse_with_mention_strips_at() {
        let payload = serde_json::json!({
            "header": {
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "sender": {
                    "sender_id": { "open_id": "ou_user" }
                },
                "message": {
                    "message_id": "om_ws_002",
                    "chat_id": "oc_ws_002",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": "{\"text\":\"@_user_1 你好\"}",
                    "mentions": [{
                        "key": "@_user_1",
                        "id": { "open_id": "ou_bot", "id_type": "open_id" },
                        "name": "XiaoLin Bot"
                    }]
                }
            }
        });
        let evt = WsEvent {
            message_type: "event".into(),
            message_id: "m2".into(),
            trace_id: "t2".into(),
            payload: serde_json::to_vec(&payload).unwrap(),
        };
        let msg = parse_event_payload(&evt, Some("ou_bot")).unwrap();
        assert_eq!(msg.text, "你好");
        assert!(msg.bot_mentioned);
        assert_eq!(msg.chat_type, "group");
    }

    #[test]
    fn parse_p2p_message() {
        let payload = serde_json::json!({
            "header": { "event_type": "im.message.receive_v1" },
            "event": {
                "sender": { "sender_id": { "open_id": "ou_user" } },
                "message": {
                    "message_id": "om_p2p_001",
                    "chat_id": "oc_p2p_001",
                    "chat_type": "p2p",
                    "message_type": "text",
                    "content": "{\"text\":\"private message\"}"
                }
            }
        });
        let evt = WsEvent {
            message_type: "event".into(),
            message_id: "m3".into(),
            trace_id: "t3".into(),
            payload: serde_json::to_vec(&payload).unwrap(),
        };
        let msg = parse_event_payload(&evt, Some("ou_bot")).unwrap();
        assert_eq!(msg.chat_type, "p2p");
        assert!(!msg.bot_mentioned);
    }

    #[test]
    fn parse_non_text_produces_placeholder() {
        let payload = serde_json::json!({
            "header": { "event_type": "im.message.receive_v1" },
            "event": {
                "sender": { "sender_id": { "open_id": "ou_test" } },
                "message": {
                    "message_id": "om_1",
                    "chat_id": "oc_1",
                    "message_type": "image",
                    "content": "{}"
                }
            }
        });
        let evt = WsEvent {
            message_type: "event".into(),
            message_id: "m1".into(),
            trace_id: "t1".into(),
            payload: serde_json::to_vec(&payload).unwrap(),
        };
        let msg = parse_event_payload(&evt, None).unwrap();
        assert_eq!(msg.text, "[收到一张图片]");
    }

    #[test]
    fn parse_file_produces_placeholder_with_name() {
        let payload = serde_json::json!({
            "header": { "event_type": "im.message.receive_v1" },
            "event": {
                "sender": { "sender_id": { "open_id": "ou_test" } },
                "message": {
                    "message_id": "om_2",
                    "chat_id": "oc_1",
                    "message_type": "file",
                    "content": "{\"file_name\":\"report.pdf\"}"
                }
            }
        });
        let evt = WsEvent {
            message_type: "event".into(),
            message_id: "m2".into(),
            trace_id: "t2".into(),
            payload: serde_json::to_vec(&payload).unwrap(),
        };
        let msg = parse_event_payload(&evt, None).unwrap();
        assert_eq!(msg.text, "[收到一个文件: report.pdf]");
    }
}
