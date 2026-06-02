use xiaolin_core::types::{ChatMessage, Role};

/// Append plain text to a message `content`, preserving prior text via [`ChatMessage::text_content`].
pub(crate) fn append_text_to_chat_content(content: &mut Option<serde_json::Value>, block: &str) {
    let tmp = ChatMessage {
        role: Role::System,
        content: content.clone(),
        reasoning_content: None,
        name: None,
        tool_calls: None,
        tool_call_id: None,
        compact_metadata: None,
    };
    let mut s = tmp.text_content().map(|c| c.into_owned()).unwrap_or_default();
    s.push_str(block);
    *content = if s.is_empty() {
        None
    } else {
        Some(serde_json::Value::String(s))
    };
}

pub(crate) fn last_user_turn_text(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .rev()
        .filter(|m| matches!(m.role, Role::User))
        .find_map(|m| m.text_content().map(|c| c.into_owned()))
        .unwrap_or_default()
}

pub(crate) fn truncate_for_trajectory(s: &str) -> String {
    const MAX_CHARS: usize = 400;
    let mut iter = s.chars();
    let chunk: String = iter.by_ref().take(MAX_CHARS).collect();
    if iter.next().is_some() {
        format!("{chunk}…")
    } else {
        chunk
    }
}
