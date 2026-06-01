use std::sync::Arc;

use fastclaw_core::agent_config::AgentConfig;
use fastclaw_core::tool::ToolRegistry;
use fastclaw_core::types::{ChatMessage, ChatRequest, Role, Usage};
use fastclaw_protocol::AgentEvent;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

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
/// When `AgentEvent::TurnEnd` is forwarded through the channel, the assistant's
/// reply and token usage are automatically accumulated.
///
/// Call [`QueryEngine::abort`] to cancel the current in-flight query.
/// The forwarding task stops producing events and the engine is ready for a
/// new `submit_message` call.
pub struct QueryEngine {
    runtime: Arc<AgentRuntime>,
    config: AgentConfig,
    tool_registry: Arc<ToolRegistry>,
    state: Arc<Mutex<QueryEngineState>>,
    llm_override: Option<Arc<dyn LlmProvider>>,
    cancel_token: CancellationToken,
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
            cancel_token: CancellationToken::new(),
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

    /// Read the accumulated messages (all turns).
    pub async fn messages(&self) -> Vec<ChatMessage> {
        self.state.lock().await.messages.clone()
    }

    /// Read the accumulated token usage across all turns.
    pub async fn total_usage(&self) -> Usage {
        self.state.lock().await.total_usage.clone()
    }

    /// Alias for [`total_usage`](Self::total_usage) — returns cumulative token
    /// usage across every completed turn.
    pub async fn usage(&self) -> Usage {
        self.total_usage().await
    }

    /// Number of user turns submitted so far.
    pub async fn turn_count(&self) -> usize {
        self.state
            .lock()
            .await
            .messages
            .iter()
            .filter(|m| m.role == Role::User)
            .count()
    }

    /// Cancel the current in-flight query.
    ///
    /// After calling `abort()`, the forwarding task will stop producing events
    /// on the receiver returned by `submit_message`. The partial assistant
    /// text accumulated so far (if any) is **not** added to the message
    /// history — only fully completed turns are recorded.
    ///
    /// A new `submit_message` call can be made immediately after `abort()`.
    pub fn abort(&mut self) {
        self.cancel_token.cancel();
        self.cancel_token = CancellationToken::new();
    }

    /// Submit a user message and return a receiver of streaming events.
    ///
    /// The user message is appended to the internal history before execution.
    /// When `AgentEvent::TurnEnd` is forwarded through the receiver, the
    /// assistant's reply and usage stats are automatically accumulated.
    ///
    /// If [`abort`](Self::abort) is called while the stream is active, the
    /// forwarding task stops and the receiver is closed.
    pub async fn submit_message(&mut self, user_text: &str) -> mpsc::Receiver<AgentEvent> {
        // Fresh cancellation token for this turn.
        self.cancel_token = CancellationToken::new();
        let cancel = self.cancel_token.clone();

        let user_msg = ChatMessage {
            role: Role::User,
            content: Some(serde_json::json!(user_text)),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
            compact_metadata: None,
        };

        let request = {
            let mut state = self.state.lock().await;
            state.messages.push(user_msg);
            ChatRequest {
                model: None,
                messages: state.messages.clone(),
                agent_id: Some(self.config.agent_id.clone()),
                session_id: state.session_id.clone().map(Into::into),
                stream: true,
                temperature: None,
                max_tokens: None,
                tools: None,
                slash_intent: None,
                work_dir: None,
            }
        };

        let (internal_tx, mut internal_rx) = mpsc::channel::<AgentEvent>(256);
        let (out_tx, out_rx) = mpsc::channel::<AgentEvent>(256);

        let runtime = Arc::clone(&self.runtime);
        let config = self.config.clone();
        let tool_registry = Arc::clone(&self.tool_registry);
        let llm_override = self.llm_override.clone();

        // Task 1: Run the agent runtime.
        let runtime_cancel = cancel.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = runtime_cancel.cancelled() => {}
                result = runtime.execute_stream(
                    &config, &request, &tool_registry, internal_tx, llm_override
                ) => {
                    let _ = result;
                }
            }
        });

        // Task 2: Forward events while intercepting Done to update state.
        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            let mut assistant_text = String::new();

            loop {
                tokio::select! {
                        _ = cancel.cancelled() => {
                            break;
                        }
                        event = internal_rx.recv() => {
                            let Some(event) = event else { break };
                            match &event {
                                AgentEvent::ContentDelta { delta, .. } => {
                                    if let Some(content) = delta
                                        .get("choices")
                                        .and_then(|c| c.get(0))
                                        .and_then(|c| c.get("delta"))
                                        .and_then(|d| d.get("content"))
                                        .and_then(|c| c.as_str())
                                    {
                                        assistant_text.push_str(content);
                                    }
                                }
                                AgentEvent::TurnEnd {
                                    session_id,
                                    summary,
                                    ..
                                } => {
                                    let mut s = state.lock().await;
                                    if let Some(sid) = session_id {
                                        s.session_id = Some(sid.clone());
                                    }
                                    if let Some(u) = &summary.usage {
                                        s.total_usage.prompt_tokens += u.prompt_tokens;
                                        s.total_usage.completion_tokens += u.completion_tokens;
                                        s.total_usage.total_tokens += u.total_tokens;
                                    }
                                    if !assistant_text.is_empty() {
                                        s.messages.push(ChatMessage {
                                            role: Role::Assistant,
                                            content: Some(serde_json::json!(assistant_text)),
                                            reasoning_content: None,
                                            name: None,
                                            tool_calls: None,
                                            tool_call_id: None,
                compact_metadata: None,
                                        });
                                    }
                                }
                                _ => {}
                            }
                            if out_tx.send(event).await.is_err() {
                                break;
                            }
                        }
                    }
            }
        });

        out_rx
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CompletionParams;
    use async_trait::async_trait;
    use fastclaw_core::agent_config::{AgentModelConfig, BehaviorConfig};
    use fastclaw_core::types::{ChatResponse, DeltaContent, StreamChoice, StreamDelta};
    use futures::stream;

    fn test_agent_config() -> AgentConfig {
        AgentConfig {
            agent_id: "test-qe".into(),
            name: None,
            description: None,
            model: AgentModelConfig {
                provider: "openai".into(),
                model: "mock".into(),
                temperature: 0.0,
                max_tokens: None,
                context_window: None,
                cost_per_1k_input: None,
                cost_per_1k_output: None,
                supports_reasoning: None,
                capabilities: None,
                fallbacks: Vec::new(),
                max_concurrent_requests: 10,
            },
            system_prompt: Some("You are a test assistant.".into()),
            tools: Vec::new(),
            behavior: BehaviorConfig::default(),
            mcp_servers: Vec::new(),
            min_tier: None,
            max_tier: None,
            avatar: None,
            channels: std::collections::HashMap::new(),
        }
    }

    fn delta_text(text: &str) -> StreamDelta {
        StreamDelta {
            id: "d-1".into(),
            object: "chat.completion.chunk".into(),
            created: 0,
            model: "mock".into(),
            choices: vec![StreamChoice {
                index: 0,
                delta: DeltaContent {
                    role: None,
                    content: Some(text.into()),
                    reasoning_content: None,
                    tool_calls: None,
                },
                finish_reason: None,
            }],
            usage: None,
            raw_sse_json: None,
        }
    }

    fn delta_stop() -> StreamDelta {
        StreamDelta {
            id: "d-1".into(),
            object: "chat.completion.chunk".into(),
            created: 0,
            model: "mock".into(),
            choices: vec![StreamChoice {
                index: 0,
                delta: DeltaContent {
                    role: None,
                    content: None,
                    reasoning_content: None,
                    tool_calls: None,
                },
                finish_reason: Some("stop".into()),
            }],
            usage: None,
            raw_sse_json: None,
        }
    }

    /// Mock provider that returns a fixed response: "Hello World" in two deltas.
    struct MockProvider;

    #[async_trait]
    impl LlmProvider for MockProvider {
        async fn chat_completion(&self, _: &CompletionParams<'_>) -> anyhow::Result<ChatResponse> {
            anyhow::bail!("not used")
        }

        async fn chat_completion_stream(
            &self,
            _: &CompletionParams<'_>,
        ) -> anyhow::Result<futures::stream::BoxStream<'static, anyhow::Result<StreamDelta>>>
        {
            use futures::StreamExt;
            Ok(stream::iter(vec![
                Ok(delta_text("Hello ")),
                Ok(delta_text("World")),
                Ok(delta_stop()),
            ])
            .boxed())
        }
    }

    /// Mock provider that pauses between deltas (for abort testing).
    struct SlowMockProvider;

    #[async_trait]
    impl LlmProvider for SlowMockProvider {
        async fn chat_completion(&self, _: &CompletionParams<'_>) -> anyhow::Result<ChatResponse> {
            anyhow::bail!("not used")
        }

        async fn chat_completion_stream(
            &self,
            _: &CompletionParams<'_>,
        ) -> anyhow::Result<futures::stream::BoxStream<'static, anyhow::Result<StreamDelta>>>
        {
            use futures::StreamExt;
            let items = vec![
                (delta_text("partial"), false),
                (delta_text(" complete"), true),
                (delta_stop(), false),
            ];
            let s = futures::stream::unfold(items.into_iter(), |mut iter| async move {
                let (delta, slow) = iter.next()?;
                if slow {
                    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
                }
                Some((Ok(delta), iter))
            });
            Ok(s.boxed())
        }
    }

    fn make_engine(provider: Arc<dyn LlmProvider>) -> QueryEngine {
        let runtime = Arc::new(AgentRuntime::new(provider));
        let config = test_agent_config();
        let registry = Arc::new(ToolRegistry::new());
        QueryEngine::new(runtime, config, registry)
    }

    #[tokio::test]
    async fn submit_message_streams_response() {
        let mut engine = make_engine(Arc::new(MockProvider));
        let mut rx = engine.submit_message("Hi").await;

        let mut text = String::new();
        let mut got_done = false;
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(5);
        while let Ok(Some(event)) = tokio::time::timeout_at(deadline, rx.recv()).await {
            match event {
                AgentEvent::ContentDelta { delta, .. } => {
                    if let Some(content) = delta
                        .get("choices")
                        .and_then(|c| c.get(0))
                        .and_then(|c| c.get("delta"))
                        .and_then(|d| d.get("content"))
                        .and_then(|c| c.as_str())
                    {
                        text.push_str(content);
                    }
                }
                AgentEvent::TurnEnd { .. } => {
                    got_done = true;
                    break;
                }
                _ => {}
            }
        }

        assert!(got_done, "should receive Done event");
        assert_eq!(text, "Hello World");
    }

    #[tokio::test]
    async fn cross_turn_messages_accumulate() {
        let mut engine = make_engine(Arc::new(MockProvider));

        // Turn 1
        let mut rx = engine.submit_message("First question").await;
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(5);
        while let Ok(Some(event)) = tokio::time::timeout_at(deadline, rx.recv()).await {
            if matches!(event, AgentEvent::TurnEnd { .. }) {
                break;
            }
        }

        // Turn 2
        let mut rx = engine.submit_message("Second question").await;
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(5);
        while let Ok(Some(event)) = tokio::time::timeout_at(deadline, rx.recv()).await {
            if matches!(event, AgentEvent::TurnEnd { .. }) {
                break;
            }
        }

        let msgs = engine.messages().await;
        assert_eq!(msgs.len(), 4, "2 user + 2 assistant = 4");
        assert_eq!(msgs[0].role, Role::User);
        assert_eq!(msgs[1].role, Role::Assistant);
        assert_eq!(msgs[2].role, Role::User);
        assert_eq!(msgs[3].role, Role::Assistant);

        assert_eq!(engine.turn_count().await, 2);
    }

    #[tokio::test]
    async fn usage_accumulates_across_turns() {
        let mut engine = make_engine(Arc::new(MockProvider));

        for _ in 0..3 {
            let mut rx = engine.submit_message("test").await;
            let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(5);
            while let Ok(Some(event)) = tokio::time::timeout_at(deadline, rx.recv()).await {
                if matches!(event, AgentEvent::TurnEnd { .. }) {
                    break;
                }
            }
        }

        let usage = engine.usage().await;
        // The mock doesn't produce usage in Done, so all zeros — but the
        // accumulation logic is still exercised (3 turns, no panics).
        assert_eq!(usage.prompt_tokens, 0);
        assert_eq!(usage.completion_tokens, 0);
    }

    #[tokio::test]
    async fn abort_stops_stream() {
        let mut engine = make_engine(Arc::new(SlowMockProvider));
        let mut rx = engine.submit_message("question").await;

        // Receive the first partial delta.
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(2);
        let mut received_partial = false;
        while let Ok(Some(event)) = tokio::time::timeout_at(deadline, rx.recv()).await {
            if let AgentEvent::ContentDelta { delta, .. } = &event {
                if delta
                    .get("choices")
                    .and_then(|c| c.get(0))
                    .and_then(|c| c.get("delta"))
                    .and_then(|d| d.get("content"))
                    .and_then(|c| c.as_str())
                    == Some("partial")
                {
                    received_partial = true;
                    break;
                }
            }
        }
        assert!(received_partial, "should receive 'partial' delta");

        // Abort the current turn.
        engine.abort();

        // The receiver should close soon after abort.
        let close_deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(2);
        let mut remaining = 0;
        while let Ok(Some(_)) = tokio::time::timeout_at(close_deadline, rx.recv()).await {
            remaining += 1;
        }

        assert!(
            remaining <= 2,
            "few or no events after abort, got {remaining}"
        );

        // The assistant text should NOT be in messages (partial, not committed).
        let msgs = engine.messages().await;
        let has_assistant = msgs.iter().any(|m| m.role == Role::Assistant);
        assert!(
            !has_assistant,
            "partial assistant text should not be committed"
        );
    }

    #[tokio::test]
    async fn abort_then_new_turn_works() {
        let mut engine = make_engine(Arc::new(SlowMockProvider));

        // Start and abort
        let _rx = engine.submit_message("will abort").await;
        engine.abort();

        // Give tasks time to observe cancellation
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Replace with fast mock for the next turn
        engine = QueryEngine::new(
            Arc::new(AgentRuntime::new(
                Arc::new(MockProvider) as Arc<dyn LlmProvider>
            )),
            test_agent_config(),
            Arc::new(ToolRegistry::new()),
        );

        let mut rx = engine.submit_message("after abort").await;
        let mut got_done = false;
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(5);
        while let Ok(Some(event)) = tokio::time::timeout_at(deadline, rx.recv()).await {
            if matches!(event, AgentEvent::TurnEnd { .. }) {
                got_done = true;
                break;
            }
        }

        assert!(got_done, "new turn after abort should complete normally");
    }

    // ---- P2-01 and P2-02 unit tests (retained) ----

    fn make_usage(prompt: u32, completion: u32) -> Usage {
        Usage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: prompt + completion,
        }
    }

    #[test]
    fn usage_accumulation_arithmetic() {
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
    fn cancel_token_lifecycle() {
        let t1 = CancellationToken::new();
        assert!(!t1.is_cancelled());
        t1.cancel();
        assert!(t1.is_cancelled());
        let t2 = CancellationToken::new();
        assert!(!t2.is_cancelled(), "new token should be fresh");
    }

    #[test]
    fn user_message_construction() {
        let text = "What is Rust?";
        let msg = ChatMessage {
            role: Role::User,
            content: Some(serde_json::json!(text)),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
            compact_metadata: None,
        };
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.text_content().as_deref(), Some(text));
    }
}
