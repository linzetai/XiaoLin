use serde::{Deserialize, Serialize};

/// Feishu event types we handle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeishuEventType {
    UrlVerification,
    ImMessageReceive,
    ImMessageReactionCreated,
    ImMessageReactionDeleted,
    ImChatMemberAdd,
    ImChatMemberDelete,
    InteractiveCard,
    Unknown(String),
}

impl FeishuEventType {
    pub fn from_str(s: &str) -> Self {
        match s {
            "url_verification" => Self::UrlVerification,
            "im.message.receive_v1" => Self::ImMessageReceive,
            "im.message.reaction.created_v1" => Self::ImMessageReactionCreated,
            "im.message.reaction.deleted_v1" => Self::ImMessageReactionDeleted,
            "im.chat.member.bot.added_v1" | "im.chat.member.user.added_v1" => Self::ImChatMemberAdd,
            "im.chat.member.bot.deleted_v1" | "im.chat.member.user.deleted_v1" => {
                Self::ImChatMemberDelete
            }
            "card.action.trigger" => Self::InteractiveCard,
            other => Self::Unknown(other.to_string()),
        }
    }
}

/// Parse event type from a raw webhook payload.
pub fn parse_event(payload: &serde_json::Value) -> FeishuEventType {
    if payload.get("challenge").is_some() {
        return FeishuEventType::UrlVerification;
    }

    let event_type = payload
        .get("header")
        .and_then(|h| h.get("event_type"))
        .and_then(|v| v.as_str())
        .or_else(|| payload.get("type").and_then(|v| v.as_str()))
        .unwrap_or("");

    FeishuEventType::from_str(event_type)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_url_verification() {
        let payload = json!({"challenge": "abc", "type": "url_verification"});
        assert_eq!(parse_event(&payload), FeishuEventType::UrlVerification);
    }

    #[test]
    fn parse_im_message() {
        let payload = json!({"header": {"event_type": "im.message.receive_v1"}});
        assert_eq!(parse_event(&payload), FeishuEventType::ImMessageReceive);
    }

    #[test]
    fn parse_reaction() {
        let payload = json!({"header": {"event_type": "im.message.reaction.created_v1"}});
        assert_eq!(
            parse_event(&payload),
            FeishuEventType::ImMessageReactionCreated
        );
    }

    #[test]
    fn parse_unknown() {
        let payload = json!({"header": {"event_type": "something.new"}});
        assert_eq!(
            parse_event(&payload),
            FeishuEventType::Unknown("something.new".into())
        );
    }
}
