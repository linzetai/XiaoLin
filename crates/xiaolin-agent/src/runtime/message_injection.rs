//! Helpers for injecting dynamic context without polluting the cacheable system prefix.

use xiaolin_core::types::{ChatMessage, Role};

use super::trajectory::append_text_to_chat_content;

const SYSTEM_CONTEXT_OPEN: &str = "\n\n<system_context>\n";
const SYSTEM_CONTEXT_CLOSE: &str = "\n</system_context>";

/// Inject per-turn dynamic content into the last user message as `<system_context>`.
pub fn inject_user_context(messages: &mut Vec<ChatMessage>, block: &str) {
    let block = block.trim();
    if block.is_empty() {
        return;
    }
    let wrapped = format!("{SYSTEM_CONTEXT_OPEN}{block}{SYSTEM_CONTEXT_CLOSE}");
    if let Some(idx) = messages.iter().rposition(|m| m.role == Role::User) {
        append_text_to_chat_content(&mut messages[idx].content, &wrapped);
        return;
    }
    messages.push(ChatMessage {
        role: Role::User,
        content: Some(serde_json::Value::String(wrapped)),
        ..Default::default()
    });
}

/// Append session-stable content to the Tier-2 system message (second system block).
pub fn append_to_tier2_system(messages: &mut Vec<ChatMessage>, block: &str) {
    let block = block.trim();
    if block.is_empty() {
        return;
    }
    let Some(idx) = tier2_system_index(messages) else {
        push_tier2_system_prefix(messages, block);
        return;
    };
    append_text_to_chat_content(&mut messages[idx].content, &format!("\n\n{block}"));
}

/// Prepend a session-stable system block before conversation messages (gateway path).
///
/// `build_messages` merges consecutive leading system messages into Tier-2.
pub fn push_tier2_system_prefix(messages: &mut Vec<ChatMessage>, block: &str) {
    let block = block.trim();
    if block.is_empty() {
        return;
    }
    messages.insert(
        0,
        ChatMessage {
            role: Role::System,
            content: Some(serde_json::Value::String(block.to_string())),
            ..Default::default()
        },
    );
}

/// Merge gateway-injected leading system messages into the Tier-2 system block.
pub fn merge_leading_system_into_tier2(messages: &mut Vec<ChatMessage>) {
    let Some(tier2_idx) = tier2_system_index(messages) else {
        return;
    };
    loop {
        let peel_idx = tier2_idx + 1;
        if peel_idx >= messages.len() {
            break;
        }
        if messages[peel_idx].role != Role::System {
            break;
        }
        let text = messages[peel_idx]
            .text_content()
            .unwrap_or_default()
            .to_string();
        messages.remove(peel_idx);
        if !text.trim().is_empty() {
            append_text_to_chat_content(&mut messages[tier2_idx].content, &format!("\n\n{text}"));
        }
    }
}

fn tier2_system_index(messages: &[ChatMessage]) -> Option<usize> {
    if messages.len() >= 2
        && messages[0].role == Role::System
        && messages[1].role == Role::System
    {
        Some(1)
    } else if messages.first().is_some_and(|m| m.role == Role::System) {
        Some(0)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inject_user_context_appends_to_last_user() {
        let mut messages = vec![
            ChatMessage {
                role: Role::System,
                content: Some(serde_json::Value::String("sys".into())),
                ..Default::default()
            },
            ChatMessage {
                role: Role::User,
                content: Some(serde_json::Value::String("hello".into())),
                ..Default::default()
            },
        ];
        inject_user_context(&mut messages, "git status");
        let user = messages[1].text_content().unwrap_or_default();
        assert!(user.contains("<system_context>"));
        assert!(user.contains("git status"));
        assert_eq!(messages[0].text_content().unwrap_or_default(), "sys");
    }

    #[test]
    fn test_merge_leading_system_into_tier2() {
        let mut messages = vec![
            ChatMessage {
                role: Role::System,
                content: Some(serde_json::Value::String("tier1".into())),
                ..Default::default()
            },
            ChatMessage {
                role: Role::System,
                content: Some(serde_json::Value::String("tier2".into())),
                ..Default::default()
            },
            ChatMessage {
                role: Role::System,
                content: Some(serde_json::Value::String("gateway skills".into())),
                ..Default::default()
            },
            ChatMessage {
                role: Role::User,
                content: Some(serde_json::Value::String("hi".into())),
                ..Default::default()
            },
        ];
        merge_leading_system_into_tier2(&mut messages);
        assert_eq!(messages.len(), 3);
        assert!(messages[1].text_content().unwrap_or_default().contains("gateway skills"));
    }
}
