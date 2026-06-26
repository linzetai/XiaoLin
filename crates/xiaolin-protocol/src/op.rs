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
    #[serde(
        default,
        alias = "slashIntent",
        skip_serializing_if = "Option::is_none"
    )]
    pub slash_intent: Option<String>,
    #[serde(default, alias = "workDir", skip_serializing_if = "Option::is_none")]
    pub work_dir: Option<String>,
    #[serde(
        default,
        alias = "responseLanguage",
        skip_serializing_if = "Option::is_none"
    )]
    pub response_language: Option<String>,
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
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Transport type: "stdio" (default), "sse", "streamable_http", "http".
    #[serde(default = "McpAddParams::default_transport")]
    pub transport: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub env: std::collections::HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bearer_token_env_var: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_headers: Option<std::collections::HashMap<String, String>>,
    /// Catch-all for forward compatibility
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl McpAddParams {
    fn default_transport() -> String {
        "stdio".to_string()
    }
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

/// Typed parameters for SkillsRead.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SkillsReadParams {
    #[serde(default, alias = "skillId")]
    pub skill_id: String,
    #[serde(default, alias = "agentId", skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

/// Typed parameters for SkillsUpdate.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SkillsUpdateParams {
    #[serde(default, alias = "skillId")]
    pub skill_id: String,
    #[serde(default)]
    pub content: String,
}

/// Typed parameters for SkillsDelete.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SkillsDeleteParams {
    #[serde(default, alias = "skillId")]
    pub skill_id: String,
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
pub enum ClientOp {
    // ── Dialogue ────────────────────────────────────────────────────
    Chat {
        #[serde(flatten)]
        params: ChatParams,
    },
    ChatCancel {
        request_id: Option<String>,
        session_id: Option<String>,
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
    SubAgentsRuns {
        session_id: Option<String>,
    },
    SubAgentsConcurrency,
    SubAgentCancel {
        run_id: String,
    },

    // ── Agent CRUD ────────────────────────────────────────────────
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
    SkillsRead {
        #[serde(flatten)]
        params: SkillsReadParams,
    },
    SkillsUpdate {
        #[serde(flatten)]
        params: SkillsUpdateParams,
    },
    SkillsDelete {
        #[serde(flatten)]
        params: SkillsDeleteParams,
    },
    SkillsRefresh,
    EvolutionList,
    EvolutionPromote {
        skill_id: String,
    },

    // ── Marketplace ────────────────────────────────────────────────
    MarketplaceBrowse {
        query: Option<String>,
        limit: Option<usize>,
    },
    MarketplaceInstall {
        skill_id: String,
        version: Option<String>,
    },
    MarketplaceUninstall {
        skill_id: String,
    },

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
    ExecutionRejectPlan {
        session_id: SessionId,
        feedback: Option<String>,
    },
    ExecutionGetPlanMeta {
        session_id: SessionId,
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

    SubAgentSteer {
        run_id: String,
        message: String,
        #[serde(default)]
        priority: Option<String>,
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

    // ── Cost ─────────────────────────────────────────────────────────
    CostSummary,
    CostDaily {
        start: Option<String>,
        end: Option<String>,
    },
    CostTools {
        start: Option<String>,
        end: Option<String>,
    },
    CostSessions {
        limit: Option<i64>,
    },

    // ── Search ───────────────────────────────────────────────────────
    SearchQuery {
        params: crate::search::SearchQueryRequest,
    },
    SearchIndexStatus,

    // ── Automations (user-facing wrapper over cron) ──────────────────
    AutomationsList,
    AutomationsCreate {
        params: serde_json::Value,
    },
    AutomationsUpdate {
        job_id: String,
        params: serde_json::Value,
    },
    AutomationsDelete {
        job_id: String,
    },
    AutomationsRuns {
        job_id: String,
        limit: Option<i64>,
    },
    AutomationsRunNow {
        job_id: String,
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

    // ── Projects ──────────────────────────────────────────────────────
    ProjectsList {
        #[serde(
            default,
            alias = "includeArchived",
            skip_serializing_if = "Option::is_none"
        )]
        include_archived: Option<bool>,
    },
    ProjectsCreate {
        #[serde(alias = "rootPath")]
        root_path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<String>,
    },
    ProjectsUpdate {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pinned: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        archived: Option<bool>,
    },
    ProjectsDelete {
        id: String,
    },
    ProjectsDetect {
        path: String,
    },

    // ── Permissions ──────────────────────────────────────────────────
    PermissionsGetPresets,
    PermissionsGetSession {
        session_id: String,
    },
    PermissionsSetSession {
        session_id: String,
        preset_id: String,
    },

    // ── Plugins ──────────────────────────────────────────────────────
    PluginsList,
    PluginsEnable {
        id: String,
    },
    PluginsDisable {
        id: String,
    },
    PluginsRestart {
        id: String,
    },
    PluginsTools {
        id: String,
    },
    PluginsApprove {
        id: String,
    },
    PluginsReject {
        id: String,
    },
    PluginsOauthLogin {
        id: String,
    },
    PluginsResources {
        server_name: String,
    },
    PluginsPrompts,
    PluginsGetPrompt {
        server_name: String,
        prompt_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arguments: Option<std::collections::HashMap<String, String>>,
    },
    PluginsElicitationReply {
        elicitation_id: String,
        action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content: Option<serde_json::Value>,
    },

    // ── Workspace ────────────────────────────────────────────────────
    WorkspaceInit {
        #[serde(alias = "workDir", skip_serializing_if = "Option::is_none")]
        work_dir: Option<String>,
    },

    // ── Git ─────────────────────────────────────────────────────────
    GitStatus {
        #[serde(alias = "projectId")]
        project_id: String,
    },
    GitDiff {
        #[serde(alias = "projectId")]
        project_id: String,
        path: String,
        #[serde(default)]
        staged: bool,
    },
    GitBranches {
        #[serde(alias = "projectId")]
        project_id: String,
    },
    GitLog {
        #[serde(alias = "projectId")]
        project_id: String,
        #[serde(default = "default_git_log_limit")]
        limit: u32,
    },
    GitStage {
        #[serde(alias = "projectId")]
        project_id: String,
        #[serde(default)]
        files: Vec<String>,
    },
    GitUnstage {
        #[serde(alias = "projectId")]
        project_id: String,
        #[serde(default)]
        files: Vec<String>,
    },
    GitCommit {
        #[serde(alias = "projectId")]
        project_id: String,
        message: String,
    },
    GitRevert {
        #[serde(alias = "projectId")]
        project_id: String,
        files: Vec<String>,
    },
    GitInit {
        #[serde(alias = "projectId")]
        project_id: String,
    },

    // ── Goal management ──────────────────────────────────────────────
    GoalPause {
        session_id: String,
    },
    GoalResume {
        session_id: String,
    },
    GoalClear {
        session_id: String,
    },
    GoalEdit {
        session_id: String,
        description: String,
    },
    GoalAddBudget {
        session_id: String,
        amount: u64,
    },

    // ── Keepalive ───────────────────────────────────────────────────
    Ping,

    // ── File artifacts ──────────────────────────────────────────────
    ArtifactsList {
        #[serde(alias = "sessionId")]
        session_id: String,
    },
}

fn default_git_log_limit() -> u32 {
    20
}

/// JSON-RPC style parse error for WS `ClientOp` requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientOpParseError {
    pub code: i32,
    pub message: String,
}

impl ClientOpParseError {
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: message.into(),
        }
    }

    pub fn unknown_method(message: impl Into<String>) -> Self {
        Self {
            code: -32601,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ClientOpParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// All wire-level WS method names supported by [`ClientOp::parse_request`], plus `auth`.
pub fn all_ws_method_names() -> &'static [&'static str] {
    const METHODS: &[&str] = &[
        "auth",
        "ping",
        "chat",
        "cancel",
        "answer",
        "set_mode",
        "sessions.list",
        "sessions.get",
        "sessions.messages",
        "sessions.delete",
        "sessions.new",
        "sessions.claim",
        "sessions.update_title",
        "sessions.set_work_dir",
        "models.list",
        "config.get",
        "config.set",
        "mcp.status",
        "mcp.reload",
        "mcp.add",
        "mcp.remove",
        "mcp.detail",
        "sub_agents.list",
        "sub_agents.runs",
        "subagents.runs",
        "sub_agents.concurrency",
        "subagents.cancel",
        "sub_agents.cancel",
        "agents",
        "agents.list",
        "agents.get",
        "agents.create",
        "agents.update",
        "agents.delete",
        "tools.list",
        "tools.update",
        "tools.submit_answer",
        "skills.list",
        "skills.read",
        "skills.update",
        "skills.delete",
        "skills.refresh",
        "evolution.list",
        "evolution.promote",
        "marketplace.browse",
        "marketplace.search",
        "marketplace.install",
        "marketplace.uninstall",
        "execution.set_mode",
        "execution.get_plan",
        "execution.approve_plan",
        "execution.reject_plan",
        "execution.get_plan_meta",
        "subscribe",
        "unsubscribe",
        "chat.compact",
        "compact",
        "chat.steer",
        "steer",
        "subagent.steer",
        "steering_message",
        "resolve_approval",
        "approval.resolve",
        "cron.list_jobs",
        "cron.get_job",
        "cron.upsert_job",
        "cron.delete_job",
        "cron.list_runs",
        "cost.summary",
        "cost.daily",
        "cost.tools",
        "cost.sessions",
        "search.query",
        "search.index_status",
        "automations.list",
        "automations.create",
        "automations.update",
        "automations.delete",
        "automations.runs",
        "automations.run_now",
        "notifications.unread_count",
        "notifications.list",
        "notifications.mark_read",
        "notifications.mark_all_read",
        "notifications.delete",
        "channels.list",
        "channels.detail",
        "channels.wechat_login",
        "channels.wechat_poll",
        "channels.wechat_verify",
        "channels.connect",
        "channels.update",
        "channels.restore",
        "channels.disconnect",
        "projects.list",
        "projects.create",
        "projects.update",
        "projects.delete",
        "projects.detect",
        "permissions.get_presets",
        "permissions.get_session",
        "permissions.set_session",
        "plugins.list",
        "plugins.enable",
        "plugins.disable",
        "plugins.restart",
        "plugins.tools",
        "plugins.approve",
        "plugins.reject",
        "plugins.oauth_login",
        "plugins.resources",
        "plugins.prompts",
        "plugins.get_prompt",
        "plugins.elicitation_reply",
        "workspace.init",
        "git.status",
        "git.diff",
        "git.branches",
        "git.log",
        "git.stage",
        "git.unstage",
        "git.commit",
        "git.revert",
        "git.init",
        "goal.pause",
        "goal.resume",
        "goal.clear",
        "goal.edit",
        "goal.add_budget",
        "artifacts.list",
    ];
    METHODS
}

impl ClientOp {
    /// Parse a WS request into a typed `ClientOp`.
    ///
    /// Accepts `method` + `params` from the wire format `{ "method": "...", "params": {...} }`.
    pub fn parse_request(
        method: &str,
        params: serde_json::Value,
    ) -> Result<Self, ClientOpParseError> {
        match method {
            "ping" => Ok(Self::Ping),
            "chat" => {
                let chat_params: ChatParams = serde_json::from_value(params).map_err(|e| {
                    ClientOpParseError::invalid_params(format!("invalid chat params: {e}"))
                })?;
                Ok(Self::Chat {
                    params: chat_params,
                })
            }
            "submit" => Err(ClientOpParseError::unknown_method(
                "the 'submit' method has been removed; use 'chat' instead",
            )),
            "cancel" => Ok(Self::ChatCancel {
                request_id: extract_string(&params, "requestId")
                    .or_else(|_| extract_string(&params, "request_id"))
                    .ok(),
                session_id: extract_string(&params, "sessionId")
                    .or_else(|_| extract_string(&params, "session_id"))
                    .ok(),
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
                        .ok_or_else(|| ClientOpParseError::invalid_params("missing 'mode'"))?,
                )
                .map_err(|e| ClientOpParseError::invalid_params(e.to_string()))?,
            }),
            "sessions.list" => {
                let list_params: SessionsListParams = serde_json::from_value(params)
                    .map_err(|e| ClientOpParseError::invalid_params(e.to_string()))?;
                Ok(Self::SessionsList {
                    params: list_params,
                })
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
                work_dir: params.get("workDir").and_then(|v| {
                    if v.is_null() {
                        None
                    } else {
                        v.as_str().map(String::from)
                    }
                }),
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
                    .ok_or_else(|| ClientOpParseError::invalid_params("missing 'value'"))?,
            }),
            "mcp.status" => Ok(Self::McpStatus),
            "mcp.reload" => Ok(Self::McpReload),
            "mcp.add" => {
                let mcp_params: McpAddParams = serde_json::from_value(params).map_err(|e| {
                    ClientOpParseError::invalid_params(format!("invalid mcp.add params: {e}"))
                })?;
                Ok(Self::McpAdd { params: mcp_params })
            }
            "mcp.remove" => Ok(Self::McpRemove {
                id: extract_string(&params, "id")?,
            }),
            "mcp.detail" => Ok(Self::McpDetail {
                id: extract_string(&params, "id")?,
            }),
            "sub_agents.list" => Ok(Self::SubAgentsList),
            "sub_agents.runs" | "subagents.runs" => {
                let session_id = extract_string(&params, "sessionId")
                    .or_else(|_| extract_string(&params, "session_id"))
                    .ok();
                Ok(Self::SubAgentsRuns { session_id })
            }
            "sub_agents.concurrency" => Ok(Self::SubAgentsConcurrency),
            "subagents.cancel" | "sub_agents.cancel" => {
                let run_id = extract_string(&params, "runId")
                    .or_else(|_| extract_string(&params, "run_id"))?;
                Ok(Self::SubAgentCancel { run_id })
            }
            "agents" | "agents.list" => Ok(Self::AgentsList),
            "agents.get" => Ok(Self::AgentsGet {
                agent_id: AgentId::new(
                    extract_string(&params, "agentId")
                        .or_else(|_| extract_string(&params, "agent_id"))?,
                ),
            }),
            "agents.create" => Ok(Self::AgentsCreate { params }),
            "agents.update" => Ok(Self::AgentsUpdate {
                agent_id: AgentId::new(
                    extract_string(&params, "agentId")
                        .or_else(|_| extract_string(&params, "agent_id"))?,
                ),
                params,
            }),
            "agents.delete" => Ok(Self::AgentsDelete {
                agent_id: AgentId::new(
                    extract_string(&params, "agentId")
                        .or_else(|_| extract_string(&params, "agent_id"))?,
                ),
            }),
            "tools.list" => {
                let list_params: ToolsListParams = serde_json::from_value(params)
                    .map_err(|e| ClientOpParseError::invalid_params(e.to_string()))?;
                Ok(Self::ToolsList {
                    params: list_params,
                })
            }
            "tools.update" => {
                let update_params: ToolsUpdateParams =
                    serde_json::from_value(params).unwrap_or_default();
                Ok(Self::ToolsUpdate {
                    params: update_params,
                })
            }
            "skills.list" => {
                let list_params: SkillsListParams = serde_json::from_value(params)
                    .map_err(|e| ClientOpParseError::invalid_params(e.to_string()))?;
                Ok(Self::SkillsList {
                    params: list_params,
                })
            }
            "skills.read" => {
                let read_params: SkillsReadParams =
                    serde_json::from_value(params).map_err(|e| {
                        ClientOpParseError::invalid_params(format!("invalid params: {e}"))
                    })?;
                if read_params.skill_id.is_empty() {
                    return Err(ClientOpParseError::invalid_params(
                        "invalid params: skillId is required",
                    ));
                }
                Ok(Self::SkillsRead {
                    params: read_params,
                })
            }
            "skills.update" => {
                let update_params: SkillsUpdateParams =
                    serde_json::from_value(params).map_err(|e| {
                        ClientOpParseError::invalid_params(format!("invalid params: {e}"))
                    })?;
                if update_params.skill_id.is_empty() {
                    return Err(ClientOpParseError::invalid_params(
                        "invalid params: skillId is required",
                    ));
                }
                Ok(Self::SkillsUpdate {
                    params: update_params,
                })
            }
            "skills.delete" => {
                let delete_params: SkillsDeleteParams =
                    serde_json::from_value(params).map_err(|e| {
                        ClientOpParseError::invalid_params(format!("invalid params: {e}"))
                    })?;
                if delete_params.skill_id.is_empty() {
                    return Err(ClientOpParseError::invalid_params(
                        "invalid params: skillId is required",
                    ));
                }
                Ok(Self::SkillsDelete {
                    params: delete_params,
                })
            }
            "skills.refresh" => Ok(Self::SkillsRefresh),
            "evolution.list" => Ok(Self::EvolutionList),
            "evolution.promote" => {
                let skill_id = params
                    .get("skill_id")
                    .or_else(|| params.get("skillId"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| {
                        ClientOpParseError::invalid_params("invalid params: skillId is required")
                    })?;
                Ok(Self::EvolutionPromote { skill_id })
            }
            "marketplace.browse" | "marketplace.search" => Ok(Self::MarketplaceBrowse {
                query: params
                    .get("query")
                    .or_else(|| params.get("q"))
                    .and_then(|v| v.as_str())
                    .map(String::from),
                limit: params
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize),
            }),
            "marketplace.install" => {
                let skill_id = extract_string(&params, "skillId")
                    .or_else(|_| extract_string(&params, "skill_id"))?;
                Ok(Self::MarketplaceInstall {
                    skill_id,
                    version: params
                        .get("version")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                })
            }
            "marketplace.uninstall" => {
                let skill_id = extract_string(&params, "skillId")
                    .or_else(|_| extract_string(&params, "skill_id"))?;
                Ok(Self::MarketplaceUninstall { skill_id })
            }
            "execution.set_mode" => Ok(Self::ExecutionSetMode {
                session_id: extract_session_id(&params)?,
                mode: serde_json::from_value(
                    params
                        .get("mode")
                        .cloned()
                        .ok_or_else(|| ClientOpParseError::invalid_params("missing 'mode'"))?,
                )
                .map_err(|e| ClientOpParseError::invalid_params(e.to_string()))?,
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
                .map_err(|e| ClientOpParseError::invalid_params(e.to_string()))?,
            }),
            "execution.reject_plan" => Ok(Self::ExecutionRejectPlan {
                session_id: extract_session_id(&params)?,
                feedback: params
                    .get("feedback")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            }),
            "execution.get_plan_meta" => Ok(Self::ExecutionGetPlanMeta {
                session_id: extract_session_id(&params)?,
            }),
            "subscribe" => {
                let events_val = params
                    .get("events")
                    .ok_or_else(|| ClientOpParseError::invalid_params("missing 'events'"))?;
                let events: Vec<String> =
                    serde_json::from_value(events_val.clone()).map_err(|e| {
                        ClientOpParseError::invalid_params(format!("invalid 'events': {e}"))
                    })?;
                Ok(Self::Subscribe { events })
            }
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
                    .ok_or_else(|| {
                        ClientOpParseError::invalid_params("missing or invalid 'messages'")
                    })?;
                Ok(Self::ChatSteer {
                    session_id,
                    messages,
                })
            }
            "subagent.steer" | "steering_message" => {
                let run_id = extract_string(&params, "runId")
                    .or_else(|_| extract_string(&params, "run_id"))?;
                let message = extract_string(&params, "message")?;
                let priority = params
                    .get("priority")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                Ok(Self::SubAgentSteer {
                    run_id,
                    message,
                    priority,
                })
            }
            "resolve_approval" | "approval.resolve" => {
                let approval_id = params
                    .get("approvalId")
                    .or_else(|| params.get("approval_id"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ClientOpParseError::invalid_params("approvalId required"))?
                    .to_string();
                let decision: crate::approval::ApprovalDecision = serde_json::from_value(
                    params
                        .get("decision")
                        .cloned()
                        .ok_or_else(|| ClientOpParseError::invalid_params("decision required"))?,
                )
                .map_err(|e| {
                    ClientOpParseError::invalid_params(format!("invalid decision: {e}"))
                })?;
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
                limit: params.get("limit").and_then(|v| v.as_i64()),
            }),
            "cost.summary" => Ok(Self::CostSummary),
            "cost.daily" => Ok(Self::CostDaily {
                start: params
                    .get("start")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                end: params.get("end").and_then(|v| v.as_str()).map(String::from),
            }),
            "cost.tools" => Ok(Self::CostTools {
                start: params
                    .get("start")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                end: params.get("end").and_then(|v| v.as_str()).map(String::from),
            }),
            "cost.sessions" => Ok(Self::CostSessions {
                limit: params.get("limit").and_then(|v| v.as_i64()),
            }),
            "search.query" => {
                let search_params: crate::search::SearchQueryRequest =
                    serde_json::from_value(params).map_err(|e| {
                        ClientOpParseError::invalid_params(format!(
                            "invalid search.query params: {e}"
                        ))
                    })?;
                Ok(Self::SearchQuery {
                    params: search_params,
                })
            }
            "search.index_status" => Ok(Self::SearchIndexStatus),
            "automations.list" => Ok(Self::AutomationsList),
            "automations.create" => Ok(Self::AutomationsCreate { params }),
            "automations.update" => Ok(Self::AutomationsUpdate {
                job_id: extract_string(&params, "jobId").or_else(|_| {
                    extract_string(&params, "job_id").or_else(|_| extract_string(&params, "id"))
                })?,
                params,
            }),
            "automations.delete" => Ok(Self::AutomationsDelete {
                job_id: extract_string(&params, "jobId").or_else(|_| {
                    extract_string(&params, "job_id").or_else(|_| extract_string(&params, "id"))
                })?,
            }),
            "automations.runs" => Ok(Self::AutomationsRuns {
                job_id: extract_string(&params, "jobId")
                    .or_else(|_| extract_string(&params, "job_id"))?,
                limit: params.get("limit").and_then(|v| v.as_i64()),
            }),
            "automations.run_now" => Ok(Self::AutomationsRunNow {
                job_id: extract_string(&params, "jobId").or_else(|_| {
                    extract_string(&params, "job_id").or_else(|_| extract_string(&params, "id"))
                })?,
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
                    .ok_or_else(|| ClientOpParseError::invalid_params("missing 'config'"))?,
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
            "projects.list" => Ok(Self::ProjectsList {
                include_archived: params
                    .get("includeArchived")
                    .or_else(|| params.get("include_archived"))
                    .and_then(|v| v.as_bool()),
            }),
            "projects.create" => Ok(Self::ProjectsCreate {
                root_path: extract_string(&params, "rootPath")
                    .or_else(|_| extract_string(&params, "root_path"))?,
                name: params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                color: params
                    .get("color")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            }),
            "projects.update" => Ok(Self::ProjectsUpdate {
                id: extract_string(&params, "id")?,
                name: params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                color: params
                    .get("color")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                pinned: params.get("pinned").and_then(|v| v.as_bool()),
                archived: params.get("archived").and_then(|v| v.as_bool()),
            }),
            "projects.delete" => Ok(Self::ProjectsDelete {
                id: extract_string(&params, "id")?,
            }),
            "projects.detect" => Ok(Self::ProjectsDetect {
                path: extract_string(&params, "path")?,
            }),
            "permissions.get_presets" => Ok(Self::PermissionsGetPresets),
            "permissions.get_session" => Ok(Self::PermissionsGetSession {
                session_id: extract_string(&params, "sessionId")
                    .or_else(|_| extract_string(&params, "session_id"))?,
            }),
            "permissions.set_session" => Ok(Self::PermissionsSetSession {
                session_id: extract_string(&params, "sessionId")
                    .or_else(|_| extract_string(&params, "session_id"))?,
                preset_id: extract_string(&params, "presetId")
                    .or_else(|_| extract_string(&params, "preset_id"))?,
            }),
            "plugins.list" => Ok(Self::PluginsList),
            "plugins.enable" => Ok(Self::PluginsEnable {
                id: extract_string(&params, "id")?,
            }),
            "plugins.disable" => Ok(Self::PluginsDisable {
                id: extract_string(&params, "id")?,
            }),
            "plugins.restart" => Ok(Self::PluginsRestart {
                id: extract_string(&params, "id")?,
            }),
            "plugins.tools" => Ok(Self::PluginsTools {
                id: extract_string(&params, "id")?,
            }),
            "plugins.approve" => Ok(Self::PluginsApprove {
                id: extract_string(&params, "id")?,
            }),
            "plugins.reject" => Ok(Self::PluginsReject {
                id: extract_string(&params, "id")?,
            }),
            "plugins.oauth_login" => Ok(Self::PluginsOauthLogin {
                id: extract_string(&params, "id")?,
            }),
            "plugins.resources" => Ok(Self::PluginsResources {
                server_name: extract_string(&params, "server_name")?,
            }),
            "plugins.prompts" => Ok(Self::PluginsPrompts),
            "plugins.get_prompt" => Ok(Self::PluginsGetPrompt {
                server_name: extract_string(&params, "server_name")?,
                prompt_name: extract_string(&params, "prompt_name")?,
                arguments: params
                    .get("arguments")
                    .and_then(|v| serde_json::from_value(v.clone()).ok()),
            }),
            "plugins.elicitation_reply" => Ok(Self::PluginsElicitationReply {
                elicitation_id: extract_string(&params, "elicitation_id")?,
                action: extract_string(&params, "action")?,
                content: params.get("content").cloned(),
            }),
            "workspace.init" => Ok(Self::WorkspaceInit {
                work_dir: params
                    .get("workDir")
                    .or_else(|| params.get("work_dir"))
                    .and_then(|v| v.as_str())
                    .map(String::from),
            }),
            "git.status" => Ok(Self::GitStatus {
                project_id: extract_string(&params, "projectId")
                    .or_else(|_| extract_string(&params, "project_id"))?,
            }),
            "git.diff" => Ok(Self::GitDiff {
                project_id: extract_string(&params, "projectId")
                    .or_else(|_| extract_string(&params, "project_id"))?,
                path: extract_string(&params, "path")?,
                staged: params
                    .get("staged")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            }),
            "git.branches" => Ok(Self::GitBranches {
                project_id: extract_string(&params, "projectId")
                    .or_else(|_| extract_string(&params, "project_id"))?,
            }),
            "git.log" => Ok(Self::GitLog {
                project_id: extract_string(&params, "projectId")
                    .or_else(|_| extract_string(&params, "project_id"))?,
                limit: params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as u32,
            }),
            "git.stage" => Ok(Self::GitStage {
                project_id: extract_string(&params, "projectId")
                    .or_else(|_| extract_string(&params, "project_id"))?,
                files: params
                    .get("files")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default(),
            }),
            "git.unstage" => Ok(Self::GitUnstage {
                project_id: extract_string(&params, "projectId")
                    .or_else(|_| extract_string(&params, "project_id"))?,
                files: params
                    .get("files")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default(),
            }),
            "git.commit" => Ok(Self::GitCommit {
                project_id: extract_string(&params, "projectId")
                    .or_else(|_| extract_string(&params, "project_id"))?,
                message: extract_string(&params, "message")?,
            }),
            "git.revert" => Ok(Self::GitRevert {
                project_id: extract_string(&params, "projectId")
                    .or_else(|_| extract_string(&params, "project_id"))?,
                files: params
                    .get("files")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default(),
            }),
            "git.init" => Ok(Self::GitInit {
                project_id: extract_string(&params, "projectId")
                    .or_else(|_| extract_string(&params, "project_id"))?,
            }),
            "goal.pause" => Ok(Self::GoalPause {
                session_id: extract_string(&params, "sessionId")
                    .or_else(|_| extract_string(&params, "session_id"))?,
            }),
            "goal.resume" => Ok(Self::GoalResume {
                session_id: extract_string(&params, "sessionId")
                    .or_else(|_| extract_string(&params, "session_id"))?,
            }),
            "goal.clear" => Ok(Self::GoalClear {
                session_id: extract_string(&params, "sessionId")
                    .or_else(|_| extract_string(&params, "session_id"))?,
            }),
            "goal.edit" => Ok(Self::GoalEdit {
                session_id: extract_string(&params, "sessionId")
                    .or_else(|_| extract_string(&params, "session_id"))?,
                description: extract_string(&params, "description")?,
            }),
            "goal.add_budget" => Ok(Self::GoalAddBudget {
                session_id: extract_string(&params, "sessionId")
                    .or_else(|_| extract_string(&params, "session_id"))?,
                amount: params
                    .get("amount")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| {
                        ClientOpParseError::invalid_params("missing or invalid 'amount'")
                    })?,
            }),
            "artifacts.list" => Ok(Self::ArtifactsList {
                session_id: extract_string(&params, "sessionId")
                    .or_else(|_| extract_string(&params, "session_id"))?,
            }),
            other => Err(ClientOpParseError::unknown_method(format!(
                "unknown method: {other}"
            ))),
        }
    }
}

fn extract_string(params: &serde_json::Value, key: &str) -> Result<String, ClientOpParseError> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| ClientOpParseError::invalid_params(format!("missing '{key}'")))
}

fn extract_session_id(params: &serde_json::Value) -> Result<SessionId, ClientOpParseError> {
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
        let op = ClientOp::parse_request("sessions.get", json!({"sessionId": "s1"})).unwrap();
        if let ClientOp::SessionsGet { session_id } = op {
            assert_eq!(session_id, "s1");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn parse_chat_answer() {
        let op =
            ClientOp::parse_request("answer", json!({"requestId": "r1", "answer": "yes"})).unwrap();
        if let ClientOp::ChatAnswer {
            request_id, answer, ..
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
        let op = ClientOp::parse_request("config.set", json!({"key": "a.b", "value": 42})).unwrap();
        if let ClientOp::ConfigSet { key, value } = op {
            assert_eq!(key, "a.b");
            assert_eq!(value, json!(42));
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn parse_subscribe_missing_events() {
        let result = ClientOp::parse_request("subscribe", json!({}));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code, -32602);
        assert!(err.message.contains("events"));
    }

    #[test]
    fn parse_subscribe() {
        let op =
            ClientOp::parse_request("subscribe", json!({"events": ["chat", "tools"]})).unwrap();
        if let ClientOp::Subscribe { events } = op {
            assert_eq!(events, vec!["chat", "tools"]);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn parse_mcp_add() {
        let op =
            ClientOp::parse_request("mcp.add", json!({"id": "test", "command": "echo"})).unwrap();
        assert!(matches!(op, ClientOp::McpAdd { .. }));
    }

    #[test]
    #[allow(deprecated)]
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
        let _ = ClientOp::parse_request("skills.read", json!({"skillId": "test"})).unwrap();
        let _ = ClientOp::parse_request(
            "skills.update",
            json!({"skillId": "test", "content": "# Test"}),
        )
        .unwrap();
        let _ = ClientOp::parse_request("skills.delete", json!({"skillId": "test"})).unwrap();
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

        let _ = ClientOp::parse_request("execution.get_plan", json!({"sessionId": "s1"})).unwrap();
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

    #[test]
    fn parse_elicitation_reply() {
        let op = ClientOp::parse_request(
            "plugins.elicitation_reply",
            json!({
                "elicitation_id": "elic-123",
                "action": "accept",
                "content": { "name": "Alice", "remember": true }
            }),
        )
        .unwrap();
        if let ClientOp::PluginsElicitationReply {
            elicitation_id,
            action,
            content,
        } = op
        {
            assert_eq!(elicitation_id, "elic-123");
            assert_eq!(action, "accept");
            assert_eq!(content.unwrap()["name"], "Alice");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn parse_elicitation_reply_decline() {
        let op = ClientOp::parse_request(
            "plugins.elicitation_reply",
            json!({
                "elicitation_id": "elic-456",
                "action": "decline"
            }),
        )
        .unwrap();
        if let ClientOp::PluginsElicitationReply {
            elicitation_id,
            action,
            content,
        } = op
        {
            assert_eq!(elicitation_id, "elic-456");
            assert_eq!(action, "decline");
            assert!(content.is_none());
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn parse_artifacts_list() {
        let op =
            ClientOp::parse_request("artifacts.list", json!({"sessionId": "sess-abc"})).unwrap();
        assert!(matches!(
            op,
            ClientOp::ArtifactsList { session_id } if session_id == "sess-abc"
        ));
    }
}
