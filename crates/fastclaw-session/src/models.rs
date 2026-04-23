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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub agent_id: String,
    pub title: Option<String>,
    pub work_dir: Option<String>,
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
