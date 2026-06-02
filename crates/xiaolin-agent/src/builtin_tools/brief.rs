use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use xiaolin_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolResult};
use xiaolin_protocol::{AgentEvent, TurnId};

use super::ask_question::ASK_QUESTION_STREAM_KEY;

type EventTxMap = Arc<DashMap<String, tokio::sync::mpsc::Sender<AgentEvent>>>;

/// Pushes a markdown message (and optional file attachments) to the user
/// without waiting for a response.  Two modes are supported:
///
/// * **normal** — the agent is replying to a user action.
/// * **proactive** — the agent initiates communication unprompted.
pub struct BriefTool {
    stream_event_txs: EventTxMap,
}

impl BriefTool {
    pub fn new(stream_event_txs: EventTxMap) -> Self {
        Self { stream_event_txs }
    }
}

#[async_trait]
impl Tool for BriefTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Think
    }

    fn name(&self) -> &str {
        "send_user_message"
    }

    fn description(&self) -> &str {
        "Push a markdown message to the user without waiting for a reply. \
         Use this to proactively share updates, progress reports, or results. \
         Input: {\"content\": \"markdown text\", \"attachments\": [\"path/to/file\"], \"mode\": \"normal\"|\"proactive\"}. \
         'content' is required; 'attachments' and 'mode' are optional (mode defaults to 'normal')."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "content".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Markdown-formatted message to display to the user."
            }),
        );
        props.insert(
            "attachments".to_string(),
            serde_json::json!({
                "type": "array",
                "items": {"type": "string"},
                "description": "Optional list of file paths to attach."
            }),
        );
        props.insert(
            "mode".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["normal", "proactive"],
                "description": "Message mode: 'normal' (default) or 'proactive'."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["content".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::err(format!(
                    "send_user_message arguments are not valid JSON: {e}. \
                     Pass {{\"content\": \"your message\"}}."
                ))
            }
        };

        let content = match args.get("content").and_then(|v| v.as_str()) {
            Some(c) if !c.trim().is_empty() => c.to_string(),
            Some(_) => {
                return ToolResult::err(
                    "send_user_message 'content' must be non-empty.".to_string(),
                )
            }
            None => {
                return ToolResult::err(
                    "send_user_message is missing required string field 'content'. \
                     Example: {\"content\": \"Here are the results...\"}."
                        .to_string(),
                )
            }
        };

        let attachments: Vec<String> = args
            .get("attachments")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let mode = args
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("normal");
        let mode = match mode {
            "normal" | "proactive" => mode.to_string(),
            other => {
                return ToolResult::err(format!(
                    "send_user_message 'mode' must be 'normal' or 'proactive', got '{other}'."
                ))
            }
        };

        let stream_key = match ASK_QUESTION_STREAM_KEY.try_with(|k| k.clone()) {
            Ok(k) => k,
            Err(_) => {
                return ToolResult::err(
                    "send_user_message not available outside chat stream context",
                );
            }
        };

        let stream_tx = self
            .stream_event_txs
            .get(&stream_key)
            .map(|r| r.value().clone());

        let attachment_count = attachments.len();
        if let Some(tx) = stream_tx {
            let _ = tx
                .send(AgentEvent::BriefMessage {
                    turn_id: TurnId::new("builtin"),
                    content: content.clone(),
                    attachments,
                    mode: mode.clone(),
                })
                .await;
        }

        ToolResult::ok(
            serde_json::json!({
                "sent": true,
                "mode": mode,
                "content_length": content.len(),
                "attachment_count": attachment_count,
            })
            .to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (
        EventTxMap,
        tokio::sync::mpsc::Receiver<AgentEvent>,
        BriefTool,
    ) {
        let txs: EventTxMap = Arc::new(DashMap::new());
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        txs.insert("test-stream".to_string(), tx);
        let tool = BriefTool::new(txs.clone());
        (txs, rx, tool)
    }

    async fn run_with_ctx<F: std::future::Future>(fut: F) -> F::Output {
        ASK_QUESTION_STREAM_KEY
            .scope("test-stream".to_string(), fut)
            .await
    }

    #[tokio::test]
    async fn sends_brief_message() {
        let (_txs, mut rx, tool) = setup();
        let result =
            run_with_ctx(tool.execute(r#"{"content": "Hello user!", "mode": "proactive"}"#)).await;
        assert!(result.success);
        let v: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["sent"], true);
        assert_eq!(v["mode"], "proactive");

        let event = rx.try_recv().unwrap();
        match event {
            AgentEvent::BriefMessage {
                content,
                attachments,
                mode,
                ..
            } => {
                assert_eq!(content, "Hello user!");
                assert!(attachments.is_empty());
                assert_eq!(mode, "proactive");
            }
            other => panic!("expected BriefMessage, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn sends_with_attachments() {
        let (_txs, mut rx, tool) = setup();
        let result = run_with_ctx(tool.execute(
            r#"{"content": "See attached", "attachments": ["/tmp/a.txt", "/tmp/b.png"]}"#,
        ))
        .await;
        assert!(result.success);
        let v: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["attachment_count"], 2);

        let event = rx.try_recv().unwrap();
        match event {
            AgentEvent::BriefMessage { attachments, .. } => {
                assert_eq!(attachments, vec!["/tmp/a.txt", "/tmp/b.png"]);
            }
            other => panic!("expected BriefMessage, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn defaults_to_normal_mode() {
        let (_txs, mut rx, tool) = setup();
        let result = run_with_ctx(tool.execute(r#"{"content": "update"}"#)).await;
        assert!(result.success);
        let v: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["mode"], "normal");

        match rx.try_recv().unwrap() {
            AgentEvent::BriefMessage { mode, .. } => assert_eq!(mode, "normal"),
            other => panic!("expected BriefMessage, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_empty_content() {
        let (_txs, _rx, tool) = setup();
        let result = run_with_ctx(tool.execute(r#"{"content": "   "}"#)).await;
        assert!(!result.success);
        assert!(result.output.contains("non-empty"));
    }

    #[tokio::test]
    async fn rejects_missing_content() {
        let (_txs, _rx, tool) = setup();
        let result = run_with_ctx(tool.execute(r#"{}"#)).await;
        assert!(!result.success);
        assert!(result.output.contains("missing"));
    }

    #[tokio::test]
    async fn rejects_invalid_mode() {
        let (_txs, _rx, tool) = setup();
        let result = run_with_ctx(tool.execute(r#"{"content": "hi", "mode": "aggressive"}"#)).await;
        assert!(!result.success);
        assert!(result.output.contains("'normal' or 'proactive'"));
    }

    #[tokio::test]
    async fn rejects_invalid_json() {
        let (_txs, _rx, tool) = setup();
        let result = run_with_ctx(tool.execute("not json")).await;
        assert!(!result.success);
        assert!(result.output.contains("not valid JSON"));
    }
}
