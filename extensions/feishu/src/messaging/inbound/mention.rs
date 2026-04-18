use crate::messaging::types::MentionRef;

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
