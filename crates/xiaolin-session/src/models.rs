use serde::{Deserialize, Serialize};

/// Result of [`crate::SessionStore::create_session`]: whether the row was inserted or refreshed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionCreateOutcome {
    Created,
    AlreadyExisted,
}

/// A registered project in the global project registry.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub root_path: String,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default)]
    pub pinned: i64,
    #[serde(default)]
    pub archived: i64,
    pub created_at: String,
    pub last_opened_at: String,
}

fn default_color() -> String {
    "#0066cc".to_string()
}

/// Patch for updating project properties.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectPatch {
    pub name: Option<String>,
    pub color: Option<String>,
    pub pinned: Option<bool>,
    pub archived: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Session {
    pub id: String,
    pub agent_id: String,
    pub title: Option<String>,
    pub work_dir: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
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
    pub segment_order_json: Option<String>,
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
    /// Path to the sidechain transcript file (legacy column name: stores path, not JSON content).
    pub transcript_json: Option<String>,
}

impl From<SubAgentRunRow> for xiaolin_core::types::SubAgentRun {
    fn from(r: SubAgentRunRow) -> Self {
        let status = match r.status.as_str() {
            "pending" => xiaolin_core::types::SubAgentStatus::Pending,
            "running" => xiaolin_core::types::SubAgentStatus::Running,
            "completed" => xiaolin_core::types::SubAgentStatus::Completed,
            "cancelled" => xiaolin_core::types::SubAgentStatus::Cancelled,
            "failed" => {
                xiaolin_core::types::SubAgentStatus::Failed(r.result.clone().unwrap_or_default())
            }
            other => xiaolin_core::types::SubAgentStatus::Failed(format!("unknown status: {other}")),
        };
        let token_usage: Option<xiaolin_core::types::Usage> =
            r.token_usage_json.as_ref().and_then(|j| {
                let v: serde_json::Value = serde_json::from_str(j).ok()?;
                Some(xiaolin_core::types::Usage {
                    prompt_tokens: v.get("prompt_tokens")?.as_u64()? as u32,
                    completion_tokens: v.get("completion_tokens")?.as_u64()? as u32,
                    total_tokens: v.get("total_tokens")?.as_u64()? as u32,
                    ..Default::default()
                })
            });
        // Restore truncated flag that was encoded into token_usage_json by build_db_row.
        let truncated = r.token_usage_json.as_ref().and_then(|j| {
            let v: serde_json::Value = serde_json::from_str(j).ok()?;
            v.get("truncated")?.as_bool()
        }).unwrap_or(false);
        let created_at_ms = chrono::DateTime::parse_from_rfc3339(&r.created_at)
            .map(|dt| dt.timestamp_millis() as u64)
            .unwrap_or(0);
        let completed_at_ms = r.completed_at.as_ref().and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.timestamp_millis() as u64)
                .ok()
        });
        Self {
            run_id: r.run_id,
            parent_session_id: r.parent_session_id,
            parent_message_id: r.parent_message_id,
            agent_id: xiaolin_core::types::AgentId::from(r.agent_id.as_str()),
            subagent_type: match r.subagent_type.as_str() {
                "explore" => xiaolin_core::types::SubAgentType::Explore,
                "shell" => xiaolin_core::types::SubAgentType::Shell,
                "browser" => xiaolin_core::types::SubAgentType::Browser,
                "general" => xiaolin_core::types::SubAgentType::General,
                other => xiaolin_core::types::SubAgentType::Custom(other.into()),
            },
            task: r.task,
            status,
            created_at: created_at_ms,
            completed_at: completed_at_ms,
            result: r.result,
            tool_calls_made: r.tool_calls_made as u32,
            iterations: r.iterations as u32,
            token_usage,
            depth: r.depth as u32,
            elapsed_ms: r.elapsed_ms.map(|e| e as u64),
            current_tool: None,
            truncated,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub agent_id: String,
    pub title: Option<String>,
    pub work_dir: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
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
