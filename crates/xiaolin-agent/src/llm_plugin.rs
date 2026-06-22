//! LLM provider plugin system.
//!
//! Two plugin modes:
//! - **Middleware**: wraps an existing OpenAI/Anthropic-compatible endpoint with
//!   custom headers, auth (OAuth2, custom header, pre-request hook), model mapping.
//! - **Process**: spawns an external executable that speaks a JSON-over-stdio protocol.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use xiaolin_core::llm_plugin::{
    AuthConfig, LlmPluginConfig, LlmPluginType, LlmProtocol, MiddlewareConfig, ProcessPluginConfig,
};
use xiaolin_core::types::{
    ChatChoice, ChatMessage, ChatResponse, DeltaContent, StreamChoice, StreamDelta,
    StreamFunctionDelta, StreamToolCallDelta, Usage,
};
use futures::stream::BoxStream;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::llm::{classify_llm_error, CompletionParams, LlmProvider, RetryConfig};

// =========================================================================
// Auth middleware
// =========================================================================

struct CachedToken {
    value: String,
    expires_at: Instant,
}

/// Resolved auth headers ready to be injected into a request.
type AuthHeaders = Vec<(HeaderName, HeaderValue)>;

/// Handles token acquisition and caching for different auth strategies.
enum AuthMiddleware {
    None,
    BearerToken {
        header_name: HeaderName,
        header_value: HeaderValue,
    },
    CustomHeader {
        header_name: HeaderName,
        header_value: HeaderValue,
    },
    OAuth2 {
        client: reqwest::Client,
        token_endpoint: String,
        client_id: String,
        client_secret: String,
        scope: Option<String>,
        token_header: HeaderName,
        token_prefix: String,
        cached: RwLock<Option<CachedToken>>,
    },
    PreRequestHook {
        client: reqwest::Client,
        url: String,
        method: reqwest::Method,
        body: Option<serde_json::Value>,
        request_headers: HeaderMap,
        extract_path: Vec<String>,
        token_header: HeaderName,
        token_prefix: String,
        cache_ttl: Duration,
        cached: RwLock<Option<CachedToken>>,
    },
}

impl AuthMiddleware {
    fn from_config(config: &AuthConfig) -> anyhow::Result<Self> {
        match config {
            AuthConfig::None => Ok(Self::None),
            AuthConfig::BearerToken { token } => Ok(Self::BearerToken {
                header_name: HeaderName::from_static("authorization"),
                header_value: HeaderValue::from_str(&format!("Bearer {token}"))
                    .map_err(|e| anyhow::anyhow!("invalid bearer token header value: {e}"))?,
            }),
            AuthConfig::CustomHeader { header, value } => Ok(Self::CustomHeader {
                header_name: HeaderName::from_bytes(header.as_bytes())
                    .map_err(|e| anyhow::anyhow!("invalid custom header name '{header}': {e}"))?,
                header_value: HeaderValue::from_str(value)
                    .map_err(|e| anyhow::anyhow!("invalid custom header value: {e}"))?,
            }),
            AuthConfig::OAuth2ClientCredentials {
                token_endpoint,
                client_id,
                client_secret,
                scope,
                token_header,
                token_prefix,
            } => Ok(Self::OAuth2 {
                client: reqwest::Client::builder()
                    .timeout(Duration::from_secs(30))
                    .build()?,
                token_endpoint: token_endpoint.clone(),
                client_id: client_id.clone(),
                client_secret: client_secret.clone(),
                scope: scope.clone(),
                token_header: HeaderName::from_bytes(token_header.as_bytes())
                    .map_err(|e| anyhow::anyhow!("invalid token_header '{token_header}': {e}"))?,
                token_prefix: token_prefix.clone(),
                cached: RwLock::new(None),
            }),
            AuthConfig::PreRequestHook {
                url,
                method,
                body,
                headers,
                extract_path,
                token_header,
                token_prefix,
                cache_ttl_secs,
            } => {
                let mut hm = HeaderMap::new();
                for (k, v) in headers {
                    let name = HeaderName::from_bytes(k.as_bytes())
                        .map_err(|e| anyhow::anyhow!("invalid header name '{k}': {e}"))?;
                    let val = HeaderValue::from_str(v)
                        .map_err(|e| anyhow::anyhow!("invalid header value for '{k}': {e}"))?;
                    hm.insert(name, val);
                }
                let method = match method.to_uppercase().as_str() {
                    "GET" => reqwest::Method::GET,
                    "PUT" => reqwest::Method::PUT,
                    _ => reqwest::Method::POST,
                };
                Ok(Self::PreRequestHook {
                    client: reqwest::Client::builder()
                        .timeout(Duration::from_secs(30))
                        .build()?,
                    url: url.clone(),
                    method,
                    body: body.clone(),
                    request_headers: hm,
                    extract_path: extract_path.split('.').map(String::from).collect(),
                    token_header: HeaderName::from_bytes(token_header.as_bytes())
                        .map_err(|e| anyhow::anyhow!("invalid token_header: {e}"))?,
                    token_prefix: token_prefix.clone(),
                    cache_ttl: Duration::from_secs(*cache_ttl_secs),
                    cached: RwLock::new(None),
                })
            }
        }
    }

    async fn resolve_headers(&self) -> anyhow::Result<AuthHeaders> {
        match self {
            Self::None => Ok(vec![]),
            Self::BearerToken {
                header_name,
                header_value,
            } => Ok(vec![(header_name.clone(), header_value.clone())]),
            Self::CustomHeader {
                header_name,
                header_value,
            } => Ok(vec![(header_name.clone(), header_value.clone())]),
            Self::OAuth2 {
                client,
                token_endpoint,
                client_id,
                client_secret,
                scope,
                token_header,
                token_prefix,
                cached,
            } => {
                // Fast path: check cache
                {
                    let guard = cached.read().await;
                    if let Some(ref tok) = *guard {
                        if Instant::now() < tok.expires_at {
                            let val = format!("{} {}", token_prefix, tok.value);
                            return Ok(vec![(token_header.clone(), HeaderValue::from_str(&val)?)]);
                        }
                    }
                }
                // Slow path: acquire token
                let mut params = vec![
                    ("grant_type", "client_credentials"),
                    ("client_id", client_id.as_str()),
                    ("client_secret", client_secret.as_str()),
                ];
                if let Some(ref s) = scope {
                    params.push(("scope", s.as_str()));
                }
                let resp = client
                    .post(token_endpoint)
                    .form(&params)
                    .send()
                    .await
                    .map_err(|e| anyhow::anyhow!("OAuth2 token request failed: {e}"))?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    anyhow::bail!("OAuth2 token endpoint returned {status}: {body}");
                }
                let body: serde_json::Value = resp.json().await?;
                let access_token = body
                    .get("access_token")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("OAuth2 response missing access_token"))?
                    .to_string();
                let expires_in = body
                    .get("expires_in")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(3600);
                // Cache with safety margin
                let margin = (expires_in / 10).clamp(30, 300);
                let ttl = Duration::from_secs(expires_in.saturating_sub(margin));
                {
                    let mut guard = cached.write().await;
                    *guard = Some(CachedToken {
                        value: access_token.clone(),
                        expires_at: Instant::now() + ttl,
                    });
                }
                let val = format!("{} {}", token_prefix, access_token);
                Ok(vec![(token_header.clone(), HeaderValue::from_str(&val)?)])
            }
            Self::PreRequestHook {
                client,
                url,
                method,
                body,
                request_headers,
                extract_path,
                token_header,
                token_prefix,
                cache_ttl,
                cached,
            } => {
                // Fast path
                if cache_ttl.as_secs() > 0 {
                    let guard = cached.read().await;
                    if let Some(ref tok) = *guard {
                        if Instant::now() < tok.expires_at {
                            let val = format!("{} {}", token_prefix, tok.value);
                            return Ok(vec![(token_header.clone(), HeaderValue::from_str(&val)?)]);
                        }
                    }
                }
                let mut req = client
                    .request(method.clone(), url)
                    .headers(request_headers.clone());
                if let Some(ref b) = body {
                    req = req.json(b);
                }
                let resp = req
                    .send()
                    .await
                    .map_err(|e| anyhow::anyhow!("pre-request hook call failed: {e}"))?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    anyhow::bail!("pre-request hook returned {status}: {text}");
                }
                let json: serde_json::Value = resp.json().await?;
                let mut current = &json;
                for key in extract_path {
                    current = current.get(key).ok_or_else(|| {
                        anyhow::anyhow!("pre-request hook: key '{}' not found in response", key)
                    })?;
                }
                let token_str = current
                    .as_str()
                    .ok_or_else(|| {
                        anyhow::anyhow!("pre-request hook: extracted value is not a string")
                    })?
                    .to_string();

                if cache_ttl.as_secs() > 0 {
                    let mut guard = cached.write().await;
                    *guard = Some(CachedToken {
                        value: token_str.clone(),
                        expires_at: Instant::now() + *cache_ttl,
                    });
                }
                let val = if token_prefix.is_empty() {
                    token_str
                } else {
                    format!("{} {}", token_prefix, token_str)
                };
                Ok(vec![(token_header.clone(), HeaderValue::from_str(&val)?)])
            }
        }
    }
}

// =========================================================================
// Middleware LLM Provider
// =========================================================================

/// An `LlmProvider` backed by a middleware plugin config.
/// Delegates to OpenAI or Anthropic wire protocol with custom headers/auth.
pub struct MiddlewareLlmProvider {
    plugin_id: String,
    protocol: LlmProtocol,
    client: reqwest::Client,
    stream_client: reqwest::Client,
    base_url: String,
    static_headers: HeaderMap,
    auth: AuthMiddleware,
    model_mapping: HashMap<String, String>,
    #[allow(dead_code)]
    retry: RetryConfig,
}

impl MiddlewareLlmProvider {
    pub fn from_config(plugin_id: &str, mw: &MiddlewareConfig) -> anyhow::Result<Self> {
        let auth = AuthMiddleware::from_config(&mw.auth)?;

        let mut static_headers = HeaderMap::new();
        for (k, v) in &mw.headers {
            let name = HeaderName::from_bytes(k.as_bytes())
                .map_err(|e| anyhow::anyhow!("invalid header name '{k}': {e}"))?;
            let val = HeaderValue::from_str(v)
                .map_err(|e| anyhow::anyhow!("invalid header value for '{k}': {e}"))?;
            static_headers.insert(name, val);
        }

        let timeout = Duration::from_secs(mw.timeout_secs.unwrap_or(300));
        let stream_timeout = Duration::from_secs(mw.timeout_secs.unwrap_or(600));
        let client = reqwest::Client::builder()
            .user_agent("XiaoLin/0.1.0")
            .connect_timeout(Duration::from_secs(10))
            .timeout(timeout)
            .build()?;
        let stream_client = reqwest::Client::builder()
            .user_agent("XiaoLin/0.1.0")
            .connect_timeout(Duration::from_secs(10))
            .timeout(stream_timeout)
            .build()?;

        let retry = RetryConfig {
            max_retries: mw.max_retries.unwrap_or(3),
            ..RetryConfig::default()
        };

        Ok(Self {
            plugin_id: plugin_id.to_string(),
            protocol: mw.protocol,
            client,
            stream_client,
            base_url: mw.base_url.trim_end_matches('/').to_string(),
            static_headers,
            auth,
            model_mapping: mw.model_mapping.clone(),
            retry,
        })
    }

    fn map_model<'a>(&'a self, model: &'a str) -> &'a str {
        self.model_mapping
            .get(model)
            .map(|s| s.as_str())
            .unwrap_or(model)
    }

    async fn build_headers(&self) -> anyhow::Result<HeaderMap> {
        let mut headers = self.static_headers.clone();
        for (name, value) in self.auth.resolve_headers().await? {
            headers.insert(name, value);
        }
        Ok(headers)
    }
}

// Reuse OpenAI request/response structs from llm.rs.
// We re-define lightweight local versions to avoid coupling to private types.

#[derive(Serialize)]
struct PluginOpenAiRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<PluginStreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [xiaolin_core::tool::ToolDefinition]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'a str>,
}

#[derive(Serialize)]
struct PluginStreamOptions {
    include_usage: bool,
}

#[derive(Deserialize)]
struct PluginOpenAiResponse {
    id: String,
    object: String,
    created: u64,
    model: String,
    choices: Vec<PluginOpenAiChoice>,
    usage: Option<PluginOpenAiUsage>,
}

#[derive(Deserialize)]
struct PluginOpenAiChoice {
    index: u32,
    message: ChatMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct PluginOpenAiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[async_trait]
impl LlmProvider for MiddlewareLlmProvider {
    async fn chat_completion(&self, params: &CompletionParams<'_>) -> anyhow::Result<ChatResponse> {
        match self.protocol {
            LlmProtocol::Openai => self.openai_chat_completion(params).await,
            LlmProtocol::Anthropic => {
                // Delegate to a thin Anthropic adapter.
                // For middleware mode we reuse the existing AnthropicProvider logic
                // by creating an ad-hoc provider with the plugin's headers/auth.
                self.anthropic_chat_completion(params).await
            }
        }
    }

    async fn chat_completion_stream(
        &self,
        params: &CompletionParams<'_>,
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<StreamDelta>>> {
        match self.protocol {
            LlmProtocol::Openai => self.openai_chat_completion_stream(params).await,
            LlmProtocol::Anthropic => self.anthropic_chat_completion_stream(params).await,
        }
    }

    fn provider_name(&self) -> &str {
        &self.plugin_id
    }
}

impl MiddlewareLlmProvider {
    // ---- OpenAI protocol ----

    async fn openai_chat_completion(
        &self,
        params: &CompletionParams<'_>,
    ) -> anyhow::Result<ChatResponse> {
        let mapped_model = self.map_model(params.model);
        let url = format!("{}/chat/completions", self.base_url);
        let tool_choice = params.tools.filter(|t| !t.is_empty()).map(|_| "auto");
        let body = PluginOpenAiRequest {
            model: mapped_model,
            messages: params.messages,
            temperature: params.temperature,
            max_tokens: params.max_tokens,
            stream: false,
            stream_options: None,
            tools: params.tools,
            tool_choice,
        };

        let headers = self.build_headers().await?;
        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            let classified = classify_llm_error(status, &text);
            return Err(classified.into());
        }

        let api_resp: PluginOpenAiResponse = resp.json().await?;
        Ok(ChatResponse {
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
                ..Default::default()
            }),
        })
    }

    async fn openai_chat_completion_stream(
        &self,
        params: &CompletionParams<'_>,
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<StreamDelta>>> {
        use futures::stream::{self, StreamExt};

        let mapped_model = self.map_model(params.model);
        let url = format!("{}/chat/completions", self.base_url);
        let tool_choice = params.tools.filter(|t| !t.is_empty()).map(|_| "auto");
        let body = PluginOpenAiRequest {
            model: mapped_model,
            messages: params.messages,
            temperature: params.temperature,
            max_tokens: params.max_tokens,
            stream: true,
            stream_options: Some(PluginStreamOptions {
                include_usage: true,
            }),
            tools: params.tools,
            tool_choice,
        };

        let headers = self.build_headers().await?;
        let resp = self
            .stream_client
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            let classified = classify_llm_error(status, &text);
            return Err(classified.into());
        }

        let byte_stream = resp.bytes_stream();
        let delta_stream = {
            let mut line_buf = String::new();
            byte_stream
                .map(move |chunk_result| {
                    let chunk = match chunk_result {
                        Ok(c) => c,
                        Err(e) => {
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
                            Ok(mut delta) => {
                                delta.raw_sse_json =
                                    Some(bytes::Bytes::copy_from_slice(data.as_bytes()));
                                deltas.push(Ok(delta));
                            }
                            Err(e) => {
                                tracing::debug!(error = %e, "plugin stream: failed to parse SSE chunk");
                            }
                        }
                    }
                    deltas
                })
                .flat_map(stream::iter)
        };
        Ok(Box::pin(delta_stream))
    }

    // ---- Anthropic protocol ----
    // Thin adapter: convert CompletionParams to Anthropic format, apply
    // plugin headers, then convert back. Reuses the conversion logic
    // from crate::llm::AnthropicProvider where possible.

    async fn anthropic_chat_completion(
        &self,
        params: &CompletionParams<'_>,
    ) -> anyhow::Result<ChatResponse> {
        let mapped_model = self.map_model(params.model);
        let url = format!("{}/v1/messages", self.base_url);

        let (system, messages) = crate::llm::AnthropicProvider::convert_messages(params.messages);
        let tools = params
            .tools
            .filter(|t| !t.is_empty())
            .map(crate::llm::AnthropicProvider::convert_tools);

        let body = serde_json::json!({
            "model": mapped_model,
            "messages": messages,
            "max_tokens": params.max_tokens.unwrap_or(4096),
            "system": system,
            "tools": tools,
            "stream": false,
        });

        let mut headers = self.build_headers().await?;
        headers.insert(
            HeaderName::from_static("anthropic-version"),
            HeaderValue::from_static("2023-06-01"),
        );

        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API error via plugin: {status} — {text}");
        }

        let api_resp: serde_json::Value = resp.json().await?;
        // Reuse the conversion by re-deserializing through the known response struct.
        // This is intentionally simple — a future refactoring can share more code.
        let chat_resp = anthropic_value_to_chat_response(api_resp)?;
        Ok(chat_resp)
    }

    async fn anthropic_chat_completion_stream(
        &self,
        params: &CompletionParams<'_>,
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<StreamDelta>>> {
        use futures::stream::{self, StreamExt};

        let mapped_model = self.map_model(params.model);
        let url = format!("{}/v1/messages", self.base_url);

        let (system, messages) = crate::llm::AnthropicProvider::convert_messages(params.messages);
        let tools = params
            .tools
            .filter(|t| !t.is_empty())
            .map(crate::llm::AnthropicProvider::convert_tools);

        let body = serde_json::json!({
            "model": mapped_model,
            "messages": messages,
            "max_tokens": params.max_tokens.unwrap_or(4096),
            "system": system,
            "tools": tools,
            "stream": true,
        });

        let mut headers = self.build_headers().await?;
        headers.insert(
            HeaderName::from_static("anthropic-version"),
            HeaderValue::from_static("2023-06-01"),
        );

        let resp = self
            .stream_client
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic stream API error via plugin: {status} — {text}");
        }

        // Reuse the Anthropic SSE parsing logic. The event structure is identical
        // regardless of custom headers, so we replicate the core stream parser.
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
                    let chunk = match chunk_result {
                        Ok(c) => c,
                        Err(e) => {
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
                                            input_tokens = u
                                                .get("input_tokens")
                                                .and_then(|x| x.as_u64())
                                                .unwrap_or(0)
                                                as u32;
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
                                        output_tokens = u
                                            .get("output_tokens")
                                            .and_then(|x| x.as_u64())
                                            .unwrap_or(0)
                                            as u32;
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
                                        let dt = delta_obj
                                            .get("type")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        match dt {
                                            "text_delta" => {
                                                let t = delta_obj
                                                    .get("text")
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("");
                                                if !t.is_empty() {
                                                    let now = now_secs();
                                                    deltas.push(Ok(StreamDelta {
                                                        id: msg_id.clone(),
                                                        object: "chat.completion.chunk".to_string(),
                                                        created: now,
                                                        model: model_name.clone(),
                                                        choices: vec![StreamChoice {
                                                            index: 0,
                                                            delta: DeltaContent {
                                                                role: None,
                                                                content: Some(t.to_string()),
                                                                reasoning_content: None,
                                                                tool_calls: None,
                                                            },
                                                            finish_reason: None,
                                                        }],
                                                        usage: None,
                                                        raw_sse_json: None,
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
                                            let now = now_secs();
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
                                                raw_sse_json: None,
                                            }));
                                        }
                                    }
                                }
                            }
                            "message_stop" => {
                                let now = now_secs();
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
                                        ..Default::default()
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
                                    raw_sse_json: None,
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

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Convert a raw Anthropic Messages API JSON response into a `ChatResponse`.
fn anthropic_value_to_chat_response(v: serde_json::Value) -> anyhow::Result<ChatResponse> {
    let id = v
        .get("id")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let model = v
        .get("model")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let stop_reason = v
        .get("stop_reason")
        .and_then(|x| x.as_str())
        .unwrap_or("stop");

    let content_blocks = v
        .get("content")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    let mut content_parts = Vec::new();
    let mut tool_calls = Vec::new();
    for block in &content_blocks {
        let btype = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match btype {
            "text" => {
                if let Some(t) = block.get("text").and_then(|x| x.as_str()) {
                    content_parts.push(t.to_string());
                }
            }
            "tool_use" => {
                let tc_id = block
                    .get("id")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = block
                    .get("name")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let input = block
                    .get("input")
                    .cloned()
                    .unwrap_or(serde_json::Value::Object(Default::default()));
                tool_calls.push(xiaolin_core::types::ToolCall {
                    id: tc_id,
                    call_type: "function".to_string(),
                    function: xiaolin_core::types::FunctionCall {
                        name,
                        arguments: serde_json::to_string(&input).unwrap_or_default(),
                    },
                    output: None,
                    success: None,
                    duration_ms: None,
                });
            }
            _ => {}
        }
    }

    let finish_reason = match stop_reason {
        "tool_use" => "tool_calls".to_string(),
        r => r.to_string(),
    };

    let usage_obj = v.get("usage");
    let input_tokens = usage_obj
        .and_then(|u| u.get("input_tokens"))
        .and_then(|x| x.as_u64())
        .unwrap_or(0) as u32;
    let output_tokens = usage_obj
        .and_then(|u| u.get("output_tokens"))
        .and_then(|x| x.as_u64())
        .unwrap_or(0) as u32;

    Ok(ChatResponse {
        id,
        object: "chat.completion".to_string(),
        created: now_secs(),
        model,
        choices: vec![ChatChoice {
            index: 0,
            message: ChatMessage {
                role: xiaolin_core::types::Role::Assistant,
                content: if content_parts.is_empty() {
                    None
                } else {
                    Some(serde_json::Value::String(content_parts.join("")))
                },
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(tool_calls)
                },
                ..Default::default()
            },
            finish_reason: Some(finish_reason),
        }],
        usage: Some(Usage {
            prompt_tokens: input_tokens,
            completion_tokens: output_tokens,
            total_tokens: input_tokens + output_tokens,
            ..Default::default()
        }),
    })
}

// =========================================================================
// Process LLM Provider
// =========================================================================

/// JSON-over-stdio protocol request.
#[derive(Serialize)]
struct ProcessRequest<'a> {
    method: &'a str,
    params: ProcessRequestParams<'a>,
}

#[derive(Serialize)]
struct ProcessRequestParams<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [xiaolin_core::tool::ToolDefinition]>,
}

/// JSON-over-stdio protocol response for non-streaming.
#[derive(Deserialize)]
struct ProcessResponse {
    result: Option<ChatResponse>,
    error: Option<ProcessError>,
}

#[derive(Deserialize)]
struct ProcessError {
    message: String,
    #[serde(default)]
    #[allow(dead_code)]
    code: Option<String>,
}

/// An `LlmProvider` backed by an external process.
pub struct ProcessLlmProvider {
    plugin_id: String,
    config: ProcessPluginConfig,
    process: std::sync::Arc<tokio::sync::Mutex<Option<ProcessHandle>>>,
}

struct ProcessHandle {
    stdin: tokio::process::ChildStdin,
    stdout: tokio::io::BufReader<tokio::process::ChildStdout>,
    #[allow(dead_code)]
    child: tokio::process::Child,
}

impl ProcessLlmProvider {
    pub fn new(plugin_id: &str, config: &ProcessPluginConfig) -> Self {
        Self {
            plugin_id: plugin_id.to_string(),
            config: config.clone(),
            process: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    async fn ensure_process(&self) -> anyhow::Result<()> {
        let mut guard = self.process.lock().await;
        if let Some(handle) = guard.as_mut() {
            match handle.child.try_wait() {
                Ok(Some(status)) => {
                    tracing::warn!(
                        plugin_id = %self.plugin_id,
                        ?status,
                        "LLM plugin process exited, respawning"
                    );
                    *guard = None;
                }
                Ok(None) => return Ok(()),
                Err(e) => {
                    tracing::warn!(
                        plugin_id = %self.plugin_id,
                        error = %e,
                        "failed to check plugin process status, respawning"
                    );
                    *guard = None;
                }
            }
        }

        tracing::info!(
            plugin_id = %self.plugin_id,
            command = %self.config.command,
            "spawning LLM plugin process"
        );

        let mut cmd = tokio::process::Command::new(&self.config.command);
        cmd.args(&self.config.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit());
        for (k, v) in &self.config.env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!(
                "failed to spawn LLM plugin process '{}': {e}",
                self.config.command
            )
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("plugin process stdin not captured"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("plugin process stdout not captured"))?;

        *guard = Some(ProcessHandle {
            stdin,
            stdout: tokio::io::BufReader::new(stdout),
            child,
        });
        Ok(())
    }

    async fn send_request(&self, req: &ProcessRequest<'_>) -> anyhow::Result<String> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

        self.ensure_process().await?;
        let mut guard = self.process.lock().await;
        let handle = guard
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("plugin process not running"))?;

        let mut line = serde_json::to_string(req)?;
        line.push('\n');
        if let Err(e) = handle.stdin.write_all(line.as_bytes()).await {
            tracing::warn!(
                plugin_id = %self.plugin_id,
                error = %e,
                "plugin stdin write failed (process may have crashed), clearing handle for respawn"
            );
            *guard = None;
            anyhow::bail!("failed to write to plugin process stdin: {e}");
        }
        if let Err(e) = handle.stdin.flush().await {
            *guard = None;
            anyhow::bail!("failed to flush plugin process stdin: {e}");
        }

        let mut response_line = String::new();
        let bytes_read = handle
            .stdout
            .read_line(&mut response_line)
            .await
            .map_err(|e| {
                tracing::warn!(
                    plugin_id = %self.plugin_id,
                    error = %e,
                    "plugin stdout read failed, clearing handle for respawn"
                );
                anyhow::anyhow!("failed to read from plugin process stdout: {e}")
            })?;

        if bytes_read == 0 {
            *guard = None;
            anyhow::bail!("plugin process closed stdout (EOF) without responding");
        }
        tracing::debug!(
            plugin_id = %self.plugin_id,
            bytes_read,
            "plugin: read response line"
        );
        Ok(response_line)
    }
}

#[async_trait]
impl LlmProvider for ProcessLlmProvider {
    async fn chat_completion(&self, params: &CompletionParams<'_>) -> anyhow::Result<ChatResponse> {
        let req = ProcessRequest {
            method: "chat_completion",
            params: ProcessRequestParams {
                model: params.model,
                messages: params.messages,
                temperature: params.temperature,
                max_tokens: params.max_tokens,
                tools: params.tools,
            },
        };

        let response_line = self.send_request(&req).await?;
        let proc_resp: ProcessResponse = serde_json::from_str(&response_line)
            .map_err(|e| anyhow::anyhow!("failed to parse plugin process response: {e}"))?;

        if let Some(err) = proc_resp.error {
            anyhow::bail!("plugin process error: {}", err.message);
        }

        proc_resp
            .result
            .ok_or_else(|| anyhow::anyhow!("plugin process returned neither result nor error"))
    }

    async fn chat_completion_stream(
        &self,
        params: &CompletionParams<'_>,
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<StreamDelta>>> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

        self.ensure_process().await?;

        // Try the streaming method first; if the process returns an
        // "unsupported_method" error, fall back to non-streaming.
        let stream_req = ProcessRequest {
            method: "chat_completion_stream",
            params: ProcessRequestParams {
                model: params.model,
                messages: params.messages,
                temperature: params.temperature,
                max_tokens: params.max_tokens,
                tools: params.tools,
            },
        };
        let mut req_line = serde_json::to_string(&stream_req)?;
        req_line.push('\n');

        // Probe: send the stream request and read the first response line.
        // If it's an error (especially unsupported_method), fall back.
        let first_line = {
            let mut guard = self.process.lock().await;
            let handle = guard
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("plugin process not running"))?;
            if let Err(e) = handle.stdin.write_all(req_line.as_bytes()).await {
                tracing::warn!(
                    plugin_id = %self.plugin_id,
                    error = %e,
                    "plugin stdin write failed during stream, clearing handle for respawn"
                );
                *guard = None;
                anyhow::bail!("failed to write to plugin stdin: {e}");
            }
            if let Err(e) = handle.stdin.flush().await {
                *guard = None;
                anyhow::bail!("failed to flush plugin stdin: {e}");
            }

            let mut first = String::new();
            let bytes_read = handle
                .stdout
                .read_line(&mut first)
                .await
                .map_err(|e| anyhow::anyhow!("failed to read from plugin stdout: {e}"))?;

            tracing::info!(
                plugin_id = %self.plugin_id,
                bytes_read,
                first_line_len = first.trim().len(),
                "plugin stream: read first response line"
            );

            if bytes_read == 0 {
                anyhow::bail!("plugin process closed stdout (EOF) without a response");
            }

            first
        };

        let trimmed = first_line.trim();

        // Check if the first line is an error indicating unsupported streaming.
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(err) = v.get("error") {
                let code = err.get("code").and_then(|c| c.as_str()).unwrap_or("");
                let msg = err
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown error");
                tracing::warn!(
                    plugin_id = %self.plugin_id,
                    error = msg,
                    code,
                    raw_line = &trimmed[..trimmed.floor_char_boundary(trimmed.len().min(500))],
                    "plugin process stream: first line is error"
                );
                if code == "unknown_method" || code == "unsupported_method" {
                    tracing::info!(
                        plugin_id = %self.plugin_id,
                        "falling back to chat_completion"
                    );
                    let chat_resp = self.chat_completion(params).await?;
                    let deltas = chat_response_to_stream_deltas(chat_resp);
                    return Ok(Box::pin(futures::stream::iter(deltas.into_iter().map(Ok))));
                }
                anyhow::bail!("plugin process error: {msg}");
            }
        }

        // The first line is a valid streaming response. Start the stream reader.
        let first_owned = trimmed.to_string();
        let process = self.process.clone();
        let plugin_id = self.plugin_id.clone();
        let (tx, rx) = tokio::sync::mpsc::channel::<anyhow::Result<StreamDelta>>(64);

        tokio::spawn(async move {
            let mut line_count: u64 = 0;

            // Process the already-read first line.
            line_count += 1;
            if let Err(should_stop) = process_stream_line(&first_owned, &plugin_id, &tx).await {
                tracing::info!(
                    plugin_id = %plugin_id,
                    line_count,
                    should_stop,
                    "plugin stream task: first line caused stop"
                );
                if should_stop {
                    return;
                }
            }

            // Continue reading subsequent lines.
            let mut guard = process.lock().await;
            let handle = match guard.as_mut() {
                Some(h) => h,
                None => {
                    let _ = tx
                        .send(Err(anyhow::anyhow!("plugin process not running")))
                        .await;
                    return;
                }
            };

            loop {
                let mut line = String::new();
                match handle.stdout.read_line(&mut line).await {
                    Ok(0) => {
                        tracing::info!(
                            plugin_id = %plugin_id,
                            line_count,
                            "plugin stream task: EOF from process"
                        );
                        break;
                    }
                    Ok(n) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        line_count += 1;
                        if let Err(should_stop) =
                            process_stream_line(trimmed, &plugin_id, &tx).await
                        {
                            tracing::info!(
                                plugin_id = %plugin_id,
                                line_count,
                                bytes = n,
                                should_stop,
                                "plugin stream task: line caused stop"
                            );
                            if should_stop {
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            plugin_id = %plugin_id,
                            line_count,
                            error = %e,
                            "plugin stream task: stdout read error"
                        );
                        let _ = tx
                            .send(Err(anyhow::anyhow!(
                                "plugin process stdout read error: {e}"
                            )))
                            .await;
                        break;
                    }
                }
            }
            tracing::info!(
                plugin_id = %plugin_id,
                line_count,
                "plugin stream task: finished"
            );
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    fn provider_name(&self) -> &str {
        &self.plugin_id
    }
}

/// Process one JSON line from a streaming process plugin.
/// Returns `Ok(())` to continue reading, `Err(true)` to stop normally, `Err(false)` to skip.
async fn process_stream_line(
    trimmed: &str,
    plugin_id: &str,
    tx: &tokio::sync::mpsc::Sender<anyhow::Result<StreamDelta>>,
) -> Result<(), bool> {
    let v: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                plugin_id = %plugin_id,
                error = %e,
                line_preview = &trimmed[..trimmed.floor_char_boundary(trimmed.len().min(120))],
                "plugin stream: line is not valid JSON, skipping"
            );
            return Ok(());
        }
    };

    // Done signal: {"done": true, ...}
    if v.get("done").and_then(|d| d.as_bool()).unwrap_or(false) {
        if let Some(usage_val) = v.get("usage") {
            if let Ok(usage) = serde_json::from_value::<Usage>(usage_val.clone()) {
                let model = v
                    .get("model")
                    .and_then(|m| m.as_str())
                    .unwrap_or("")
                    .to_string();
                let _ = tx
                    .send(Ok(StreamDelta {
                        id: String::new(),
                        object: "chat.completion.chunk".to_string(),
                        created: now_secs(),
                        model,
                        choices: vec![StreamChoice {
                            index: 0,
                            delta: DeltaContent {
                                role: None,
                                content: None,
                                reasoning_content: None,
                                tool_calls: None,
                            },
                            finish_reason: Some("stop".to_string()),
                        }],
                        usage: Some(usage),
                        raw_sse_json: None,
                    }))
                    .await;
            }
        }
        return Err(true);
    }

    // Error signal: {"error": {...}}
    if let Some(err) = v.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        tracing::warn!(
            plugin_id = %plugin_id,
            error = msg,
            "plugin process stream error"
        );
        let _ = tx
            .send(Err(anyhow::anyhow!("plugin process error: {msg}")))
            .await;
        return Err(true);
    }

    // Try strict parse first.
    match serde_json::from_str::<StreamDelta>(trimmed) {
        Ok(mut delta) => {
            delta.raw_sse_json = Some(bytes::Bytes::copy_from_slice(trimmed.as_bytes()));
            if tx.send(Ok(delta)).await.is_err() {
                return Err(true);
            }
            return Ok(());
        }
        Err(strict_err) => {
            tracing::info!(
                plugin_id = %plugin_id,
                error = %strict_err,
                line_preview = &trimmed[..trimmed.floor_char_boundary(trimmed.len().min(200))],
                "plugin stream: strict StreamDelta parse failed, trying lenient"
            );
        }
    }

    // Strict parse failed — build StreamDelta leniently from the Value.
    // Many OpenAI-compatible APIs omit optional top-level fields.
    let delta = stream_delta_from_value(&v);
    if delta.choices.is_empty() && delta.usage.is_none() {
        tracing::warn!(
            plugin_id = %plugin_id,
            line_preview = &trimmed[..trimmed.floor_char_boundary(trimmed.len().min(300))],
            "plugin stream: unparseable line with no choices/usage, skipping"
        );
        return Ok(());
    }

    if tx.send(Ok(delta)).await.is_err() {
        return Err(true);
    }
    Ok(())
}

/// Build a `StreamDelta` from a raw JSON Value, tolerating missing top-level
/// fields that strict deserialization would reject.
fn stream_delta_from_value(v: &serde_json::Value) -> StreamDelta {
    let id = v
        .get("id")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let object = v
        .get("object")
        .and_then(|x| x.as_str())
        .unwrap_or("chat.completion.chunk")
        .to_string();
    let created = v
        .get("created")
        .and_then(|x| x.as_u64())
        .unwrap_or_else(now_secs);
    let model = v
        .get("model")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();

    let choices: Vec<StreamChoice> = match v.get("choices") {
        Some(c) => match serde_json::from_value::<Vec<StreamChoice>>(c.clone()) {
            Ok(parsed) => parsed,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    choices_raw = %c,
                    "stream_delta_from_value: failed to parse choices"
                );
                Vec::new()
            }
        },
        None => Vec::new(),
    };

    let usage: Option<Usage> = v.get("usage").and_then(|u| {
        if u.is_null() {
            None
        } else {
            serde_json::from_value(u.clone()).ok()
        }
    });

    StreamDelta {
        id,
        object,
        created,
        model,
        choices,
        usage,
        raw_sse_json: None,
    }
}

/// Convert a ChatResponse into a sequence of StreamDelta.
/// Used as a fallback when a process plugin does not support streaming natively.
fn chat_response_to_stream_deltas(resp: ChatResponse) -> Vec<StreamDelta> {
    let mut deltas = Vec::new();
    for choice in &resp.choices {
        if let Some(ref content) = choice.message.content {
            let text = match content {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            deltas.push(StreamDelta {
                id: resp.id.clone(),
                object: "chat.completion.chunk".to_string(),
                created: resp.created,
                model: resp.model.clone(),
                choices: vec![StreamChoice {
                    index: choice.index,
                    delta: DeltaContent {
                        role: Some(xiaolin_core::types::Role::Assistant),
                        content: Some(text),
                        reasoning_content: None,
                        tool_calls: None,
                    },
                    finish_reason: None,
                }],
                usage: None,
                raw_sse_json: None,
            });
        }
        if let Some(ref tcs) = choice.message.tool_calls {
            for (i, tc) in tcs.iter().enumerate() {
                deltas.push(StreamDelta {
                    id: resp.id.clone(),
                    object: "chat.completion.chunk".to_string(),
                    created: resp.created,
                    model: resp.model.clone(),
                    choices: vec![StreamChoice {
                        index: choice.index,
                        delta: DeltaContent {
                            role: None,
                            content: None,
                            reasoning_content: None,
                            tool_calls: Some(vec![StreamToolCallDelta {
                                index: i as u32,
                                id: Some(tc.id.clone()),
                                call_type: Some("function".to_string()),
                                function: Some(StreamFunctionDelta {
                                    name: Some(tc.function.name.clone()),
                                    arguments: Some(tc.function.arguments.clone()),
                                }),
                            }]),
                        },
                        finish_reason: None,
                    }],
                    usage: None,
                    raw_sse_json: None,
                });
            }
        }
        // Final chunk with finish_reason
        deltas.push(StreamDelta {
            id: resp.id.clone(),
            object: "chat.completion.chunk".to_string(),
            created: resp.created,
            model: resp.model.clone(),
            choices: vec![StreamChoice {
                index: choice.index,
                delta: DeltaContent {
                    role: None,
                    content: None,
                    reasoning_content: None,
                    tool_calls: None,
                },
                finish_reason: choice.finish_reason.clone(),
            }],
            usage: resp.usage.clone(),
            raw_sse_json: None,
        });
    }
    deltas
}

// =========================================================================
// Plugin Registry
// =========================================================================

/// Registry of loaded LLM provider plugins.
pub struct LlmPluginRegistry {
    plugins: HashMap<String, LlmPluginConfig>,
}

impl Default for LlmPluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmPluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    pub fn from_configs(configs: Vec<LlmPluginConfig>) -> Self {
        let mut plugins = HashMap::new();
        for cfg in configs {
            if cfg.enabled {
                plugins.insert(cfg.id.clone(), cfg);
            }
        }
        Self { plugins }
    }

    pub fn register(&mut self, config: LlmPluginConfig) {
        tracing::info!(
            plugin_id = %config.id,
            plugin_type = ?config.plugin_type,
            "registered LLM provider plugin"
        );
        self.plugins.insert(config.id.clone(), config);
    }

    pub fn unregister(&mut self, plugin_id: &str) -> Option<LlmPluginConfig> {
        self.plugins.remove(plugin_id)
    }

    pub fn get(&self, plugin_id: &str) -> Option<&LlmPluginConfig> {
        self.plugins.get(plugin_id)
    }

    pub fn list(&self) -> Vec<&LlmPluginConfig> {
        self.plugins.values().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Look up the context window for a model exposed by a specific plugin.
    ///
    /// Returns `Some(window)` when the plugin declares the model with a
    /// non-zero `context_window`; `None` otherwise.
    pub fn find_model_context_window(&self, plugin_id: &str, model: &str) -> Option<u32> {
        let config = self.plugins.get(plugin_id)?;
        config
            .models
            .iter()
            .find(|m| m.id == model)
            .map(|m| m.context_window)
            .filter(|&w| w > 0)
    }

    /// Create a provider instance for the given plugin.
    pub fn create_provider(&self, plugin_id: &str) -> anyhow::Result<Box<dyn LlmProvider>> {
        let config = self
            .plugins
            .get(plugin_id)
            .ok_or_else(|| anyhow::anyhow!("LLM plugin '{}' not found", plugin_id))?;

        if !config.enabled {
            anyhow::bail!("LLM plugin '{}' is disabled", plugin_id);
        }

        match config.plugin_type {
            LlmPluginType::Middleware => {
                let mw = config.middleware.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "LLM plugin '{}' is type=middleware but missing middleware config",
                        plugin_id
                    )
                })?;
                Ok(Box::new(MiddlewareLlmProvider::from_config(plugin_id, mw)?))
            }
            LlmPluginType::Process => {
                let proc = config.process.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "LLM plugin '{}' is type=process but missing process config",
                        plugin_id
                    )
                })?;
                Ok(Box::new(ProcessLlmProvider::new(plugin_id, proc)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaolin_core::llm_plugin::*;

    fn middleware_plugin_config() -> LlmPluginConfig {
        LlmPluginConfig {
            id: "test-mw".to_string(),
            name: "Test Middleware".to_string(),
            version: "1.0".to_string(),
            description: "test".to_string(),
            plugin_type: LlmPluginType::Middleware,
            enabled: true,
            middleware: Some(MiddlewareConfig {
                base_url: "https://api.example.com/v1".to_string(),
                protocol: LlmProtocol::Openai,
                headers: {
                    let mut h = HashMap::new();
                    h.insert("x-custom".to_string(), "val".to_string());
                    h
                },
                auth: AuthConfig::CustomHeader {
                    header: "x-api-key".to_string(),
                    value: "secret".to_string(),
                },
                model_mapping: {
                    let mut m = HashMap::new();
                    m.insert("gpt-4o".to_string(), "custom-gpt4".to_string());
                    m
                },
                max_retries: Some(2),
                timeout_secs: Some(60),
            }),
            process: None,
            models: vec![LlmPluginModelEntry {
                id: "custom-gpt4".to_string(),
                name: "Custom GPT-4".to_string(),
                description: "".to_string(),
                context_window: 128000,
                capabilities: None,
            }],
        }
    }

    fn process_plugin_config() -> LlmPluginConfig {
        LlmPluginConfig {
            id: "test-proc".to_string(),
            name: "Test Process".to_string(),
            version: "1.0".to_string(),
            description: "test".to_string(),
            plugin_type: LlmPluginType::Process,
            enabled: true,
            middleware: None,
            process: Some(ProcessPluginConfig {
                command: "echo".to_string(),
                args: vec!["hello".to_string()],
                env: HashMap::new(),
                transport: ProcessTransport::Stdio,
                url: None,
            }),
            models: vec![],
        }
    }

    #[test]
    fn registry_register_and_lookup() {
        let mut reg = LlmPluginRegistry::new();
        assert!(reg.is_empty());

        reg.register(middleware_plugin_config());
        assert!(!reg.is_empty());
        assert!(reg.get("test-mw").is_some());
        assert!(reg.get("nonexistent").is_none());
        assert_eq!(reg.list().len(), 1);
    }

    #[test]
    fn registry_from_configs_filters_disabled() {
        let mut disabled = middleware_plugin_config();
        disabled.enabled = false;
        disabled.id = "disabled-one".to_string();

        let reg = LlmPluginRegistry::from_configs(vec![middleware_plugin_config(), disabled]);
        assert_eq!(reg.list().len(), 1);
        assert!(reg.get("test-mw").is_some());
        assert!(reg.get("disabled-one").is_none());
    }

    #[test]
    fn registry_unregister() {
        let mut reg = LlmPluginRegistry::new();
        reg.register(middleware_plugin_config());
        assert_eq!(reg.list().len(), 1);

        let removed = reg.unregister("test-mw");
        assert!(removed.is_some());
        assert!(reg.is_empty());

        let removed_again = reg.unregister("test-mw");
        assert!(removed_again.is_none());
    }

    #[test]
    fn registry_create_middleware_provider() {
        let mut reg = LlmPluginRegistry::new();
        reg.register(middleware_plugin_config());

        let provider = reg.create_provider("test-mw");
        assert!(provider.is_ok());
        assert_eq!(provider.unwrap().provider_name(), "test-mw");
    }

    #[test]
    fn registry_create_process_provider() {
        let mut reg = LlmPluginRegistry::new();
        reg.register(process_plugin_config());

        let provider = reg.create_provider("test-proc");
        assert!(provider.is_ok());
        assert_eq!(provider.unwrap().provider_name(), "test-proc");
    }

    #[test]
    fn registry_create_provider_not_found() {
        let reg = LlmPluginRegistry::new();
        let result = reg.create_provider("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn registry_create_disabled_plugin_fails() {
        let mut reg = LlmPluginRegistry::new();
        let mut cfg = middleware_plugin_config();
        cfg.enabled = true;
        reg.register(cfg);

        let mut cfg2 = middleware_plugin_config();
        cfg2.enabled = false;
        reg.register(cfg2);

        let result = reg.create_provider("test-mw");
        assert!(result.is_err());
    }

    #[test]
    fn middleware_provider_model_mapping() {
        let mw = middleware_plugin_config().middleware.unwrap();
        let provider = MiddlewareLlmProvider::from_config("test-mw", &mw).unwrap();

        assert_eq!(provider.map_model("gpt-4o"), "custom-gpt4");
        assert_eq!(provider.map_model("gpt-3.5-turbo"), "gpt-3.5-turbo");
    }

    #[test]
    fn middleware_provider_from_config_static_headers() {
        let mw = middleware_plugin_config().middleware.unwrap();
        let provider = MiddlewareLlmProvider::from_config("test-mw", &mw).unwrap();

        assert!(provider.static_headers.contains_key("x-custom"));
        assert_eq!(
            provider
                .static_headers
                .get("x-custom")
                .unwrap()
                .to_str()
                .unwrap(),
            "val"
        );
    }

    #[tokio::test]
    async fn auth_middleware_none_produces_empty_headers() {
        let auth = AuthMiddleware::from_config(&AuthConfig::None).unwrap();
        let headers = auth.resolve_headers().await.unwrap();
        assert!(headers.is_empty());
    }

    #[tokio::test]
    async fn auth_middleware_bearer_token() {
        let auth = AuthMiddleware::from_config(&AuthConfig::BearerToken {
            token: "test-token".to_string(),
        })
        .unwrap();
        let headers = auth.resolve_headers().await.unwrap();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].0.as_str(), "authorization");
        assert_eq!(headers[0].1.to_str().unwrap(), "Bearer test-token");
    }

    #[tokio::test]
    async fn auth_middleware_custom_header() {
        let auth = AuthMiddleware::from_config(&AuthConfig::CustomHeader {
            header: "x-api-key".to_string(),
            value: "my-secret".to_string(),
        })
        .unwrap();
        let headers = auth.resolve_headers().await.unwrap();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].0.as_str(), "x-api-key");
        assert_eq!(headers[0].1.to_str().unwrap(), "my-secret");
    }

    #[test]
    fn chat_response_to_stream_deltas_produces_correct_chunks() {
        let resp = ChatResponse {
            id: "test-id".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: "test-model".to_string(),
            choices: vec![ChatChoice {
                index: 0,
                message: ChatMessage {
                    role: xiaolin_core::types::Role::Assistant,
                    content: Some(serde_json::Value::String("Hello!".to_string())),
                ..Default::default()
                },
                finish_reason: Some("stop".to_string()),
            }],
            usage: Some(Usage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
                ..Default::default()
            }),
        };

        let deltas = chat_response_to_stream_deltas(resp);
        assert_eq!(deltas.len(), 2);
        assert!(deltas[0].choices[0].delta.content.is_some());
        assert_eq!(
            deltas[0].choices[0].delta.content.as_deref().unwrap(),
            "Hello!"
        );
        assert!(deltas[0].choices[0].finish_reason.is_none());
        assert!(deltas[1].choices[0].delta.content.is_none());
        assert_eq!(
            deltas[1].choices[0].finish_reason.as_deref().unwrap(),
            "stop"
        );
        assert!(deltas[1].usage.is_some());
    }

    #[test]
    fn chat_response_with_tool_calls_to_stream_deltas() {
        let resp = ChatResponse {
            id: "tc-id".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: "test-model".to_string(),
            choices: vec![ChatChoice {
                index: 0,
                message: ChatMessage {
                    role: xiaolin_core::types::Role::Assistant,
                    content: None,
                    tool_calls: Some(vec![xiaolin_core::types::ToolCall {
                        id: "call_123".to_string(),
                        call_type: "function".to_string(),
                        function: xiaolin_core::types::FunctionCall {
                            name: "search".to_string(),
                            arguments: r#"{"q":"test"}"#.to_string(),
                        },
                        output: None,
                        success: None,
                        duration_ms: None,
                    }]),
                ..Default::default()
                },
                finish_reason: Some("tool_calls".to_string()),
            }],
            usage: None,
        };

        let deltas = chat_response_to_stream_deltas(resp);
        assert_eq!(deltas.len(), 2);
        let tc_delta = &deltas[0].choices[0].delta.tool_calls.as_ref().unwrap()[0];
        assert_eq!(tc_delta.id.as_deref().unwrap(), "call_123");
        assert_eq!(
            tc_delta.function.as_ref().unwrap().name.as_deref().unwrap(),
            "search"
        );
    }

    #[test]
    fn anthropic_value_to_chat_response_basic() {
        let v = serde_json::json!({
            "id": "msg_001",
            "model": "claude-3-opus",
            "type": "message",
            "role": "assistant",
            "stop_reason": "end_turn",
            "content": [
                { "type": "text", "text": "Hello there!" }
            ],
            "usage": { "input_tokens": 10, "output_tokens": 3 }
        });

        let resp = anthropic_value_to_chat_response(v).unwrap();
        assert_eq!(resp.id, "msg_001");
        assert_eq!(resp.model, "claude-3-opus");
        assert_eq!(resp.choices.len(), 1);
        let msg = &resp.choices[0].message;
        assert_eq!(msg.text_content().unwrap(), "Hello there!");
        assert_eq!(resp.usage.as_ref().unwrap().prompt_tokens, 10);
        assert_eq!(resp.usage.as_ref().unwrap().completion_tokens, 3);
    }

    #[test]
    fn anthropic_value_to_chat_response_with_tool_use() {
        let v = serde_json::json!({
            "id": "msg_002",
            "model": "claude-3",
            "stop_reason": "tool_use",
            "content": [
                { "type": "text", "text": "Let me search." },
                { "type": "tool_use", "id": "toolu_01", "name": "web_search", "input": { "query": "test" } }
            ],
            "usage": { "input_tokens": 20, "output_tokens": 10 }
        });

        let resp = anthropic_value_to_chat_response(v).unwrap();
        assert_eq!(
            resp.choices[0].finish_reason.as_deref().unwrap(),
            "tool_calls"
        );
        let msg = &resp.choices[0].message;
        assert!(msg.tool_calls.is_some());
        let tcs = msg.tool_calls.as_ref().unwrap();
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].id, "toolu_01");
        assert_eq!(tcs[0].function.name, "web_search");
    }

    #[test]
    fn plugin_aware_provider_creation_with_plugin_prefix() {
        let mut reg = LlmPluginRegistry::new();
        reg.register(middleware_plugin_config());

        let result = crate::llm::create_provider_with_plugins(
            "plugin:test-mw",
            None,
            None,
            None,
            None,
            Some(&reg),
        );
        assert!(result.is_ok());
        assert_eq!(result.unwrap().provider_name(), "test-mw");
    }

    #[test]
    fn plugin_aware_provider_creation_falls_back_for_builtin() {
        let reg = LlmPluginRegistry::new();

        let result = crate::llm::create_provider_with_plugins(
            "openai",
            Some("https://api.openai.com/v1"),
            Some("test-key"),
            None,
            None,
            Some(&reg),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn plugin_aware_provider_creation_no_registry_for_plugin() {
        let result = crate::llm::create_provider_with_plugins(
            "plugin:some-plugin",
            None,
            None,
            None,
            None,
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn find_model_context_window_found() {
        let mut reg = LlmPluginRegistry::new();
        reg.register(middleware_plugin_config());
        assert_eq!(
            reg.find_model_context_window("test-mw", "custom-gpt4"),
            Some(128000)
        );
    }

    #[test]
    fn find_model_context_window_unknown_model() {
        let mut reg = LlmPluginRegistry::new();
        reg.register(middleware_plugin_config());
        assert_eq!(
            reg.find_model_context_window("test-mw", "nonexistent-model"),
            None
        );
    }

    #[test]
    fn find_model_context_window_unknown_plugin() {
        let reg = LlmPluginRegistry::new();
        assert_eq!(reg.find_model_context_window("nonexistent", "any"), None);
    }

    #[test]
    fn find_model_context_window_zero_is_none() {
        let reg = LlmPluginRegistry::new();
        // No plugins registered — anything returns None.
        assert_eq!(
            reg.find_model_context_window("nonexistent", "anything"),
            None
        );
    }

    #[test]
    fn load_plugins_from_temp_dir() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_json = serde_json::json!({
            "id": "temp-test",
            "name": "Temp Test",
            "type": "middleware",
            "middleware": {
                "baseUrl": "https://api.example.com"
            },
            "models": [
                { "id": "m1", "name": "Model 1", "contextWindow": 8192 }
            ]
        });
        std::fs::write(
            dir.path().join("temp-test.json"),
            serde_json::to_string_pretty(&plugin_json).unwrap(),
        )
        .unwrap();

        std::fs::write(dir.path().join("bad.json"), "not valid json").unwrap();

        let plugins = xiaolin_core::llm_plugin::load_llm_plugins(dir.path());
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].id, "temp-test");
        assert_eq!(plugins[0].models.len(), 1);
    }
}
