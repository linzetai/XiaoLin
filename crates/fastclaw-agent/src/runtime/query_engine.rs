use std::sync::Arc;

use fastclaw_core::agent_config::AgentConfig;
use fastclaw_core::tool::ToolRegistry;
use fastclaw_core::types::{ChatMessage, ChatRequest, Role, StreamEvent, Usage};
use tokio::sync::{mpsc, Mutex};

use super::AgentRuntime;
use crate::LlmProvider;

/// Internal mutable state shared between the engine and its forwarding task.
#[derive(Debug)]
struct QueryEngineState {
    session_id: Option<String>,
    messages: Vec<ChatMessage>,
    total_usage: Usage,
}

/// Stateful wrapper around [`AgentRuntime`] that maintains conversation history
/// across turns.
///
/// Each call to [`QueryEngine::submit_message`] appends the user message,
/// invokes the runtime's streaming execution, and returns a channel receiver.
/// When `StreamEvent::Done` is forwarded through the channel, the assistant's
/// reply and token usage are automatically accumulated.
pub struct QueryEngine {
    runtime: Arc<AgentRuntime>,
    config: AgentConfig,
    tool_registry: Arc<ToolRegistry>,
    state: Arc<Mutex<QueryEngineState>>,
    llm_override: Option<Arc<dyn LlmProvider>>,
}

impl QueryEngine {
    pub fn new(
        runtime: Arc<AgentRuntime>,
        config: AgentConfig,
        tool_registry: Arc<ToolRegistry>,
    ) -> Self {
        Self {
            runtime,
            config,
            tool_registry,
            state: Arc::new(Mutex::new(QueryEngineState {
                session_id: None,
                messages: Vec::new(),
                total_usage: Usage {
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: 0,
                },
            })),
            llm_override: None,
        }
    }

    /// Set the initial session ID.
    pub async fn set_session_id(&self, session_id: String) {
        self.state.lock().await.session_id = Some(session_id);
    }

    pub fn with_llm_override(mut self, provider: Arc<dyn LlmProvider>) -> Self {
        self.llm_override = Some(provider);
        self
    }

    /// Read the current session ID.
    pub async fn session_id(&self) -> Option<String> {
        self.state.lock().await.session_id.clone()
    }

    /// Read the accumulated messages.
    pub async fn messages(&self) -> Vec<ChatMessage> {
        self.state.lock().await.messages.clone()
    }

    /// Read the accumulated token usage across all turns.
    pub async fn total_usage(&self) -> Usage {
        self.state.lock().await.total_usage.clone()
    }

    /// Number of user+assistant turns completed.
    pub async fn turn_count(&self) -> usize {
        self.state
            .lock()
            .await
            .messages
            .iter()
            .filter(|m| m.role == Role::User)
            .count()
    }

    /// Submit a user message and return a receiver of streaming events.
    ///
    /// The user message is appended to the internal history before execution.
    /// When `StreamEvent::Done` is forwarded through the receiver, the
    /// assistant's reply and usage stats are automatically accumulated.
    pub async fn submit_message(
        &self,
        user_text: &str,
    ) -> mpsc::Receiver<StreamEvent> {
        let user_msg = ChatMessage {
            role: Role::User,
            content: Some(serde_json::json!(user_text)),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        };

        let request = {
            let mut state = self.state.lock().await;
            state.messages.push(user_msg);
            ChatRequest {
                model: None,
                messages: state.messages.clone(),
                agent_id: Some(self.config.agent_id.clone()),
                session_id: state.session_id.clone(),
                stream: true,
                temperature: None,
                max_tokens: None,
                tools: None,
                slash_intent: None,
                work_dir: None,
            }
        };

        let (internal_tx, mut internal_rx) = mpsc::channel::<StreamEvent>(256);
        let (out_tx, out_rx) = mpsc::channel::<StreamEvent>(256);

        let runtime = Arc::clone(&self.runtime);
        let config = self.config.clone();
        let tool_registry = Arc::clone(&self.tool_registry);
        let llm_override = self.llm_override.clone();

        // Task 1: Run the agent runtime.
        tokio::spawn(async move {
            let _result = runtime
                .execute_stream(&config, &request, &tool_registry, internal_tx, llm_override)
                .await;
        });

        // Task 2: Forward events while intercepting Done to update state.
        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            let mut assistant_text = String::new();

            while let Some(event) = internal_rx.recv().await {
                match &event {
                    StreamEvent::Delta(delta) => {
                        for choice in &delta.choices {
                            if let Some(ref content) = choice.delta.content {
                                assistant_text.push_str(content);
                            }
                        }
                    }
                    StreamEvent::Done {
                        session_id, usage, ..
                    } => {
                        let mut s = state.lock().await;
                        if let Some(sid) = session_id {
                            s.session_id = Some(sid.clone());
                        }
                        if let Some(u) = usage {
                            s.total_usage.prompt_tokens += u.prompt_tokens;
                            s.total_usage.completion_tokens += u.completion_tokens;
                            s.total_usage.total_tokens += u.total_tokens;
                        }
                        if !assistant_text.is_empty() {
                            s.messages.push(ChatMessage {
                                role: Role::Assistant,
                                content: Some(serde_json::json!(assistant_text)),
                                name: None,
                                tool_calls: None,
                                tool_call_id: None,
                            });
                        }
                    }
                    _ => {}
                }
                if out_tx.send(event).await.is_err() {
                    break;
                }
            }
        });

        out_rx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_usage(prompt: u32, completion: u32) -> Usage {
        Usage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: prompt + completion,
        }
    }

    #[test]
    fn usage_accumulation() {
        let mut total = Usage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
        };
        for u in [make_usage(100, 50), make_usage(200, 80)] {
            total.prompt_tokens += u.prompt_tokens;
            total.completion_tokens += u.completion_tokens;
            total.total_tokens += u.total_tokens;
        }
        assert_eq!(total.prompt_tokens, 300);
        assert_eq!(total.completion_tokens, 130);
        assert_eq!(total.total_tokens, 430);
    }

    #[test]
    fn messages_accumulate_across_turns() {
        let mut messages: Vec<ChatMessage> = Vec::new();

        let pairs = [("hello", "hi there"), ("how are you?", "I'm good!")];
        for (user_text, assistant_text) in pairs {
            messages.push(ChatMessage {
                role: Role::User,
                content: Some(serde_json::json!(user_text)),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            });
            messages.push(ChatMessage {
                role: Role::Assistant,
                content: Some(serde_json::json!(assistant_text)),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            });
        }

        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages[1].role, Role::Assistant);
        assert_eq!(messages[2].role, Role::User);
        assert_eq!(messages[3].role, Role::Assistant);
    }

    #[tokio::test]
    async fn state_mutex_serializes_cross_turn_access() {
        let state = Arc::new(Mutex::new(QueryEngineState {
            session_id: None,
            messages: Vec::new(),
            total_usage: Usage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
            },
        }));

        // Turn 1: user message + usage
        {
            let mut s = state.lock().await;
            s.messages.push(ChatMessage {
                role: Role::User,
                content: Some(serde_json::json!("turn 1")),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            });
            s.total_usage.prompt_tokens += 100;
            s.session_id = Some("sess-1".to_string());
        }

        // Turn 1: assistant reply + completion usage
        {
            let mut s = state.lock().await;
            s.messages.push(ChatMessage {
                role: Role::Assistant,
                content: Some(serde_json::json!("reply 1")),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            });
            s.total_usage.completion_tokens += 50;
        }

        // Turn 2: user message + usage
        {
            let mut s = state.lock().await;
            s.messages.push(ChatMessage {
                role: Role::User,
                content: Some(serde_json::json!("turn 2")),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            });
            s.total_usage.prompt_tokens += 120;
        }

        let s = state.lock().await;
        assert_eq!(s.messages.len(), 3);
        assert_eq!(s.total_usage.prompt_tokens, 220);
        assert_eq!(s.total_usage.completion_tokens, 50);
        assert_eq!(s.session_id, Some("sess-1".to_string()));
    }

    #[test]
    fn user_message_construction() {
        let text = "What is Rust?";
        let msg = ChatMessage {
            role: Role::User,
            content: Some(serde_json::json!(text)),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        };
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.text_content().as_deref(), Some(text));
    }
}
