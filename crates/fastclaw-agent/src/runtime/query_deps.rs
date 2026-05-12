use std::sync::Arc;

use async_trait::async_trait;
use fastclaw_core::types::{ChatMessage, ChatResponse, StreamDelta};
use futures::stream::BoxStream;
use tokio::sync::Mutex;

use super::unified_compact::{unified_pre_query_compact, UnifiedCompactResult};
use crate::llm::{CompletionParams, LlmProvider};

/// Dependency injection trait for the agent query loop.
///
/// Abstracts LLM calls and context compression so that:
/// - `ProductionDeps` delegates to real providers and the context pipeline.
/// - `MockDeps` allows unit tests to verify loop logic without LLM calls.
#[allow(dead_code, clippy::too_many_arguments)]
#[async_trait]
pub(crate) trait QueryDeps: Send + Sync {
    /// Non-streaming LLM call.
    async fn call_model(&self, params: &CompletionParams<'_>) -> anyhow::Result<ChatResponse>;

    /// Streaming LLM call.
    async fn call_model_stream(
        &self,
        params: &CompletionParams<'_>,
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<StreamDelta>>>;

    /// Run the unified pre-query compression pipeline.
    async fn pre_query_compact(
        &self,
        messages: &mut Vec<ChatMessage>,
        context_window: u32,
        max_tokens: Option<u32>,
        model: &str,
        last_estimated_tokens: usize,
        iteration_boundaries: &[(usize, std::time::Instant)],
        todo_store: Option<&crate::builtin_tools::TodoStore>,
        enable_smart_compression: bool,
    ) -> UnifiedCompactResult;

    /// Emergency reactive compaction (prompt_too_long recovery).
    fn reactive_compact(&self, messages: &[ChatMessage])
        -> fastclaw_context::ReactiveCompactResult;

    /// Provider name for metrics.
    fn provider_name(&self) -> &str;
}

/// Production implementation of QueryDeps.
///
/// Wraps a real LlmProvider and a stateful ContextPipeline (behind Mutex for
/// interior mutability since trait methods take `&self`).
pub(crate) struct ProductionDeps {
    provider: Arc<dyn LlmProvider>,
    pipeline: Mutex<fastclaw_context::ContextPipeline>,
}

impl ProductionDeps {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        pipeline: fastclaw_context::ContextPipeline,
    ) -> Self {
        Self {
            provider,
            pipeline: Mutex::new(pipeline),
        }
    }
}

#[async_trait]
impl QueryDeps for ProductionDeps {
    async fn call_model(&self, params: &CompletionParams<'_>) -> anyhow::Result<ChatResponse> {
        self.provider.chat_completion(params).await
    }

    async fn call_model_stream(
        &self,
        params: &CompletionParams<'_>,
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<StreamDelta>>> {
        self.provider.chat_completion_stream(params).await
    }

    async fn pre_query_compact(
        &self,
        messages: &mut Vec<ChatMessage>,
        context_window: u32,
        max_tokens: Option<u32>,
        model: &str,
        last_estimated_tokens: usize,
        iteration_boundaries: &[(usize, std::time::Instant)],
        todo_store: Option<&crate::builtin_tools::TodoStore>,
        enable_smart_compression: bool,
    ) -> UnifiedCompactResult {
        let mut pipeline = self.pipeline.lock().await;
        unified_pre_query_compact(
            messages,
            &mut pipeline,
            context_window,
            max_tokens,
            &self.provider,
            model,
            last_estimated_tokens,
            iteration_boundaries,
            todo_store,
            enable_smart_compression,
        )
        .await
    }

    fn reactive_compact(
        &self,
        messages: &[ChatMessage],
    ) -> fastclaw_context::ReactiveCompactResult {
        let pipeline = self.pipeline.blocking_lock();
        pipeline.reactive_compact(messages)
    }

    fn provider_name(&self) -> &str {
        self.provider.provider_name()
    }
}

#[cfg(test)]
pub(crate) mod mock {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Mock implementation for unit testing loop behavior.
    ///
    /// Configurable responses allow tests to simulate tool_calls, end_turn,
    /// errors, and compression outcomes without real LLM calls.
    pub(crate) struct MockDeps {
        responses: Mutex<Vec<anyhow::Result<ChatResponse>>>,
        compact_results: Mutex<Vec<UnifiedCompactResult>>,
        call_count: AtomicU32,
    }

    impl MockDeps {
        pub fn new(responses: Vec<anyhow::Result<ChatResponse>>) -> Self {
            Self {
                responses: Mutex::new(responses),
                compact_results: Mutex::new(Vec::new()),
                call_count: AtomicU32::new(0),
            }
        }

        #[allow(dead_code)]
        pub fn with_compact_results(mut self, results: Vec<UnifiedCompactResult>) -> Self {
            self.compact_results = Mutex::new(results);
            self
        }

        pub fn call_count(&self) -> u32 {
            self.call_count.load(Ordering::Relaxed)
        }
    }

    #[async_trait]
    impl QueryDeps for MockDeps {
        async fn call_model(&self, _params: &CompletionParams<'_>) -> anyhow::Result<ChatResponse> {
            self.call_count.fetch_add(1, Ordering::Relaxed);
            let mut responses = self.responses.lock().await;
            if responses.is_empty() {
                anyhow::bail!("MockDeps: no more responses configured");
            }
            responses.remove(0)
        }

        async fn call_model_stream(
            &self,
            _params: &CompletionParams<'_>,
        ) -> anyhow::Result<BoxStream<'static, anyhow::Result<StreamDelta>>> {
            anyhow::bail!("MockDeps: call_model_stream not implemented for unit tests")
        }

        async fn pre_query_compact(
            &self,
            messages: &mut Vec<ChatMessage>,
            _context_window: u32,
            _max_tokens: Option<u32>,
            _model: &str,
            _last_estimated_tokens: usize,
            _iteration_boundaries: &[(usize, std::time::Instant)],
            _todo_store: Option<&crate::builtin_tools::TodoStore>,
            _enable_smart_compression: bool,
        ) -> UnifiedCompactResult {
            let mut results = self.compact_results.lock().await;
            if results.is_empty() {
                let est = fastclaw_context::estimate_messages_tokens(messages);
                UnifiedCompactResult {
                    estimated_tokens: est,
                    compressed_by_llm: false,
                    tokens_saved_by_llm: 0,
                    pipeline_applied: false,
                    session_memory_extracted: false,
                }
            } else {
                results.remove(0)
            }
        }

        fn reactive_compact(
            &self,
            _messages: &[ChatMessage],
        ) -> fastclaw_context::ReactiveCompactResult {
            fastclaw_context::ReactiveCompactResult {
                recovered: false,
                messages: Vec::new(),
                tokens_after: 0,
                level_used: None,
            }
        }

        fn provider_name(&self) -> &str {
            "mock"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockDeps;
    use super::*;
    use fastclaw_core::types::{ChatChoice, ChatMessage, ChatResponse, Role, Usage};

    fn end_turn_response(content: &str) -> ChatResponse {
        ChatResponse {
            id: "resp_1".into(),
            object: "chat.completion".into(),
            created: 0,
            model: "test".into(),
            choices: vec![ChatChoice {
                index: 0,
                message: ChatMessage {
                    role: Role::Assistant,
                    content: Some(serde_json::json!(content)),
                    reasoning_content: None,
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                finish_reason: Some("stop".into()),
            }],
            usage: Some(Usage {
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: 150,
            }),
        }
    }

    #[tokio::test]
    async fn mock_deps_returns_configured_responses() {
        let deps = MockDeps::new(vec![
            Ok(end_turn_response("Hello")),
            Ok(end_turn_response("World")),
        ]);

        let params = CompletionParams {
            model: "test",
            messages: &[],
            temperature: 0.7,
            max_tokens: None,
            tools: None,
        };

        let r1 = deps.call_model(&params).await.unwrap();
        assert_eq!(r1.choices[0].message.text_content().unwrap(), "Hello");

        let r2 = deps.call_model(&params).await.unwrap();
        assert_eq!(r2.choices[0].message.text_content().unwrap(), "World");

        assert_eq!(deps.call_count(), 2);
    }

    #[tokio::test]
    async fn mock_deps_errors_when_exhausted() {
        let deps = MockDeps::new(vec![]);
        let params = CompletionParams {
            model: "test",
            messages: &[],
            temperature: 0.7,
            max_tokens: None,
            tools: None,
        };
        let result = deps.call_model(&params).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn mock_deps_pre_query_compact_default() {
        let deps = MockDeps::new(vec![]);
        let mut messages = vec![ChatMessage {
            role: Role::User,
            content: Some(serde_json::json!("test message")),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }];

        let result = deps
            .pre_query_compact(&mut messages, 128_000, None, "test", 0, &[], None, true)
            .await;
        assert!(!result.compressed_by_llm);
        assert!(!result.pipeline_applied);
    }

    #[tokio::test]
    async fn mock_deps_can_simulate_failures() {
        let deps = MockDeps::new(vec![
            Err(anyhow::anyhow!("rate limited")),
            Ok(end_turn_response("recovered")),
        ]);

        let params = CompletionParams {
            model: "test",
            messages: &[],
            temperature: 0.7,
            max_tokens: None,
            tools: None,
        };

        let r1 = deps.call_model(&params).await;
        assert!(r1.is_err());

        let r2 = deps.call_model(&params).await.unwrap();
        assert_eq!(r2.choices[0].message.text_content().unwrap(), "recovered");
    }

    #[tokio::test]
    async fn production_deps_provider_name() {
        use std::sync::Arc;

        struct TestProvider;

        #[async_trait]
        impl LlmProvider for TestProvider {
            async fn chat_completion(
                &self,
                _params: &CompletionParams<'_>,
            ) -> anyhow::Result<ChatResponse> {
                unimplemented!()
            }
            async fn chat_completion_stream(
                &self,
                _params: &CompletionParams<'_>,
            ) -> anyhow::Result<BoxStream<'static, anyhow::Result<StreamDelta>>> {
                unimplemented!()
            }
            fn provider_name(&self) -> &str {
                "test-provider"
            }
        }

        let pipeline = fastclaw_context::ContextPipeline::new(Default::default());
        let deps = ProductionDeps::new(Arc::new(TestProvider), pipeline);
        assert_eq!(deps.provider_name(), "test-provider");
    }
}
