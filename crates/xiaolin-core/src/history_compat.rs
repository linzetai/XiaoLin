use xiaolin_protocol::{ContentPart, HistoryItem, TurnId};

use crate::types::{ChatMessage, Role};

/// Convert a ChatMessage to zero or more HistoryItems.
///
/// A single ChatMessage may produce multiple items (e.g., one Message plus
/// multiple ToolUse items when the message has tool_calls).
pub fn chat_message_to_history(msg: &ChatMessage, turn_id: TurnId) -> Vec<HistoryItem> {
    let mut items = Vec::new();

    if let Some(ref meta) = msg.compact_metadata {
        let summary = msg
            .content
            .as_ref()
            .and_then(|v| v.as_str())
            .unwrap_or("[Context compacted]")
            .to_string();
        items.push(HistoryItem::CompactBoundary {
            turn_id,
            trigger: meta.trigger,
            pre_compact_tokens: meta.pre_compact_token_count,
            post_compact_tokens: meta.post_compact_token_count,
            summary,
        });
        return items;
    }

    let content = match &msg.content {
        Some(serde_json::Value::String(s)) => vec![ContentPart::Text { text: s.clone() }],
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|item| {
                let t = item.get("type")?.as_str()?;
                match t {
                    "text" => {
                        let text = item.get("text")?.as_str()?.to_string();
                        Some(ContentPart::Text { text })
                    }
                    "image_url" => {
                        let url = item
                            .get("image_url")
                            .and_then(|iu| iu.get("url"))
                            .and_then(|u| u.as_str())
                            .unwrap_or("")
                            .to_string();
                        Some(ContentPart::Image { url })
                    }
                    _ => None,
                }
            })
            .collect(),
        _ => vec![],
    };

    if !content.is_empty() || msg.content.is_none() {
        items.push(HistoryItem::Message {
            turn_id: turn_id.clone(),
            role: msg.role.clone(),
            content,
            phase: None,
            reasoning_content: msg.reasoning_content.clone(),
        });
    }

    if let Some(ref tool_calls) = msg.tool_calls {
        for tc in tool_calls {
            items.push(HistoryItem::ToolUse {
                turn_id: turn_id.clone(),
                call_id: tc.id.clone(),
                tool_name: tc.function.name.clone(),
                arguments: tc.function.arguments.clone(),
                output: tc.output.clone().unwrap_or_default(),
                success: tc.success.unwrap_or(true),
                duration_ms: tc.duration_ms,
            });
        }
    }

    items
}

/// Convert a HistoryItem back to a ChatMessage.
///
/// Only `Message` and `CompactBoundary` variants can be converted; other
/// variants return `None`.
pub fn history_to_chat_message(item: &HistoryItem) -> Option<ChatMessage> {
    match item {
        HistoryItem::Message {
            role,
            content,
            reasoning_content,
            ..
        } => {
            let msg_content = if content.is_empty() {
                None
            } else if content.len() == 1 {
                if let ContentPart::Text { text } = &content[0] {
                    Some(serde_json::Value::String(text.clone()))
                } else {
                    Some(serde_json::to_value(content).unwrap_or_default())
                }
            } else {
                Some(serde_json::to_value(content).unwrap_or_default())
            };
            Some(ChatMessage {
                role: role.clone(),
                content: msg_content,
                reasoning_content: reasoning_content.clone(),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                compact_metadata: None,
            })
        }
        HistoryItem::CompactBoundary {
            trigger,
            pre_compact_tokens,
            post_compact_tokens,
            ..
        } => Some(ChatMessage::compact_boundary(
            *trigger,
            *pre_compact_tokens,
            *post_compact_tokens,
        )),
        _ => None,
    }
}

/// Convert a slice of HistoryItems back into ChatMessages.
///
/// Groups items by turn, then:
/// - `Message` → user/assistant/system ChatMessage
/// - `ToolUse` → attaches as tool_calls on the preceding assistant message,
///   plus generates a separate tool-role response message
/// - `CompactBoundary` → compact_boundary() ChatMessage
/// - `TurnUsage` → skipped (no ChatMessage equivalent)
pub fn history_items_to_chat_messages(items: &[HistoryItem]) -> Vec<ChatMessage> {
    use crate::types::{FunctionCall, ToolCall};

    let mut result = Vec::new();

    // Group by turn for tool_calls aggregation
    let mut turn_groups: Vec<(String, Vec<&HistoryItem>)> = Vec::new();
    let mut current_turn: Option<String> = None;

    for item in items {
        let tid = item.turn_id().as_str().to_string();
        if current_turn.as_ref() != Some(&tid) {
            current_turn = Some(tid.clone());
            turn_groups.push((tid, vec![item]));
        } else {
            turn_groups.last_mut().unwrap().1.push(item);
        }
    }

    for (_turn_id, group) in &turn_groups {
        // Collect tool uses for this turn
        let tool_uses: Vec<&HistoryItem> = group
            .iter()
            .filter(|i| matches!(i, HistoryItem::ToolUse { .. }))
            .copied()
            .collect();

        for item in group {
            match item {
                HistoryItem::Message {
                    role,
                    content,
                    reasoning_content,
                    ..
                } => {
                    let msg_content = if content.is_empty() {
                        None
                    } else if content.len() == 1 {
                        if let ContentPart::Text { text } = &content[0] {
                            Some(serde_json::Value::String(text.clone()))
                        } else {
                            Some(serde_json::to_value(content).unwrap_or_default())
                        }
                    } else {
                        Some(serde_json::to_value(content).unwrap_or_default())
                    };

                    // Attach tool_calls to assistant messages
                    let tool_calls = if *role == Role::Assistant && !tool_uses.is_empty() {
                        let calls: Vec<ToolCall> = tool_uses
                            .iter()
                            .filter_map(|tu| {
                                if let HistoryItem::ToolUse {
                                    call_id,
                                    tool_name,
                                    arguments,
                                    output,
                                    success,
                                    duration_ms,
                                    ..
                                } = tu
                                {
                                    Some(ToolCall {
                                        id: call_id.clone(),
                                        call_type: "function".into(),
                                        function: FunctionCall {
                                            name: tool_name.clone(),
                                            arguments: arguments.clone(),
                                        },
                                        output: Some(output.clone()),
                                        success: Some(*success),
                                        duration_ms: *duration_ms,
                                    })
                                } else {
                                    None
                                }
                            })
                            .collect();
                        if calls.is_empty() {
                            None
                        } else {
                            Some(calls)
                        }
                    } else {
                        None
                    };

                    result.push(ChatMessage {
                        role: role.clone(),
                        content: msg_content,
                        reasoning_content: reasoning_content.clone(),
                        name: None,
                        tool_calls,
                        tool_call_id: None,
                        compact_metadata: None,
                    });
                }
                HistoryItem::ToolUse {
                    call_id, output, ..
                } => {
                    // Generate a tool-role response message
                    result.push(ChatMessage {
                        role: Role::Tool,
                        content: Some(serde_json::Value::String(output.clone())),
                        reasoning_content: None,
                        name: None,
                        tool_calls: None,
                        tool_call_id: Some(call_id.clone()),
                        compact_metadata: None,
                    });
                }
                HistoryItem::CompactBoundary {
                    trigger,
                    pre_compact_tokens,
                    post_compact_tokens,
                    ..
                } => {
                    result.push(ChatMessage::compact_boundary(
                        *trigger,
                        *pre_compact_tokens,
                        *post_compact_tokens,
                    ));
                }
                HistoryItem::TurnUsage { .. } => {
                    // No ChatMessage equivalent
                }
                _ => {}
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CompactMetadata, FunctionCall, Role, ToolCall};
    use xiaolin_protocol::CompactTrigger;

    #[test]
    fn text_message_roundtrip() {
        let turn_id = TurnId::new("t1");
        let msg = ChatMessage {
            role: Role::User,
            content: Some(serde_json::Value::String("hello".into())),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
            compact_metadata: None,
        };

        let items = chat_message_to_history(&msg, turn_id.clone());
        assert_eq!(items.len(), 1);
        let back = history_to_chat_message(&items[0]).expect("message should convert back");
        assert_eq!(back.role, Role::User);
        assert_eq!(back.text_content().as_deref(), Some("hello"));
    }

    #[test]
    fn assistant_with_reasoning_roundtrip() {
        let turn_id = TurnId::new("t1");
        let msg = ChatMessage {
            role: Role::Assistant,
            content: Some(serde_json::Value::String("answer".into())),
            reasoning_content: Some("thinking...".into()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            compact_metadata: None,
        };

        let items = chat_message_to_history(&msg, turn_id);
        assert_eq!(items.len(), 1);
        let back = history_to_chat_message(&items[0]).expect("message should convert back");
        assert_eq!(back.reasoning_content.as_deref(), Some("thinking..."));
        assert_eq!(back.text_content().as_deref(), Some("answer"));
    }

    #[test]
    fn multimodal_message_to_history() {
        let turn_id = TurnId::new("t1");
        let msg = ChatMessage {
            role: Role::User,
            content: Some(serde_json::json!([
                {"type": "text", "text": "look at this"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,abc"}}
            ])),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
            compact_metadata: None,
        };

        let items = chat_message_to_history(&msg, turn_id);
        assert_eq!(items.len(), 1);
        if let HistoryItem::Message { content, .. } = &items[0] {
            assert_eq!(content.len(), 2);
        } else {
            panic!("expected Message variant");
        }
    }

    #[test]
    fn tool_calls_produce_message_and_tool_use_items() {
        let turn_id = TurnId::new("t1");
        let msg = ChatMessage {
            role: Role::Assistant,
            content: None,
            reasoning_content: None,
            name: None,
            tool_calls: Some(vec![ToolCall {
                id: "tc-1".into(),
                call_type: "function".into(),
                function: FunctionCall {
                    name: "read_file".into(),
                    arguments: r#"{"path":"a.txt"}"#.into(),
                },
                output: Some("file contents".into()),
                success: Some(true),
                duration_ms: Some(42),
            }]),
            tool_call_id: None,
            compact_metadata: None,
        };

        let items = chat_message_to_history(&msg, turn_id);
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], HistoryItem::Message { .. }));
        if let HistoryItem::ToolUse {
            call_id,
            tool_name,
            output,
            duration_ms,
            ..
        } = &items[1]
        {
            assert_eq!(call_id, "tc-1");
            assert_eq!(tool_name, "read_file");
            assert_eq!(output, "file contents");
            assert_eq!(*duration_ms, Some(42));
        } else {
            panic!("expected ToolUse variant");
        }
    }

    #[test]
    fn compact_boundary_from_chat_message() {
        let turn_id = TurnId::new("t1");
        let msg = ChatMessage {
            role: Role::System,
            content: Some(serde_json::Value::String("summary text".into())),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
            compact_metadata: Some(CompactMetadata {
                trigger: CompactTrigger::Auto,
                pre_compact_token_count: 100_000,
                post_compact_token_count: 20_000,
            }),
        };

        let items = chat_message_to_history(&msg, turn_id);
        assert_eq!(items.len(), 1);
        if let HistoryItem::CompactBoundary {
            trigger,
            pre_compact_tokens,
            post_compact_tokens,
            summary,
            ..
        } = &items[0]
        {
            assert_eq!(*trigger, CompactTrigger::Auto);
            assert_eq!(*pre_compact_tokens, 100_000);
            assert_eq!(*post_compact_tokens, 20_000);
            assert_eq!(summary, "summary text");
        } else {
            panic!("expected CompactBoundary variant");
        }
    }

    #[test]
    fn compact_boundary_roundtrip_metadata() {
        let turn_id = TurnId::new("t1");
        let item = HistoryItem::CompactBoundary {
            turn_id,
            trigger: CompactTrigger::Manual,
            pre_compact_tokens: 50_000,
            post_compact_tokens: 10_000,
            summary: "custom summary".into(),
        };

        let back = history_to_chat_message(&item).expect("compact boundary should convert");
        assert!(back.is_compact_boundary());
        let meta = back.compact_metadata.expect("metadata present");
        assert_eq!(meta.trigger, CompactTrigger::Manual);
        assert_eq!(meta.pre_compact_token_count, 50_000);
        assert_eq!(meta.post_compact_token_count, 10_000);
    }

    #[test]
    fn tool_use_does_not_convert_to_chat_message() {
        let item = HistoryItem::ToolUse {
            turn_id: TurnId::new("t1"),
            call_id: "tc-1".into(),
            tool_name: "grep".into(),
            arguments: "{}".into(),
            output: "matches".into(),
            success: true,
            duration_ms: None,
        };
        assert!(history_to_chat_message(&item).is_none());
    }
}
