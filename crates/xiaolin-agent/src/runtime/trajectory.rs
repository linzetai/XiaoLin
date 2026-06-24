use xiaolin_core::types::{ChatMessage, Role};

/// Append plain text to a message `content`.
///
/// Preserves multimodal structure: when `content` is an array
/// (`[{"type":"text",...}, {"type":"image_url",...}]`), a new text part is
/// appended so image/other parts are NOT lost. Plain strings are concatenated;
/// null/absent becomes the block. This matters because per-turn context
/// injection (e.g. `inject_user_context`) targets the last user message, which
/// may be multimodal.
pub(crate) fn append_text_to_chat_content(content: &mut Option<serde_json::Value>, block: &str) {
    if block.is_empty() {
        return;
    }
    match content {
        Some(serde_json::Value::Array(arr)) => {
            arr.push(serde_json::json!({ "type": "text", "text": block }));
        }
        Some(serde_json::Value::String(s)) => {
            s.push_str(block);
        }
        _ => {
            *content = Some(serde_json::Value::String(block.to_string()));
        }
    }
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
