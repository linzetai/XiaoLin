//! Unified side-query abstraction for non-main-loop LLM calls.
//!
//! All auxiliary LLM queries (memory consolidation, skill extraction,
//! auto-classification, agent summary, etc.) go through `side_query()`.
//! Features:
//!
//! - Independent retry/abort/cost tracking per call
//! - `optional=true` → failures return `None` instead of `Err`
//! - `Background` source queries skip 529 retries to avoid amplification
//! - `CancellationToken` can interrupt the query at any await point

use std::sync::Arc;
use std::time::{Duration, Instant};

use fastclaw_core::types::ChatMessage;
use tokio_util::sync::CancellationToken;

use super::retry::{with_retry, QuerySource, RetryConfig};
use crate::llm::{CompletionParams, LlmProvider};

/// Where the side-query originates, affecting retry aggressiveness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideQuerySource {
    /// Memory consolidation, dreaming pipeline, skill extraction.
    Background,
    /// Auto-mode classifier, agent summary, memory selection.
    Foreground,
}

impl SideQuerySource {
    fn to_query_source(self) -> QuerySource {
        match self {
            Self::Background => QuerySource::Background,
            Self::Foreground => QuerySource::Agent,
        }
    }
}

/// Options for a single side-query invocation.
#[derive(Debug, Clone)]
pub struct SideQueryOptions {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<ChatMessage>,
    pub max_tokens: Option<u32>,
    pub temperature: f32,
    pub max_retries: u32,
    pub query_source: SideQuerySource,
    /// If true, LLM failure returns Ok(None) instead of Err.
    pub optional: bool,
    pub abort: Option<CancellationToken>,
}

impl Default for SideQueryOptions {
    fn default() -> Self {
        Self {
            model: String::new(),
            system: None,
            messages: Vec::new(),
            max_tokens: Some(1024),
            temperature: 0.0,
            max_retries: 2,
            query_source: SideQuerySource::Foreground,
            optional: false,
            abort: None,
        }
    }
}

/// Outcome of a side-query, including timing and token usage.
#[derive(Debug, Clone)]
pub struct SideQueryResult {
    pub content: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub duration: Duration,
}

/// Handle that tools can use to invoke side queries during execution.
///
/// Stored in a task-local so tools don't need direct access to the LLM provider.
/// Usage from a tool's `execute()`:
/// ```ignore
/// if let Some(handle) = SideQueryHandle::current() {
///     let answer = handle.query("Summarize this error", &messages).await?;
/// }
/// ```
#[derive(Clone)]
pub struct SideQueryHandle {
    provider: Arc<dyn LlmProvider>,
    model: String,
    abort: CancellationToken,
}

tokio::task_local! {
    static SIDE_QUERY_HANDLE: SideQueryHandle;
}

impl SideQueryHandle {
    pub fn new(provider: Arc<dyn LlmProvider>, model: String, abort: CancellationToken) -> Self {
        Self {
            provider,
            model,
            abort,
        }
    }

    /// Execute a closure with this handle available as the task-local.
    pub async fn scope<F, R>(self, f: F) -> R
    where
        F: std::future::Future<Output = R>,
    {
        SIDE_QUERY_HANDLE.scope(self, f).await
    }

    /// Get the current handle from the task-local, if set.
    pub fn current() -> Option<Self> {
        SIDE_QUERY_HANDLE.try_with(|h| h.clone()).ok()
    }

    /// Run a quick side-query with a system prompt and user messages.
    pub async fn query(
        &self,
        system: &str,
        messages: Vec<ChatMessage>,
    ) -> anyhow::Result<Option<SideQueryResult>> {
        let opts = SideQueryOptions {
            model: self.model.clone(),
            system: Some(system.to_string()),
            messages,
            max_tokens: Some(1024),
            temperature: 0.0,
            max_retries: 1,
            query_source: SideQuerySource::Foreground,
            optional: true,
            abort: Some(self.abort.clone()),
        };
        side_query(&self.provider, opts).await
    }

    /// Quick one-shot: pass a single user message and get a text response.
    pub async fn ask(&self, system: &str, question: &str) -> Option<String> {
        let msgs = vec![ChatMessage {
            role: fastclaw_core::types::Role::User,
            content: Some(serde_json::Value::String(question.to_string())),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
            compact_metadata: None,
        }];
        match self.query(system, msgs).await {
            Ok(Some(r)) => Some(r.content),
            _ => None,
        }
    }
}

/// Execute a side-query against the given LLM provider.
///
/// Returns `Ok(Some(result))` on success, `Ok(None)` if optional and failed,
/// or `Err` if non-optional and failed.
pub async fn side_query(
    provider: &Arc<dyn LlmProvider>,
    opts: SideQueryOptions,
) -> anyhow::Result<Option<SideQueryResult>> {
    let start = Instant::now();
    let abort = opts.abort.clone().unwrap_or_default();

    let retry_config = RetryConfig {
        max_retries: opts.max_retries,
        base_delay: Duration::from_millis(500),
        max_delay: Duration::from_secs(15),
        max_529_retries: if opts.query_source == SideQuerySource::Background {
            0
        } else {
            2
        },
        allow_credential_refresh: false,
    };

    let provider_ref = provider.clone();
    let model = opts.model.clone();
    let messages = opts.messages.clone();
    let max_tokens = opts.max_tokens;
    let temperature = opts.temperature;
    let tools: Option<&[fastclaw_core::tool::ToolDefinition]> = None;
    let source = opts.query_source.to_query_source();

    let result = tokio::select! {
        _ = abort.cancelled() => {
            Err(anyhow::anyhow!("side-query cancelled via abort token"))
        }
        res = with_retry(
            &retry_config,
            source,
            || {
                let p = provider_ref.clone();
                let m = model.clone();
                let msgs = messages.clone();
                async move {
                    let params = CompletionParams {
                        model: &m,
                        messages: &msgs,
                        max_tokens,
                        temperature,
                        tools,
                    };
                    p.chat_completion(&params).await
                }
            },
            |_state, _decision, _kind| {},
        ) => { res }
    };

    match result {
        Ok(response) => {
            let content = response
                .choices
                .first()
                .and_then(|c| c.message.content.as_ref())
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let (input_tokens, output_tokens) = response
                .usage
                .as_ref()
                .map(|u| (u.prompt_tokens, u.completion_tokens))
                .unwrap_or((0, 0));

            Ok(Some(SideQueryResult {
                content,
                input_tokens,
                output_tokens,
                duration: start.elapsed(),
            }))
        }
        Err(e) => {
            if opts.optional {
                tracing::debug!(error = %e, "optional side-query failed, returning None");
                Ok(None)
            } else {
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use fastclaw_core::types::{ChatChoice, ChatResponse, Role, StreamDelta, Usage};
    use std::sync::atomic::{AtomicU32, Ordering};

    struct MockProvider {
        response: Option<String>,
        call_count: Arc<AtomicU32>,
        fail_times: u32,
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        async fn chat_completion(
            &self,
            _params: &CompletionParams<'_>,
        ) -> anyhow::Result<ChatResponse> {
            let n = self.call_count.fetch_add(1, Ordering::Relaxed);
            if n < self.fail_times {
                return Err(anyhow::anyhow!("Connection reset by peer"));
            }
            let content = self.response.clone().unwrap_or_default();
            Ok(ChatResponse {
                id: "test".into(),
                object: "chat.completion".into(),
                created: 0,
                model: "test".into(),
                choices: vec![ChatChoice {
                    index: 0,
                    message: ChatMessage {
                        role: Role::Assistant,
                        content: Some(serde_json::Value::String(content)),
                        reasoning_content: None,
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
            compact_metadata: None,
                    },
                    finish_reason: Some("stop".into()),
                }],
                usage: Some(Usage {
                    prompt_tokens: 50,
                    completion_tokens: 20,
                    total_tokens: 70,
                }),
            })
        }

        async fn chat_completion_stream(
            &self,
            _params: &CompletionParams<'_>,
        ) -> anyhow::Result<futures::stream::BoxStream<'static, anyhow::Result<StreamDelta>>>
        {
            unimplemented!("not needed for side_query tests")
        }
    }

    fn mock_provider(response: &str, fail_times: u32) -> (Arc<dyn LlmProvider>, Arc<AtomicU32>) {
        let count = Arc::new(AtomicU32::new(0));
        let p = MockProvider {
            response: Some(response.to_string()),
            call_count: count.clone(),
            fail_times,
        };
        (Arc::new(p), count)
    }

    fn base_opts() -> SideQueryOptions {
        SideQueryOptions {
            model: "test-model".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: Some(serde_json::Value::String("test".into())),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
            compact_metadata: None,
            }],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn successful_side_query() {
        let (provider, count) = mock_provider("hello world", 0);
        let opts = base_opts();
        let result = side_query(&provider, opts).await.unwrap().unwrap();
        assert_eq!(result.content, "hello world");
        assert_eq!(result.input_tokens, 50);
        assert_eq!(result.output_tokens, 20);
        assert_eq!(count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn optional_returns_none_on_failure() {
        let count = Arc::new(AtomicU32::new(0));
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider {
            response: None,
            call_count: count.clone(),
            fail_times: 100,
        });
        let mut opts = base_opts();
        opts.optional = true;
        opts.max_retries = 1;
        let result = side_query(&provider, opts).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn non_optional_returns_err_on_failure() {
        let count = Arc::new(AtomicU32::new(0));
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider {
            response: None,
            call_count: count.clone(),
            fail_times: 100,
        });
        let mut opts = base_opts();
        opts.optional = false;
        opts.max_retries = 1;
        let result = side_query(&provider, opts).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn abort_cancels_side_query() {
        let token = CancellationToken::new();
        let token_clone = token.clone();

        let count = Arc::new(AtomicU32::new(0));
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider {
            response: Some("result".into()),
            call_count: count,
            fail_times: 100,
        });

        let mut opts = base_opts();
        opts.abort = Some(token_clone);
        opts.max_retries = 10;

        // Cancel immediately
        token.cancel();

        let result = side_query(&provider, opts).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cancelled"));
    }

    #[tokio::test]
    async fn retry_recovers_after_transient_failure() {
        let (provider, count) = mock_provider("recovered", 1);
        let mut opts = base_opts();
        opts.max_retries = 3;
        let result = side_query(&provider, opts).await.unwrap().unwrap();
        assert_eq!(result.content, "recovered");
        assert_eq!(count.load(Ordering::Relaxed), 2);
    }
}
