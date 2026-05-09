use serde::{Deserialize, Serialize};

/// Type-safe wrapper for agent identifiers, preventing accidental misuse of
/// unrelated strings as agent IDs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct AgentId(String);

impl AgentId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::ops::Deref for AgentId {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl From<String> for AgentId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for AgentId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<AgentId> for String {
    fn from(id: AgentId) -> Self {
        id.0
    }
}

impl std::borrow::Borrow<str> for AgentId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for AgentId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for AgentId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<String> for AgentId {
    fn eq(&self, other: &String) -> bool {
        self.0 == *other
    }
}
pub type SessionId = String;
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
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
}

impl ChatMessage {
    /// Extract the text portion of `content`, regardless of format.
    pub fn text_content(&self) -> Option<String> {
        match &self.content {
            Some(serde_json::Value::String(s)) => Some(s.clone()),
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
                    Some(texts.join("\n"))
                }
            }
            _ => None,
        }
    }

    /// Check if this message contains any image content parts.
    pub fn has_images(&self) -> bool {
        match &self.content {
            Some(serde_json::Value::Array(arr)) => arr.iter().any(|item| {
                item.get("type").and_then(|v| v.as_str()) == Some("image_url")
            }),
            _ => false,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpStatus {
    Connecting,
    Connected,
    Failed,
    Disabled,
}

/// Runtime status information for an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerStatus {
    pub id: String,
    pub status: McpStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub tool_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connected_at: Option<String>,
}

/// A single option presented to the user by `ask_question`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AskQuestionOption {
    pub id: String,
    pub label: String,
}

/// Execution mode controlling which tools are available and how the agent behaves.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Full execution mode: all tools available.
    #[default]
    Agent,
    /// Read-only planning mode: write/edit/execute tools are blocked.
    Plan,
}

impl std::fmt::Display for ExecutionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Agent => write!(f, "agent"),
            Self::Plan => write!(f, "plan"),
        }
    }
}

/// Event emitted by the streaming agentic loop.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    Delta(StreamDelta),
    ToolExecuting {
        tool_name: String,
        call_id: String,
        args: Option<String>,
    },
    ToolResult {
        tool_name: String,
        call_id: String,
        output: String,
        /// Richer output for the UI; `None` means fall back to `output`.
        display_output: Option<String>,
        success: bool,
        /// Optional structured metadata for frontend rendering.
        metadata: Option<serde_json::Value>,
    },
    AskQuestion {
        request_id: String,
        question: String,
        options: Vec<AskQuestionOption>,
        timeout_secs: u32,
        allow_multiple: bool,
    },
    Done {
        session_id: Option<String>,
        tool_calls_made: u32,
        iterations: u32,
        /// Accumulated tool calls from the final assistant turn (if any).
        /// Allows the gateway to persist tool_calls alongside the assistant message.
        final_tool_calls: Option<Vec<ToolCall>>,
        /// Accumulated token usage across all LLM iterations.
        usage: Option<Usage>,
        /// Wall-clock elapsed time for the entire streaming run (ms).
        elapsed_ms: u64,
        /// Estimated input context tokens used (from API usage or pre-call estimate).
        context_tokens: Option<u32>,
        /// The model's effective context window limit.
        context_window: Option<u32>,
    },
    /// Intermediate progress update from a long-running tool.
    /// Useful for shell commands, downloads, large file operations, etc.
    ToolProgress {
        tool_name: String,
        call_id: String,
        /// Progress message (e.g., "downloading 50%", "line 1000 of 5000")
        message: String,
        /// Optional numeric progress (0.0 to 1.0)
        progress: Option<f64>,
        /// Optional current output so far (streaming partial output)
        partial_output: Option<String>,
    },

    /// Agent-initiated message pushed to the user (via BriefTool / SendUserMessage).
    BriefMessage {
        /// Markdown-formatted message body.
        content: String,
        /// Optional file paths attached to the message.
        attachments: Vec<String>,
        /// `"normal"` when responding to a user action, `"proactive"` when
        /// the agent initiates communication unprompted.
        mode: String,
    },

    Error(String),

    /// Emitted when execution mode changes (e.g. Agent ↔ Plan).
    ModeChange {
        from: ExecutionMode,
        to: ExecutionMode,
    },

    /// Emitted when context token usage exceeds a safety threshold.
    ContextLimitWarning {
        used_tokens: u32,
        limit_tokens: u32,
        message: String,
    },

    /// Softer warning at 85% context usage suggesting /compact.
    /// Non-blocking — the LLM call proceeds normally.
    /// Sent at most once per session to avoid noise.
    CompactWarning {
        used_tokens: u32,
        limit_tokens: u32,
        message: String,
    },

    /// Emitted after each iteration's context management phase to keep
    /// the frontend updated on live context token usage.
    ContextUsageUpdate {
        used_tokens: u32,
        limit_tokens: u32,
        /// Whether LLM-based compression was applied this iteration.
        compressed: bool,
        /// Tokens saved by compression (0 if not compressed).
        tokens_saved: u32,
    },

    // ── Sub-agent streaming events ──────────────────────────────────

    /// A sub-agent has been spawned and is starting execution.
    SubAgentStart {
        run_id: String,
        agent_id: String,
        subagent_type: String,
        task: String,
        depth: u32,
    },
    /// Incremental text output from a running sub-agent.
    SubAgentDelta {
        run_id: String,
        content: String,
    },
    /// A sub-agent is executing a tool.
    SubAgentToolExecuting {
        run_id: String,
        tool_name: String,
        call_id: String,
        args: Option<String>,
    },
    /// A sub-agent tool call has completed.
    SubAgentToolResult {
        run_id: String,
        tool_name: String,
        call_id: String,
        output: String,
        success: bool,
    },
    /// A sub-agent run has finished (completed, failed, or cancelled).
    SubAgentComplete {
        run_id: String,
        status: String,
        result: Option<String>,
        tool_calls_made: u32,
        iterations: u32,
        usage: Option<Usage>,
        elapsed_ms: u64,
    },

    /// Suggested next actions generated at the end of an assistant turn.
    Suggestions {
        items: Vec<String>,
    },
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
