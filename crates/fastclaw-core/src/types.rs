use serde::{Deserialize, Serialize};

pub type AgentId = String;
pub type SessionId = String;
pub type ThreadId = String;
pub type MessageId = String;
pub type ToolCallId = String;

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
                item.get("type")
                    .and_then(|v| v.as_str())
                    .map_or(false, |t| t == "image_url")
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<StreamToolCallDelta>>,
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
        success: bool,
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
    Error(String),
}
