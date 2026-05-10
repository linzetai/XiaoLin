use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use fastclaw_core::tool::ToolDefinition;
use fastclaw_core::types::{
    ChatChoice, ChatMessage, ChatResponse, DeltaContent, StreamChoice, StreamDelta,
    StreamFunctionDelta, StreamToolCallDelta, Usage,
};
use rand::Rng;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

/// Parameters for a single LLM call.
pub struct CompletionParams<'a> {
    pub model: &'a str,
    pub messages: &'a [ChatMessage],
    pub temperature: f32,
    pub max_tokens: Option<u32>,
    pub tools: Option<&'a [ToolDefinition]>,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat_completion(&self, params: &CompletionParams<'_>) -> anyhow::Result<ChatResponse>;

    async fn chat_completion_stream(
        &self,
        params: &CompletionParams<'_>,
    ) -> anyhow::Result<futures::stream::BoxStream<'static, anyhow::Result<StreamDelta>>>;

    /// Human-readable provider name for metrics labeling.
    fn provider_name(&self) -> &str {
        "unknown"
    }
}

/// Backoff / retry policy for OpenAI-compatible HTTP calls.
#[derive(Clone, Debug)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    pub jitter_factor: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay_ms: 1000,
            max_delay_ms: 30000,
            jitter_factor: 0.5,
        }
    }
}

/// Per-request timeouts for the OpenAI-compatible HTTP client.
#[derive(Clone, Debug)]
pub struct TimeoutConfig {
    pub connect_timeout_ms: u64,
    pub request_timeout_ms: u64,
    /// Stream timeout for SSE streaming requests (default 10 minutes).
    pub stream_timeout_ms: u64,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            connect_timeout_ms: 10000,
            request_timeout_ms: 300000, // 5 minutes (was 2 minutes)
            stream_timeout_ms: 600000, // 10 minutes for streaming
        }
    }
}

fn build_openai_http_client(timeouts: &TimeoutConfig) -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("FastClaw/0.1.0")
        .connect_timeout(Duration::from_millis(timeouts.connect_timeout_ms))
        .timeout(Duration::from_millis(timeouts.request_timeout_ms))
        .build()
        .unwrap_or_else(|e| {
            tracing::warn!(
                error = %e,
                "failed to build OpenAI reqwest client with timeouts, falling back to default client"
            );
            reqwest::Client::new()
        })
}

fn is_retryable_http_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504)
}

fn is_retryable_reqwest_error(e: &reqwest::Error) -> bool {
    if e.is_decode() {
        return false;
    }
    e.is_timeout() || e.is_connect()
}

/// Structured LLM error with both user-facing message and technical details.
#[derive(Debug, Clone)]
pub struct LlmApiError {
    pub user_message: String,
    pub technical_detail: String,
    pub error_code: LlmErrorCode,
    pub retryable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum LlmErrorCode {
    RateLimited,
    QuotaExceeded,
    BalanceError,
    ModelOverloaded,
    InvalidRequest,
    AuthenticationFailed,
    ServerError,
    Timeout,
    NetworkError,
    Unknown,
}

impl std::fmt::Display for LlmApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.user_message)
    }
}

impl std::error::Error for LlmApiError {}

/// Parse an HTTP error response into a structured, user-friendly error.
pub fn classify_llm_error(status: StatusCode, body: &str) -> LlmApiError {
    let parsed: Option<serde_json::Value> = serde_json::from_str(body).ok();

    let error_code_str = parsed.as_ref()
        .and_then(|v| v.get("error"))
        .and_then(|e| e.get("code"))
        .and_then(|c| c.as_str())
        .unwrap_or("");

    let error_message = parsed.as_ref()
        .and_then(|v| v.get("error"))
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or(body);

    let error_type = parsed.as_ref()
        .and_then(|v| v.get("error"))
        .and_then(|e| e.get("type"))
        .and_then(|t| t.as_str())
        .unwrap_or("");

    let (code, user_msg, retryable) = match (status.as_u16(), error_code_str, error_type) {
        (429, _, _) if error_message.contains("quota") || error_code_str == "insufficient_quota" => (
            LlmErrorCode::QuotaExceeded,
            "API 配额已用完，请检查账户余额或升级计划。".to_string(),
            false,
        ),
        (429, _, _) => (
            LlmErrorCode::RateLimited,
            "请求频率过高，正在自动重试...".to_string(),
            true,
        ),
        (_, "BalanceError", _) | (_, _, "BalanceError") => (
            LlmErrorCode::BalanceError,
            "模型服务暂时不可用（无可用集群）。这通常是临时性的，请稍后重试。".to_string(),
            true,
        ),
        (500, _, _) if error_message.contains("no suitable cluster") || error_message.contains("No suitable") => (
            LlmErrorCode::BalanceError,
            "模型服务暂时不可用（无可用集群）。这通常是临时性的，请稍后重试。".to_string(),
            true,
        ),
        (500, _, _) if error_message.contains("overloaded") || error_message.contains("capacity") => (
            LlmErrorCode::ModelOverloaded,
            "模型当前负载过高，请稍后重试。".to_string(),
            true,
        ),
        (503, _, _) | (502, _, _) => (
            LlmErrorCode::ModelOverloaded,
            "模型服务暂时不可用，正在自动重试...".to_string(),
            true,
        ),
        (500, _, _) | (504, _, _) => (
            LlmErrorCode::ServerError,
            "模型服务内部错误，正在自动重试...".to_string(),
            true,
        ),
        (401, _, _) | (403, _, _) => (
            LlmErrorCode::AuthenticationFailed,
            "API 密钥无效或已过期，请在设置中检查并更新。".to_string(),
            false,
        ),
        (400, _, _) if error_message.contains("context_length") || error_message.contains("token") => (
            LlmErrorCode::InvalidRequest,
            "对话内容超出模型上下文长度限制，请尝试清理历史消息或开始新对话。".to_string(),
            false,
        ),
        (400, _, _) => (
            LlmErrorCode::InvalidRequest,
            format!("请求参数错误：{}", truncate_for_user(error_message, 100)),
            false,
        ),
        _ => (
            LlmErrorCode::Unknown,
            format!("模型服务返回错误 (HTTP {})，请稍后重试。", status.as_u16()),
            is_retryable_http_status(status),
        ),
    };

    LlmApiError {
        user_message: user_msg,
        technical_detail: format!("HTTP {status}: {}", truncate_for_user(body, 500)),
        error_code: code,
        retryable,
    }
}

fn truncate_for_user(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        let end = s.floor_char_boundary(max_len);
        &s[..end]
    }
}

async fn acquire_llm_semaphore_permit(
    sem: &Option<Arc<tokio::sync::Semaphore>>,
) -> anyhow::Result<Option<tokio::sync::OwnedSemaphorePermit>> {
    match sem {
        None => Ok(None),
        Some(s) => {
            let permit = s
                .clone()
                .acquire_owned()
                .await
                .map_err(|e| anyhow::anyhow!("LLM concurrency semaphore closed: {e}"))?;
            Ok(Some(permit))
        }
    }
}

fn openai_backoff_delay_ms(cfg: &RetryConfig, consecutive_failures_before_wait: u32) -> u64 {
    let exp = consecutive_failures_before_wait.min(30);
    let mut base = cfg.initial_delay_ms;
    for _ in 0..exp {
        base = base.saturating_mul(2).min(cfg.max_delay_ms);
    }
    base = base.min(cfg.max_delay_ms);
    let jf = cfg.jitter_factor.clamp(0.0, 1.0);
    let low = (1.0_f64 - jf).max(0.0);
    let high = 1.0_f64 + jf;
    let mult = rand::thread_rng().gen_range(low..=high);
    ((base as f64) * mult).round().max(1.0) as u64
}

pub struct OpenAiProvider {
    client: reqwest::Client,
    /// Separate client with longer timeout for SSE streaming requests.
    stream_client: reqwest::Client,
    base_url: String,
    api_key: String,
    retry: RetryConfig,
    /// Limits concurrent in-flight HTTP requests for this provider instance.
    pub concurrency_semaphore: Option<Arc<tokio::sync::Semaphore>>,
}

impl OpenAiProvider {
    pub fn new(base_url: &str, api_key: &str) -> Self {
        Self::with_options(
            base_url,
            api_key,
            TimeoutConfig::default(),
            RetryConfig::default(),
            None,
        )
    }

    /// Build an OpenAI-compatible provider with explicit timeout and retry settings.
    pub fn with_options(
        base_url: &str,
        api_key: &str,
        timeouts: TimeoutConfig,
        retry: RetryConfig,
        max_concurrent_requests: Option<u32>,
    ) -> Self {
        let concurrency_semaphore = {
            let n = max_concurrent_requests.unwrap_or(10).max(1) as usize;
            Some(Arc::new(tokio::sync::Semaphore::new(n)))
        };
        let stream_timeouts = TimeoutConfig {
            connect_timeout_ms: timeouts.connect_timeout_ms,
            request_timeout_ms: timeouts.stream_timeout_ms,
            stream_timeout_ms: timeouts.stream_timeout_ms,
        };
        Self {
            client: build_openai_http_client(&timeouts),
            stream_client: build_openai_http_client(&stream_timeouts),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            retry,
            concurrency_semaphore,
        }
    }

    fn build_request<'a>(&self, params: &CompletionParams<'a>, stream: bool) -> OpenAiRequest<'a> {
        let tool_choice = params.tools.filter(|t| !t.is_empty()).map(|_| "auto");
        let stream_options = if stream {
            Some(StreamOptions { include_usage: true })
        } else {
            None
        };
        OpenAiRequest {
            model: params.model,
            messages: params.messages,
            temperature: params.temperature,
            max_tokens: params.max_tokens,
            stream,
            stream_options,
            tools: params.tools,
            tool_choice,
        }
    }

    async fn send_openai_chat_request(
        &self,
        params: &CompletionParams<'_>,
        stream: bool,
    ) -> Result<reqwest::Response, reqwest::Error> {
        let url = format!("{}/chat/completions", self.base_url);
        let body = self.build_request(params, stream);
        let client = if stream { &self.stream_client } else { &self.client };
        client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
    }

    /// Establish a streaming chat completion response, retrying only connection / early HTTP failures.
    async fn connect_openai_chat_stream(
        &self,
        params: &CompletionParams<'_>,
    ) -> anyhow::Result<reqwest::Response> {
        let max_attempts = (self.retry.max_retries as usize).saturating_add(1).max(1);

        for attempt in 0..max_attempts {
            match self.send_openai_chat_request(params, true).await {
                Ok(resp) => {
                    if resp.status().is_success() {
                        return Ok(resp);
                    }
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    let classified = classify_llm_error(status, &text);
                    if classified.retryable && attempt + 1 < max_attempts {
                        let delay_ms = openai_backoff_delay_ms(&self.retry, attempt as u32);
                        tracing::warn!(
                            retry_after_attempt = attempt + 1,
                            max_retries = self.retry.max_retries,
                            delay_ms,
                            status = %status,
                            error_code = ?classified.error_code,
                            technical_detail = %classified.technical_detail,
                            "OpenAI chat stream connection failed, retrying"
                        );
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                        continue;
                    }
                    return Err(classified.into());
                }
                Err(e) => {
                    let retryable = is_retryable_reqwest_error(&e);
                    let err = anyhow::Error::from(e);
                    if retryable && attempt + 1 < max_attempts {
                        let delay_ms = openai_backoff_delay_ms(&self.retry, attempt as u32);
                        tracing::warn!(
                            retry_after_attempt = attempt + 1,
                            max_retries = self.retry.max_retries,
                            delay_ms,
                            error = %err,
                            "OpenAI chat stream connection failed, retrying"
                        );
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                        continue;
                    }
                    return Err(err);
                }
            }
        }

        unreachable!("OpenAI stream connection attempts exhausted without returning")
    }
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Serialize)]
struct OpenAiRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [ToolDefinition]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'a str>,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    id: String,
    object: String,
    created: u64,
    model: String,
    choices: Vec<OpenAiChoice>,
    usage: Option<OpenAiUsage>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    index: u32,
    message: ChatMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct OpenAiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn chat_completion(&self, params: &CompletionParams<'_>) -> anyhow::Result<ChatResponse> {
        let _llm_permit = acquire_llm_semaphore_permit(&self.concurrency_semaphore).await?;

        let max_attempts = (self.retry.max_retries as usize).saturating_add(1).max(1);

        for attempt in 0..max_attempts {
            match self.send_openai_chat_request(params, false).await {
                Ok(resp) => {
                    if !resp.status().is_success() {
                        let status = resp.status();
                        let text = resp.text().await.unwrap_or_default();
                        let classified = classify_llm_error(status, &text);
                        if classified.retryable && attempt + 1 < max_attempts {
                            let delay_ms = openai_backoff_delay_ms(&self.retry, attempt as u32);
                            tracing::warn!(
                                retry_after_attempt = attempt + 1,
                                max_retries = self.retry.max_retries,
                                delay_ms,
                                status = %status,
                                error_code = ?classified.error_code,
                                technical_detail = %classified.technical_detail,
                                "OpenAI chat completion failed, retrying"
                            );
                            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                            continue;
                        }
                        return Err(classified.into());
                    }

                    let bytes = match resp.bytes().await {
                        Ok(b) => b,
                        Err(e) => {
                            if is_retryable_reqwest_error(&e) && attempt + 1 < max_attempts {
                                let delay_ms = openai_backoff_delay_ms(&self.retry, attempt as u32);
                                let err = anyhow::Error::from(e);
                                tracing::warn!(
                                    retry_after_attempt = attempt + 1,
                                    max_retries = self.retry.max_retries,
                                    delay_ms,
                                    error = %err,
                                    "OpenAI chat completion body read failed, retrying"
                                );
                                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                                continue;
                            }
                            return Err(anyhow::Error::from(e));
                        }
                    };

                    let api_resp: OpenAiResponse = match serde_json::from_slice(&bytes) {
                        Ok(v) => v,
                        Err(e) => return Err(e.into()),
                    };

                    return Ok(ChatResponse {
                        id: api_resp.id,
                        object: api_resp.object,
                        created: api_resp.created,
                        model: api_resp.model,
                        choices: api_resp
                            .choices
                            .into_iter()
                            .map(|c| ChatChoice {
                                index: c.index,
                                message: c.message,
                                finish_reason: c.finish_reason,
                            })
                            .collect(),
                        usage: api_resp.usage.map(|u| Usage {
                            prompt_tokens: u.prompt_tokens,
                            completion_tokens: u.completion_tokens,
                            total_tokens: u.total_tokens,
                        }),
                    });
                }
                Err(e) => {
                    if is_retryable_reqwest_error(&e) && attempt + 1 < max_attempts {
                        let delay_ms = openai_backoff_delay_ms(&self.retry, attempt as u32);
                        let err = anyhow::Error::from(e);
                        tracing::warn!(
                            retry_after_attempt = attempt + 1,
                            max_retries = self.retry.max_retries,
                            delay_ms,
                            error = %err,
                            "OpenAI chat completion request failed, retrying"
                        );
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                        continue;
                    }
                    return Err(anyhow::Error::from(e));
                }
            }
        }

        anyhow::bail!("OpenAI chat completion: exceeded max retries")
    }

    async fn chat_completion_stream(
        &self,
        params: &CompletionParams<'_>,
    ) -> anyhow::Result<futures::stream::BoxStream<'static, anyhow::Result<StreamDelta>>> {
        use futures::stream::{self, StreamExt};

        let llm_permit = acquire_llm_semaphore_permit(&self.concurrency_semaphore).await?;
        let resp = self.connect_openai_chat_stream(params).await?;

        let byte_stream = resp.bytes_stream();

        // SSE lines can be split across network chunks, so buffer partial lines.
        let delta_stream = {
            let mut line_buf = String::new();

            byte_stream
                .map(move |chunk_result| {
                    let _hold = &llm_permit;
                    let chunk = match chunk_result {
                        Ok(c) => c,
                        Err(e) => {
                            if e.is_timeout() || e.is_connect() {
                                tracing::warn!(error = %e, "stream interrupted (reconnectable)");
                                return vec![Err(anyhow::anyhow!("stream read error (reconnectable): {e}"))];
                            }
                            tracing::warn!(error = %e, "stream decode error, partial data may be lost");
                            return vec![Err(anyhow::anyhow!("stream read error: {e}"))];
                        }
                    };
                    let text = String::from_utf8_lossy(&chunk);
                    line_buf.push_str(&text);

                    let mut deltas = Vec::new();

                    while let Some(pos) = line_buf.find('\n') {
                        let line: String = line_buf.drain(..=pos).collect();
                        let line = line.trim().to_string();
                        if line.is_empty() || !line.starts_with("data: ") {
                            continue;
                        }
                        let data = &line[6..];
                        if data == "[DONE]" {
                            continue;
                        }
                        match serde_json::from_str::<StreamDelta>(data) {
                            Ok(delta) => deltas.push(Ok(delta)),
                            Err(e) => {
                                tracing::debug!(error = %e, "failed to parse SSE chunk");
                            }
                        }
                    }
                    deltas
                })
                .flat_map(stream::iter)
        };

        Ok(Box::pin(delta_stream))
    }

    fn provider_name(&self) -> &str {
        "openai"
    }
}

/// Create a single provider by name.
///
/// Credentials are resolved in order:
/// 1. Explicit `base_url` / `api_key` parameters
/// 2. `credentials` config store (looked up by `provider_name`)
/// 3. Built-in defaults (base URL only; API key defaults to empty)
pub fn create_provider(
    provider_name: &str,
    base_url: Option<&str>,
    api_key: Option<&str>,
) -> anyhow::Result<Box<dyn LlmProvider>> {
    create_provider_with_credentials(provider_name, base_url, api_key, None, None)
}

/// Create a provider with an optional credentials store for fallback lookup.
///
/// `max_concurrent_requests`: `None` uses the default limit (10).
pub fn create_provider_with_credentials(
    provider_name: &str,
    base_url: Option<&str>,
    api_key: Option<&str>,
    credentials: Option<&fastclaw_core::config::CredentialsConfig>,
    max_concurrent_requests: Option<u32>,
) -> anyhow::Result<Box<dyn LlmProvider>> {
    match provider_name {
        "openai" | "openai_compatible" | "azure" | "compatible" | "dashscope" | "deepseek"
        | "together" | "groq" | "ollama" | "lmstudio" | "vllm" | "google" | "gemini" => {
            let default_base = provider_default_base_url(provider_name);
            let base = base_url
                .map(String::from)
                .or_else(|| {
                    credentials
                        .and_then(|c| c.get_base_url(provider_name))
                        .map(String::from)
                })
                .unwrap_or_else(|| default_base.to_string());
            let key = api_key
                .map(String::from)
                .or_else(|| {
                    credentials
                        .and_then(|c| c.get_api_key(provider_name))
                        .map(String::from)
                })
                .unwrap_or_default();
            tracing::info!(
                provider = provider_name,
                base_url = %base,
                api_key_len = key.len(),
                "LLM provider initialized"
            );
            Ok(Box::new(OpenAiProvider::with_options(
                &base,
                &key,
                TimeoutConfig::default(),
                RetryConfig::default(),
                max_concurrent_requests,
            )))
        }
        "anthropic" => {
            let base = base_url
                .map(String::from)
                .or_else(|| {
                    credentials
                        .and_then(|c| c.get_base_url("anthropic"))
                        .map(String::from)
                })
                .unwrap_or_else(|| "https://api.anthropic.com".to_string());
            let key = api_key
                .map(String::from)
                .or_else(|| {
                    credentials
                        .and_then(|c| c.get_api_key("anthropic"))
                        .map(String::from)
                })
                .unwrap_or_default();
            Ok(Box::new(AnthropicProvider::with_options(
                &base,
                &key,
                max_concurrent_requests,
            )))
        }
        other => {
            let base = base_url
                .map(String::from)
                .or_else(|| {
                    credentials
                        .and_then(|c| c.get_base_url(other))
                        .map(String::from)
                })
                .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
            let key = api_key
                .map(String::from)
                .or_else(|| {
                    credentials
                        .and_then(|c| c.get_api_key(other))
                        .map(String::from)
                })
                .unwrap_or_default();
            tracing::warn!(
                provider = other,
                base_url = %base,
                api_key_len = key.len(),
                "unknown provider id, using OpenAI-compatible transport"
            );
            Ok(Box::new(OpenAiProvider::with_options(
                &base,
                &key,
                TimeoutConfig::default(),
                RetryConfig::default(),
                max_concurrent_requests,
            )))
        }
    }
}

fn provider_default_base_url(name: &str) -> &'static str {
    match name {
        "dashscope" => "https://dashscope.aliyuncs.com/compatible-mode/v1",
        "deepseek" => "https://api.deepseek.com/v1",
        "together" => "https://api.together.xyz/v1",
        "groq" => "https://api.groq.com/openai/v1",
        "ollama" => "http://localhost:11434/v1",
        "lmstudio" => "http://localhost:1234/v1",
        "vllm" => "http://localhost:8000/v1",
        "google" | "gemini" => "https://generativelanguage.googleapis.com/v1beta/openai",
        "openai_compatible" => "https://api.openai.com/v1",
        _ => "https://api.openai.com/v1",
    }
}

// ---------- Anthropic Provider (Messages API) ----------

pub struct AnthropicProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    pub concurrency_semaphore: Option<Arc<tokio::sync::Semaphore>>,
}

impl AnthropicProvider {
    pub fn new(base_url: &str, api_key: &str) -> Self {
        Self::with_options(base_url, api_key, None)
    }

    pub fn with_options(base_url: &str, api_key: &str, max_concurrent_requests: Option<u32>) -> Self {
        let n = max_concurrent_requests.unwrap_or(10).max(1) as usize;
        let client = reqwest::Client::builder()
            .user_agent("FastClaw/0.1.0")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            concurrency_semaphore: Some(Arc::new(tokio::sync::Semaphore::new(n))),
        }
    }
}

#[derive(Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    messages: Vec<AnthropicMessage>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
}

#[derive(Serialize, Deserialize)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: serde_json::Value,
}

#[derive(Serialize)]
pub struct AnthropicTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    id: String,
    model: String,
    content: Vec<AnthropicContentBlock>,
    stop_reason: Option<String>,
    usage: AnthropicUsage,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum AnthropicContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

impl AnthropicProvider {
    pub fn convert_messages(messages: &[ChatMessage]) -> (Option<String>, Vec<AnthropicMessage>) {
        let mut system_parts: Vec<String> = Vec::new();
        let mut result = Vec::new();

        for msg in messages {
            match msg.role {
                fastclaw_core::types::Role::System => {
                    if let Some(t) = msg.text_content() {
                        if !t.is_empty() {
                            system_parts.push(t);
                        }
                    }
                }
                fastclaw_core::types::Role::User => {
                    let content = match &msg.content {
                        Some(serde_json::Value::Array(arr)) => {
                            let converted: Vec<serde_json::Value> = arr
                                .iter()
                                .filter_map(|part| {
                                    let ptype = part.get("type").and_then(|v| v.as_str())?;
                                    match ptype {
                                        "text" => Some(part.clone()),
                                        "image_url" => {
                                            let url = part
                                                .get("image_url")
                                                .and_then(|iu| iu.get("url"))
                                                .and_then(|u| u.as_str())?;
                                            if let Some(rest) = url.strip_prefix("data:") {
                                                let (header, data) = rest.split_once(";base64,")?;
                                                Some(serde_json::json!({
                                                    "type": "image",
                                                    "source": {
                                                        "type": "base64",
                                                        "media_type": header,
                                                        "data": data,
                                                    }
                                                }))
                                            } else {
                                                Some(serde_json::json!({
                                                    "type": "image",
                                                    "source": {
                                                        "type": "url",
                                                        "url": url,
                                                    }
                                                }))
                                            }
                                        }
                                        _ => Some(part.clone()),
                                    }
                                })
                                .collect();
                            serde_json::Value::Array(converted)
                        }
                        other => other.clone().unwrap_or(serde_json::Value::String(String::new())),
                    };
                    result.push(AnthropicMessage {
                        role: "user".to_string(),
                        content,
                    });
                }
                fastclaw_core::types::Role::Assistant => {
                    let mut blocks = Vec::new();
                    if let Some(t) = msg.text_content() {
                        if !t.is_empty() {
                            blocks.push(serde_json::json!({"type": "text", "text": t}));
                        }
                    }
                    if let Some(ref tcs) = msg.tool_calls {
                        for tc in tcs {
                            let input: serde_json::Value =
                                serde_json::from_str(&tc.function.arguments)
                                    .unwrap_or(serde_json::Value::Object(Default::default()));
                            blocks.push(serde_json::json!({
                                "type": "tool_use",
                                "id": tc.id,
                                "name": tc.function.name,
                                "input": input,
                            }));
                        }
                    }
                    result.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content: serde_json::Value::Array(blocks),
                    });
                }
                fastclaw_core::types::Role::Tool => {
                    result.push(AnthropicMessage {
                        role: "user".to_string(),
                        content: serde_json::json!([{
                            "type": "tool_result",
                            "tool_use_id": msg.tool_call_id.as_deref().unwrap_or(""),
                            "content": msg.text_content().unwrap_or_default(),
                        }]),
                    });
                }
            }
        }

        let system = if system_parts.is_empty() {
            None
        } else {
            Some(system_parts.join("\n\n"))
        };
        (system, result)
    }

    pub fn convert_tools(defs: &[ToolDefinition]) -> Vec<AnthropicTool> {
        defs.iter()
            .map(|d| AnthropicTool {
                name: d.function.name.clone(),
                description: d.function.description.clone(),
                input_schema: serde_json::to_value(&d.function.parameters)
                    .unwrap_or(serde_json::json!({"type": "object"})),
            })
            .collect()
    }

    fn to_chat_response(resp: AnthropicResponse) -> ChatResponse {
        let mut content_parts = Vec::new();
        let mut tool_calls = Vec::new();

        for block in &resp.content {
            match block {
                AnthropicContentBlock::Text { text } => content_parts.push(text.clone()),
                AnthropicContentBlock::ToolUse { id, name, input } => {
                    tool_calls.push(fastclaw_core::types::ToolCall {
                        id: id.clone(),
                        call_type: "function".to_string(),
                        function: fastclaw_core::types::FunctionCall {
                            name: name.clone(),
                            arguments: serde_json::to_string(input).unwrap_or_default(),
                        },
                        output: None,
                        success: None,
                        duration_ms: None,
                    });
                }
            }
        }

        let finish_reason = match resp.stop_reason.as_deref() {
            Some("tool_use") => "tool_calls".to_string(),
            Some(r) => r.to_string(),
            None => "stop".to_string(),
        };

        ChatResponse {
            id: resp.id,
            object: "chat.completion".to_string(),
            created: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            model: resp.model,
            choices: vec![ChatChoice {
                index: 0,
                message: ChatMessage {
                    role: fastclaw_core::types::Role::Assistant,
                    content: if content_parts.is_empty() {
                        None
                    } else {
                        Some(serde_json::Value::String(content_parts.join("")))
                    },
                    reasoning_content: None,
                    name: None,
                    tool_calls: if tool_calls.is_empty() {
                        None
                    } else {
                        Some(tool_calls)
                    },
                    tool_call_id: None,
                },
                finish_reason: Some(finish_reason),
            }],
            usage: Some(Usage {
                prompt_tokens: resp.usage.input_tokens,
                completion_tokens: resp.usage.output_tokens,
                total_tokens: resp.usage.input_tokens + resp.usage.output_tokens,
            }),
        }
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn chat_completion(&self, params: &CompletionParams<'_>) -> anyhow::Result<ChatResponse> {
        let _llm_permit = acquire_llm_semaphore_permit(&self.concurrency_semaphore).await?;

        let url = format!("{}/v1/messages", self.base_url);
        let (system, messages) = Self::convert_messages(params.messages);
        let tools = params
            .tools
            .filter(|t| !t.is_empty())
            .map(Self::convert_tools);

        let body = AnthropicRequest {
            model: params.model,
            messages,
            max_tokens: params.max_tokens.unwrap_or(4096),
            system,
            tools,
            stream: false,
        };

        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API error: {status} — {text}");
        }

        let api_resp: AnthropicResponse = resp.json().await?;
        Ok(Self::to_chat_response(api_resp))
    }

    async fn chat_completion_stream(
        &self,
        params: &CompletionParams<'_>,
    ) -> anyhow::Result<futures::stream::BoxStream<'static, anyhow::Result<StreamDelta>>> {
        use futures::stream::{self, StreamExt};

        let llm_permit = acquire_llm_semaphore_permit(&self.concurrency_semaphore).await?;

        let url = format!("{}/v1/messages", self.base_url);
        let (system, messages) = Self::convert_messages(params.messages);
        let tools = params
            .tools
            .filter(|t| !t.is_empty())
            .map(Self::convert_tools);

        let body = AnthropicRequest {
            model: params.model,
            messages,
            max_tokens: params.max_tokens.unwrap_or(4096),
            system,
            tools,
            stream: true,
        };

        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API error: {status} — {text}");
        }

        let byte_stream = resp.bytes_stream();

        let delta_stream = {
            let mut line_buf = String::new();
            let mut current_event = String::new();
            let mut msg_id = String::new();
            let mut model_name = String::new();
            let mut stop_reason: Option<String> = None;
            let mut tool_streams: HashMap<u32, (String, String, String)> = HashMap::new();
            let mut input_tokens: u32 = 0;
            let mut output_tokens: u32 = 0;

            byte_stream
                .map(move |chunk_result| {
                    let _hold = &llm_permit;
                    let chunk = match chunk_result {
                        Ok(c) => c,
                        Err(e) => {
                            if e.is_timeout() || e.is_connect() {
                                tracing::warn!(error = %e, "stream interrupted (reconnectable)");
                                return vec![Err(anyhow::anyhow!("stream read error (reconnectable): {e}"))];
                            }
                            tracing::warn!(error = %e, "stream decode error, partial data may be lost");
                            return vec![Err(anyhow::anyhow!("stream read error: {e}"))];
                        }
                    };
                    let text = String::from_utf8_lossy(&chunk);
                    line_buf.push_str(&text);

                    let mut deltas = Vec::new();

                    while let Some(pos) = line_buf.find('\n') {
                        let line: String = line_buf.drain(..=pos).collect();
                        let line = line.trim().to_string();

                        if let Some(event) = line.strip_prefix("event: ") {
                            current_event = event.to_string();
                            continue;
                        }

                        if !line.starts_with("data: ") {
                            continue;
                        }
                        let data = &line[6..];

                        match current_event.as_str() {
                            "message_start" => {
                                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                                    if let Some(m) = v.get("message") {
                                        msg_id = m
                                            .get("id")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string();
                                        model_name = m
                                            .get("model")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string();
                                        if let Some(u) = m.get("usage") {
                                            input_tokens = u.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
                                        }
                                    }
                                }
                            }
                            "message_delta" => {
                                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                                    if let Some(delta) = v.get("delta") {
                                        if let Some(sr) =
                                            delta.get("stop_reason").and_then(|x| x.as_str())
                                        {
                                            stop_reason = Some(sr.to_string());
                                        }
                                    }
                                    if let Some(u) = v.get("usage") {
                                        output_tokens = u.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
                                    }
                                }
                            }
                            "content_block_start" => {
                                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                                    let index =
                                        v.get("index").and_then(|x| x.as_u64()).map(|u| u as u32);
                                    if let (Some(idx), Some(block)) =
                                        (index, v.get("content_block"))
                                    {
                                        let typ = block
                                            .get("type")
                                            .and_then(|t| t.as_str())
                                            .unwrap_or("");
                                        if matches!(typ, "tool_use" | "server_tool_use") {
                                            if let (Some(id), Some(name)) = (
                                                block.get("id").and_then(|x| x.as_str()),
                                                block.get("name").and_then(|x| x.as_str()),
                                            ) {
                                                tool_streams.insert(
                                                    idx,
                                                    (
                                                        id.to_string(),
                                                        name.to_string(),
                                                        String::new(),
                                                    ),
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                            "content_block_delta" => {
                                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                                    let index =
                                        v.get("index").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
                                    if let Some(delta_obj) = v.get("delta") {
                                        let delta_type = delta_obj
                                            .get("type")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        match delta_type {
                                            "text_delta" => {
                                                let text = delta_obj
                                                    .get("text")
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("");
                                                if !text.is_empty() {
                                                    let now = std::time::SystemTime::now()
                                                        .duration_since(std::time::UNIX_EPOCH)
                                                        .map(|d| d.as_secs())
                                                        .unwrap_or(0);
                                                    deltas.push(Ok(StreamDelta {
                                                        id: msg_id.clone(),
                                                        object: "chat.completion.chunk".to_string(),
                                                        created: now,
                                                        model: model_name.clone(),
                                                        choices: vec![StreamChoice {
                                                            index: 0,
                                                            delta: DeltaContent {
                                                                role: None,
                                                                content: Some(text.to_string()),
                                                                reasoning_content: None,
                                                                tool_calls: None,
                                                            },
                                                            finish_reason: None,
                                                        }],
                                                        usage: None,
                                                    }));
                                                }
                                            }
                                            "input_json_delta" => {
                                                let partial = delta_obj
                                                    .get("partial_json")
                                                    .and_then(|x| x.as_str())
                                                    .unwrap_or("");
                                                if let Some(entry) = tool_streams.get_mut(&index) {
                                                    entry.2.push_str(partial);
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                            "content_block_stop" => {
                                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                                    let index =
                                        v.get("index").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
                                    if let Some((id, name, args)) = tool_streams.remove(&index) {
                                        if !name.is_empty() {
                                            let now = std::time::SystemTime::now()
                                                .duration_since(std::time::UNIX_EPOCH)
                                                .map(|d| d.as_secs())
                                                .unwrap_or(0);
                                            deltas.push(Ok(StreamDelta {
                                                id: msg_id.clone(),
                                                object: "chat.completion.chunk".to_string(),
                                                created: now,
                                                model: model_name.clone(),
                                                choices: vec![StreamChoice {
                                                    index: 0,
                                                    delta: DeltaContent {
                                                        role: None,
                                                        content: None,
                                                        reasoning_content: None,
                                                        tool_calls: Some(vec![
                                                            StreamToolCallDelta {
                                                                index,
                                                                id: Some(id),
                                                                call_type: Some(
                                                                    "function".to_string(),
                                                                ),
                                                                function: Some(
                                                                    StreamFunctionDelta {
                                                                        name: Some(name),
                                                                        arguments: Some(args),
                                                                    },
                                                                ),
                                                            },
                                                        ]),
                                                    },
                                                    finish_reason: None,
                                                }],
                                                usage: None,
                                            }));
                                        }
                                    }
                                }
                            }
                            "message_stop" => {
                                let now = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_secs())
                                    .unwrap_or(0);
                                let finish = match stop_reason.take().as_deref() {
                                    Some("tool_use") => Some("tool_calls".to_string()),
                                    Some(s) if !s.is_empty() => Some(s.to_string()),
                                    _ => Some("stop".to_string()),
                                };
                                let total = input_tokens + output_tokens;
                                let usage = if total > 0 {
                                    Some(Usage {
                                        prompt_tokens: input_tokens,
                                        completion_tokens: output_tokens,
                                        total_tokens: total,
                                    })
                                } else {
                                    None
                                };
                                deltas.push(Ok(StreamDelta {
                                    id: msg_id.clone(),
                                    object: "chat.completion.chunk".to_string(),
                                    created: now,
                                    model: model_name.clone(),
                                    choices: vec![StreamChoice {
                                        index: 0,
                                        delta: DeltaContent {
                                            role: None,
                                            content: None,
                                            reasoning_content: None,
                                            tool_calls: None,
                                        },
                                        finish_reason: finish,
                                    }],
                                    usage,
                                }));
                            }
                            _ => {}
                        }
                        current_event.clear();
                    }
                    deltas
                })
                .flat_map(stream::iter)
        };

        Ok(Box::pin(delta_stream))
    }

    fn provider_name(&self) -> &str {
        "anthropic"
    }
}

// ---------- Circuit Breaker ----------

/// Three-state circuit breaker: Closed (healthy), Open (broken), HalfOpen (probing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

struct CircuitBreakerInner {
    state: CircuitState,
    failure_count: u32,
    last_failure: Option<std::time::Instant>,
}

/// Per-provider circuit breaker with configurable thresholds.
pub struct CircuitBreaker {
    breakers: dashmap::DashMap<String, std::sync::Mutex<CircuitBreakerInner>>,
    failure_threshold: u32,
    recovery_timeout: std::time::Duration,
}

impl CircuitBreaker {
    pub fn new(failure_threshold: u32, recovery_timeout: std::time::Duration) -> Self {
        Self {
            breakers: dashmap::DashMap::new(),
            failure_threshold,
            recovery_timeout,
        }
    }

    /// Check if a provider is available (not in Open state).
    pub fn is_available(&self, provider: &str) -> bool {
        let entry = self.breakers.entry(provider.to_string()).or_insert_with(|| {
            std::sync::Mutex::new(CircuitBreakerInner {
                state: CircuitState::Closed,
                failure_count: 0,
                last_failure: None,
            })
        });
        let mut inner = entry.lock().unwrap_or_else(|e| e.into_inner());
        match inner.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                if let Some(last) = inner.last_failure {
                    if last.elapsed() >= self.recovery_timeout {
                        inner.state = CircuitState::HalfOpen;
                        tracing::info!(provider, "circuit breaker half-open, probing");
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => true,
        }
    }

    /// Record a success — resets to Closed.
    pub fn record_success(&self, provider: &str) {
        let entry = self.breakers.entry(provider.to_string()).or_insert_with(|| {
            std::sync::Mutex::new(CircuitBreakerInner {
                state: CircuitState::Closed,
                failure_count: 0,
                last_failure: None,
            })
        });
        let mut inner = entry.lock().unwrap_or_else(|e| e.into_inner());
        if inner.state != CircuitState::Closed {
            tracing::info!(provider, "circuit breaker closed (recovered)");
        }
        inner.state = CircuitState::Closed;
        inner.failure_count = 0;
        inner.last_failure = None;
    }

    /// Record a failure — may transition to Open.
    pub fn record_failure(&self, provider: &str) {
        let entry = self.breakers.entry(provider.to_string()).or_insert_with(|| {
            std::sync::Mutex::new(CircuitBreakerInner {
                state: CircuitState::Closed,
                failure_count: 0,
                last_failure: None,
            })
        });
        let mut inner = entry.lock().unwrap_or_else(|e| e.into_inner());
        inner.failure_count += 1;
        inner.last_failure = Some(std::time::Instant::now());
        if inner.failure_count >= self.failure_threshold {
            inner.state = CircuitState::Open;
            tracing::warn!(provider, failures = inner.failure_count, "circuit breaker opened");
        }
    }

    pub fn state(&self, provider: &str) -> CircuitState {
        self.breakers
            .get(provider)
            .map(|e| e.lock().unwrap_or_else(|e| e.into_inner()).state)
            .unwrap_or(CircuitState::Closed)
    }
}

// ---------- Fallback Provider ----------

/// Tries providers in order, falling back to the next on failure.
/// Integrates with an optional [`CircuitBreaker`] to skip unhealthy providers.
pub struct FallbackProvider {
    providers: Vec<(String, Box<dyn LlmProvider>)>,
    circuit_breaker: Option<Arc<CircuitBreaker>>,
}

impl FallbackProvider {
    pub fn new(providers: Vec<(String, Box<dyn LlmProvider>)>) -> Self {
        Self {
            providers,
            circuit_breaker: None,
        }
    }

    pub fn with_circuit_breaker(mut self, cb: Arc<CircuitBreaker>) -> Self {
        self.circuit_breaker = Some(cb);
        self
    }

    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }
}

#[async_trait]
impl LlmProvider for FallbackProvider {
    async fn chat_completion(&self, params: &CompletionParams<'_>) -> anyhow::Result<ChatResponse> {
        let mut last_err = anyhow::anyhow!("no providers configured");
        for (name, provider) in &self.providers {
            if let Some(ref cb) = self.circuit_breaker {
                if !cb.is_available(name) {
                    tracing::info!(provider = %name, "circuit breaker open, skipping");
                    continue;
                }
            }
            match provider.chat_completion(params).await {
                Ok(resp) => {
                    if let Some(ref cb) = self.circuit_breaker {
                        cb.record_success(name);
                    }
                    return Ok(resp);
                }
                Err(e) => {
                    tracing::warn!(provider = %name, error = %e, "provider failed, trying fallback");
                    if let Some(ref cb) = self.circuit_breaker {
                        cb.record_failure(name);
                    }
                    last_err = e;
                }
            }
        }
        Err(last_err)
    }

    async fn chat_completion_stream(
        &self,
        params: &CompletionParams<'_>,
    ) -> anyhow::Result<futures::stream::BoxStream<'static, anyhow::Result<StreamDelta>>> {
        let mut last_err = anyhow::anyhow!("no providers configured");
        for (name, provider) in &self.providers {
            if let Some(ref cb) = self.circuit_breaker {
                if !cb.is_available(name) {
                    tracing::info!(provider = %name, "circuit breaker open, skipping (stream)");
                    continue;
                }
            }
            match provider.chat_completion_stream(params).await {
                Ok(stream) => {
                    if let Some(ref cb) = self.circuit_breaker {
                        cb.record_success(name);
                    }
                    return Ok(stream);
                }
                Err(e) => {
                    tracing::warn!(provider = %name, error = %e, "stream provider failed, trying fallback");
                    if let Some(ref cb) = self.circuit_breaker {
                        cb.record_failure(name);
                    }
                    last_err = e;
                }
            }
        }
        Err(last_err)
    }

    fn provider_name(&self) -> &str {
        "fallback"
    }
}

/// Create a provider chain: primary + fallbacks from agent config.
///
/// Credentials are resolved from the centralized `CredentialsConfig` store,
/// falling back to explicit values in `FallbackModelConfig`.
pub fn create_provider_chain(
    config: &fastclaw_core::agent_config::AgentModelConfig,
    credentials: Option<&fastclaw_core::config::CredentialsConfig>,
) -> anyhow::Result<Box<dyn LlmProvider>> {
    let primary = create_provider_with_credentials(
        &config.provider,
        None,
        None,
        credentials,
        Some(config.max_concurrent_requests),
    )?;

    if config.fallbacks.is_empty() {
        return Ok(primary);
    }

    let mut chain: Vec<(String, Box<dyn LlmProvider>)> = Vec::new();
    chain.push((config.provider.clone(), primary));

    for fb in &config.fallbacks {
        match create_provider_with_credentials(
            &fb.provider,
            fb.base_url.as_deref(),
            fb.api_key.as_deref(),
            credentials,
            Some(fb.max_concurrent_requests),
        ) {
            Ok(p) => chain.push((format!("{}:{}", fb.provider, fb.model), p)),
            Err(e) => {
                tracing::warn!(
                    provider = %fb.provider,
                    error = %e,
                    "failed to create fallback provider, skipping"
                );
            }
        }
    }

    Ok(Box::new(FallbackProvider::new(chain)))
}

/// Plugin-aware provider creation. If `provider_name` starts with `plugin:`,
/// the suffix is used as a plugin ID to look up in the registry.
/// Otherwise delegates to [`create_provider_with_credentials`].
pub fn create_provider_with_plugins(
    provider_name: &str,
    base_url: Option<&str>,
    api_key: Option<&str>,
    credentials: Option<&fastclaw_core::config::CredentialsConfig>,
    max_concurrent_requests: Option<u32>,
    plugin_registry: Option<&crate::llm_plugin::LlmPluginRegistry>,
) -> anyhow::Result<Box<dyn LlmProvider>> {
    if let Some(plugin_id) = provider_name.strip_prefix("plugin:") {
        if let Some(registry) = plugin_registry {
            return registry.create_provider(plugin_id);
        }
        anyhow::bail!(
            "provider '{}' refers to a plugin but no plugin registry is available",
            provider_name
        );
    }
    create_provider_with_credentials(
        provider_name,
        base_url,
        api_key,
        credentials,
        max_concurrent_requests,
    )
}

/// Plugin-aware provider chain creation. Extends [`create_provider_chain`]
/// with plugin lookup for both primary and fallback providers.
pub fn create_provider_chain_with_plugins(
    config: &fastclaw_core::agent_config::AgentModelConfig,
    credentials: Option<&fastclaw_core::config::CredentialsConfig>,
    plugin_registry: Option<&crate::llm_plugin::LlmPluginRegistry>,
) -> anyhow::Result<Box<dyn LlmProvider>> {
    let primary = create_provider_with_plugins(
        &config.provider,
        None,
        None,
        credentials,
        Some(config.max_concurrent_requests),
        plugin_registry,
    )?;

    if config.fallbacks.is_empty() {
        return Ok(primary);
    }

    let mut chain: Vec<(String, Box<dyn LlmProvider>)> = Vec::new();
    chain.push((config.provider.clone(), primary));

    for fb in &config.fallbacks {
        match create_provider_with_plugins(
            &fb.provider,
            fb.base_url.as_deref(),
            fb.api_key.as_deref(),
            credentials,
            Some(fb.max_concurrent_requests),
            plugin_registry,
        ) {
            Ok(p) => chain.push((format!("{}:{}", fb.provider, fb.model), p)),
            Err(e) => {
                tracing::warn!(
                    provider = %fb.provider,
                    error = %e,
                    "failed to create fallback provider, skipping"
                );
            }
        }
    }

    Ok(Box::new(FallbackProvider::new(chain)))
}

/// Resolve the effective context window for an agent model config.
///
/// Priority chain (first non-None wins):
///   1. Explicit `config.context_window` from agent JSON
///   2. Plugin model entry `context_window` (when provider is `plugin:<id>`)
///   3. Model-name heuristic via [`infer_context_window_from_model`]
pub fn resolve_context_window(
    config: &fastclaw_core::agent_config::AgentModelConfig,
    plugin_registry: Option<&crate::llm_plugin::LlmPluginRegistry>,
) -> u32 {
    if let Some(w) = config.context_window {
        return w;
    }
    if let (Some(plugin_id), Some(registry)) = (
        config.provider.strip_prefix("plugin:"),
        plugin_registry,
    ) {
        if let Some(w) = registry.find_model_context_window(plugin_id, &config.model) {
            tracing::debug!(
                plugin_id,
                model = %config.model,
                context_window = w,
                "resolved context_window from plugin model entry"
            );
            return w;
        }
    }
    fastclaw_context::infer_context_window_from_model(&config.model)
}

/// Fill in `context_window` for every agent whose config leaves it `None`
/// and whose provider is a plugin with a known model entry.
///
/// Call this once after loading / hot-reloading agent configs so the runtime
/// picks up the plugin-declared limit instead of falling back to model-name
/// heuristics.
pub fn patch_agent_context_windows(
    agents: &mut [fastclaw_core::agent_config::AgentConfig],
    plugin_registry: Option<&crate::llm_plugin::LlmPluginRegistry>,
) {
    for agent in agents.iter_mut() {
        if agent.model.context_window.is_some() {
            continue;
        }
        let resolved = resolve_context_window(&agent.model, plugin_registry);
        let inferred = fastclaw_context::infer_context_window_from_model(&agent.model.model);
        if resolved != inferred {
            tracing::info!(
                agent_id = %agent.agent_id,
                provider = %agent.model.provider,
                model = %agent.model.model,
                resolved_context_window = resolved,
                inferred_context_window = inferred,
                "patched context_window from plugin model entry"
            );
            agent.model.context_window = Some(resolved);
        }
    }
}

#[cfg(test)]
mod semaphore_tests {
    use super::*;
    use fastclaw_core::config::{CredentialsConfig, ProviderCredential};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn acquire_llm_semaphore_permit_limits_parallelism() {
        let sem = Some(Arc::new(tokio::sync::Semaphore::new(3)));
        let peak = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..12 {
            let sem = sem.clone();
            let peak = peak.clone();
            let active = active.clone();
            handles.push(tokio::spawn(async move {
                let _p = acquire_llm_semaphore_permit(&sem)
                    .await
                    .expect("semaphore acquire");
                let v = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(v, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                active.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for h in handles {
            h.await.expect("join");
        }
        assert!(
            peak.load(Ordering::SeqCst) <= 3,
            "same acquire pattern as OpenAiProvider chat_completion"
        );
    }

    #[test]
    fn create_provider_supports_custom_openai_compatible_key() {
        let mut creds = CredentialsConfig::default();
        creds.providers.insert(
            "my_openai".to_string(),
            ProviderCredential {
                api_key: Some("sk-test".to_string()),
                base_url: Some("https://api.openai.com/v1".to_string()),
            },
        );

        let provider = create_provider_with_credentials("my_openai", None, None, Some(&creds), None);
        assert!(provider.is_ok(), "custom provider key should be accepted");
    }
}

#[cfg(test)]
mod circuit_breaker_tests {
    use super::*;
    use std::time::Duration;

    fn breaker(threshold: u32, timeout_ms: u64) -> CircuitBreaker {
        CircuitBreaker::new(threshold, Duration::from_millis(timeout_ms))
    }

    #[test]
    fn new_starts_closed() {
        let cb = breaker(3, 1000);
        assert_eq!(cb.state("openai"), CircuitState::Closed);
        assert!(cb.is_available("openai"));
    }

    #[test]
    fn opens_after_threshold() {
        let cb = breaker(3, 1000);
        cb.record_failure("p");
        cb.record_failure("p");
        assert_eq!(cb.state("p"), CircuitState::Closed);
        cb.record_failure("p");
        assert_eq!(cb.state("p"), CircuitState::Open);
        assert!(!cb.is_available("p"));
    }

    #[test]
    fn stays_closed_below_threshold() {
        let cb = breaker(5, 1000);
        for _ in 0..4 {
            cb.record_failure("p");
        }
        assert_eq!(cb.state("p"), CircuitState::Closed);
        assert!(cb.is_available("p"));
    }

    #[test]
    fn record_success_resets_count() {
        let cb = breaker(3, 1000);
        cb.record_failure("p");
        cb.record_failure("p");
        cb.record_success("p");
        assert_eq!(cb.state("p"), CircuitState::Closed);
        cb.record_failure("p");
        cb.record_failure("p");
        assert_eq!(cb.state("p"), CircuitState::Closed);
    }

    #[tokio::test]
    async fn half_open_after_recovery_timeout() {
        let cb = breaker(2, 50);
        cb.record_failure("p");
        cb.record_failure("p");
        assert_eq!(cb.state("p"), CircuitState::Open);
        assert!(!cb.is_available("p"));

        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(cb.is_available("p"));
        assert_eq!(cb.state("p"), CircuitState::HalfOpen);
    }

    #[tokio::test]
    async fn half_open_success_closes() {
        let cb = breaker(2, 50);
        cb.record_failure("p");
        cb.record_failure("p");
        tokio::time::sleep(Duration::from_millis(80)).await;
        let _ = cb.is_available("p"); // triggers HalfOpen
        assert_eq!(cb.state("p"), CircuitState::HalfOpen);

        cb.record_success("p");
        assert_eq!(cb.state("p"), CircuitState::Closed);
    }

    #[tokio::test]
    async fn half_open_failure_reopens() {
        let cb = breaker(2, 50);
        cb.record_failure("p");
        cb.record_failure("p");
        tokio::time::sleep(Duration::from_millis(80)).await;
        let _ = cb.is_available("p"); // triggers HalfOpen

        cb.record_failure("p");
        assert_eq!(cb.state("p"), CircuitState::Open);
    }

    #[test]
    fn per_provider_isolation() {
        let cb = breaker(2, 1000);
        cb.record_failure("openai");
        cb.record_failure("openai");
        assert_eq!(cb.state("openai"), CircuitState::Open);

        assert_eq!(cb.state("anthropic"), CircuitState::Closed);
        assert!(cb.is_available("anthropic"));
    }
}

#[cfg(test)]
mod context_window_resolution_tests {
    use super::*;
    use fastclaw_core::agent_config::{AgentConfig, AgentModelConfig};
    use fastclaw_core::llm_plugin::LlmPluginConfig;

    fn make_plugin_registry() -> crate::llm_plugin::LlmPluginRegistry {
        let plugin_json = serde_json::json!({
            "id": "test-llm",
            "name": "Test LLM Plugin",
            "version": "1.0",
            "type": "middleware",
            "middleware": {
                "baseUrl": "https://api.test-llm.example.com"
            },
            "models": [
                { "id": "qwen3.5-plus", "name": "Qwen 3.5 Plus", "contextWindow": 32000 }
            ]
        });
        let cfg: LlmPluginConfig = serde_json::from_value(plugin_json).unwrap();
        let mut reg = crate::llm_plugin::LlmPluginRegistry::new();
        reg.register(cfg);
        reg
    }

    fn make_agent(provider: &str, model: &str, cw: Option<u32>) -> AgentConfig {
        use fastclaw_core::agent_config::BehaviorConfig;
        AgentConfig {
            agent_id: "test".into(),
            name: None,
            description: None,
            model: AgentModelConfig {
                provider: provider.to_string(),
                model: model.to_string(),
                context_window: cw,
                ..Default::default()
            },
            system_prompt: None,
            tools: Vec::new(),
            behavior: BehaviorConfig::default(),
            mcp_servers: Vec::new(),
            min_tier: None,
            max_tier: None,
            avatar: None,
            channels: Default::default(),
        }
    }

    #[test]
    fn explicit_config_wins() {
        let reg = make_plugin_registry();
        let cfg = AgentModelConfig {
            provider: "plugin:test-llm".to_string(),
            model: "qwen3.5-plus".to_string(),
            context_window: Some(50000),
            ..Default::default()
        };
        assert_eq!(resolve_context_window(&cfg, Some(&reg)), 50000);
    }

    #[test]
    fn plugin_model_entry_used_when_config_is_none() {
        let reg = make_plugin_registry();
        let cfg = AgentModelConfig {
            provider: "plugin:test-llm".to_string(),
            model: "qwen3.5-plus".to_string(),
            context_window: None,
            ..Default::default()
        };
        assert_eq!(resolve_context_window(&cfg, Some(&reg)), 32000);
    }

    #[test]
    fn falls_back_to_model_heuristic_without_plugin() {
        let cfg = AgentModelConfig {
            provider: "dashscope".to_string(),
            model: "qwen3.5-plus".to_string(),
            context_window: None,
            ..Default::default()
        };
        let inferred = fastclaw_context::infer_context_window_from_model("qwen3.5-plus");
        assert_eq!(resolve_context_window(&cfg, None), inferred);
    }

    #[test]
    fn patch_fills_plugin_context_window() {
        let reg = make_plugin_registry();
        let mut agents = vec![make_agent("plugin:test-llm", "qwen3.5-plus", None)];
        patch_agent_context_windows(&mut agents, Some(&reg));
        assert_eq!(agents[0].model.context_window, Some(32000));
    }

    #[test]
    fn patch_skips_explicit_config() {
        let reg = make_plugin_registry();
        let mut agents = vec![make_agent("plugin:test-llm", "qwen3.5-plus", Some(64000))];
        patch_agent_context_windows(&mut agents, Some(&reg));
        assert_eq!(agents[0].model.context_window, Some(64000));
    }

    #[test]
    fn patch_skips_non_plugin_providers() {
        let reg = make_plugin_registry();
        let mut agents = vec![make_agent("dashscope", "qwen3.5-plus", None)];
        patch_agent_context_windows(&mut agents, Some(&reg));
        assert_eq!(agents[0].model.context_window, None);
    }
}
