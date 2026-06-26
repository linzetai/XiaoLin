use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use xiaolin_core::tool::{Tool, ToolParameterSchema, ToolResult};
use xiaolin_protocol::{AgentEvent, AskQuestionOption, TurnId};
use xiaolin_session_actor::InteractionHandle;

pub type SteerInbox = std::sync::Arc<
    tokio::sync::Mutex<
        tokio::sync::mpsc::UnboundedReceiver<xiaolin_session_actor::turn::SteerMessage>,
    >,
>;

tokio::task_local! {
    pub(crate) static ASK_QUESTION_STREAM_KEY: String;
    pub(crate) static TASK_INTERACTION_HANDLE: InteractionHandle;
    pub static STEER_INBOX: SteerInbox;
}

pub async fn with_stream_context<F, T>(stream_key: String, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    ASK_QUESTION_STREAM_KEY.scope(stream_key, fut).await
}

/// Run a future with an `InteractionHandle` available via task-local.
/// When set, builtin tools (`ask_question`, `confirm`) use the actor path
/// instead of the DashMap + oneshot path.
pub async fn with_interaction_handle<F, T>(handle: InteractionHandle, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    TASK_INTERACTION_HANDLE.scope(handle, fut).await
}

/// Run a future with a steer message inbox available via task-local.
/// The agentic loop drains this inbox at each iteration to inject mid-turn user messages.
pub async fn with_steer_inbox<F, T>(inbox: SteerInbox, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    STEER_INBOX.scope(inbox, fut).await
}

type EventTxMap = Arc<DashMap<String, tokio::sync::mpsc::Sender<AgentEvent>>>;
type PendingAnswers = Arc<DashMap<String, tokio::sync::oneshot::Sender<String>>>;

pub struct AskQuestionTool {
    stream_event_txs: EventTxMap,
    pending: PendingAnswers,
}

impl AskQuestionTool {
    pub fn new(stream_event_txs: EventTxMap, pending: PendingAnswers) -> Self {
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
         confirm a decision, or let them choose from a set of options."
    }

    fn prompt(&self) -> String {
        "Present the user with a structured multiple-choice question.\n\n\
## When to Ask\n\
- Ambiguous requirements that could go multiple ways\n\
- Destructive or irreversible actions needing confirmation\n\
- Multiple valid approaches where user preference matters\n\
- Missing information that cannot be inferred from context\n\n\
## When NOT to Ask\n\
- You can figure it out from the codebase or context\n\
- The answer is implied by the user's request\n\
- You're asking just to seem thorough (bias toward action)\n\
- Mid-implementation for minor style choices\n\n\
## Question Quality\n\
- Be specific: 'Should login redirect to /dashboard or /home?' not 'What should happen?'\n\
- Include WHY you're asking if it's not obvious\n\
- Provide sensible options (at least 2, ideally 2-4)\n\
- Make option labels clear and self-explanatory\n\
- Use allow_multiple when the options aren't mutually exclusive\n\n\
## Anti-Patterns\n\
- Don't ask multiple questions in one turn — combine or prioritize\n\
- Don't ask yes/no questions that can be inferred from context\n\
- Don't re-ask what the user already specified"
            .to_string()
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
                "description": "Seconds to wait for an answer. Omit or set to 0 for no timeout (waits indefinitely)."
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
            .unwrap_or(0) as u32;

        let allow_multiple = args
            .get("allow_multiple")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let request_id = uuid::Uuid::new_v4().to_string();
        let stream_key = match ASK_QUESTION_STREAM_KEY.try_with(|k| k.clone()) {
            Ok(k) => k,
            Err(_) => {
                return ToolResult::err("ask_question not available outside chat stream context");
            }
        };

        let ih = TASK_INTERACTION_HANDLE.try_with(|h| h.clone()).ok();

        let answer_rx = if let Some(ref handle) = ih {
            handle.request_answer(request_id.clone())
        } else {
            let (answer_tx, answer_rx) = tokio::sync::oneshot::channel::<String>();
            self.pending.insert(request_id.clone(), answer_tx);
            answer_rx
        };

        let stream_tx = self
            .stream_event_txs
            .get(&stream_key)
            .map(|r| r.value().clone());

        if let Some(tx) = stream_tx {
            let _ = tx
                .send(AgentEvent::AskQuestion {
                    turn_id: TurnId::new("builtin"),
                    request_id: request_id.clone(),
                    question: question.clone(),
                    options,
                    timeout_secs,
                    allow_multiple,
                    session_id: None,
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
                    if ih.is_none() {
                        self.pending.remove(&request_id);
                    }
                    return ToolResult::ok(
                        serde_json::json!({ "answer": null, "timed_out": true, "question": question })
                            .to_string(),
                    );
                }
            }
        };

        if ih.is_none() {
            self.pending.remove(&request_id);
        }

        match result {
            Ok(answer) => ToolResult::ok(
                serde_json::json!({ "answer": answer, "question": question }).to_string(),
            ),
            Err(_) => ToolResult::err("answer channel closed unexpectedly"),
        }
    }
}
