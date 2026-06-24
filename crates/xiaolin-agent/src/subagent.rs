use std::collections::HashMap;
use std::fmt::Display;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::mpsc;

use xiaolin_core::agent_config::SubAgentPolicy;
use xiaolin_core::tool::{
    Tool, ToolErrorType, ToolExposure, ToolKind, ToolParameterSchema, ToolRegistry, ToolResult,
    no_retry_recovery_hint,
};
use xiaolin_core::types::{SubAgentStatus, SubAgentType};
use xiaolin_protocol::AgentEvent;

use xiaolin_core::types::ChatMessage;
use xiaolin_protocol::Role;

use crate::subagent_manager::SubAgentManager;

fn subagent_invalid_args(e: impl Display) -> ToolResult {
    ToolResult::err_with_recovery(
        ToolErrorType::InvalidToolParams,
        format!("invalid arguments: {e}"),
        "Fix the tool arguments JSON schema (required fields, types) before retrying.",
    )
}

fn subagent_denied(message: impl Into<String>, hint: impl Into<String>) -> ToolResult {
    ToolResult::err_with_recovery(ToolErrorType::ExecutionDenied, message, hint)
}

fn subagent_not_found(message: impl Into<String>, hint: impl Into<String>) -> ToolResult {
    ToolResult::err_with_recovery(ToolErrorType::InvalidToolParams, message, hint)
}

fn subagent_execution_failed(message: impl Into<String>, hint: impl Into<String>) -> ToolResult {
    ToolResult::err_with_recovery(ToolErrorType::ExecutionFailed, message, hint)
}

/// Whether a spawn/wait error is caused by policy or concurrency — retrying won't help.
fn is_subagent_no_retry_spawn_error(detail: &str) -> bool {
    detail.contains("sub-agent delegation is disabled")
        || detail.contains("depth limit reached")
        || detail.contains("slot acquisition failed")
        || detail.contains("concurrency")
        || detail.contains("not allowed")
}

fn subagent_spawn_failed(e: impl Display, operation: &str) -> ToolResult {
    let detail = e.to_string();
    tracing::warn!(error = %detail, operation, "sub-agent spawn failed");
    let message = match operation {
        "background" => "failed to spawn sub-agent",
        "sync" => "sub-agent failed",
        "resume" => "resumed sub-agent failed",
        _ => "sub-agent operation failed",
    };
    if is_subagent_no_retry_spawn_error(&detail) {
        subagent_execution_failed(
            message,
            no_retry_recovery_hint(
                "Check concurrency limits and sub-agent configuration via subagent_list; do not retry spawn in a tight loop.",
            ),
        )
    } else if operation == "sync" || operation == "resume" {
        subagent_execution_failed(
            message,
            "Review the task description and context; use subagent_get for partial output or spawn a narrower follow-up task.",
        )
    } else {
        subagent_execution_failed(
            message,
            no_retry_recovery_hint(
                "Check concurrency limits and sub-agent configuration via subagent_list; do not retry spawn in a tight loop.",
            ),
        )
    }
}

tokio::task_local! {
    /// Session ID available to SubAgentTool during execution.
    /// Set by session_bridge before running the agent loop.
    pub static SUBAGENT_SESSION_ID: String;
}

/// Scope a future with the current session ID for SubAgentTool event routing.
pub async fn with_subagent_session_id<F, T>(session_id: String, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    SUBAGENT_SESSION_ID.scope(session_id, fut).await
}

/// Filter parent conversation messages for context inheritance ("Fork Agent").
///
/// Rules:
/// - Remove system messages (child has its own system prompt)
/// - Remove messages with incomplete tool_calls (no matching tool result)
/// - Keep at most `max_messages` most recent messages
/// - Preserve user/assistant/tool role ordering
pub fn filter_parent_messages(messages: &[ChatMessage], max_messages: usize) -> Vec<ChatMessage> {
    let mut filtered: Vec<ChatMessage> = Vec::new();

    for msg in messages {
        match msg.role {
            Role::System => continue,
            Role::Tool => {
                filtered.push(msg.clone());
            }
            Role::Assistant => {
                if let Some(ref tool_calls) = msg.tool_calls {
                    if tool_calls.is_empty() {
                        filtered.push(msg.clone());
                    } else {
                        // Only include if there are matching tool results after this message
                        filtered.push(msg.clone());
                    }
                } else {
                    filtered.push(msg.clone());
                }
            }
            Role::User => {
                filtered.push(msg.clone());
            }
        }
    }

    // Remove trailing assistant messages with tool_calls that have no tool results
    // (incomplete exchanges at the end of the conversation)
    while let Some(last) = filtered.last() {
        if last.role == Role::Assistant && last.tool_calls.as_ref().is_some_and(|tc| !tc.is_empty()) {
            // Check if the next messages after this (which would be tool results) exist
            // Since this is the last message, there are no results — remove it
            filtered.pop();
        } else {
            break;
        }
    }

    // Take only the most recent `max_messages`
    if filtered.len() > max_messages {
        filtered = filtered.split_off(filtered.len() - max_messages);
    }

    filtered
}

/// A tool that spawns a child agent to handle a delegated task.
///
/// Backed by [`SubAgentManager`] for lifecycle management, concurrency control,
/// and streaming. Each child agent gets a type-appropriate tool registry.
pub struct SubAgentTool {
    manager: Arc<SubAgentManager>,
    parent_tool_registry: Arc<ToolRegistry>,
    policy: SubAgentPolicy,
    current_depth: u32,
    parent_tx: Option<mpsc::Sender<AgentEvent>>,
    parent_session_id: String,
    session_store: Option<Arc<xiaolin_session::SessionStore>>,
    coordinator_mode: bool,
    /// When set, worker completions push a notification into this run's MessageQueue.
    coordinator_run_id: Option<String>,
}

impl SubAgentTool {
    pub fn new(
        manager: Arc<SubAgentManager>,
        parent_tool_registry: Arc<ToolRegistry>,
        policy: SubAgentPolicy,
    ) -> Self {
        Self {
            manager,
            parent_tool_registry,
            policy,
            current_depth: 0,
            parent_tx: None,
            parent_session_id: String::new(),
            session_store: None,
            coordinator_mode: false,
            coordinator_run_id: None,
        }
    }

    pub fn with_depth(mut self, current: u32) -> Self {
        self.current_depth = current;
        self
    }

    pub fn with_parent_tx(mut self, tx: mpsc::Sender<AgentEvent>) -> Self {
        self.parent_tx = Some(tx);
        self
    }

    pub fn with_parent_session(mut self, session_id: String) -> Self {
        self.parent_session_id = session_id;
        self
    }

    pub fn with_session_store(mut self, store: Arc<xiaolin_session::SessionStore>) -> Self {
        self.session_store = Some(store);
        self
    }

    pub fn with_coordinator_mode(mut self, enabled: bool) -> Self {
        self.coordinator_mode = enabled;
        self
    }

    pub fn with_coordinator_run_id(mut self, run_id: String) -> Self {
        self.coordinator_run_id = Some(run_id);
        self
    }
}

#[derive(Deserialize)]
struct SpawnParams {
    task: String,
    /// Sub-agent type ID (maps to a SubAgentDef). Legacy `agent_id` is accepted
    /// but treated as an alias for `type` in the new model.
    #[serde(default, alias = "agent_id")]
    r#type: Option<String>,
    /// Legacy field — still accepted for backward compatibility.
    #[serde(default)]
    subagent_type: Option<String>,
    #[serde(default)]
    context: Option<String>,
    /// Override the def's background setting for this invocation.
    #[serde(default)]
    background: Option<bool>,
    /// When true, inherit filtered parent conversation context into the child.
    #[serde(default)]
    inherit_context: bool,
    /// Override the default timeout (seconds) for this specific spawn (60..=1800).
    #[serde(default)]
    timeout_seconds: Option<u64>,
}

fn parse_subagent_type(s: Option<&str>) -> SubAgentType {
    match s {
        Some("explore") => SubAgentType::Explore,
        Some("shell") => SubAgentType::Shell,
        Some("browser") => SubAgentType::Browser,
        Some("general") | None => SubAgentType::General,
        Some(other) => SubAgentType::Custom(other.to_string()),
    }
}

/// Build a child tool registry filtered by sub-agent type.
///
/// - `General`: inherits all parent tools except `spawn_subagent` (added back if depth allows)
/// - `Explore`: read-only tools only
/// - `Shell`: shell + file tools
/// - `Browser`: browser + web tools
/// - `Custom`: same as General (custom filtering is done via agent config `tools_allow`/`tools_deny`)
pub fn build_child_registry(
    parent_registry: &ToolRegistry,
    subagent_type: &SubAgentType,
) -> ToolRegistry {
    let child = ToolRegistry::new();

    let allowed: Box<dyn Fn(&str) -> bool> = match subagent_type {
        SubAgentType::Explore => Box::new(|name: &str| {
            matches!(
                name,
                "read_file"
                    | "file_read"
                    | "search_in_files"
                    | "file_search"
                    | "list_directory"
                    | "workspace_symbols"
                    | "go_to_definition"
                    | "find_references"
                    | "web_search"
                    | "web_fetch"
                    | "http_fetch"
                    | "memory_search"
                    | "get_current_time"
                    | "calculator"
                    | "list_skills"
                    | "read_skill"
            ) || name.starts_with("mcp__")
        }),
        SubAgentType::Shell => Box::new(|name: &str| {
            matches!(
                name,
                "shell_exec"
                    | "shell"
                    | "read_file"
                    | "file_read"
                    | "write_file"
                    | "file_write"
                    | "edit_file"
                    | "list_directory"
                    | "search_in_files"
                    | "file_search"
                    | "multi_edit"
                    | "get_current_time"
            )
        }),
        SubAgentType::Browser => Box::new(|name: &str| {
            name.starts_with("browser")
                || matches!(
                    name,
                    "web_fetch" | "http_fetch" | "web_search" | "get_current_time"
                )
        }),
        SubAgentType::General | SubAgentType::Custom(_) => {
            Box::new(|name: &str| name != "spawn_subagent")
        }
    };

    for def in parent_registry.definitions().iter() {
        let name = &def.function.name;
        if allowed(name) {
            if let Some(tool) = parent_registry.get(name) {
                child.register(tool.clone());
            }
        }
    }

    child
}

#[async_trait]
impl Tool for SubAgentTool {
    fn name(&self) -> &str {
        "spawn_subagent"
    }

    fn description(&self) -> &str {
        "Spawn a sub-agent to delegate work. Returns run_id (background) or result (sync)."
    }

    fn prompt(&self) -> String {
        "Spawn a specialized sub-agent to handle delegated work in an isolated context.\n\n\
## When to Use\n\
- Multi-step exploration or implementation that benefits from a fresh context window\n\
- Parallel work: spawn multiple sub-agents with non-overlapping scopes\n\
- Specialized types (explore, code, shell, research) with tailored tool sets\n\
- Mid-conversation delegation when the sub-agent should NOT see full chat history\n\n\
## Background Delegation (This Session)\n\
- Use **spawn_subagent** with `background=true` for async in-session delegation; returns `run_id` immediately\n\
- Poll with `subagent_get` / `wait_agent`, steer with `send_message`, resume with `resume_subagent`\n\
- `task_create` (separate task_id lifecycle) is **not exposed** to the main agent — do not call it\n\n\
## Sync vs Background\n\
- **Sync** (`background=false`, default for many types): blocks until done, returns `result` directly\n\
- **Background** (`background=true`): returns `run_id` immediately; poll with `subagent_get`, \
block with `wait_agent`, or steer with `send_message`\n\
Coordinator mode always forces background.\n\n\
## Type Selection\n\
- `type`: pick from available sub-agent types (explore=read-only, code=edit, shell=commands, etc.)\n\
- `inherit_context=true`: pass filtered parent messages (use sparingly — increases token cost)\n\
- `context`: extra facts the sub-agent cannot discover on its own\n\
- `timeout_seconds`: 60–1800 override for long runs\n\n\
## Depth & Policy\n\
- Each spawn increases depth; hitting `max_depth` denies further spawns\n\
- Child registries exclude `spawn_subagent` unless depth allows nesting\n\
- Check `subagent_list` when spawn fails due to limits or concurrency\n\n\
## Tool Cooperation\n\
1. `spawn_subagent` → get `run_id`\n\
2. `subagent_get` / `wait_agent` → collect results\n\
3. `send_message` → steer running background agents\n\
4. `subagent_list` → discover all runs and valid run_ids\n\n\
## Anti-Patterns\n\
- Do NOT spawn for 1–3 trivial tool calls — do them directly\n\
- Do NOT spawn overlapping file editors without coordination (merge conflicts)\n\
- Do NOT retry spawn in a tight loop on depth/limit errors — use subagent_list or work locally\n\
- Do NOT use vague tasks — include paths, goals, and success criteria like briefing a new colleague"
            .to_string()
    }

    fn supports_parallel(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "task".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Clear, self-contained description of the task. Include all necessary context — the sub-agent cannot see your conversation."
            }),
        );

        let def_descs = self.manager.subagent_def_descriptions();
        let type_list: Vec<String> = def_descs
            .iter()
            .map(|(id, desc)| {
                if let Some(d) = desc {
                    format!("{id} ({d})")
                } else {
                    id.clone()
                }
            })
            .collect();
        props.insert(
            "type".to_string(),
            serde_json::json!({
                "type": "string",
                "description": format!(
                    "Sub-agent type to spawn. Available: {}. \
                     Each type has a specific tool set and system prompt.",
                    type_list.join(", ")
                ),
                "default": "code"
            }),
        );
        props.insert(
            "context".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional context or data to pass to the sub-agent that it cannot discover on its own"
            }),
        );
        props.insert(
            "background".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "Run in background (async). Default depends on the sub-agent type definition. When false, blocks until completion and returns the result directly."
            }),
        );
        props.insert(
            "inherit_context".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "When true, the sub-agent inherits a filtered portion of the parent conversation as initial context. This allows it to reference earlier messages without explicit context passing."
            }),
        );
        props.insert(
            "timeout_seconds".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Override default timeout (seconds) for this spawn. Range: 60-1800. Use higher values for complex tasks that generate large files."
            }),
        );

        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["task".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let params: SpawnParams = match serde_json::from_str(arguments) {
            Ok(p) => p,
            Err(e) => return subagent_invalid_args(e),
        };

        if !self.policy.enabled {
            return subagent_denied(
                "sub-agent delegation is disabled for this agent",
                no_retry_recovery_hint(
                    "Complete the work in the current agent instead of spawning sub-agents.",
                ),
            );
        }

        if self.current_depth >= self.policy.max_depth {
            return subagent_denied(
                format!(
                    "sub-agent depth limit reached ({}/{})",
                    self.current_depth, self.policy.max_depth
                ),
                no_retry_recovery_hint(
                    "Break the work into smaller steps at the current depth, or use subagent_get/subagent_list to collect results from existing runs.",
                ),
            );
        }

        let type_id = params
            .r#type
            .as_deref()
            .or(params.subagent_type.as_deref())
            .unwrap_or("code");

        if !self.policy.allowed_types.is_empty() && !self.policy.allowed_types.contains(&type_id.to_string()) {
            return subagent_denied(
                format!(
                    "sub-agent type '{type_id}' not allowed (allowed: {:?})",
                    self.policy.allowed_types
                ),
                no_retry_recovery_hint(
                    "Pick an allowed sub-agent type from the list, or complete the task without spawning a disallowed type.",
                ),
            );
        }

        let def = self.manager.resolve_subagent_def(type_id);
        let subagent_type = parse_subagent_type(Some(type_id));

        let (child_registry, use_background) = if let Some(ref def) = def {
            let registry = SubAgentManager::build_child_registry_from_def(
                &self.parent_tool_registry,
                def,
            );
            let bg = params.background.unwrap_or(def.background);
            (registry, bg)
        } else {
            let registry = build_child_registry(&self.parent_tool_registry, &subagent_type);
            let bg = params.background.unwrap_or(true);
            (registry, bg)
        };

        // Coordinator mode forces all worker spawns to background
        let use_background = if self.coordinator_mode {
            true
        } else {
            use_background
        };

        let is_coordinator = def
            .as_ref()
            .is_some_and(|d| d.mode == xiaolin_core::agent_config::SubAgentMode::Coordinator);

        if self.current_depth + 1 < self.policy.max_depth {
            let child_subagent = SubAgentTool::new(
                self.manager.clone(),
                self.parent_tool_registry.clone(),
                self.policy.clone(),
            )
            .with_depth(self.current_depth + 1)
            .with_coordinator_mode(is_coordinator);
            child_registry.register(Arc::new(child_subagent));
        }

        let child_registry = Arc::new(child_registry);

        let agent_config = match self.manager.resolve_agent("main") {
            Some(mut c) => {
                if let Some(ref def) = def {
                    if let Some(ref prompt) = def.system_prompt {
                        c.system_prompt = Some(prompt.clone());
                    }
                }
                c
            }
            None => {
                let agents = self.manager.available_agents();
                match agents.first() {
                    Some(c) => {
                        let mut c = c.clone();
                        if let Some(ref def) = def {
                            if let Some(ref prompt) = def.system_prompt {
                                c.system_prompt = Some(prompt.clone());
                            }
                        }
                        c
                    }
                    None => {
                        return subagent_denied(
                            "no agent config available",
                            no_retry_recovery_hint(
                                "Ask the operator to configure at least one agent, or continue without spawning a sub-agent.",
                            ),
                        );
                    }
                }
            }
        };

        let mut agent_config = agent_config;
        if let Some(ref d) = def {
            if let Some(ref model_override) = d.model {
                agent_config.model = model_override.clone();
            }
        }

        let concurrency_safe = def.as_ref().map(|d| d.concurrency_safe).unwrap_or(true);
        let permission_mode = def
            .as_ref()
            .map(|d| d.permission_mode)
            .unwrap_or_default();

        tracing::info!(
            parent_depth = self.current_depth,
            def_type = %type_id,
            background = use_background,
            concurrency_safe,
            ?permission_mode,
            task_len = params.task.len(),
            "spawning sub-agent"
        );

        let effective_session_id = SUBAGENT_SESSION_ID
            .try_with(|s| s.clone())
            .unwrap_or_else(|_| self.parent_session_id.clone());

        let parent_tx = match &self.parent_tx {
            Some(tx) => tx.clone(),
            None => {
                if let Some(tx) = self.manager.get_session_tx(&effective_session_id) {
                    tx
                } else {
                    tracing::warn!(
                        session_id = %effective_session_id,
                        "SubAgentTool: no parent_tx and no session tx registered — events will be lost"
                    );
                    let (tx, _rx) = mpsc::channel(16);
                    tx
                }
            }
        };

        let inherited_ctx = Some(crate::SubAgentInheritedContext {
            work_dir: xiaolin_tools_fs::filesystem::current_effective_work_dir()
                .map(|p| p.to_string_lossy().to_string()),
            file_access: xiaolin_tools_fs::filesystem::current_file_access_mode(),
            additional_allowed_paths: xiaolin_tools_fs::filesystem::current_additional_allowed_paths()
                .into_iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect(),
        });

        // Load and filter parent messages when inherit_context is requested
        let initial_messages = if params.inherit_context {
            let max_msgs = def
                .as_ref()
                .map(|d| d.max_context_messages)
                .unwrap_or(20);
            if let Some(ref store) = self.session_store {
                match store.load_chat_messages(&effective_session_id).await {
                    Ok(msgs) => {
                        let filtered = filter_parent_messages(&msgs, max_msgs);
                        if filtered.is_empty() {
                            None
                        } else {
                            tracing::info!(
                                inherited_messages = filtered.len(),
                                max = max_msgs,
                                "fork agent: inheriting parent context"
                            );
                            Some(filtered)
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to load parent messages for context inheritance");
                        None
                    }
                }
            } else {
                tracing::warn!("inherit_context requested but no session_store available");
                None
            }
        } else {
            None
        };

        if use_background {
            let mut effective_policy = self.policy.clone();
            if let Some(t) = params.timeout_seconds.filter(|&t| (60..=1800).contains(&t)) {
                effective_policy.timeout_seconds = t;
            }
            let max_result_chars = def.as_ref().and_then(|d| d.max_result_chars);
            let run_id = match self
                .manager
                .spawn(
                    agent_config,
                    subagent_type.clone(),
                    params.task.clone(),
                    params.context.clone(),
                    effective_session_id,
                    String::new(),
                    self.current_depth,
                    &effective_policy,
                    child_registry,
                    parent_tx,
                    None,
                    concurrency_safe,
                    inherited_ctx,
                    initial_messages,
                    permission_mode,
                    None,
                    max_result_chars,
                )
                .await
            {
                Ok(id) => id,
                Err(e) => return subagent_spawn_failed(e, "background"),
            };

            ToolResult::ok(serde_json::json!({
                "run_id": run_id,
                "type": type_id,
                "status": "running",
                "message": "Sub-agent spawned in background. Use subagent_get with this run_id to check results."
            }).to_string())
        } else {
            let mut effective_policy = self.policy.clone();
            if let Some(t) = params.timeout_seconds.filter(|&t| (60..=1800).contains(&t)) {
                effective_policy.timeout_seconds = t;
            }
            let max_result_chars = def.as_ref().and_then(|d| d.max_result_chars);
            #[allow(deprecated)]
            match self
                .manager
                .spawn_sync(
                    agent_config,
                    subagent_type.clone(),
                    params.task.clone(),
                    params.context.clone(),
                    effective_session_id,
                    String::new(),
                    self.current_depth,
                    &effective_policy,
                    child_registry,
                    parent_tx,
                    None,
                    concurrency_safe,
                    inherited_ctx,
                    initial_messages,
                    permission_mode,
                    None,
                    max_result_chars,
                )
                .await
            {
                Ok((result, run_id)) => {
                    ToolResult::ok(serde_json::json!({
                        "run_id": run_id,
                        "type": type_id,
                        "status": "completed",
                        "result": result,
                    }).to_string())
                }
                Err(e) => subagent_spawn_failed(e, "sync"),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SubAgentGetTool — query a specific run by ID (non-blocking)
// ---------------------------------------------------------------------------

pub struct SubAgentGetTool {
    manager: Arc<SubAgentManager>,
}

impl SubAgentGetTool {
    pub fn new(manager: Arc<SubAgentManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for SubAgentGetTool {
    fn name(&self) -> &str {
        "subagent_get"
    }

    fn description(&self) -> &str {
        "Non-blocking status and result lookup for a sub-agent run by run_id."
    }

    fn prompt(&self) -> String {
        "Check status and result of a previously spawned sub-agent (non-blocking).\n\n\
## When to Use\n\
- After `spawn_subagent` with `background=true` to poll progress\n\
- Before modifying files a sub-agent may still be editing\n\
- To retrieve the final `result` when status is completed/failed/cancelled\n\n\
## Tool Cooperation\n\
- Get `run_id` from `spawn_subagent` or find it via `subagent_list`\n\
- Use `wait_agent` when you want to block until completion instead of polling\n\
- Use `send_message` only while status is **running**\n\
- Use `resume_subagent` for interrupted runs with persisted sidechains\n\n\
## Parameters\n\
- `run_id`: exact id returned by spawn (copy verbatim)\n\n\
## Anti-Patterns\n\
- Do NOT guess run_ids — call `subagent_list` first if unsure\n\
- Do NOT poll subagent_get in a tight loop — use `wait_agent` with a timeout\n\
- Do NOT assume result is ready when status is still running"
            .to_string()
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "run_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The run_id returned by spawn_subagent."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["run_id".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        #[derive(Deserialize)]
        struct Params {
            run_id: String,
        }
        let params: Params = match serde_json::from_str(arguments) {
            Ok(p) => p,
            Err(e) => return subagent_invalid_args(e),
        };

        match self.manager.get_run(&params.run_id) {
            Some(run) => {
                let json = serde_json::json!({
                    "run_id": run.run_id,
                    "agent_id": run.agent_id.to_string(),
                    "subagent_type": run.subagent_type.to_string(),
                    "task": run.task,
                    "status": format!("{:?}", run.status),
                    "result": run.result,
                    "tool_calls_made": run.tool_calls_made,
                    "iterations": run.iterations,
                    "elapsed_ms": run.completed_at.map(|c| c.saturating_sub(run.created_at)),
                });
                ToolResult::ok(json.to_string())
            }
            None => subagent_not_found(
                format!("no sub-agent run found with id '{}'", params.run_id),
                "Call subagent_list to see valid run_ids, then retry subagent_get with a matching id.",
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// SubAgentListTool — list all sub-agent runs for the session
// ---------------------------------------------------------------------------

pub struct SubAgentListTool {
    manager: Arc<SubAgentManager>,
}

impl SubAgentListTool {
    pub fn new(manager: Arc<SubAgentManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for SubAgentListTool {
    fn name(&self) -> &str {
        "subagent_list"
    }

    fn description(&self) -> &str {
        "List all sub-agent runs in the current session with status summaries."
    }

    fn prompt(&self) -> String {
        "List all sub-agent runs in the current session.\n\n\
## When to Use\n\
- Discover valid `run_id` values before `subagent_get`, `wait_agent`, or `send_message`\n\
- Monitor parallel background spawns at a glance\n\
- Debug spawn failures (depth limits, concurrency) by seeing active runs\n\n\
## Tool Cooperation\n\
- After listing, call `subagent_get` for details on one run\n\
- Batch-wait with `wait_agent` using multiple run_ids from this list\n\
- Cross-check before `resume_subagent` that the run exists and was persisted\n\n\
## Parameters\n\
- No parameters — pass `{}`\n\n\
## Anti-Patterns\n\
- Do NOT call repeatedly every turn when nothing was spawned — once per orchestration phase is enough\n\
- Do NOT use list output as a substitute for reading `result` via subagent_get"
            .to_string()
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: HashMap::new(),
            required: vec![],
        }
    }

    async fn execute(&self, _arguments: &str) -> ToolResult {
        let runs = self.manager.list_runs(None);
        let summaries: Vec<serde_json::Value> = runs
            .iter()
            .map(|r| {
                serde_json::json!({
                    "run_id": r.run_id,
                    "agent_id": r.agent_id.to_string(),
                    "subagent_type": r.subagent_type.to_string(),
                    "status": format!("{:?}", r.status),
                    "task": if r.task.len() > 100 { let end = r.task.floor_char_boundary(100); format!("{}…", &r.task[..end]) } else { r.task.clone() },
                    "has_result": r.result.is_some(),
                })
            })
            .collect();
        ToolResult::ok(
            serde_json::json!({
                "total": runs.len(),
                "runs": summaries,
            })
            .to_string(),
        )
    }
}

// ---------------------------------------------------------------------------
// WaitAgentTool — wait for sub-agent(s) to complete
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct WaitParams {
    run_ids: Vec<String>,
    #[serde(default = "default_wait_mode")]
    mode: String,
    timeout_seconds: Option<u64>,
}

fn default_wait_mode() -> String {
    "all".to_string()
}

pub struct WaitAgentTool {
    manager: Arc<SubAgentManager>,
}

impl WaitAgentTool {
    pub fn new(manager: Arc<SubAgentManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for WaitAgentTool {
    fn name(&self) -> &str {
        "wait_agent"
    }

    fn description(&self) -> &str {
        "Block until one or more sub-agent runs finish (mode all/any, optional timeout)."
    }

    fn prompt(&self) -> String {
        "Wait for one or more sub-agent runs to reach a terminal state.\n\n\
## When to Use\n\
- After spawning multiple background agents and you need all (or any) results before continuing\n\
- Fan-in step of parallel explore/implement patterns\n\
- Prefer over tight polling loops of `subagent_get`\n\n\
## Tool Cooperation\n\
- Collect `run_id`s from `spawn_subagent` or `subagent_list`\n\
- After wait completes, read per-run results from the returned `results` map\n\
- On `timed_out: true`, use `subagent_get` on still-pending runs\n\n\
## Parameters\n\
- `run_ids`: non-empty array of run ids\n\
- `mode`: `all` (default) wait for every run; `any` return when first completes\n\
- `timeout_seconds`: max wait (default 300); returns partial results on timeout\n\n\
## Anti-Patterns\n\
- Do NOT pass unknown run_ids — validate via `subagent_list` first\n\
- Do NOT use wait_agent on sync spawns that already returned results\n\
- Do NOT set timeout to 0 expecting instant results for long tasks"
            .to_string()
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "run_ids".to_string(),
            serde_json::json!({
                "type": "array",
                "items": { "type": "string" },
                "description": "List of sub-agent run IDs to wait for."
            }),
        );
        props.insert(
            "mode".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["all", "any"],
                "description": "Wait strategy: 'all' waits for every run to complete; 'any' returns on the first completion. Default: 'all'."
            }),
        );
        props.insert(
            "timeout_seconds".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Maximum seconds to wait. Default: 300."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["run_ids".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        use crate::spawn_controller::SlotEvent;
        use xiaolin_core::types::SubAgentStatus;

        let params: WaitParams = match serde_json::from_str(arguments) {
            Ok(p) => p,
            Err(e) => return subagent_invalid_args(e),
        };

        if params.run_ids.is_empty() {
            return subagent_invalid_args("run_ids must not be empty");
        }

        let wait_all = params.mode == "all";
        let timeout = std::time::Duration::from_secs(params.timeout_seconds.unwrap_or(300));

        for rid in &params.run_ids {
            if self.manager.get_run(rid).is_none() {
                return subagent_not_found(
                    format!("unknown run_id: {rid}"),
                    "Call subagent_list to see valid run_ids before calling wait_agent.",
                );
            }
        }

        let mut results: HashMap<String, serde_json::Value> = HashMap::new();
        let mut pending: std::collections::HashSet<String> =
            params.run_ids.iter().cloned().collect();

        for rid in &params.run_ids {
            if let Some(run) = self.manager.get_run(rid) {
                if run.status.is_terminal() {
                    let entry = match &run.status {
                        SubAgentStatus::Completed => serde_json::json!({
                            "status": "completed",
                            "result": run.result
                        }),
                        SubAgentStatus::Failed(msg) => serde_json::json!({
                            "status": "failed",
                            "error": msg
                        }),
                        SubAgentStatus::Cancelled => serde_json::json!({
                            "status": "cancelled"
                        }),
                        _ => unreachable!(),
                    };
                    results.insert(rid.clone(), entry);
                    pending.remove(rid);
                }
            }
        }

        if !wait_all && !results.is_empty() {
            return ToolResult::ok(
                serde_json::json!({
                    "results": results,
                    "timed_out": false
                })
                .to_string(),
            );
        }

        if pending.is_empty() {
            return ToolResult::ok(
                serde_json::json!({
                    "results": results,
                    "timed_out": false
                })
                .to_string(),
            );
        }

        let controller = self.manager.controller();
        let mut receivers: Vec<tokio::sync::broadcast::Receiver<SlotEvent>> = Vec::new();
        for (_, pool) in controller.snapshot().sessions.iter().map(|s| {
            (
                s.session_id.clone(),
                controller.get_or_create_session_pool(&s.session_id),
            )
        }) {
            receivers.push(pool.subscribe_events());
        }
        if receivers.is_empty() {
            receivers.push(controller.get_or_create_session_pool("__wait__").subscribe_events());
        }

        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return ToolResult::ok(
                    serde_json::json!({
                        "results": results,
                        "timed_out": true
                    })
                    .to_string(),
                );
            }

            tokio::select! {
                _ = tokio::time::sleep(remaining) => {
                    return ToolResult::ok(serde_json::json!({
                        "results": results,
                        "timed_out": true
                    }).to_string());
                }
                _ = async {
                    if receivers.is_empty() {
                        tokio::time::sleep(remaining).await;
                    } else {
                        // Poll ALL receivers concurrently — any event unblocks us
                        let futs: Vec<_> = receivers.iter_mut().map(|rx| Box::pin(rx.recv())).collect();
                        let _ = futures::future::select_all(futs).await;
                    }
                } => {}
            }

            let mut newly_done = Vec::new();
            for rid in &pending {
                if let Some(run) = self.manager.get_run(rid) {
                    if run.status.is_terminal() {
                        let entry = match &run.status {
                            SubAgentStatus::Completed => serde_json::json!({
                                "status": "completed",
                                "result": run.result
                            }),
                            SubAgentStatus::Failed(msg) => serde_json::json!({
                                "status": "failed",
                                "error": msg
                            }),
                            SubAgentStatus::Cancelled => serde_json::json!({
                                "status": "cancelled"
                            }),
                            _ => unreachable!(),
                        };
                        results.insert(rid.clone(), entry);
                        newly_done.push(rid.clone());
                    }
                }
            }

            for rid in &newly_done {
                pending.remove(rid);
            }

            if !wait_all && !newly_done.is_empty() {
                return ToolResult::ok(
                    serde_json::json!({
                        "results": results,
                        "timed_out": false
                    })
                    .to_string(),
                );
            }

            if pending.is_empty() {
                return ToolResult::ok(
                    serde_json::json!({
                        "results": results,
                        "timed_out": false
                    })
                    .to_string(),
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ResumeSubagentTool — resume a previously interrupted sub-agent run
// ---------------------------------------------------------------------------

pub struct ResumeSubagentTool {
    manager: Arc<SubAgentManager>,
    parent_tool_registry: Arc<ToolRegistry>,
    policy: SubAgentPolicy,
    current_depth: u32,
    parent_tx: Option<mpsc::Sender<AgentEvent>>,
    parent_session_id: String,
}

impl ResumeSubagentTool {
    pub fn new(
        manager: Arc<SubAgentManager>,
        parent_tool_registry: Arc<ToolRegistry>,
        policy: SubAgentPolicy,
    ) -> Self {
        Self {
            manager,
            parent_tool_registry,
            policy,
            current_depth: 0,
            parent_tx: None,
            parent_session_id: String::new(),
        }
    }

    pub fn with_depth(mut self, current: u32) -> Self {
        self.current_depth = current;
        self
    }

    pub fn with_parent_tx(mut self, tx: mpsc::Sender<AgentEvent>) -> Self {
        self.parent_tx = Some(tx);
        self
    }

    pub fn with_parent_session(mut self, session_id: String) -> Self {
        self.parent_session_id = session_id;
        self
    }
}

#[async_trait]
impl Tool for ResumeSubagentTool {
    fn name(&self) -> &str {
        "resume_subagent"
    }

    fn description(&self) -> &str {
        "Resume an interrupted sub-agent from its persisted sidechain transcript."
    }

    fn prompt(&self) -> String {
        "Resume a previously interrupted sub-agent by replaying its sidechain transcript.\n\n\
## When to Use\n\
- A background run was interrupted but its sidechain was persisted\n\
- You need to continue with optional new instructions (`message`)\n\
- `subagent_get` shows incomplete work and sidechain exists\n\n\
## Tool Cooperation\n\
- Confirm `run_id` via `subagent_list` before resuming\n\
- If resume fails (corrupt/missing sidechain), `spawn_subagent` fresh with context\n\
- Runs synchronously and returns a new completion result\n\n\
## Parameters\n\
- `run_id`: the persisted run to resume (required)\n\
- `message`: optional new user message appended before continuing\n\n\
## Anti-Patterns\n\
- Do NOT resume runs that already completed — use `subagent_get` for results\n\
- Do NOT retry resume in a loop on metadata errors — spawn anew\n\
- Do NOT resume when a simple new spawn with a clear task is cleaner"
            .to_string()
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "run_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The run_id of the sub-agent run to resume."
            }),
        );
        props.insert(
            "message".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional new user message to append before continuing."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["run_id".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        use crate::sidechain::SidechainReader;

        #[derive(Deserialize)]
        struct Params {
            run_id: String,
            #[serde(default)]
            message: Option<String>,
        }
        let params: Params = match serde_json::from_str(arguments) {
            Ok(p) => p,
            Err(e) => return subagent_invalid_args(e),
        };

        let effective_session_id = SUBAGENT_SESSION_ID
            .try_with(|s| s.clone())
            .unwrap_or_else(|_| self.parent_session_id.clone());

        if !SidechainReader::exists(&effective_session_id, &params.run_id).await {
            return subagent_not_found(
                format!("sidechain not found for run_id: {}", params.run_id),
                "Call subagent_list to confirm the run_id exists and was persisted before resuming.",
            );
        }

        // Load the original run's metadata to determine agent type
        let meta = match SidechainReader::load_meta(&effective_session_id, &params.run_id).await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    run_id = %params.run_id,
                    "failed to read sidechain metadata for resume"
                );
                return subagent_execution_failed(
                    "failed to read sidechain metadata",
                    no_retry_recovery_hint(
                        "The sidechain may be corrupted; use subagent_list and spawn a new sub-agent if resume keeps failing.",
                    ),
                );
            }
        };

        // Resolve the agent config for the resumed run
        let agent_config = match self.manager.resolve_agent(&meta.agent_id) {
            Some(cfg) => cfg,
            None => {
                return subagent_denied(
                    format!("agent '{}' is no longer available", meta.agent_id),
                    no_retry_recovery_hint(
                        "Spawn a new sub-agent with an available agent type instead of resuming this run.",
                    ),
                );
            }
        };

        // Load sidechain messages as initial context
        let mut initial_messages =
            match SidechainReader::load_as_chat_messages(&effective_session_id, &params.run_id)
                .await
            {
                Ok(msgs) => msgs,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        run_id = %params.run_id,
                        "failed to load sidechain transcript for resume"
                    );
                    return subagent_execution_failed(
                        "failed to load sidechain transcript",
                        no_retry_recovery_hint(
                            "If the transcript is missing, spawn a fresh sub-agent with the needed context instead of retrying resume.",
                        ),
                    );
                }
            };

        // Append new user message if provided
        if let Some(msg_text) = &params.message {
            initial_messages.push(xiaolin_core::types::ChatMessage {
                role: xiaolin_protocol::Role::User,
                content: Some(serde_json::Value::String(msg_text.clone())),
                ..Default::default()
            });
        }

        // Build task description for the resumed run
        let task = if let Some(msg) = &params.message {
            msg.clone()
        } else {
            format!("(resumed) {}", meta.task)
        };

        let subagent_type = parse_subagent_type(Some(&meta.agent_id));
        let child_registry = Arc::new(build_child_registry(
            &self.parent_tool_registry,
            &subagent_type,
        ));

        let parent_tx = match &self.parent_tx {
            Some(tx) => tx.clone(),
            None => {
                if let Some(tx) = self.manager.get_session_tx(&effective_session_id) {
                    tx
                } else {
                    let (tx, _rx) = mpsc::channel(16);
                    tx
                }
            }
        };

        #[allow(deprecated)]
        match self
            .manager
            .spawn_sync(
                agent_config,
                subagent_type.clone(),
                task,
                None,
                effective_session_id,
                String::new(),
                self.current_depth,
                &self.policy,
                child_registry,
                parent_tx,
                None,
                true,
                None,
                Some(initial_messages),
                xiaolin_core::agent_config::PermissionMode::AutoApprove,
                None,
                None,
            )
            .await
        {
            Ok((result, run_id)) => ToolResult::ok(
                serde_json::json!({
                    "run_id": run_id,
                    "resumed_from": params.run_id,
                    "status": "completed",
                    "result": result,
                })
                .to_string(),
            ),
            Err(e) => subagent_spawn_failed(e, "resume"),
        }
    }
}

// ---------------------------------------------------------------------------
// SendMessageTool — send a steering message to a running sub-agent
// ---------------------------------------------------------------------------

pub struct SendMessageTool {
    manager: Arc<SubAgentManager>,
}

impl SendMessageTool {
    pub fn new(manager: Arc<SubAgentManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for SendMessageTool {
    fn name(&self) -> &str {
        "send_message"
    }

    fn description(&self) -> &str {
        "Inject a steering message into a running background sub-agent."
    }

    fn prompt(&self) -> String {
        "Send a steering message to a **running** background sub-agent.\n\n\
## When to Use\n\
- Redirect or refine a background agent mid-flight without cancelling\n\
- Provide new constraints discovered after spawn\n\
- Urgent corrections (`priority: high`)\n\n\
## Tool Cooperation\n\
- Requires `spawn_subagent` with `background=true` (sync runs have no message queue)\n\
- Verify status via `subagent_get` — only **running** agents accept messages\n\
- Find `run_id` via `subagent_list` if needed\n\n\
## Parameters\n\
- `run_id`: target sub-agent (required)\n\
- `message`: steering content (required)\n\
- `priority`: `low` | `normal` (default) | `high`\n\n\
## Anti-Patterns\n\
- Do NOT send_message to completed/failed runs — spawn a follow-up instead\n\
- Do NOT spam high-priority messages — dilutes urgency\n\
- Do NOT use send_message when you could have passed context in the initial spawn task"
            .to_string()
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "run_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The run_id of the target sub-agent (from spawn_subagent)."
            }),
        );
        props.insert(
            "message".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The steering message content to inject."
            }),
        );
        props.insert(
            "priority".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["low", "normal", "high"],
                "description": "Message priority. Default: normal."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["run_id".to_string(), "message".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        #[derive(Deserialize)]
        struct Params {
            run_id: String,
            message: String,
            #[serde(default)]
            priority: Option<String>,
        }
        let params: Params = match serde_json::from_str(arguments) {
            Ok(p) => p,
            Err(e) => return subagent_invalid_args(e),
        };

        let priority = match params.priority.as_deref() {
            Some("high") => crate::message_queue::Priority::High,
            Some("low") => crate::message_queue::Priority::Low,
            _ => crate::message_queue::Priority::Normal,
        };

        let run = self.manager.get_run(&params.run_id);
        match run {
            Some(r) if r.status == SubAgentStatus::Running => {}
            Some(_) => {
                return subagent_denied(
                    format!(
                        "sub-agent '{}' is not running (cannot receive messages)",
                        params.run_id
                    ),
                    "Call subagent_get to check status; send_message only works while status is running.",
                );
            }
            None => {
                return subagent_not_found(
                    format!("no sub-agent run found with id '{}'", params.run_id),
                    "Call subagent_list to see valid run_ids.",
                );
            }
        }

        match self.manager.get_run_queue(&params.run_id) {
            Some(queue) => {
                queue.push(priority, "parent_agent", &params.message);
                ToolResult::ok(serde_json::json!({
                    "status": "delivered",
                    "run_id": params.run_id,
                    "queue_size": queue.len(),
                }).to_string())
            }
            None => subagent_execution_failed(
                "sub-agent does not have a message queue",
                no_retry_recovery_hint(
                    "Spawn the sub-agent in background mode to enable send_message, or wait for sync completion via subagent_get.",
                ),
            ),
        }
    }
}

// ── TaskStopTool ─────────────────────────────────────────────────────────

/// Tool used by coordinator agents to signal task completion with a final summary.
#[derive(Default)]
pub struct TaskStopTool;

impl TaskStopTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for TaskStopTool {
    fn name(&self) -> &str {
        "task_stop"
    }

    fn description(&self) -> &str {
        "Coordinator-only: signal orchestration complete with a final summary."
    }

    fn prompt(&self) -> String {
        "Signal that a **coordinator-mode** sub-agent has finished orchestrating workers.\n\n\
## When to Use\n\
- You are a coordinator sub-agent that spawned worker sub-agents and collected outcomes\n\
- All delegated work is done (or acceptably partial) and you must return a final summary\n\
- Calling this ends the coordinator's execution loop\n\n\
## vs background_task_stop\n\
- **task_stop** (this tool): coordinator-only signal that orchestration is complete\n\
- **background_task_stop**: cancels TaskManager background jobs by task_id — a different system; \
not the same as ending coordinator orchestration\n\n\
## Tool Cooperation\n\
- Workers: `spawn_subagent` (background) → `wait_agent` / `subagent_get` → aggregate here\n\
- Parent agent receives the summary as the coordinator's final tool result\n\n\
## Parameters\n\
- `summary`: final orchestration report (required)\n\
- `status`: `success` (default) | `partial` | `failed`\n\n\
## Anti-Patterns\n\
- Do NOT call from non-coordinator agents — tool may be unavailable\n\
- Do NOT call before workers finish unless status is `partial` with explanation\n\
- Do NOT use task_stop for regular single sub-agent work or to cancel background TaskManager jobs — \
only coordinator orchestration; use `background_task_stop` for the latter when available"
            .to_string()
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "summary".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Final summary of the orchestration results to return to the caller."
            }),
        );
        props.insert(
            "status".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["success", "partial", "failed"],
                "description": "Overall status of the coordinated task. Default: success."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["summary".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        #[derive(Deserialize)]
        struct Params {
            summary: String,
            #[serde(default = "default_status")]
            status: String,
        }
        fn default_status() -> String {
            "success".into()
        }

        let params: Params = match serde_json::from_str(arguments) {
            Ok(p) => p,
            Err(e) => return subagent_invalid_args(e),
        };

        ToolResult::ok(
            serde_json::json!({
                "task_stopped": true,
                "status": params.status,
                "summary": params.summary,
            })
            .to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaolin_core::agent_config::{AgentConfig, SubAgentPolicy, builtin_subagent_defs};

    #[tokio::test]
    async fn subagent_tool_definition() {
        let runtime = Arc::new(crate::AgentRuntime::new(Arc::from(
            crate::OpenAiProvider::new("http://example.com", "fake"),
        )));
        runtime.init_self_arc();
        let tool_reg = Arc::new(ToolRegistry::new());
        let agents = vec![AgentConfig {
            agent_id: "main".into(),
            name: Some("Main Agent".into()),
            description: None,
            model: Default::default(),
            system_prompt: None,
            tools: vec![],
            behavior: Default::default(),
            mcp_servers: vec![],
            min_tier: None,
            max_tier: None,
            avatar: None,
            channels: std::collections::HashMap::new(),
        }];

        let controller = Arc::new(crate::spawn_controller::SpawnController::new(
            crate::spawn_controller::SpawnConfig::default(),
        ));
        let manager = Arc::new(SubAgentManager::new(
            runtime,
            agents,
            SubAgentPolicy::default(),
            controller,
        ));
        manager.set_subagent_defs(builtin_subagent_defs());
        let tool = SubAgentTool::new(manager, tool_reg, SubAgentPolicy::default());
        let def = tool.to_definition();
        assert_eq!(def.function.name, "spawn_subagent");
        assert!(def.function.description.contains("sub-agent"));
    }

    #[tokio::test]
    async fn spawn_subagent_prompt_does_not_imply_task_create_available() {
        let runtime = Arc::new(crate::AgentRuntime::new(Arc::from(
            crate::OpenAiProvider::new("http://example.com", "fake"),
        )));
        runtime.init_self_arc();
        let tool_reg = Arc::new(ToolRegistry::new());
        let controller = Arc::new(crate::spawn_controller::SpawnController::new(
            crate::spawn_controller::SpawnConfig::default(),
        ));
        let manager = Arc::new(SubAgentManager::new(
            runtime,
            vec![],
            SubAgentPolicy::default(),
            controller,
        ));
        let tool = SubAgentTool::new(manager, tool_reg, SubAgentPolicy::default());
        let prompt = tool.prompt();
        assert!(
            prompt.contains("not exposed"),
            "prompt should state task_create is not exposed"
        );
        assert!(
            !prompt.contains("Use spawn_subagent for interactive orchestration; task_create"),
            "prompt should not suggest task_create as an alternative"
        );
    }

    #[test]
    fn coordinator_task_stop_prompt_distinguishes_background_task_stop() {
        let tool = TaskStopTool::new();
        let prompt = tool.prompt();
        assert!(prompt.contains("background_task_stop"));
        assert!(prompt.contains("coordinator"));
    }

    #[tokio::test]
    async fn deferred_subagent_tools_use_deferred_exposure() {
        let runtime = Arc::new(crate::AgentRuntime::new(Arc::from(
            crate::OpenAiProvider::new("http://example.com", "fake"),
        )));
        runtime.init_self_arc();
        let controller = Arc::new(crate::spawn_controller::SpawnController::new(
            crate::spawn_controller::SpawnConfig::default(),
        ));
        let manager = Arc::new(SubAgentManager::new(
            runtime,
            vec![],
            SubAgentPolicy::default(),
            controller,
        ));
        let deferred: Vec<Box<dyn Tool>> = vec![
            Box::new(SubAgentGetTool::new(Arc::clone(&manager))),
            Box::new(SubAgentListTool::new(Arc::clone(&manager))),
            Box::new(WaitAgentTool::new(Arc::clone(&manager))),
            Box::new(ResumeSubagentTool::new(
                Arc::clone(&manager),
                Arc::new(ToolRegistry::new()),
                SubAgentPolicy::default(),
            )),
            Box::new(SendMessageTool::new(Arc::clone(&manager))),
            Box::new(TaskStopTool::new()),
        ];
        for tool in &deferred {
            assert!(
                tool.exposure() == ToolExposure::Deferred,
                "{} should be deferred",
                tool.name()
            );
        }
    }

    #[tokio::test]
    async fn spawn_subagent_stays_direct_exposure() {
        let runtime = Arc::new(crate::AgentRuntime::new(Arc::from(
            crate::OpenAiProvider::new("http://example.com", "fake"),
        )));
        runtime.init_self_arc();
        let controller = Arc::new(crate::spawn_controller::SpawnController::new(
            crate::spawn_controller::SpawnConfig::default(),
        ));
        let manager = Arc::new(SubAgentManager::new(
            runtime,
            vec![],
            SubAgentPolicy::default(),
            controller,
        ));
        let tool = SubAgentTool::new(manager, Arc::new(ToolRegistry::new()), SubAgentPolicy::default());
        assert_eq!(tool.exposure(), ToolExposure::Direct);
    }

    #[test]
    fn parse_subagent_types() {
        assert_eq!(parse_subagent_type(None), SubAgentType::General);
        assert_eq!(parse_subagent_type(Some("general")), SubAgentType::General);
        assert_eq!(parse_subagent_type(Some("explore")), SubAgentType::Explore);
        assert_eq!(parse_subagent_type(Some("shell")), SubAgentType::Shell);
        assert_eq!(parse_subagent_type(Some("browser")), SubAgentType::Browser);
        assert_eq!(
            parse_subagent_type(Some("custom_thing")),
            SubAgentType::Custom("custom_thing".into())
        );
    }

    #[test]
    fn build_explore_registry_is_readonly() {
        let parent = ToolRegistry::new();
        let child = build_child_registry(&parent, &SubAgentType::Explore);
        for def in child.definitions().iter() {
            assert!(
                !matches!(
                    def.function.name.as_str(),
                    "write_file" | "file_write" | "shell_exec" | "shell" | "edit_file"
                ),
                "explore registry should not contain write tool: {}",
                def.function.name
            );
        }
    }

    // ===== WaitAgentTool tests =====

    use xiaolin_core::types::SubAgentRun;

    fn make_wait_manager() -> Arc<SubAgentManager> {
        let runtime = Arc::new(crate::AgentRuntime::new(Arc::from(
            crate::OpenAiProvider::new("http://example.com", "fake"),
        )));
        runtime.init_self_arc();
        let controller = Arc::new(crate::spawn_controller::SpawnController::new(
            crate::spawn_controller::SpawnConfig::default(),
        ));
        Arc::new(SubAgentManager::new(
            runtime,
            vec![],
            SubAgentPolicy::default(),
            controller,
        ))
    }

    fn completed_run(run_id: &str, result: &str) -> SubAgentRun {
        SubAgentRun {
            run_id: run_id.into(),
            agent_id: "a".into(),
            subagent_type: SubAgentType::General,
            task: "t".into(),
            status: xiaolin_core::types::SubAgentStatus::Completed,
            parent_session_id: "s1".into(),
            parent_message_id: "m1".into(),
            depth: 0,
            result: Some(result.into()),
            tool_calls_made: 0,
            iterations: 0,
            created_at: 0,
            completed_at: Some(1),
            token_usage: None,
            elapsed_ms: None,
            current_tool: None,
        }
    }

    fn running_run(run_id: &str) -> SubAgentRun {
        SubAgentRun {
            run_id: run_id.into(),
            agent_id: "a".into(),
            subagent_type: SubAgentType::General,
            task: "t".into(),
            status: xiaolin_core::types::SubAgentStatus::Running,
            parent_session_id: "s1".into(),
            parent_message_id: "m1".into(),
            depth: 0,
            result: None,
            tool_calls_made: 0,
            iterations: 0,
            created_at: 0,
            completed_at: None,
            token_usage: None,
            elapsed_ms: None,
            current_tool: None,
        }
    }

    // --- 8.1 wait-all returns when all complete ---
    #[tokio::test]
    async fn wait_all_returns_when_all_complete() {
        let mgr = make_wait_manager();
        mgr.insert_run(completed_run("r1", "res1"));
        mgr.insert_run(completed_run("r2", "res2"));
        let tool = WaitAgentTool::new(mgr);

        let result = tool
            .execute(r#"{"run_ids":["r1","r2"],"mode":"all"}"#)
            .await;
        assert!(result.success);
        let v: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["timed_out"], false);
        assert!(v["results"]["r1"]["status"] == "completed");
        assert!(v["results"]["r2"]["status"] == "completed");
    }

    // --- 8.2 wait-any returns on first completion ---
    #[tokio::test]
    async fn wait_any_returns_on_first_completion() {
        let mgr = make_wait_manager();
        mgr.insert_run(completed_run("r1", "first"));
        mgr.insert_run(running_run("r2"));
        let tool = WaitAgentTool::new(mgr);

        let result = tool
            .execute(r#"{"run_ids":["r1","r2"],"mode":"any"}"#)
            .await;
        assert!(result.success);
        let v: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["timed_out"], false);
        assert!(v["results"]["r1"]["status"] == "completed");
        assert!(v["results"]["r2"].is_null());
    }

    // --- 8.3 wait timeout returns partial ---
    #[tokio::test]
    async fn wait_timeout_returns_partial() {
        let mgr = make_wait_manager();
        mgr.insert_run(completed_run("r1", "done"));
        mgr.insert_run(running_run("r2"));
        let tool = WaitAgentTool::new(mgr);

        let result = tool
            .execute(r#"{"run_ids":["r1","r2"],"mode":"all","timeout_seconds":1}"#)
            .await;
        assert!(result.success);
        let v: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["timed_out"], true);
        assert!(v["results"]["r1"]["status"] == "completed");
    }

    // --- 8.4 wait already completed returns immediately ---
    #[tokio::test]
    async fn wait_already_completed_returns_immediately() {
        let mgr = make_wait_manager();
        mgr.insert_run(completed_run("r1", "instant"));
        let tool = WaitAgentTool::new(mgr);

        let t0 = tokio::time::Instant::now();
        let result = tool
            .execute(r#"{"run_ids":["r1"],"mode":"all"}"#)
            .await;
        let elapsed = t0.elapsed();
        assert!(result.success);
        assert!(elapsed < std::time::Duration::from_millis(50));
    }

    // --- 8.5 unknown run_id returns error ---
    #[tokio::test]
    async fn wait_unknown_run_id_returns_error() {
        let mgr = make_wait_manager();
        let tool = WaitAgentTool::new(mgr);

        let result = tool
            .execute(r#"{"run_ids":["unknown"],"mode":"all"}"#)
            .await;
        assert!(!result.success);
        assert!(result.output.contains("unknown run_id"));
    }

    // --- recovery field assertions ---

    fn make_subagent_tool(
        policy: SubAgentPolicy,
        depth: u32,
    ) -> SubAgentTool {
        let runtime = Arc::new(crate::AgentRuntime::new(Arc::from(
            crate::OpenAiProvider::new("http://example.com", "fake"),
        )));
        runtime.init_self_arc();
        let tool_reg = Arc::new(ToolRegistry::new());
        let controller = Arc::new(crate::spawn_controller::SpawnController::new(
            crate::spawn_controller::SpawnConfig::default(),
        ));
        let manager = Arc::new(SubAgentManager::new(
            runtime,
            vec![],
            policy.clone(),
            controller,
        ));
        SubAgentTool::new(manager, tool_reg, policy)
            .with_depth(depth)
    }

    #[tokio::test]
    async fn spawn_subagent_disabled_policy_recovery() {
        let mut policy = SubAgentPolicy::default();
        policy.enabled = false;
        let tool = make_subagent_tool(policy.clone(), 0);

        let result = tool
            .execute(r#"{"task": "do something", "type": "explore"}"#)
            .await;
        assert!(!result.success);
        assert_eq!(result.error_type, Some(ToolErrorType::ExecutionDenied));
        assert!(result.output.contains("disabled"));
        assert!(result.output.contains("What to do next:"));
        assert!(result.output.contains("Stop retrying"));
    }

    #[tokio::test]
    async fn spawn_subagent_depth_limit_recovery() {
        let mut policy = SubAgentPolicy::default();
        policy.max_depth = 1;
        let tool = make_subagent_tool(policy.clone(), 1);

        let result = tool
            .execute(r#"{"task": "nested work", "type": "explore"}"#)
            .await;
        assert!(!result.success);
        assert_eq!(result.error_type, Some(ToolErrorType::ExecutionDenied));
        assert!(result.output.contains("depth limit"));
        assert!(result.output.contains("What to do next:"));
        assert!(result.output.contains("Stop retrying"));
    }

    #[tokio::test]
    async fn send_message_non_running_uses_execution_denied() {
        let mgr = make_wait_manager();
        mgr.insert_run(completed_run("done-run", "finished"));
        let tool = SendMessageTool::new(mgr);

        let result = tool
            .execute(r#"{"run_id": "done-run", "message": "hello"}"#)
            .await;
        assert!(!result.success);
        assert_eq!(result.error_type, Some(ToolErrorType::ExecutionDenied));
        assert!(result.output.contains("not running"));
        assert!(result.output.contains("What to do next:"));
    }
}
