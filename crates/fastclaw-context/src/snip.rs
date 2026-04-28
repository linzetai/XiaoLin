use fastclaw_core::types::{ChatMessage, Role};

/// A contiguous group of messages that form one "API round" — typically a user
/// turn followed by an assistant reply, possibly with interleaved tool calls.
/// System messages at the start of the conversation are folded into the first
/// round rather than forming their own group.
#[derive(Debug, Clone)]
pub struct ApiRound {
    /// Index of this round (0-based, in chronological order).
    pub index: usize,
    /// The messages belonging to this round. Borrows from the source slice.
    pub messages: Vec<ChatMessage>,
    /// Estimated token count for all messages in this round.
    pub estimated_tokens: usize,
}

/// Partition a flat message list into logical API rounds.
///
/// A new round boundary is placed **before** each `Role::Assistant` message
/// whose preceding context does not already belong to the same assistant turn
/// (i.e. each top-level assistant reply starts a new round). Tool messages
/// that follow an assistant message with `tool_calls` stay in the same round.
///
/// `Role::System` messages at the very beginning are attached to the first
/// conversational round instead of creating a standalone group.
pub fn group_by_api_round(messages: &[ChatMessage]) -> Vec<ApiRound> {
    if messages.is_empty() {
        return Vec::new();
    }

    let mut rounds: Vec<Vec<ChatMessage>> = Vec::new();
    let mut current: Vec<ChatMessage> = Vec::new();
    let mut seen_non_system = false;

    for msg in messages {
        match msg.role {
            Role::System => {
                // System messages always attach to the current (or first) group.
                current.push(msg.clone());
            }
            Role::User => {
                if seen_non_system && !current.is_empty() {
                    // A new user turn after we already have content means we
                    // should check whether the previous round is "complete"
                    // (has an assistant reply). If it does, start a new round.
                    let has_assistant = current.iter().any(|m| m.role == Role::Assistant);
                    if has_assistant {
                        rounds.push(std::mem::take(&mut current));
                    }
                }
                seen_non_system = true;
                current.push(msg.clone());
            }
            Role::Assistant => {
                seen_non_system = true;
                current.push(msg.clone());
            }
            Role::Tool => {
                // Tool results stay with the current round (the assistant
                // message that triggered them).
                current.push(msg.clone());
            }
        }
    }

    if !current.is_empty() {
        rounds.push(current);
    }

    rounds
        .into_iter()
        .enumerate()
        .map(|(i, msgs)| {
            let estimated_tokens = super::estimate_messages_tokens(&msgs);
            ApiRound {
                index: i,
                messages: msgs,
                estimated_tokens,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastclaw_core::types::ChatMessage;
    use serde_json::json;

    fn sys(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::System,
            content: Some(json!(text)),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn user(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::User,
            content: Some(json!(text)),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn assistant(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::Assistant,
            content: Some(json!(text)),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn tool(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::Tool,
            content: Some(json!(text)),
            name: None,
            tool_calls: None,
            tool_call_id: Some("call_1".into()),
        }
    }

    #[test]
    fn empty_messages_returns_empty_groups() {
        let rounds = group_by_api_round(&[]);
        assert!(rounds.is_empty());
    }

    #[test]
    fn single_round_user_assistant() {
        let msgs = vec![user("hello"), assistant("hi there")];
        let rounds = group_by_api_round(&msgs);
        assert_eq!(rounds.len(), 1);
        assert_eq!(rounds[0].messages.len(), 2);
        assert_eq!(rounds[0].index, 0);
    }

    #[test]
    fn system_message_folds_into_first_round() {
        let msgs = vec![
            sys("You are helpful"),
            user("hello"),
            assistant("hi"),
        ];
        let rounds = group_by_api_round(&msgs);
        assert_eq!(rounds.len(), 1, "system + 1 turn = 1 round");
        assert_eq!(rounds[0].messages.len(), 3);
        assert_eq!(rounds[0].messages[0].role, Role::System);
    }

    #[test]
    fn multiple_system_messages_fold_into_first_round() {
        let msgs = vec![
            sys("You are helpful"),
            sys("Additional instructions"),
            user("hello"),
            assistant("hi"),
        ];
        let rounds = group_by_api_round(&msgs);
        assert_eq!(rounds.len(), 1);
        assert_eq!(rounds[0].messages.len(), 4);
    }

    #[test]
    fn ten_rounds_produces_ten_groups() {
        let mut msgs = vec![sys("system prompt")];
        for i in 0..10 {
            msgs.push(user(&format!("question {i}")));
            msgs.push(assistant(&format!("answer {i}")));
        }
        let rounds = group_by_api_round(&msgs);
        assert_eq!(rounds.len(), 10, "10 user-assistant pairs = 10 rounds");
        // System message should be in the first round.
        assert_eq!(rounds[0].messages[0].role, Role::System);
        assert_eq!(rounds[0].messages.len(), 3); // sys + user + assistant
        for r in &rounds[1..] {
            assert_eq!(r.messages.len(), 2); // user + assistant
        }
    }

    #[test]
    fn tool_messages_stay_with_their_round() {
        let msgs = vec![
            user("search for X"),
            assistant("calling tool"),
            tool("result of tool"),
            assistant("based on the result..."),
            user("thanks"),
            assistant("you're welcome"),
        ];
        let rounds = group_by_api_round(&msgs);
        assert_eq!(rounds.len(), 2);
        // First round: user + assistant + tool + assistant
        assert_eq!(rounds[0].messages.len(), 4);
        // Second round: user + assistant
        assert_eq!(rounds[1].messages.len(), 2);
    }

    #[test]
    fn estimated_tokens_are_nonzero() {
        let msgs = vec![user("hello world"), assistant("hi there buddy")];
        let rounds = group_by_api_round(&msgs);
        assert_eq!(rounds.len(), 1);
        assert!(rounds[0].estimated_tokens > 0);
    }

    #[test]
    fn incomplete_round_is_still_returned() {
        let msgs = vec![user("hello")];
        let rounds = group_by_api_round(&msgs);
        assert_eq!(rounds.len(), 1);
        assert_eq!(rounds[0].messages.len(), 1);
    }
}
