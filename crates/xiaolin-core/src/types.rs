use std::borrow::Cow;
use serde::{Deserialize, Serialize};

pub use xiaolin_protocol::{AgentId, AskQuestionOption, CompactTrigger, ExecutionMode, Role, SessionId};

pub type ThreadId = String;
pub type MessageId = String;
pub type ToolCallId = String;

// ---------------------------------------------------------------------------
// Model capability declarations
// ---------------------------------------------------------------------------

/// What kind of content the model can accept as input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputModality {
    Text,
    Image,
    Audio,
    Video,
}

/// What kind of output the model can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputModality {
    Text,
    ToolCalls,
    Reasoning,
}

/// Declared capabilities of a model. When absent the system falls back to
/// heuristic detection based on model name.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCapabilities {
    #[serde(default = "default_input_modalities")]
    pub input: Vec<InputModality>,
    #[serde(default = "default_output_modalities")]
    pub output: Vec<OutputModality>,
}

fn default_input_modalities() -> Vec<InputModality> {
    vec![InputModality::Text]
}

fn default_output_modalities() -> Vec<OutputModality> {
    vec![OutputModality::Text, OutputModality::ToolCalls]
}

impl Default for ModelCapabilities {
    fn default() -> Self {
        Self {
            input: default_input_modalities(),
            output: default_output_modalities(),
        }
    }
}

impl ModelCapabilities {
    pub fn supports_vision(&self) -> bool {
        self.input.contains(&InputModality::Image)
    }

    pub fn supports_tool_calling(&self) -> bool {
        self.output.contains(&OutputModality::ToolCalls)
    }

    pub fn supports_reasoning(&self) -> bool {
        self.output.contains(&OutputModality::Reasoning)
    }
}

/// Metadata attached to a compact boundary system message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactMetadata {
    /// What triggered the compaction.
    pub trigger: CompactTrigger,
    /// Token count before compaction.
    pub pre_compact_token_count: usize,
    /// Token count after compaction.
    pub post_compact_token_count: usize,
}

/// A chat message. `content` can be:
/// - `null` / absent
/// - a plain string: `"hello"`
/// - a multimodal array: `[{"type":"text","text":"..."}, {"type":"image_url","image_url":{"url":"data:..."}}]`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
    /// DeepSeek thinking-mode chain-of-thought. Must be passed back to the API
    /// on subsequent turns when tool calls are involved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<ToolCallId>,
    /// When `role == System` and this is a compact boundary marker,
    /// contains metadata about the compaction event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_metadata: Option<CompactMetadata>,
    /// Raw JSON override for `tool_calls_json` in the DB. When set,
    /// `SessionStore::append_message` writes this string directly instead of
    /// serializing `self.tool_calls`. This allows the gateway to embed extra
    /// UI-only fields (`display_output`, `metadata`) that are not part of
    /// the `ToolCall` struct and should not be sent to the LLM.
    #[serde(skip)]
    pub enriched_tool_calls_json: Option<String>,
}

impl ChatMessage {
    /// Extract the text portion of `content`, regardless of format.
    /// Returns `Cow::Borrowed` for plain strings (zero-copy) and
    /// `Cow::Owned` for multimodal arrays (join required).
    pub fn text_content(&self) -> Option<Cow<'_, str>> {
        match &self.content {
            Some(serde_json::Value::String(s)) => Some(Cow::Borrowed(s.as_str())),
            Some(serde_json::Value::Array(arr)) => {
                let mut texts = Vec::new();
                for item in arr {
                    if let Some(t) = item.get("text").and_then(|v| v.as_str()) {
                        texts.push(t);
                    }
                }
                if texts.is_empty() {
                    None
                } else {
                    Some(Cow::Owned(texts.join("\n")))
                }
            }
            _ => None,
        }
    }

    /// Check if this message contains any image content parts.
    pub fn has_images(&self) -> bool {
        match &self.content {
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .any(|item| item.get("type").and_then(|v| v.as_str()) == Some("image_url")),
            _ => false,
        }
    }

    /// Create a compact boundary system message marking where compaction occurred.
    pub fn compact_boundary(
        trigger: CompactTrigger,
        pre_tokens: usize,
        post_tokens: usize,
    ) -> Self {
        Self {
            role: Role::System,
            content: Some(serde_json::Value::String(
                "[Context was compacted: earlier conversation summarized]".to_string(),
            )),
            compact_metadata: Some(CompactMetadata {
                trigger,
                pre_compact_token_count: pre_tokens,
                post_compact_token_count: post_tokens,
            }),
            ..Default::default()
        }
    }

    /// Whether this message is a compact boundary marker.
    pub fn is_compact_boundary(&self) -> bool {
        self.compact_metadata.is_some()
    }
}

impl Default for ChatMessage {
    fn default() -> Self {
        Self {
            role: Role::User,
            content: None,
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
            compact_metadata: None,
            enriched_tool_calls_json: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: ToolCallId,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    #[serde(default)]
    pub model: Option<String>,
    pub messages: Vec<ChatMessage>,
    #[serde(default, alias = "agentId")]
    pub agent_id: Option<AgentId>,
    #[serde(default, alias = "sessionId")]
    pub session_id: Option<SessionId>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<crate::tool::ToolDefinition>>,
    #[serde(default, alias = "slashIntent")]
    pub slash_intent: Option<SlashIntent>,
    #[serde(default, alias = "workDir")]
    pub work_dir: Option<String>,
    #[serde(default, alias = "responseLanguage")]
    pub response_language: Option<String>,
    /// When true, the user entered Goal mode — autonomous execution with auto-approved tools.
    #[serde(default, alias = "goalMode", skip_serializing_if = "Option::is_none")]
    pub goal_mode: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashIntent {
    #[serde(rename = "type")]
    pub intent_type: String,
    pub value: String,
    #[serde(default, alias = "exactMatch")]
    pub exact_match: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<ChatChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChoice {
    pub index: u32,
    pub message: ChatMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
    #[serde(default)]
    pub total_tokens: u32,
    /// Anthropic: cache_read_input_tokens
    #[serde(default, alias = "cache_read_input_tokens")]
    pub cache_read_tokens: u32,
    /// Anthropic: cache_creation_input_tokens
    #[serde(default, alias = "cache_creation_input_tokens")]
    pub cache_creation_tokens: u32,
    /// DeepSeek: prompt_cache_hit_tokens
    #[serde(default)]
    pub prompt_cache_hit_tokens: u32,
    /// DeepSeek: prompt_cache_miss_tokens
    #[serde(default)]
    pub prompt_cache_miss_tokens: u32,
}

impl Usage {
    /// Unified cache read tokens across providers.
    pub fn effective_cache_read_tokens(&self) -> u32 {
        if self.cache_read_tokens > 0 {
            self.cache_read_tokens
        } else {
            self.prompt_cache_hit_tokens
        }
    }

    /// Unified cache creation tokens across providers.
    pub fn effective_cache_creation_tokens(&self) -> u32 {
        self.cache_creation_tokens
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamDelta {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<StreamChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Original JSON from an SSE `data:` line (OpenAI-compatible streams).
    #[serde(skip)]
    pub raw_sse_json: Option<bytes::Bytes>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChoice {
    pub index: u32,
    pub delta: DeltaContent,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaContent {
    #[serde(
        default,
        deserialize_with = "deserialize_lenient_role",
        skip_serializing_if = "Option::is_none"
    )]
    pub role: Option<Role>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// DeepSeek thinking-mode CoT streamed in chunks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<StreamToolCallDelta>>,
}

/// Deserialize `role` leniently: unknown or empty string values become `None`
/// instead of causing a deserialization error. Some APIs (e.g. ZhiPu) send
/// `"role": ""` in streaming delta chunks.
fn deserialize_lenient_role<'de, D>(deserializer: D) -> Result<Option<Role>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(deserializer)?;
    match opt.as_deref() {
        None | Some("") => Ok(None),
        Some("system") => Ok(Some(Role::System)),
        Some("user") => Ok(Some(Role::User)),
        Some("assistant") => Ok(Some(Role::Assistant)),
        Some("tool") => Ok(Some(Role::Tool)),
        Some(_) => Ok(None),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamToolCallDelta {
    pub index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<ToolCallId>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub call_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<StreamFunctionDelta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamFunctionDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

// ── Sub-Agent types ──────────────────────────────────────────────────

/// The kind of sub-agent to spawn, determining its tool set and behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubAgentType {
    /// Full-capability child agent inheriting the parent's tool set.
    #[default]
    General,
    /// Read-only exploration agent (file_read, search, web, memory).
    Explore,
    /// Command execution specialist (shell, file read/write).
    Shell,
    /// Browser automation agent (browser_*, web_fetch).
    Browser,
    /// User-defined type with custom tool filtering.
    Custom(String),
}

impl std::fmt::Display for SubAgentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::General => f.write_str("general"),
            Self::Explore => f.write_str("explore"),
            Self::Shell => f.write_str("shell"),
            Self::Browser => f.write_str("browser"),
            Self::Custom(name) => write!(f, "custom:{name}"),
        }
    }
}

/// Lifecycle status of a sub-agent run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubAgentStatus {
    Pending,
    Running,
    Completed,
    Failed(String),
    Cancelled,
}

impl SubAgentStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed(_) | Self::Cancelled)
    }
}

/// Tracks a single sub-agent execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentRun {
    pub run_id: String,
    pub parent_session_id: String,
    pub parent_message_id: String,
    pub agent_id: AgentId,
    pub subagent_type: SubAgentType,
    pub task: String,
    pub status: SubAgentStatus,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(default)]
    pub tool_calls_made: u32,
    #[serde(default)]
    pub iterations: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<Usage>,
    #[serde(default)]
    pub depth: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
}

/// Status of an MCP server connection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpStatus {
    #[default]
    Connecting,
    Connected,
    Failed,
    Disabled,
    /// Project-level server awaiting user approval before connecting.
    #[serde(rename = "pending_approval")]
    PendingApproval,
    /// Server requires OAuth authentication; user must click "Login" in the UI.
    #[serde(rename = "needs_auth")]
    NeedsAuth,
}

/// Runtime status information for an MCP server.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerStatus {
    pub id: String,
    pub status: McpStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub tool_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connected_at: Option<String>,
    /// `"user"` or `"project"` — origin of this server's configuration.
    #[serde(default = "default_scope")]
    pub scope: String,
    /// For pending_approval servers, preview of the command that will be executed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_preview: Option<String>,
    /// Transport type: "stdio", "sse", or "streamable_http".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    /// Whether the server declared `capabilities.resources`.
    #[serde(default)]
    pub has_resources: bool,
    /// Whether the server declared `capabilities.prompts`.
    #[serde(default)]
    pub has_prompts: bool,
}

fn default_scope() -> String {
    "global".to_string()
}

// ---------------------------------------------------------------------------
// Conversation trace types (harness / eval / debugging)
// ---------------------------------------------------------------------------

/// Record of a single tool call within a traced turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceToolCall {
    pub tool_name: String,
    pub call_id: String,
    pub arguments: serde_json::Value,
    pub output: String,
    pub success: bool,
    pub latency_ms: u64,
}

/// Captured LLM request metadata (inputs sent to the provider).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceLlmRequest {
    pub model: String,
    pub message_count: u32,
    pub estimated_tokens: u32,
}

/// Captured LLM response metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceLlmResponse {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    pub finish_reason: Option<String>,
    pub latency_ms: u64,
}

/// A single request-response turn inside a traced conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceTurn {
    pub turn_index: u32,
    pub user_message: ChatMessage,
    pub assistant_message: ChatMessage,
    pub tool_calls: Vec<TraceToolCall>,
    pub llm_request: TraceLlmRequest,
    pub llm_response: TraceLlmResponse,
    pub context_tokens: u32,
    pub latency_ms: u64,
    pub compaction_applied: bool,
}

/// Complete trace of a conversation session, suitable for replay and eval.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationTrace {
    pub trace_id: String,
    pub session_id: String,
    pub agent_id: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    pub turns: Vec<TraceTurn>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub metadata: serde_json::Map<String, serde_json::Value>,
}
