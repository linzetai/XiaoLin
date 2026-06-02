use serde::{Deserialize, Serialize};

/// Result of [`crate::SessionStore::create_session`]: whether the row was inserted or refreshed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionCreateOutcome {
    Created,
    AlreadyExisted,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Session {
    pub id: String,
    pub agent_id: String,
    pub title: Option<String>,
    pub work_dir: Option<String>,
    #[serde(default = "default_source")]
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: i64,
    #[serde(default)]
    pub total_prompt_tokens: i64,
    #[serde(default)]
    pub total_completion_tokens: i64,
    #[serde(default)]
    pub total_elapsed_ms: i64,
}

fn default_source() -> String {
    "client".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SessionMessage {
    pub id: i64,
    pub session_id: String,
    pub role: String,
    pub content: Option<String>,
    pub name: Option<String>,
    pub tool_calls_json: Option<String>,
    pub tool_call_id: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,
    #[serde(default)]
    pub elapsed_ms: i64,
    pub reasoning_content: Option<String>,
    pub compact_metadata_json: Option<String>,
}

/// A persisted sub-agent run record.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentRunRow {
    pub run_id: String,
    pub parent_session_id: String,
    pub parent_message_id: String,
    pub agent_id: String,
    pub subagent_type: String,
    pub task: String,
    pub status: String,
    pub result: Option<String>,
    pub tool_calls_made: i64,
    pub iterations: i64,
    pub token_usage_json: Option<String>,
    pub depth: i64,
    pub elapsed_ms: Option<i64>,
    pub created_at: String,
    pub completed_at: Option<String>,
    /// JSON-serialized sidechain transcript: messages exchanged during the sub-agent run.
    pub transcript_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub agent_id: String,
    pub title: Option<String>,
    pub work_dir: Option<String>,
    #[serde(default = "default_source")]
    pub source: String,
    pub message_count: i64,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub total_prompt_tokens: i64,
    #[serde(default)]
    pub total_completion_tokens: i64,
    #[serde(default)]
    pub total_elapsed_ms: i64,
}

/// A persisted content replacement record for tool result budget enforcement.
/// Stores decisions made by `enforce_per_message_budget` so that session resume
/// can reconstruct the identical `ContentReplacementState`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ContentReplacementRow {
    pub tool_use_id: String,
    pub replacement: String,
}
