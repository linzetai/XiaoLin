use serde::{Deserialize, Serialize};

/// Parsed message context from a Feishu event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageContext {
    pub message_id: String,
    pub chat_id: String,
    pub chat_type: String,
    pub sender_open_id: String,
    pub text: String,
    pub message_type: String,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub root_id: Option<String>,
    #[serde(default)]
    pub mentions: Vec<MentionRef>,
    #[serde(default)]
    pub raw_content: Option<String>,
}

/// A mention reference within a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MentionRef {
    pub key: String,
    pub open_id: String,
    pub name: String,
}

/// Feishu reaction event data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReactionEvent {
    pub message_id: String,
    pub reaction_type: String,
    pub operator_id: String,
    pub action_time: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_context_serde() {
        let ctx = MessageContext {
            message_id: "om_123".into(),
            chat_id: "oc_456".into(),
            chat_type: "group".into(),
            sender_open_id: "ou_789".into(),
            text: "hello".into(),
            message_type: "text".into(),
            thread_id: None,
            root_id: None,
            mentions: vec![],
            raw_content: None,
        };
        let json = serde_json::to_string(&ctx).unwrap();
        let parsed: MessageContext = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.message_id, "om_123");
    }
}
