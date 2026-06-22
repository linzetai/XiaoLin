use crate::messaging::types::MentionRef;

/// Parse Feishu IM message mentions and strip bot @-markers from text.
pub fn parse_im_mentions_from_message(
    message: &serde_json::Value,
    mut text: String,
    bot_open_id: Option<&str>,
) -> (bool, String) {
    let mentions = message
        .get("mentions")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut bot_mentioned = false;
    for m in &mentions {
        let m_key = m.get("key").and_then(|v| v.as_str()).unwrap_or("");
        let m_open_id = m
            .get("id")
            .and_then(|v| v.get("open_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let is_bot = if let Some(bot_id) = bot_open_id {
            m_open_id == bot_id
        } else {
            m.get("id")
                .and_then(|v| v.get("id_type"))
                .and_then(|v| v.as_str())
                == Some("app_id")
        };

        if is_bot {
            bot_mentioned = true;
            text = text.replace(m_key, "");
        }
    }

    (bot_mentioned, text.trim().to_string())
}

/// Check if any mention in the list is the bot itself.
pub fn mentioned_bot(mentions: &[MentionRef], bot_open_id: &str) -> bool {
    mentions.iter().any(|m| m.open_id == bot_open_id)
}

/// Extract the message body text, stripping bot @-mentions if present.
pub fn extract_message_body(text: &str, mentions: &[MentionRef], bot_open_id: &str) -> String {
    let mut result = text.to_string();
    for m in mentions {
        if m.open_id == bot_open_id {
            result = result.replace(&m.key, "");
        }
    }
    result.trim().to_string()
}

/// Format a mention for use in a text message.
pub fn format_mention_for_text(open_id: &str, name: &str) -> String {
    format!("<at user_id=\"{}\">{}</at>", open_id, name)
}

/// Get all non-bot mentions from the list.
pub fn non_bot_mentions(mentions: &[MentionRef], bot_open_id: &str) -> Vec<MentionRef> {
    mentions
        .iter()
        .filter(|m| m.open_id != bot_open_id)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_mention(key: &str, open_id: &str, name: &str) -> MentionRef {
        MentionRef {
            key: key.to_string(),
            open_id: open_id.to_string(),
            name: name.to_string(),
        }
    }

    #[test]
    fn detect_bot_mention() {
        let mentions = vec![make_mention("@_user_1", "ou_bot", "Bot")];
        assert!(mentioned_bot(&mentions, "ou_bot"));
        assert!(!mentioned_bot(&mentions, "ou_other"));
    }

    #[test]
    fn extract_body_strips_bot() {
        let mentions = vec![make_mention("@_user_1", "ou_bot", "Bot")];
        let body = extract_message_body("@_user_1 hello world", &mentions, "ou_bot");
        assert_eq!(body, "hello world");
    }

    #[test]
    fn non_bot_filters_correctly() {
        let mentions = vec![
            make_mention("@_user_1", "ou_bot", "Bot"),
            make_mention("@_user_2", "ou_human", "Alice"),
        ];
        let filtered = non_bot_mentions(&mentions, "ou_bot");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "Alice");
    }
}
