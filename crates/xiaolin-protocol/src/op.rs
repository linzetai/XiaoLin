use serde::{Deserialize, Serialize};

#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::id::{AgentId, SessionId};
use crate::message::ExecutionMode;

/// Typed parameters for Chat operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ChatParams {
    pub messages: serde_json::Value,
    #[serde(default, alias = "sessionId", skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, alias = "agentId", skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, alias = "maxTokens", skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, alias = "slashIntent", skip_serializing_if = "Option::is_none")]
    pub slash_intent: Option<String>,
    #[serde(default, alias = "workDir", skip_serializing_if = "Option::is_none")]
    pub work_dir: Option<String>,
    /// Catch-all for forward compatibility
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// (Deprecated) Typed parameters for the removed `submit` operation.
/// Kept for deserialization compatibility; new code should use `chat`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[deprecated(note = "Use ChatParams via the `chat` method instead")]
pub struct ChatSubmitParams {
    pub message: String,
    #[serde(default, alias = "sessionId", skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, alias = "agentId", skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Typed parameters for SessionsList.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SessionsListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
}

/// Typed parameters for SessionsNew.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SessionsNewParams {
    #[serde(default, alias = "agentId", skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    #[serde(default, alias = "workDir", skip_serializing_if = "Option::is_none")]
    pub work_dir: Option<String>,
}

/// Typed parameters for McpAdd.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct McpAddParams {
    pub id: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Catch-all for forward compatibility
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Typed parameters for ToolsList.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ToolsListParams {
    #[serde(default, alias = "agentId", skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, flatten)]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Typed parameters for ToolsUpdate.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ToolsUpdateParams {
    #[serde(default)]
    pub tool_id: String,
    #[serde(default, flatten)]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Typed parameters for SkillsList.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SkillsListParams {
    #[serde(default, alias = "agentId", skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, flatten)]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// A single message in a steer request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ChatSteerMessage {
    pub role: String,
    pub content: String,
}

/// Type-safe client operations replacing string-based WS dispatch.
///
/// Each variant maps to a WS method string (see `parse_request`).
/// New operations can be added without touching the gateway dispatch code —
/// just add a variant here and a handler function.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "op", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ClientOp {
    // ── Dialogue ────────────────────────────────────────────────────
    Chat {
        #[serde(flatten)]
        params: ChatParams,
    },
    ChatCancel {
        request_id: String,
    },
    ChatAnswer {
        request_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        answer: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        selected_ids: Vec<String>,
        #[serde(default, alias = "sessionId", skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
    ChatSetMode {
        session_id: SessionId,
        mode: ExecutionMode,
    },

    // ── Session management ──────────────────────────────────────────
    SessionsList {
        #[serde(flatten)]
        params: SessionsListParams,
    },
    SessionsGet {
        session_id: SessionId,
    },
    SessionsMessages {
        session_id: SessionId,
    },
    SessionsDelete {
        session_id: SessionId,
    },
    SessionsNew {
        #[serde(flatten)]
        params: SessionsNewParams,
    },
    SessionsClaim {
        session_id: SessionId,
    },
    SessionsUpdateTitle {
        session_id: SessionId,
        title: String,
    },
    SessionsSetWorkDir {
        session_id: SessionId,
        work_dir: Option<String>,
    },

    // ── Configuration ───────────────────────────────────────────────
    ModelsList,
    ConfigGet {
        #[serde(skip_serializing_if = "Option::is_none")]
        key: Option<String>,
    },
    ConfigSet {
        key: String,
        #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
        value: serde_json::Value,
    },

    // ── MCP ─────────────────────────────────────────────────────────
    McpStatus,
    McpReload,
    McpAdd {
        #[serde(flatten)]
        params: McpAddParams,
    },
    McpRemove {
        id: String,
    },
    McpDetail {
        id: String,
    },

    // ── Sub-Agent definitions ────────────────────────────────────────
    SubAgentsList,
    SubAgentsConcurrency,

    // ── Agent CRUD (deprecated) ───────────────────────────────────
    AgentsList,
    AgentsGet {
        agent_id: AgentId,
    },
    AgentsCreate {
        #[serde(flatten)]
        params: serde_json::Value,
    },
    AgentsUpdate {
        agent_id: AgentId,
        #[serde(flatten)]
        params: serde_json::Value,
    },
    AgentsDelete {
        agent_id: AgentId,
    },

    // ── Tools ───────────────────────────────────────────────────────
    ToolsList {
        #[serde(flatten)]
        params: ToolsListParams,
    },
    ToolsUpdate {
        #[serde(flatten)]
        params: ToolsUpdateParams,
    },
    ToolsSubmitAnswer {
        request_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        answer: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        selected_ids: Vec<String>,
        #[serde(default, alias = "sessionId", skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },

    // ── Skills ──────────────────────────────────────────────────────
    SkillsList {
        #[serde(flatten)]
        params: SkillsListParams,
    },
    SkillsRefresh,

    // ── Execution ───────────────────────────────────────────────────
    ExecutionSetMode {
        session_id: SessionId,
        mode: ExecutionMode,
    },
    ExecutionGetPlan {
        session_id: SessionId,
    },
    ExecutionApprovePlan {
        session_id: SessionId,
        mode: ExecutionMode,
    },

    // ── Pub/Sub ─────────────────────────────────────────────────────
    Subscribe {
        events: Vec<String>,
    },
    Unsubscribe {
        events: Vec<String>,
    },

    // ── Compaction ──────────────────────────────────────────────────
    ChatCompact {
        session_id: String,
    },

    // ── Steering ────────────────────────────────────────────────────
    ChatSteer {
        session_id: String,
        messages: Vec<ChatSteerMessage>,
    },

    // ── Approval ──────────────────────────────────────────────────
    ResolveApproval {
        approval_id: String,
        decision: crate::approval::ApprovalDecision,
        #[serde(default, alias = "sessionId", skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },

    // ── Cron ─────────────────────────────────────────────────────────
    CronListJobs {
        agent_id: Option<String>,
    },
    CronGetJob {
        job_id: String,
    },
    CronUpsertJob {
        params: serde_json::Value,
    },
    CronDeleteJob {
        job_id: String,
    },
    CronListRuns {
        job_id: String,
        limit: Option<i64>,
    },

    // ── Notifications ────────────────────────────────────────────────
    NotificationsUnreadCount,
    NotificationsList {
        limit: Option<i64>,
    },
    NotificationsMarkRead {
        notification_id: String,
    },
    NotificationsMarkAllRead,
    NotificationsDelete {
        notification_id: String,
    },

    // ── Channels ──────────────────────────────────────────────────────
    ChannelsList,
    ChannelsDetail {
        id: String,
    },
    ChannelsWechatLogin,
    ChannelsWechatPoll {
        session_key: String,
    },
    ChannelsWechatVerify {
        session_key: String,
        code: String,
    },
    ChannelsConnect {
        id: String,
    },
    ChannelsUpdate {
        id: String,
        #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
        config: serde_json::Value,
    },
    ChannelsRestore {
        id: String,
    },
    ChannelsDisconnect {
        channel_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account_id: Option<String>,
    },

    // ── Workspace ────────────────────────────────────────────────────
    WorkspaceInit {
        #[serde(alias = "workDir", skip_serializing_if = "Option::is_none")]
        work_dir: Option<String>,
    },

    // ── Keepalive ───────────────────────────────────────────────────
    Ping,
}

impl ClientOp {
    /// Parse a WS request into a typed `ClientOp`.
    ///
    /// Accepts `method` + `params` from the wire format `{ "method": "...", "params": {...} }`.
    pub fn parse_request(method: &str, params: serde_json::Value) -> Result<Self, String> {
        match method {
            "ping" => Ok(Self::Ping),
            "chat" => {
                let chat_params: ChatParams = serde_json::from_value(params)
                    .map_err(|e| format!("invalid chat params: {e}"))?;
                Ok(Self::Chat { params: chat_params })
            }
            "submit" => Err("the 'submit' method has been removed; use 'chat' instead".into()),
            "cancel" => Ok(Self::ChatCancel {
                request_id: extract_string(&params, "requestId")
                    .or_else(|_| extract_string(&params, "request_id"))?,
            }),
            "answer" => Ok(Self::ChatAnswer {
                request_id: extract_string(&params, "requestId")
                    .or_else(|_| extract_string(&params, "request_id"))?,
                answer: params
                    .get("answer")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                selected_ids: params
                    .get("selectedIds")
                    .or_else(|| params.get("selected_ids"))
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default(),
                session_id: params
                    .get("sessionId")
                    .or_else(|| params.get("session_id"))
                    .and_then(|v| v.as_str())
                    .map(String::from),
            }),
            "set_mode" => Ok(Self::ChatSetMode {
                session_id: extract_session_id(&params)?,
                mode: serde_json::from_value(
                    params
                        .get("mode")
                        .cloned()
                        .ok_or("missing 'mode'")?,
                )
                .map_err(|e| e.to_string())?,
            }),
            "sessions.list" => {
                let list_params: SessionsListParams =
                    serde_json::from_value(params).unwrap_or_default();
                Ok(Self::SessionsList { params: list_params })
            }
            "sessions.get" => Ok(Self::SessionsGet {
                session_id: extract_session_id(&params)?,
            }),
            "sessions.messages" => Ok(Self::SessionsMessages {
                session_id: extract_session_id(&params)?,
            }),
            "sessions.delete" => Ok(Self::SessionsDelete {
                session_id: extract_session_id(&params)?,
            }),
            "sessions.new" => {
                let new_params: SessionsNewParams =
                    serde_json::from_value(params).unwrap_or_default();
                Ok(Self::SessionsNew { params: new_params })
            }
            "sessions.claim" => Ok(Self::SessionsClaim {
                session_id: extract_session_id(&params)?,
            }),
            "sessions.update_title" => Ok(Self::SessionsUpdateTitle {
                session_id: extract_session_id(&params)?,
                title: extract_string(&params, "title")?,
            }),
            "sessions.set_work_dir" => Ok(Self::SessionsSetWorkDir {
                session_id: extract_session_id(&params)?,
                work_dir: params
                    .get("workDir")
                    .and_then(|v| if v.is_null() { None } else { v.as_str().map(String::from) }),
            }),
            "models.list" => Ok(Self::ModelsList),
            "config.get" => Ok(Self::ConfigGet {
                key: params.get("key").and_then(|v| v.as_str()).map(String::from),
            }),
            "config.set" => Ok(Self::ConfigSet {
                key: extract_string(&params, "key")?,
                value: params
                    .get("value")
                    .cloned()
                    .ok_or("missing 'value'")?,
            }),
            "mcp.status" => Ok(Self::McpStatus),
            "mcp.reload" => Ok(Self::McpReload),
            "mcp.add" => {
                let mcp_params: McpAddParams = serde_json::from_value(params)
                    .map_err(|e| format!("invalid mcp.add params: {e}"))?;
                Ok(Self::McpAdd { params: mcp_params })
            }
            "mcp.remove" => Ok(Self::McpRemove {
                id: extract_string(&params, "id")?,
            }),
            "mcp.detail" => Ok(Self::McpDetail {
                id: extract_string(&params, "id")?,
            }),
            "sub_agents.list" => Ok(Self::SubAgentsList),
            "sub_agents.concurrency" => Ok(Self::SubAgentsConcurrency),
            "agents" | "agents.list" => Ok(Self::AgentsList),
            "agents.get" => Ok(Self::AgentsGet {
                agent_id: AgentId::new(extract_string(&params, "agentId")
                    .or_else(|_| extract_string(&params, "agent_id"))?),
            }),
            "agents.create" => Ok(Self::AgentsCreate { params }),
            "agents.update" => Ok(Self::AgentsUpdate {
                agent_id: AgentId::new(extract_string(&params, "agentId")
                    .or_else(|_| extract_string(&params, "agent_id"))?),
                params,
            }),
            "agents.delete" => Ok(Self::AgentsDelete {
                agent_id: AgentId::new(extract_string(&params, "agentId")
                    .or_else(|_| extract_string(&params, "agent_id"))?),
            }),
            "tools.list" => {
                let list_params: ToolsListParams =
                    serde_json::from_value(params).unwrap_or_default();
                Ok(Self::ToolsList { params: list_params })
            }
            "tools.update" => {
                let update_params: ToolsUpdateParams =
                    serde_json::from_value(params).unwrap_or_default();
                Ok(Self::ToolsUpdate { params: update_params })
            }
            "skills.list" => {
                let list_params: SkillsListParams =
                    serde_json::from_value(params).unwrap_or_default();
                Ok(Self::SkillsList { params: list_params })
            }
            "skills.refresh" => Ok(Self::SkillsRefresh),
            "execution.set_mode" => Ok(Self::ExecutionSetMode {
                session_id: extract_session_id(&params)?,
                mode: serde_json::from_value(
                    params
                        .get("mode")
                        .cloned()
                        .ok_or("missing 'mode'")?,
                )
                .map_err(|e| e.to_string())?,
            }),
            "execution.get_plan" => Ok(Self::ExecutionGetPlan {
                session_id: extract_session_id(&params)?,
            }),
            "execution.approve_plan" => Ok(Self::ExecutionApprovePlan {
                session_id: extract_session_id(&params)?,
                mode: serde_json::from_value(
                    params
                        .get("mode")
                        .cloned()
                        .unwrap_or(serde_json::json!("agent")),
                )
                .map_err(|e| e.to_string())?,
            }),
            "subscribe" => Ok(Self::Subscribe {
                events: params
                    .get("events")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default(),
            }),
            "unsubscribe" => Ok(Self::Unsubscribe {
                events: params
                    .get("events")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default(),
            }),
            "chat.compact" | "compact" => {
                let session_id = extract_string(&params, "sessionId")
                    .or_else(|_| extract_string(&params, "session_id"))?;
                Ok(Self::ChatCompact { session_id })
            }
            "chat.steer" | "steer" => {
                let session_id = extract_string(&params, "sessionId")
                    .or_else(|_| extract_string(&params, "session_id"))?;
                let messages: Vec<ChatSteerMessage> = params
                    .get("messages")
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .ok_or("missing or invalid 'messages'")?;
                Ok(Self::ChatSteer {
                    session_id,
                    messages,
                })
            }
            "resolve_approval" | "approval.resolve" => {
                let approval_id = params
                    .get("approvalId")
                    .or_else(|| params.get("approval_id"))
                    .and_then(|v| v.as_str())
                    .ok_or("approvalId required")?
                    .to_string();
                let decision: crate::approval::ApprovalDecision = serde_json::from_value(
                    params
                        .get("decision")
                        .cloned()
                        .ok_or("decision required")?,
                )
                .map_err(|e| format!("invalid decision: {e}"))?;
                let session_id = params
                    .get("sessionId")
                    .or_else(|| params.get("session_id"))
                    .and_then(|v| v.as_str())
                    .map(String::from);
                Ok(Self::ResolveApproval {
                    approval_id,
                    decision,
                    session_id,
                })
            }
            "tools.submit_answer" => Ok(Self::ToolsSubmitAnswer {
                request_id: extract_string(&params, "requestId")
                    .or_else(|_| extract_string(&params, "request_id"))?,
                answer: params
                    .get("answer")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                selected_ids: params
                    .get("selectedIds")
                    .or_else(|| params.get("selected_ids"))
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default(),
                session_id: params
                    .get("sessionId")
                    .or_else(|| params.get("session_id"))
                    .and_then(|v| v.as_str())
                    .map(String::from),
            }),
            "cron.list_jobs" => Ok(Self::CronListJobs {
                agent_id: params
                    .get("agentId")
                    .or_else(|| params.get("agent_id"))
                    .and_then(|v| v.as_str())
                    .map(String::from),
            }),
            "cron.get_job" => Ok(Self::CronGetJob {
                job_id: extract_string(&params, "jobId")
                    .or_else(|_| extract_string(&params, "job_id"))?,
            }),
            "cron.upsert_job" => Ok(Self::CronUpsertJob { params }),
            "cron.delete_job" => Ok(Self::CronDeleteJob {
                job_id: extract_string(&params, "jobId")
                    .or_else(|_| extract_string(&params, "job_id"))?,
            }),
            "cron.list_runs" => Ok(Self::CronListRuns {
                job_id: extract_string(&params, "jobId")
                    .or_else(|_| extract_string(&params, "job_id"))?,
                limit: params
                    .get("limit")
                    .and_then(|v| v.as_i64()),
            }),
            "notifications.unread_count" => Ok(Self::NotificationsUnreadCount),
            "notifications.list" => Ok(Self::NotificationsList {
                limit: params.get("limit").and_then(|v| v.as_i64()),
            }),
            "notifications.mark_read" => Ok(Self::NotificationsMarkRead {
                notification_id: extract_string(&params, "notificationId")
                    .or_else(|_| extract_string(&params, "notification_id"))?,
            }),
            "notifications.mark_all_read" => Ok(Self::NotificationsMarkAllRead),
            "notifications.delete" => Ok(Self::NotificationsDelete {
                notification_id: extract_string(&params, "notificationId")
                    .or_else(|_| extract_string(&params, "notification_id"))?,
            }),
            "channels.list" => Ok(Self::ChannelsList),
            "channels.detail" => Ok(Self::ChannelsDetail {
                id: extract_string(&params, "id")?,
            }),
            "channels.wechat_login" => Ok(Self::ChannelsWechatLogin),
            "channels.wechat_poll" => Ok(Self::ChannelsWechatPoll {
                session_key: extract_string(&params, "sessionKey")
                    .or_else(|_| extract_string(&params, "session_key"))?,
            }),
            "channels.wechat_verify" => Ok(Self::ChannelsWechatVerify {
                session_key: extract_string(&params, "sessionKey")
                    .or_else(|_| extract_string(&params, "session_key"))?,
                code: extract_string(&params, "code")?,
            }),
            "channels.connect" => Ok(Self::ChannelsConnect {
                id: extract_string(&params, "id")?,
            }),
            "channels.update" => Ok(Self::ChannelsUpdate {
                id: extract_string(&params, "id")?,
                config: params
                    .get("config")
                    .cloned()
                    .ok_or("missing 'config'")?,
            }),
            "channels.restore" => Ok(Self::ChannelsRestore {
                id: extract_string(&params, "id")?,
            }),
            "channels.disconnect" => Ok(Self::ChannelsDisconnect {
                channel_id: extract_string(&params, "channelId")
                    .or_else(|_| extract_string(&params, "channel_id"))?,
                account_id: params
                    .get("accountId")
                    .or_else(|| params.get("account_id"))
                    .and_then(|v| v.as_str())
                    .map(String::from),
            }),
            "workspace.init" => Ok(Self::WorkspaceInit {
                work_dir: params
                    .get("workDir")
                    .or_else(|| params.get("work_dir"))
                    .and_then(|v| v.as_str())
                    .map(String::from),
            }),
            other => Err(format!("unknown method: {other}")),
        }
    }
}

fn extract_string(params: &serde_json::Value, key: &str) -> Result<String, String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| format!("missing '{key}'"))
}

fn extract_session_id(params: &serde_json::Value) -> Result<SessionId, String> {
    extract_string(params, "sessionId")
        .or_else(|_| extract_string(params, "session_id"))
        .map(SessionId::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_ping() {
        let op = ClientOp::parse_request("ping", json!({})).unwrap();
        assert!(matches!(op, ClientOp::Ping));
    }

    #[test]
    fn parse_chat() {
        let op = ClientOp::parse_request("chat", json!({"messages": []})).unwrap();
        assert!(matches!(op, ClientOp::Chat { .. }));
    }

    #[test]
    fn parse_sessions_list() {
        let op = ClientOp::parse_request("sessions.list", json!({})).unwrap();
        assert!(matches!(op, ClientOp::SessionsList { .. }));
    }

    #[test]
    fn parse_sessions_get() {
        let op =
            ClientOp::parse_request("sessions.get", json!({"sessionId": "s1"})).unwrap();
        if let ClientOp::SessionsGet { session_id } = op {
            assert_eq!(session_id, "s1");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn parse_chat_answer() {
        let op = ClientOp::parse_request(
            "answer",
            json!({"requestId": "r1", "answer": "yes"}),
        )
        .unwrap();
        if let ClientOp::ChatAnswer {
            request_id,
            answer,
            ..
        } = op
        {
            assert_eq!(request_id, "r1");
            assert_eq!(answer, Some("yes".into()));
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn parse_unknown_method() {
        let result = ClientOp::parse_request("not.exist", json!({}));
        assert!(result.is_err());
    }

    #[test]
    fn parse_config_set() {
        let op =
            ClientOp::parse_request("config.set", json!({"key": "a.b", "value": 42})).unwrap();
        if let ClientOp::ConfigSet { key, value } = op {
            assert_eq!(key, "a.b");
            assert_eq!(value, json!(42));
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn parse_subscribe() {
        let op = ClientOp::parse_request(
            "subscribe",
            json!({"events": ["chat", "tools"]}),
        )
        .unwrap();
        if let ClientOp::Subscribe { events } = op {
            assert_eq!(events, vec!["chat", "tools"]);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn parse_mcp_add() {
        let op = ClientOp::parse_request(
            "mcp.add",
            json!({"id": "test", "command": "echo"}),
        )
        .unwrap();
        assert!(matches!(op, ClientOp::McpAdd { .. }));
    }

    #[test]
    fn parse_agents_crud() {
        let _ = ClientOp::parse_request("agents", json!({})).unwrap();
        let _ = ClientOp::parse_request("agents.get", json!({"agentId": "a1"})).unwrap();
        let _ = ClientOp::parse_request("agents.create", json!({"name": "n"})).unwrap();
        let _ = ClientOp::parse_request("agents.update", json!({"agentId": "a1"})).unwrap();
        let _ = ClientOp::parse_request("agents.delete", json!({"agentId": "a1"})).unwrap();
    }

    #[test]
    fn parse_tools_and_skills() {
        let _ = ClientOp::parse_request("tools.list", json!({})).unwrap();
        let _ = ClientOp::parse_request("tools.update", json!({})).unwrap();
        let _ = ClientOp::parse_request("skills.list", json!({})).unwrap();
        let _ = ClientOp::parse_request("skills.refresh", json!({})).unwrap();
    }

    #[test]
    fn parse_execution() {
        let op = ClientOp::parse_request(
            "execution.set_mode",
            json!({"sessionId": "s1", "mode": "plan"}),
        )
        .unwrap();
        if let ClientOp::ExecutionSetMode { mode, .. } = op {
            assert_eq!(mode, ExecutionMode::Plan);
        } else {
            panic!("wrong variant");
        }

        let _ = ClientOp::parse_request(
            "execution.get_plan",
            json!({"sessionId": "s1"}),
        )
        .unwrap();
    }

    #[test]
    fn parse_resolve_approval() {
        let op = ClientOp::parse_request(
            "resolve_approval",
            json!({"approvalId": "ap-1", "decision": {"decision": "approved"}}),
        )
        .unwrap();
        if let ClientOp::ResolveApproval {
            approval_id,
            decision,
            ..
        } = op
        {
            assert_eq!(approval_id, "ap-1");
            assert_eq!(decision, crate::approval::ApprovalDecision::Approved);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn client_op_serde_roundtrip() {
        let op = ClientOp::Ping;
        let json = serde_json::to_string(&op).unwrap();
        let back: ClientOp = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, ClientOp::Ping));
    }
}
