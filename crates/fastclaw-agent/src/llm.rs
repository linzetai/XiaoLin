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
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            connect_timeout_ms: 10000,
            request_timeout_ms: 120000,
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
        Self {
            client: build_openai_http_client(&timeouts),
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
        self.client
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
                    let err = anyhow::anyhow!("LLM API error: {status} — {text}");
                    let retryable = is_retryable_http_status(status);
                    if retryable && attempt + 1 < max_attempts {
                        let delay_ms = openai_backoff_delay_ms(&self.retry, attempt as u32);
                        tracing::warn!(
                            retry_after_attempt = attempt + 1,
                            max_retries = self.retry.max_retries,
                            delay_ms,
                            status = %status,
                            error = %err,
                            "OpenAI chat stream connection failed, retrying"
                        );
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                        continue;
                    }
                    return Err(err);
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
                        let retryable = is_retryable_http_status(status);
                        if retryable && attempt + 1 < max_attempts {
                            let delay_ms = openai_backoff_delay_ms(&self.retry, attempt as u32);
                            let err = format!("LLM API error: {status} — {text}");
                            tracing::warn!(
                                retry_after_attempt = attempt + 1,
                                max_retries = self.retry.max_retries,
                                delay_ms,
                                status = %status,
                                error = %err,
                                "OpenAI chat completion failed, retrying"
                            );
                            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                            continue;
                        }
                        anyhow::bail!("LLM API error: {status} — {text}");
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
                        Err(e) => return vec![Err(anyhow::anyhow!("stream read error: {e}"))],
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
struct AnthropicMessage {
    role: String,
    content: serde_json::Value,
}

#[derive(Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
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
    fn convert_messages(messages: &[ChatMessage]) -> (Option<String>, Vec<AnthropicMessage>) {
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

    fn convert_tools(defs: &[ToolDefinition]) -> Vec<AnthropicTool> {
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
                        Err(e) => return vec![Err(anyhow::anyhow!("stream read error: {e}"))],
                    };
                    let text = String::from_utf8_lossy(&chunk);
                    line_buf.push_str(&text);

                    let mut deltas = Vec::new();

                    while let Some(pos) = line_buf.find('\n') {
                        let line: String = line_buf.drain(..=pos).collect();
                        let line = line.trim().to_string();

                        if line.starts_with("event: ") {
                            current_event = line[7..].to_string();
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
}

// ---------- Fallback Provider ----------

/// Tries providers in order, falling back to the next on failure.
pub struct FallbackProvider {
    providers: Vec<(String, Box<dyn LlmProvider>)>,
}

impl FallbackProvider {
    pub fn new(providers: Vec<(String, Box<dyn LlmProvider>)>) -> Self {
        Self { providers }
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
            match provider.chat_completion(params).await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    tracing::warn!(provider = %name, error = %e, "provider failed, trying fallback");
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
            match provider.chat_completion_stream(params).await {
                Ok(stream) => return Ok(stream),
                Err(e) => {
                    tracing::warn!(provider = %name, error = %e, "stream provider failed, trying fallback");
                    last_err = e;
                }
            }
        }
        Err(last_err)
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
