use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolResult};
use fastclaw_core::types::{ChatMessage, Role};

/// Allows the agent to manually remove specific messages from context.
///
/// Operates on shared message state provided by the runtime. System
/// messages and the last user turn are protected from removal.
pub struct SnipTool {
    messages: Arc<Mutex<Vec<ChatMessage>>>,
}

impl SnipTool {
    pub fn new(messages: Arc<Mutex<Vec<ChatMessage>>>) -> Self {
        Self { messages }
    }
}

fn estimate_tokens(msg: &ChatMessage) -> usize {
    let text_len = msg
        .text_content()
        .map(|s| s.len())
        .unwrap_or(0);
    text_len / 4 + 4
}

#[async_trait]
impl Tool for SnipTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Think
    }

    fn name(&self) -> &str {
        "snip"
    }

    fn description(&self) -> &str {
        "Remove specific messages from conversation context to free tokens. \
         Input: {\"message_indices\": [2, 5, 8], \"reason\": \"old search results no longer needed\"}. \
         System messages (index 0) and the last user turn cannot be snipped. \
         Non-existent indices are silently skipped. \
         Returns {\"snipped_count\": N, \"tokens_freed\": M}."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "message_indices".to_string(),
            serde_json::json!({
                "type": "array",
                "items": {"type": "integer"},
                "description": "0-based indices of messages to remove from context."
            }),
        );
        props.insert(
            "reason".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Brief reason for snipping (stored as summary replacement)."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["message_indices".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::err(format!(
                    "snip arguments are not valid JSON: {e}. \
                     Pass {{\"message_indices\": [2, 5], \"reason\": \"...\"}}"
                ))
            }
        };

        let indices: Vec<usize> = match args.get("message_indices").and_then(|v| v.as_array()) {
            Some(arr) => arr
                .iter()
                .filter_map(|v| v.as_u64().map(|n| n as usize))
                .collect(),
            None => {
                return ToolResult::err(
                    "snip is missing required array field 'message_indices'. \
                     Example: {\"message_indices\": [2, 5, 8]}."
                        .to_string(),
                )
            }
        };

        if indices.is_empty() {
            return ToolResult::ok(
                "{\"snipped_count\": 0, \"tokens_freed\": 0, \"message\": \"nothing to snip\"}",
            );
        }

        let reason = args
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("messages removed by agent");

        let mut guard = self.messages.lock().expect("snip messages poisoned");
        let msg_count = guard.len();

        let last_user_idx = guard
            .iter()
            .rposition(|m| matches!(m.role, Role::User));

        let mut tokens_freed: usize = 0;
        let mut skipped: Vec<String> = Vec::new();

        let mut to_remove: Vec<usize> = indices
            .into_iter()
            .filter(|&idx| {
                if idx >= msg_count {
                    return false;
                }
                if matches!(guard[idx].role, Role::System) {
                    skipped.push(format!("{idx}(system)"));
                    return false;
                }
                if Some(idx) == last_user_idx {
                    skipped.push(format!("{idx}(last_user)"));
                    return false;
                }
                true
            })
            .collect();

        to_remove.sort_unstable();
        to_remove.dedup();

        for &idx in &to_remove {
            tokens_freed += estimate_tokens(&guard[idx]);
        }
        let snipped_count = to_remove.len();

        for &idx in to_remove.iter().rev() {
            guard.remove(idx);
        }

        if snipped_count > 0 {
            let insert_pos = guard
                .iter()
                .position(|m| !matches!(m.role, Role::System))
                .unwrap_or(0);
            guard.insert(
                insert_pos,
                ChatMessage {
                    role: Role::System,
                    content: Some(serde_json::Value::String(format!(
                        "[{snipped_count} message(s) snipped: {reason}]"
                    ))),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            );
        }

        drop(guard);

        let mut result = serde_json::json!({
            "snipped_count": snipped_count,
            "tokens_freed": tokens_freed,
        });
        if !skipped.is_empty() {
            result["skipped"] = serde_json::json!(skipped);
        }
        ToolResult::ok(result.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastclaw_core::types::Role;

    fn make_messages() -> Vec<ChatMessage> {
        vec![
            ChatMessage {
                role: Role::System,
                content: Some(serde_json::Value::String("You are helpful.".into())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: Role::User,
                content: Some(serde_json::Value::String("Hello".into())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: Role::Assistant,
                content: Some(serde_json::Value::String("Hi there! How can I help?".into())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: Role::User,
                content: Some(serde_json::Value::String("Search for X".into())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::Value::String("result of search...".repeat(50))),
                name: Some("web_search".into()),
                tool_calls: None,
                tool_call_id: Some("call_1".into()),
            },
            ChatMessage {
                role: Role::User,
                content: Some(serde_json::Value::String("Now do Y".into())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
        ]
    }

    fn setup() -> (Arc<Mutex<Vec<ChatMessage>>>, SnipTool) {
        let msgs = Arc::new(Mutex::new(make_messages()));
        let tool = SnipTool::new(msgs.clone());
        (msgs, tool)
    }

    #[tokio::test]
    async fn snip_removes_specified_messages() {
        let (msgs, tool) = setup();
        let result = tool
            .execute(r#"{"message_indices": [2, 4], "reason": "old data"}"#)
            .await;
        assert!(result.success);
        let v: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["snipped_count"], 2);
        assert!(v["tokens_freed"].as_u64().unwrap() > 0);

        let guard = msgs.lock().unwrap();
        assert_eq!(guard.len(), 5); // 6 - 2 removed + 1 summary = 5
        assert!(guard[1].text_content().unwrap().contains("snipped"));
    }

    #[tokio::test]
    async fn snip_protects_system_messages() {
        let (msgs, tool) = setup();
        let result = tool
            .execute(r#"{"message_indices": [0], "reason": "test"}"#)
            .await;
        assert!(result.success);
        let v: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["snipped_count"], 0);
        assert!(v["skipped"].as_array().unwrap()[0].as_str().unwrap().contains("system"));

        let guard = msgs.lock().unwrap();
        assert_eq!(guard.len(), 6);
    }

    #[tokio::test]
    async fn snip_protects_last_user_turn() {
        let (_msgs, tool) = setup();
        let result = tool
            .execute(r#"{"message_indices": [5], "reason": "test"}"#)
            .await;
        assert!(result.success);
        let v: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["snipped_count"], 0);
        assert!(v["skipped"].as_array().unwrap()[0].as_str().unwrap().contains("last_user"));
    }

    #[tokio::test]
    async fn snip_skips_out_of_bounds() {
        let (msgs, tool) = setup();
        let result = tool
            .execute(r#"{"message_indices": [99, 100], "reason": "test"}"#)
            .await;
        assert!(result.success);
        let v: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["snipped_count"], 0);

        let guard = msgs.lock().unwrap();
        assert_eq!(guard.len(), 6);
    }

    #[tokio::test]
    async fn snip_empty_indices() {
        let (_, tool) = setup();
        let result = tool
            .execute(r#"{"message_indices": [], "reason": "test"}"#)
            .await;
        assert!(result.success);
        assert!(result.output.contains("nothing to snip"));
    }

    #[tokio::test]
    async fn snip_missing_field() {
        let (_, tool) = setup();
        let result = tool.execute(r#"{}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("missing"));
    }

    #[tokio::test]
    async fn snip_inserts_summary() {
        let (msgs, tool) = setup();
        tool.execute(r#"{"message_indices": [1, 2], "reason": "cleaned old exchange"}"#)
            .await;

        let guard = msgs.lock().unwrap();
        let summary = guard.iter().find(|m| {
            m.text_content()
                .map(|t| t.contains("snipped"))
                .unwrap_or(false)
        });
        assert!(summary.is_some());
        assert!(summary.unwrap().text_content().unwrap().contains("cleaned old exchange"));
    }

    #[tokio::test]
    async fn snip_invalid_json() {
        let (_, tool) = setup();
        let result = tool.execute("not json").await;
        assert!(!result.success);
        assert!(result.output.contains("not valid JSON"));
    }

    #[tokio::test]
    async fn snip_duplicate_indices_deduped() {
        let (msgs, tool) = setup();
        let result = tool
            .execute(r#"{"message_indices": [2, 2, 2], "reason": "dup test"}"#)
            .await;
        assert!(result.success);
        let v: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["snipped_count"], 1);

        let guard = msgs.lock().unwrap();
        // 6 original - 1 removed + 1 summary = 6
        assert_eq!(guard.len(), 6);
    }

    #[tokio::test]
    async fn snip_default_reason() {
        let (msgs, tool) = setup();
        tool.execute(r#"{"message_indices": [2]}"#).await;

        let guard = msgs.lock().unwrap();
        let summary = guard.iter().find(|m| {
            m.text_content()
                .map(|t| t.contains("snipped"))
                .unwrap_or(false)
        });
        assert!(summary.is_some());
        assert!(summary.unwrap().text_content().unwrap().contains("removed by agent"));
    }

    #[tokio::test]
    async fn snip_mixed_valid_and_protected() {
        let (msgs, tool) = setup();
        // index 0 = system (protected), index 2 = assistant (removable), index 5 = last user (protected)
        let result = tool
            .execute(r#"{"message_indices": [0, 2, 5], "reason": "mixed"}"#)
            .await;
        assert!(result.success);
        let v: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["snipped_count"], 1);
        let skipped = v["skipped"].as_array().unwrap();
        assert_eq!(skipped.len(), 2);

        let guard = msgs.lock().unwrap();
        // 6 original - 1 removed + 1 summary = 6
        assert_eq!(guard.len(), 6);
    }
}
