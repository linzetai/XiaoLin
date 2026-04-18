use crate::messaging::types::{MentionRef, MessageContext};

/// Parse a raw Feishu `im.message.receive_v1` event into a MessageContext.
pub fn parse_message_event(event: &serde_json::Value) -> Option<MessageContext> {
    let message = event.get("message")?;
    let sender = event.get("sender")?;

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
    let message_type = message
        .get("message_type")
        .and_then(|v| v.as_str())
        .unwrap_or("text")
        .to_string();

    let sender_open_id = sender
        .get("sender_id")
        .and_then(|s| s.get("open_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let raw_content = message
        .get("content")
        .and_then(|v| v.as_str())
        .map(String::from);

    let text = raw_content
        .as_deref()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(c).ok())
        .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(String::from))
        .unwrap_or_default();

    let thread_id = message
        .get("thread_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let root_id = message
        .get("root_id")
        .and_then(|v| v.as_str())
        .map(String::from);

    let mentions = message
        .get("mentions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    Some(MentionRef {
                        key: m.get("key")?.as_str()?.to_string(),
                        open_id: m
                            .get("id")
                            .and_then(|id| id.get("open_id"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        name: m
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Some(MessageContext {
        message_id,
        chat_id,
        chat_type,
        sender_open_id,
        text,
        message_type,
        thread_id,
        root_id,
        mentions,
        raw_content,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_text_message() {
        let event = json!({
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
        assert_eq!(ctx.chat_id, "oc_456");
        assert_eq!(ctx.sender_open_id, "ou_abc");
    }

    #[test]
    fn parse_with_thread() {
        let event = json!({
            "sender": {"sender_id": {"open_id": "ou_abc"}},
            "message": {
                "message_id": "om_123",
                "chat_id": "oc_456",
                "message_type": "text",
                "content": "{\"text\": \"reply\"}",
                "thread_id": "omt_789"
            }
        });
        let ctx = parse_message_event(&event).unwrap();
        assert_eq!(ctx.thread_id, Some("omt_789".into()));
    }

    #[test]
    fn parse_missing_message() {
        let event = json!({"sender": {"sender_id": {"open_id": "ou_abc"}}});
        assert!(parse_message_event(&event).is_none());
    }
}
