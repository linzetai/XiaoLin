use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolResult};
use fastclaw_core::types::{AskQuestionOption, StreamEvent};

tokio::task_local! {
    pub(crate) static ASK_QUESTION_STREAM_KEY: String;
}

pub async fn with_stream_context<F, T>(stream_key: String, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    ASK_QUESTION_STREAM_KEY.scope(stream_key, fut).await
}

type StreamEventTxMap =
    Arc<tokio::sync::Mutex<HashMap<String, tokio::sync::mpsc::Sender<StreamEvent>>>>;
type PendingAnswers =
    Arc<tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<String>>>>;

pub struct AskQuestionTool {
    stream_event_txs: StreamEventTxMap,
    pending: PendingAnswers,
}

impl AskQuestionTool {
    pub fn new(stream_event_txs: StreamEventTxMap, pending: PendingAnswers) -> Self {
        Self {
            stream_event_txs,
            pending,
        }
    }
}

#[async_trait]
impl Tool for AskQuestionTool {
    fn name(&self) -> &str {
        "ask_question"
    }

    fn description(&self) -> &str {
        "Present the user with a structured question and wait for their answer. \
         Use this tool when you need to gather specific information from the user, \
         confirm a decision, or let them choose from a set of options. \
         Each option needs an 'id' (returned as the answer) and a 'label' (shown to the user). \
         Set allow_multiple to true if the user should be able to select more than one option. \
         The tool blocks until the user responds or the timeout expires. \
         Example: {\"question\": \"Which language?\", \"options\": [{\"id\": \"rust\", \"label\": \"Rust\"}, {\"id\": \"go\", \"label\": \"Go\"}]}"
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "question".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The question to ask the user"
            }),
        );
        props.insert(
            "options".to_string(),
            serde_json::json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "Option identifier returned as the answer" },
                        "label": { "type": "string", "description": "Display text shown to the user" }
                    },
                    "required": ["id", "label"]
                },
                "description": "List of options for the user to choose from"
            }),
        );
        props.insert(
            "timeout_secs".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Seconds to wait for an answer (default 60)"
            }),
        );
        props.insert(
            "allow_multiple".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "Whether the user can select multiple options (default false)"
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["question".to_string(), "options".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("invalid JSON: {e}")),
        };

        let question = match args.get("question").and_then(|v| v.as_str()) {
            Some(q) => q.to_string(),
            None => return ToolResult::err("missing required field 'question'"),
        };

        let options: Vec<AskQuestionOption> = match args.get("options") {
            Some(arr) => match serde_json::from_value(arr.clone()) {
                Ok(opts) => opts,
                Err(e) => return ToolResult::err(format!("invalid options: {e}")),
            },
            None => return ToolResult::err("missing required field 'options'"),
        };

        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(60) as u32;

        let allow_multiple = args
            .get("allow_multiple")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let request_id = uuid::Uuid::new_v4().to_string();
        let stream_key = match ASK_QUESTION_STREAM_KEY.try_with(|k| k.clone()) {
            Ok(k) => k,
            Err(_) => {
                return ToolResult::err(
                    "ask_question not available outside chat stream context",
                );
            }
        };

        let (answer_tx, answer_rx) = tokio::sync::oneshot::channel::<String>();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(request_id.clone(), answer_tx);
        }

        let stream_tx = {
            let txs = self.stream_event_txs.lock().await;
            txs.get(&stream_key).cloned()
        };

        {
            if let Some(tx) = stream_tx {
                let _ = tx
                    .send(StreamEvent::AskQuestion {
                        request_id: request_id.clone(),
                        question: question.clone(),
                        options,
                        timeout_secs,
                        allow_multiple,
                    })
                    .await;
            }
        }

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs as u64),
            answer_rx,
        )
        .await;

        {
            let mut pending = self.pending.lock().await;
            pending.remove(&request_id);
        }

        match result {
            Ok(Ok(answer)) => ToolResult::ok(
                serde_json::json!({ "answer": answer, "question": question }).to_string(),
            ),
            Ok(Err(_)) => ToolResult::err("answer channel closed unexpectedly"),
            Err(_) => ToolResult::ok(
                serde_json::json!({ "answer": null, "timed_out": true, "question": question })
                    .to_string(),
            ),
        }
    }
}
