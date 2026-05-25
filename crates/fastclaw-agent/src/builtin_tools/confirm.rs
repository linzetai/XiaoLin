use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolResult};
use fastclaw_protocol::{AgentEvent, AskQuestionOption, TurnId};

pub(crate) use super::ask_question::ASK_QUESTION_STREAM_KEY;

type EventTxMap = Arc<DashMap<String, tokio::sync::mpsc::Sender<AgentEvent>>>;
type PendingAnswers = Arc<DashMap<String, tokio::sync::oneshot::Sender<String>>>;

/// Lightweight yes/no confirmation tool. Presents a message to the user
/// and waits for them to Allow or Deny. Reuses the same streaming
/// infrastructure as `ask_question`.
pub struct ConfirmTool {
    stream_event_txs: EventTxMap,
    pending: PendingAnswers,
}

impl ConfirmTool {
    pub fn new(stream_event_txs: EventTxMap, pending: PendingAnswers) -> Self {
        Self {
            stream_event_txs,
            pending,
        }
    }
}

#[async_trait]
impl Tool for ConfirmTool {
    fn name(&self) -> &str {
        "confirm"
    }

    fn description(&self) -> &str {
        "Ask the user for a simple yes/no confirmation before proceeding with a potentially \
         dangerous or irreversible action. Returns {\"confirmed\": true} if the user allows, \
         or {\"confirmed\": false} if they deny. Always present a clear description of the \
         action. Example: {\"message\": \"Delete /tmp/data? This cannot be undone.\"}"
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "message".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Clear description of the action that requires confirmation"
            }),
        );
        props.insert(
            "timeout_secs".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Seconds to wait for user response. Omit or set to 0 for no timeout (waits indefinitely)."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["message".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("invalid JSON: {e}")),
        };

        let message = match args.get("message").and_then(|v| v.as_str()) {
            Some(m) => m.to_string(),
            None => return ToolResult::err("missing required field 'message'"),
        };

        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        let stream_key = match ASK_QUESTION_STREAM_KEY.try_with(|k| k.clone()) {
            Ok(k) => k,
            Err(_) => {
                return ToolResult::err("confirm not available outside chat stream context");
            }
        };

        let request_id = uuid::Uuid::new_v4().to_string();
        let (answer_tx, answer_rx) = tokio::sync::oneshot::channel::<String>();
        self.pending.insert(request_id.clone(), answer_tx);

        let stream_tx = self
            .stream_event_txs
            .get(&stream_key)
            .map(|r| r.value().clone());

        if let Some(tx) = stream_tx {
            let _ = tx
                .send(AgentEvent::AskQuestion {
                    turn_id: TurnId::new("builtin"),
                    request_id: request_id.clone(),
                    question: message.clone(),
                    options: vec![
                        AskQuestionOption {
                            id: "allow".to_string(),
                            label: "Allow".to_string(),
                        },
                        AskQuestionOption {
                            id: "deny".to_string(),
                            label: "Deny".to_string(),
                        },
                    ],
                    timeout_secs,
                    allow_multiple: false,
                })
                .await;
        }

        let result = if timeout_secs == 0 {
            answer_rx.await.map_err(|_| ())
        } else {
            match tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs as u64),
                answer_rx,
            )
            .await
            {
                Ok(inner) => inner.map_err(|_| ()),
                Err(_) => {
                    self.pending.remove(&request_id);
                    return ToolResult::ok(
                        serde_json::json!({
                            "confirmed": false,
                            "timed_out": true,
                            "message": message,
                        })
                        .to_string(),
                    );
                }
            }
        };

        self.pending.remove(&request_id);

        match result {
            Ok(answer) => {
                let confirmed = answer == "allow";
                ToolResult::ok(
                    serde_json::json!({
                        "confirmed": confirmed,
                        "answer": answer,
                        "message": message,
                    })
                    .to_string(),
                )
            }
            Err(_) => ToolResult::err("confirmation channel closed unexpectedly"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastclaw_core::tool::Tool;

    async fn make_tool() -> (
        ConfirmTool,
        Arc<DashMap<String, tokio::sync::mpsc::Sender<AgentEvent>>>,
        Arc<DashMap<String, tokio::sync::oneshot::Sender<String>>>,
        tokio::sync::mpsc::Receiver<AgentEvent>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        let txs: EventTxMap = Arc::new(DashMap::new());
        let pending: PendingAnswers = Arc::new(DashMap::new());
        txs.insert("test-stream".to_string(), tx);
        let tool = ConfirmTool::new(txs.clone(), pending.clone());
        (tool, txs, pending, rx)
    }

    #[tokio::test]
    async fn confirm_tool_metadata() {
        let (tool, _, _, _) = make_tool().await;
        assert_eq!(tool.name(), "confirm");
        assert!(tool.description().contains("yes/no"));
        let schema = tool.parameters_schema();
        assert!(schema.required.contains(&"message".to_string()));
        assert!(schema.properties.contains_key("message"));
        assert!(schema.properties.contains_key("timeout_secs"));
    }

    #[tokio::test]
    async fn confirm_allow_flow() {
        let (tool, _txs, pending, mut rx) = make_tool().await;

        let tool_handle = tokio::spawn(async move {
            ASK_QUESTION_STREAM_KEY
                .scope("test-stream".to_string(), async {
                    tool.execute(r#"{"message": "Delete /tmp/foo?"}"#).await
                })
                .await
        });

        let event = rx.recv().await.expect("should receive AskQuestion");
        if let AgentEvent::AskQuestion {
            request_id,
            question,
            options,
            allow_multiple,
            ..
        } = event
        {
            assert_eq!(question, "Delete /tmp/foo?");
            assert_eq!(options.len(), 2);
            assert_eq!(options[0].id, "allow");
            assert_eq!(options[1].id, "deny");
            assert!(!allow_multiple);

            let tx = pending.remove(&request_id).map(|(_k, v)| v).unwrap();
            tx.send("allow".to_string()).unwrap();
        } else {
            panic!("expected AskQuestion event");
        }

        let result = tool_handle.await.unwrap();
        assert!(result.success);
        let val: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(val["confirmed"], true);
        assert_eq!(val["answer"], "allow");
    }

    #[tokio::test]
    async fn confirm_deny_flow() {
        let (tool, _txs, pending, mut rx) = make_tool().await;

        let tool_handle = tokio::spawn(async move {
            ASK_QUESTION_STREAM_KEY
                .scope("test-stream".to_string(), async {
                    tool.execute(r#"{"message": "Run rm -rf /?"}"#).await
                })
                .await
        });

        let event = rx.recv().await.unwrap();
        if let AgentEvent::AskQuestion { request_id, .. } = event {
            let tx = pending.remove(&request_id).map(|(_k, v)| v).unwrap();
            tx.send("deny".to_string()).unwrap();
        }

        let result = tool_handle.await.unwrap();
        assert!(result.success);
        let val: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(val["confirmed"], false);
        assert_eq!(val["answer"], "deny");
    }

    #[tokio::test]
    async fn confirm_timeout_returns_false() {
        let (tool, _txs, _pending, _rx) = make_tool().await;

        let result = ASK_QUESTION_STREAM_KEY
            .scope("test-stream".to_string(), async {
                tool.execute(r#"{"message": "test?", "timeout_secs": 1}"#)
                    .await
            })
            .await;

        assert!(result.success);
        let val: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(val["confirmed"], false);
        assert_eq!(val["timed_out"], true);
    }

    #[tokio::test]
    async fn confirm_missing_message_errors() {
        let (tool, _txs, _pending, _rx) = make_tool().await;

        let result = ASK_QUESTION_STREAM_KEY
            .scope("test-stream".to_string(), async {
                tool.execute(r#"{}"#).await
            })
            .await;

        assert!(!result.success);
        assert!(result.output.contains("missing"));
    }

    #[tokio::test]
    async fn confirm_outside_stream_context_errors() {
        let (tool, _, _, _) = make_tool().await;
        let result = tool.execute(r#"{"message": "test"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("not available outside"));
    }
}
