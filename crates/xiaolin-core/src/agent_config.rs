use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::complexity::ComplexityTier;
use crate::config::ChannelConfig;
use crate::types::{AgentId, ModelCapabilities};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfig {
    pub agent_id: AgentId,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub model: AgentModelConfig,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub tools: Vec<ToolConfig>,
    #[serde(default)]
    pub behavior: BehaviorConfig,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    /// When set, the model router never picks a model weaker than this tier.
    #[serde(default)]
    pub min_tier: Option<ComplexityTier>,
    /// When set, the model router never picks a model stronger than this tier (cost cap).
    #[serde(default)]
    pub max_tier: Option<ComplexityTier>,
    /// Local filesystem path or URL to the agent's avatar image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    /// Per-agent channel configurations (e.g. Feishu bot credentials).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub channels: HashMap<String, ChannelConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerConfig {
    pub id: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// SSE URL for HTTP-based MCP servers (alternative to command+args stdio transport).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Transport type: "stdio" (default) or "sse".
    #[serde(default = "default_transport")]
    pub transport: String,
}

fn default_transport() -> String {
    "stdio".to_string()
}

/// Cursor-compatible project-level MCP configuration.
///
/// File: `<workspace_root>/.xiaolin/mcp.json` or `<workspace_root>/.cursor/mcp.json`
///
/// Format (Cursor-compatible):
/// ```json
/// {
///   "mcpServers": {
///     "server-id": {
///       "command": "npx",
///       "args": ["-y", "@some/mcp-server"],
///       "env": { "KEY": "value" },
///       "disabled": true
///     }
///   }
/// }
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMcpConfig {
    #[serde(default)]
    pub mcp_servers: HashMap<String, ProjectMcpServerEntry>,
}

/// A single MCP server entry in the project-level config.
/// Compatible with Cursor's `.cursor/mcp.json` format.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMcpServerEntry {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub disabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default = "default_transport")]
    pub transport: String,
}

impl ProjectMcpConfig {
    /// Convert to a flat list of `McpServerConfig`, compatible with the global config format.
    pub fn to_mcp_server_configs(&self) -> Vec<McpServerConfig> {
        self.mcp_servers
            .iter()
            .map(|(id, entry)| McpServerConfig {
                id: id.clone(),
                command: entry.command.clone(),
                args: entry.args.clone(),
                enabled: entry.disabled.map(|d| !d),
                env: entry.env.clone(),
                url: entry.url.clone(),
                transport: entry.transport.clone(),
            })
            .collect()
    }
}

/// Load project-level MCP config from the workspace root.
///
/// Searches: `.xiaolin/mcp.json` (preferred), then `.cursor/mcp.json` (compatibility).
pub fn load_project_mcp_config(workspace_root: &std::path::Path) -> Option<ProjectMcpConfig> {
    let candidates = [
        workspace_root.join(".xiaolin/mcp.json"),
        workspace_root.join(".cursor/mcp.json"),
    ];
    for path in &candidates {
        if path.exists() {
            match std::fs::read_to_string(path) {
                Ok(content) => match serde_json::from_str::<ProjectMcpConfig>(&content) {
                    Ok(cfg) => {
                        tracing::info!(
                            path = %path.display(),
                            server_count = cfg.mcp_servers.len(),
                            "loaded project-level MCP config"
                        );
                        return Some(cfg);
                    }
                    Err(e) => {
                        tracing::warn!(
                            path = %path.display(),
                            error = %e,
                            "failed to parse project MCP config"
                        );
                    }
                },
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "failed to read project MCP config"
                    );
                }
            }
        }
    }
    None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelConfig {
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub context_window: Option<u32>,
    #[serde(default)]
    pub cost_per_1k_input: Option<f64>,
    #[serde(default)]
    pub cost_per_1k_output: Option<f64>,
    #[serde(default)]
    pub supports_reasoning: Option<bool>,
    /// Explicit model capability declaration. When set, overrides heuristic
    /// detection (e.g. `model_supports_vision`). Multi-select for input/output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<ModelCapabilities>,
    #[serde(default)]
    pub fallbacks: Vec<FallbackModelConfig>,
    /// Max in-flight LLM HTTP requests for this provider chain entry (default 10).
    #[serde(default = "default_max_concurrent_requests")]
    pub max_concurrent_requests: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FallbackModelConfig {
    pub provider: String,
    pub model: String,
    #[serde(default = "default_max_concurrent_requests")]
    pub max_concurrent_requests: u32,
    #[serde(default)]
    pub base_url: Option<String>,
    /// Explicit API key for this fallback provider.
    /// If omitted, the system looks up credentials from the central config store.
    #[serde(default)]
    pub api_key: Option<String>,
}

fn default_provider() -> String {
    String::new()
}
fn default_model() -> String {
    String::new()
}
fn default_temperature() -> f32 {
    0.7
}

fn default_max_concurrent_requests() -> u32 {
    10
}

impl Default for AgentModelConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            model: default_model(),
            temperature: default_temperature(),
            max_tokens: None,
            context_window: None,
            cost_per_1k_input: None,
            cost_per_1k_output: None,
            supports_reasoning: None,
            capabilities: None,
            fallbacks: Vec::new(),
            max_concurrent_requests: default_max_concurrent_requests(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolConfig {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub config: serde_json::Value,
}

fn default_true() -> bool {
    true
}

/// Three-level tool permission: allow (run freely), ask (needs user confirmation), deny (blocked).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermission {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BehaviorConfig {
    /// Maximum tool calls per turn. 0 = unlimited (runs until the LLM
    /// stops issuing tool_calls or the context window is exhausted).
    #[serde(default = "default_max_tool_calls")]
    pub max_tool_calls_per_turn: u32,
    #[serde(default = "default_max_errors")]
    pub max_consecutive_errors: u32,
    /// Legacy field — tools matching these patterns require user confirmation.
    /// Prefer `tools_ask` for new configs; both are merged at runtime.
    #[serde(default)]
    pub require_confirmation_for: Vec<String>,
    /// Tools matching these patterns require user confirmation before each execution.
    #[serde(default)]
    pub tools_ask: Vec<String>,
    #[serde(default)]
    pub tools_allow: Vec<String>,
    #[serde(default)]
    pub tools_deny: Vec<String>,
    #[serde(default)]
    pub file_access: FileAccessMode,
    /// Extra filesystem paths the agent may access in Workspace mode,
    /// beyond the workspace root, state dir, and well-known skill directories.
    /// Each entry is expanded with `~` → home and checked via prefix match.
    #[serde(default)]
    pub additional_allowed_paths: Vec<String>,
    #[serde(default)]
    pub subagent: SubAgentPolicy,
    /// When true, tools begin execution as soon as they are received during LLM
    /// streaming output, rather than waiting for the full response before batch
    /// execution. Concurrent-safe tools run in parallel; mutating tools serialize.
    #[serde(default)]
    pub streaming_tool_execution: bool,
    /// Enable the smart compression pipeline: dynamic thresholds, protection
    /// windows, eviction manifests, and semantic importance scoring.
    /// When false, falls back to the legacy fixed-threshold behavior.
    #[serde(default = "default_true")]
    pub enable_smart_compression: bool,
    /// Optional budget limit in USD. When set, the runtime tracks accumulated
    /// LLM costs and stops execution when the limit is exceeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_limit_usd: Option<f64>,
    /// Approval strategy override. When set to "auto_approve", tools execute
    /// without confirmation. Default (None) uses Interactive mode with ExecPolicy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_strategy: Option<String>,
}

/// Policy governing sub-agent delegation for an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentPolicy {
    /// Whether this agent is allowed to use sub-agents.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Maximum nesting depth for sub-agent chains (default 3).
    #[serde(default = "default_subagent_max_depth")]
    pub max_depth: u32,
    /// Maximum number of sub-agents that can run in parallel (default 5).
    #[serde(default = "default_subagent_max_parallel")]
    pub max_parallel: u32,
    /// Timeout in seconds for a single sub-agent run (default 300).
    #[serde(default = "default_subagent_timeout")]
    pub timeout_seconds: u64,
    /// Optional token budget cap for sub-agent runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<u64>,
    /// Allowed sub-agent types (empty = all types allowed).
    #[serde(default)]
    pub allowed_types: Vec<String>,
    /// Allowed child agent IDs to delegate to (empty = all agents allowed).
    #[serde(default)]
    pub allowed_agents: Vec<String>,

    // ── Reactive loop settings ───────────────────────────────────────
    /// Enable the reactive loop (harness auto-waits for sub-agent completions).
    #[serde(default = "default_true")]
    pub reactive_loop_enabled: bool,
    /// Batch window in ms: after first completion, wait this long to collect more (default 2000).
    #[serde(default = "default_batch_window_ms")]
    pub batch_window_ms: u64,
    /// Max re-prompts per turn to prevent infinite loops (default 10).
    #[serde(default = "default_max_reprompts")]
    pub max_reprompts_per_turn: u32,
    /// Max sub-agent spawns per turn (default 20).
    #[serde(default = "default_max_spawns_per_turn")]
    pub max_spawns_per_turn: u32,
    /// Suppress intermediate ack text when LLM responds without tool calls and active runs remain.
    #[serde(default)]
    pub suppress_intermediate_ack: bool,
}

fn default_subagent_max_depth() -> u32 {
    3
}
fn default_subagent_max_parallel() -> u32 {
    5
}
fn default_subagent_timeout() -> u64 {
    300
}
fn default_batch_window_ms() -> u64 {
    2000
}
fn default_max_reprompts() -> u32 {
    10
}
fn default_max_spawns_per_turn() -> u32 {
    20
}

/// Top-level concurrency configuration (typically from `[concurrency]` in config.toml).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConcurrencyConfig {
    #[serde(default = "default_concurrency_max_global")]
    pub max_global: usize,
    #[serde(default = "default_concurrency_max_per_session")]
    pub max_per_session: usize,
    #[serde(default = "default_true")]
    pub enforce_rw_isolation: bool,
    #[serde(default = "default_slot_acquire_timeout")]
    pub slot_acquire_timeout_seconds: u64,
}

fn default_concurrency_max_global() -> usize {
    20
}
fn default_concurrency_max_per_session() -> usize {
    5
}
fn default_slot_acquire_timeout() -> u64 {
    30
}

impl Default for ConcurrencyConfig {
    fn default() -> Self {
        Self {
            max_global: default_concurrency_max_global(),
            max_per_session: default_concurrency_max_per_session(),
            enforce_rw_isolation: true,
            slot_acquire_timeout_seconds: default_slot_acquire_timeout(),
        }
    }
}

impl Default for SubAgentPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            max_depth: default_subagent_max_depth(),
            max_parallel: default_subagent_max_parallel(),
            timeout_seconds: default_subagent_timeout(),
            token_budget: None,
            allowed_types: Vec::new(),
            allowed_agents: Vec::new(),
            reactive_loop_enabled: true,
            batch_window_ms: default_batch_window_ms(),
            max_reprompts_per_turn: default_max_reprompts(),
            max_spawns_per_turn: default_max_spawns_per_turn(),
            suppress_intermediate_ack: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FileAccessMode {
    None,
    #[default]
    Workspace,
    Full,
}

/// 0 = unlimited (the loop runs until the LLM stops issuing tool_calls
/// or the context window is exhausted). Any positive value is a hard cap.
fn default_max_tool_calls() -> u32 {
    0
}
fn default_max_errors() -> u32 {
    3
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            max_tool_calls_per_turn: default_max_tool_calls(),
            max_consecutive_errors: default_max_errors(),
            require_confirmation_for: Vec::new(),
            tools_ask: Vec::new(),
            tools_allow: Vec::new(),
            tools_deny: Vec::new(),
            file_access: FileAccessMode::default(),
            additional_allowed_paths: Vec::new(),
            subagent: SubAgentPolicy::default(),
            streaming_tool_execution: false,
            enable_smart_compression: true,
            budget_limit_usd: None,
            approval_strategy: None,
        }
    }
}

// ─── Permission Presets ──────────────────────────────────────────────

/// A named permission preset that maps to a `BehaviorOverride`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PermissionPreset {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub behavior_override: BehaviorOverride,
}

/// Partial override of `BehaviorConfig` fields — only the security-relevant ones.
/// `None` means "inherit from global default".
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BehaviorOverride {
    pub approval_strategy: Option<String>,
    pub file_access: Option<FileAccessMode>,
    pub tools_ask: Option<Vec<String>>,
    pub tools_deny: Option<Vec<String>>,
}

impl PermissionPreset {
    /// Merge this preset's overrides onto a base `BehaviorConfig`, returning a new config.
    pub fn resolve_behavior(&self, base: &BehaviorConfig) -> BehaviorConfig {
        let mut resolved = base.clone();
        if let Some(ref strategy) = self.behavior_override.approval_strategy {
            resolved.approval_strategy = Some(strategy.clone());
        }
        if let Some(mode) = self.behavior_override.file_access {
            resolved.file_access = mode;
        }
        if let Some(ref ask) = self.behavior_override.tools_ask {
            resolved.tools_ask = ask.clone();
        }
        if let Some(ref deny) = self.behavior_override.tools_deny {
            resolved.tools_deny = deny.clone();
        }
        resolved
    }
}

/// The four built-in permission presets.
pub fn builtin_permission_presets() -> Vec<PermissionPreset> {
    vec![
        PermissionPreset {
            id: "suggest".into(),
            name: "Suggest edits".into(),
            description: "All write operations require confirmation".into(),
            behavior_override: BehaviorOverride {
                approval_strategy: None,
                file_access: Some(FileAccessMode::Workspace),
                tools_ask: Some(vec![
                    "write_file".into(),
                    "edit_file".into(),
                    "shell_exec".into(),
                    "mcp_*".into(),
                ]),
                tools_deny: Some(vec![]),
            },
        },
        PermissionPreset {
            id: "auto-edit".into(),
            name: "Auto edit".into(),
            description: "File edits auto-approved, shell still requires confirmation".into(),
            behavior_override: BehaviorOverride {
                approval_strategy: None,
                file_access: Some(FileAccessMode::Workspace),
                tools_ask: Some(vec!["shell_exec".into(), "mcp_*".into()]),
                tools_deny: Some(vec![]),
            },
        },
        PermissionPreset {
            id: "full-auto".into(),
            name: "Full auto".into(),
            description: "All operations auto-approved (YOLO mode)".into(),
            behavior_override: BehaviorOverride {
                approval_strategy: Some("auto_approve".into()),
                file_access: Some(FileAccessMode::Full),
                tools_ask: Some(vec![]),
                tools_deny: Some(vec![]),
            },
        },
        PermissionPreset {
            id: "plan-only".into(),
            name: "Plan only".into(),
            description: "Read-only planning mode, all writes blocked".into(),
            behavior_override: BehaviorOverride {
                approval_strategy: None,
                file_access: Some(FileAccessMode::Workspace),
                tools_ask: Some(vec![]),
                tools_deny: Some(vec![
                    "write_file".into(),
                    "edit_file".into(),
                    "shell_exec".into(),
                ]),
            },
        },
    ]
}

/// Registry of available permission presets (builtins + user-defined).
#[derive(Debug, Clone)]
pub struct PermissionPresetRegistry {
    presets: Vec<PermissionPreset>,
}

impl PermissionPresetRegistry {
    pub fn new() -> Self {
        Self {
            presets: builtin_permission_presets(),
        }
    }

    /// Load user-defined presets from a JSON file and merge with builtins.
    pub fn load_custom(&mut self, path: &std::path::Path) {
        if !path.exists() {
            return;
        }
        match std::fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str::<Vec<PermissionPreset>>(&content) {
                Ok(custom) => {
                    for preset in custom {
                        if let Some(existing) = self.presets.iter_mut().find(|p| p.id == preset.id)
                        {
                            *existing = preset;
                        } else {
                            self.presets.push(preset);
                        }
                    }
                    tracing::info!(path = %path.display(), "loaded custom permission presets");
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "failed to parse permission presets");
                }
            },
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "failed to read permission presets file");
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<&PermissionPreset> {
        self.presets.iter().find(|p| p.id == id)
    }

    pub fn list(&self) -> &[PermissionPreset] {
        &self.presets
    }
}

impl Default for PermissionPresetRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if a tool name matches a pattern from `tools_allow`/`tools_deny`.
/// Supports exact match, trailing `*` prefix glob (e.g. `mcp_*`, `mcp_chrome_*`),
/// and `!` negation prefix (e.g. `!shell_exec`).
pub fn tool_pattern_matches(pattern: &str, tool_name: &str) -> bool {
    let (negated, pat) = if let Some(rest) = pattern.strip_prefix('!') {
        (true, rest)
    } else {
        (false, pattern)
    };
    let base_match = if let Some(prefix) = pat.strip_suffix('*') {
        tool_name.starts_with(prefix)
    } else {
        tool_name == pat
    };
    if negated {
        !base_match
    } else {
        base_match
    }
}

impl BehaviorConfig {
    /// Resolve the effective permission for a tool: deny > ask > allow.
    ///
    /// Priority: `tools_deny` (highest) > `tools_ask` + `require_confirmation_for` > `tools_allow` (lowest).
    /// When `tools_allow` is non-empty, unlisted tools are implicitly denied.
    pub fn tool_permission(&self, tool_name: &str) -> ToolPermission {
        if !self.tools_deny.is_empty()
            && self
                .tools_deny
                .iter()
                .any(|d| tool_pattern_matches(d, tool_name))
        {
            return ToolPermission::Deny;
        }
        if !self.tools_allow.is_empty()
            && !self
                .tools_allow
                .iter()
                .any(|a| tool_pattern_matches(a, tool_name))
        {
            return ToolPermission::Deny;
        }
        let ask_patterns = self
            .tools_ask
            .iter()
            .chain(self.require_confirmation_for.iter());
        if ask_patterns
            .clone()
            .any(|p| tool_pattern_matches(p, tool_name))
        {
            return ToolPermission::Ask;
        }
        ToolPermission::Allow
    }

    /// Check whether a tool is allowed (not denied) by this agent's policy.
    /// Returns `true` for both `Allow` and `Ask` — use `tool_permission()` for finer control.
    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        self.tool_permission(tool_name) != ToolPermission::Deny
    }

    /// Check if a tool requires user confirmation before execution.
    pub fn requires_confirmation(&self, tool_name: &str) -> bool {
        self.tool_permission(tool_name) == ToolPermission::Ask
    }
}

/// Definition of a sub-agent type that the main agent can spawn via tool calls.
///
/// Sub-agents inherit the main agent's model unless `model` is explicitly set.
/// Tool access is controlled by `tools.allowed` / `tools.denied` patterns.
/// How tool approvals are handled for a sub-agent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// Automatically approve all tool calls (default for sub-agents).
    #[default]
    AutoApprove,
    /// Bubble approval requests up to the parent/frontend for confirmation.
    Bubble,
    /// Deny all tool calls that would normally require approval.
    Deny,
}

/// Execution mode for a sub-agent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubAgentMode {
    /// Normal worker mode — executes tasks using available tools.
    #[default]
    Normal,
    /// Coordinator mode — orchestrates other sub-agents. Limited to management
    /// tools only (spawn_subagent, send_message, task_stop, subagent_list, subagent_get).
    Coordinator,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentDef {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Model override. `None` means inherit from the main agent.
    #[serde(default)]
    pub model: Option<AgentModelConfig>,
    /// Tool access rules. When `allowed` is non-empty, only matching tools are available.
    /// `denied` patterns always take precedence over `allowed`.
    #[serde(default)]
    pub tools: SubAgentToolFilter,
    /// System prompt for this sub-agent type. Replaces the main agent's prompt.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// When true, this sub-agent runs in the background (async). The main agent
    /// receives a `run_id` immediately and can poll or wait for results.
    /// When false (default), `spawn_agent` blocks until the sub-agent completes.
    #[serde(default)]
    pub background: bool,
    /// Whether this sub-agent type is safe to run concurrently with others.
    /// Read-only agents (e.g. "explore") should set this to `true`.
    #[serde(default)]
    pub concurrency_safe: bool,
    /// Maximum number of parent messages to inherit when `inherit_context` is used.
    /// Defaults to 20.
    #[serde(default = "default_max_context_messages")]
    pub max_context_messages: usize,
    /// How tool approvals are handled for this sub-agent type.
    #[serde(default)]
    pub permission_mode: PermissionMode,
    /// Execution mode: Normal (worker) or Coordinator (orchestrator).
    #[serde(default)]
    pub mode: SubAgentMode,
    /// Where this definition was loaded from.
    #[serde(skip)]
    pub source: SubAgentDefSource,
}

fn default_max_context_messages() -> usize {
    20
}

impl Default for SubAgentDef {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: None,
            description: None,
            model: None,
            tools: SubAgentToolFilter::default(),
            system_prompt: None,
            background: false,
            concurrency_safe: false,
            max_context_messages: 20,
            permission_mode: PermissionMode::default(),
            mode: SubAgentMode::default(),
            source: SubAgentDefSource::default(),
        }
    }
}

/// Tool allow/deny filter for a sub-agent definition.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubAgentToolFilter {
    /// Tool name patterns to allow (glob: `read_*`, exact: `grep`).
    /// Empty = all tools from the main agent are available.
    #[serde(default)]
    pub allowed: Vec<String>,
    /// Tool name patterns to deny (takes precedence over `allowed`).
    #[serde(default)]
    pub denied: Vec<String>,
    /// Predefined profile name ("plan", "readonly"). When set, the profile's
    /// `demote` list is merged into `denied` during tool filtering.
    #[serde(default)]
    pub profile: Option<String>,
}

/// Tracks where a SubAgentDef was loaded from (for diagnostics and reload).
#[derive(Debug, Clone, Default)]
pub enum SubAgentDefSource {
    #[default]
    Builtin,
    JsonFile(std::path::PathBuf),
    MarkdownFile(std::path::PathBuf),
}

impl SubAgentToolFilter {
    /// Check whether a tool name is permitted by this filter.
    /// Profile demote list is merged with explicit denied patterns.
    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        if self.denied.iter().any(|p| tool_pattern_matches(p, tool_name)) {
            return false;
        }
        if let Some(ref profile_name) = self.profile {
            let profile = match profile_name.as_str() {
                "plan" => crate::tool::ToolProfile::plan_mode(),
                "readonly" => crate::tool::ToolProfile::readonly(),
                _ => crate::tool::ToolProfile::default(),
            };
            if profile.demote.iter().any(|d| d == tool_name) {
                return false;
            }
        }
        if self.allowed.is_empty() {
            return true;
        }
        self.allowed.iter().any(|p| tool_pattern_matches(p, tool_name))
    }
}

/// Load sub-agent definitions from a directory of JSON files.
pub fn load_subagent_defs_json(dir: &std::path::Path) -> anyhow::Result<Vec<SubAgentDef>> {
    let mut defs = Vec::new();
    if !dir.exists() {
        return Ok(defs);
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            let text = std::fs::read_to_string(&path)?;
            match json5::from_str::<SubAgentDef>(&text) {
                Ok(mut def) => {
                    def.source = SubAgentDefSource::JsonFile(path.clone());
                    tracing::info!(id = %def.id, path = %path.display(), "loaded sub-agent def");
                    defs.push(def);
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "failed to parse sub-agent def");
                }
            }
        }
    }
    Ok(defs)
}

/// Load sub-agent definitions from Markdown files with YAML frontmatter.
///
/// Format:
/// ```text
/// ---
/// id: reviewer
/// name: Code Reviewer
/// tools:
///   allowed: [read_file, grep, search]
///   denied: [write_file, shell_exec]
/// model: null
/// ---
///
/// You are a code review specialist...
/// ```
pub fn load_subagent_defs_markdown(dir: &std::path::Path) -> anyhow::Result<Vec<SubAgentDef>> {
    let mut defs = Vec::new();
    if !dir.exists() {
        return Ok(defs);
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "md") {
            let text = std::fs::read_to_string(&path)?;
            match parse_markdown_subagent_def(&text, &path) {
                Ok(def) => {
                    tracing::info!(id = %def.id, path = %path.display(), "loaded sub-agent def (markdown)");
                    defs.push(def);
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "failed to parse markdown sub-agent def");
                }
            }
        }
    }
    Ok(defs)
}

/// Parse a Markdown file with YAML frontmatter into a `SubAgentDef`.
/// Delegates to `agent_markdown::parse_agent_markdown` for robust parsing with
/// schema validation (unknown fields rejected, required `id` check).
fn parse_markdown_subagent_def(content: &str, path: &std::path::Path) -> anyhow::Result<SubAgentDef> {
    crate::agent_markdown::parse_agent_markdown(content, path)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Return the built-in sub-agent definitions that ship with XiaoLin.
const COORDINATOR_SYSTEM_PROMPT: &str = "\
You are a coordinator agent that orchestrates multiple worker sub-agents to accomplish complex tasks.

Your capabilities:
- spawn_subagent: Create worker sub-agents for specific tasks (always use background=true)
- send_message: Send steering messages to running sub-agents
- subagent_get: Check the status and results of sub-agents
- subagent_list: List all active sub-agents
- task_stop: Signal that orchestration is complete and provide final summary

Your workflow:
1. Analyze the task and break it into subtasks
2. Spawn appropriate worker sub-agents for each subtask (shell, code, explore, research)
3. Monitor their progress via subagent_get
4. Send steering messages if workers need guidance
5. Once all workers complete, synthesize results and call task_stop

Rules:
- Always spawn workers in background mode
- You cannot directly read/write files or run commands — delegate to workers
- Provide clear, specific task descriptions to each worker
- Synthesize worker results into a coherent final answer
";

pub fn builtin_subagent_defs() -> Vec<SubAgentDef> {
    vec![
        SubAgentDef {
            id: "explore".into(),
            name: Some("Explorer".into()),
            description: Some("Read-only exploration and code analysis".into()),
            model: None,
            tools: SubAgentToolFilter {
                allowed: vec![
                    "read_file".into(),
                    "list_dir".into(),
                    "search_files".into(),
                    "grep".into(),
                    "web_search".into(),
                    "web_fetch".into(),
                    "get_context_window".into(),
                    "list_skills".into(),
                    "read_skill".into(),
                    "mcp_*".into(),
                ],
                denied: vec![
                    "write_file".into(),
                    "edit_file".into(),
                    "create_file".into(),
                    "delete_file".into(),
                    "shell_exec".into(),
                    "exec_command".into(),
                    "spawn_subagent".into(),
                ],
                profile: None,
            },
            system_prompt: Some(
                "You are a code exploration assistant. Analyze code structure, find patterns, \
                 and answer questions about the codebase. You have read-only access — \
                 you cannot modify files or run commands."
                    .into(),
            ),
            background: false,
            concurrency_safe: true,
            max_context_messages: default_max_context_messages(),
            permission_mode: PermissionMode::AutoApprove,
            mode: SubAgentMode::Normal,
            source: SubAgentDefSource::Builtin,
        },
        SubAgentDef {
            id: "code".into(),
            name: Some("Coder".into()),
            description: Some("Code editing and file manipulation".into()),
            model: None,
            tools: SubAgentToolFilter {
                allowed: vec![],
                denied: vec!["spawn_subagent".into()],
                profile: None,
            },
            system_prompt: Some(
                "You are a code editing assistant. Implement the requested changes carefully, \
                 following existing code style and conventions. Test your changes when possible."
                    .into(),
            ),
            background: false,
            concurrency_safe: false,
            max_context_messages: default_max_context_messages(),
            permission_mode: PermissionMode::AutoApprove,
            mode: SubAgentMode::Normal,
            source: SubAgentDefSource::Builtin,
        },
        SubAgentDef {
            id: "shell".into(),
            name: Some("Shell".into()),
            description: Some("Command execution and system operations".into()),
            model: None,
            tools: SubAgentToolFilter {
                allowed: vec![
                    "shell_exec".into(),
                    "exec_command".into(),
                    "read_file".into(),
                    "write_file".into(),
                    "edit_file".into(),
                    "list_dir".into(),
                    "search_files".into(),
                    "grep".into(),
                ],
                denied: vec!["spawn_subagent".into()],
                profile: None,
            },
            system_prompt: Some(
                "You are a shell execution assistant. Run commands, inspect output, \
                 and perform system operations as instructed. Be careful with destructive commands."
                    .into(),
            ),
            background: false,
            concurrency_safe: false,
            max_context_messages: default_max_context_messages(),
            permission_mode: PermissionMode::AutoApprove,
            mode: SubAgentMode::Normal,
            source: SubAgentDefSource::Builtin,
        },
        SubAgentDef {
            id: "research".into(),
            name: Some("Researcher".into()),
            description: Some("Web search, analysis, and information gathering".into()),
            model: None,
            tools: SubAgentToolFilter {
                allowed: vec![
                    "web_search".into(),
                    "web_fetch".into(),
                    "http_fetch".into(),
                    "read_file".into(),
                    "list_dir".into(),
                    "search_files".into(),
                    "grep".into(),
                    "mcp_*".into(),
                ],
                denied: vec![
                    "write_file".into(),
                    "edit_file".into(),
                    "shell_exec".into(),
                    "exec_command".into(),
                    "spawn_subagent".into(),
                ],
                profile: None,
            },
            system_prompt: Some(
                "You are a research assistant. Search the web, read documents, and synthesize \
                 information to answer questions thoroughly. Cite sources when possible."
                    .into(),
            ),
            background: false,
            concurrency_safe: true,
            max_context_messages: default_max_context_messages(),
            permission_mode: PermissionMode::AutoApprove,
            mode: SubAgentMode::Normal,
            source: SubAgentDefSource::Builtin,
        },
        SubAgentDef {
            id: "coordinator".into(),
            name: Some("Coordinator".into()),
            description: Some(
                "Orchestrates multiple sub-agents to accomplish complex tasks. \
                 Spawns workers, sends steering messages, and synthesizes results."
                    .into(),
            ),
            model: None,
            tools: SubAgentToolFilter {
                allowed: vec![
                    "spawn_subagent".into(),
                    "send_message".into(),
                    "subagent_get".into(),
                    "subagent_list".into(),
                    "list_agents".into(),
                    "get_agent_info".into(),
                    "task_stop".into(),
                ],
                denied: vec![],
                profile: None,
            },
            system_prompt: Some(COORDINATOR_SYSTEM_PROMPT.into()),
            background: true,
            concurrency_safe: false,
            max_context_messages: default_max_context_messages(),
            permission_mode: PermissionMode::AutoApprove,
            mode: SubAgentMode::Coordinator,
            source: SubAgentDefSource::Builtin,
        },
    ]
}

/// Load all sub-agent definitions: builtins + JSON files + Markdown files.
/// Later definitions with the same `id` override earlier ones.
pub fn load_all_subagent_defs(
    json_dir: Option<&std::path::Path>,
    markdown_dir: Option<&std::path::Path>,
) -> Vec<SubAgentDef> {
    let mut defs = builtin_subagent_defs();

    if let Some(dir) = json_dir {
        match load_subagent_defs_json(dir) {
            Ok(custom) => {
                for d in custom {
                    if let Some(existing) = defs.iter_mut().find(|e| e.id == d.id) {
                        tracing::info!(id = %d.id, "custom sub-agent def overrides builtin");
                        *existing = d;
                    } else {
                        defs.push(d);
                    }
                }
            }
            Err(e) => tracing::warn!(dir = %dir.display(), error = %e, "failed to load JSON sub-agent defs"),
        }
    }

    if let Some(dir) = markdown_dir {
        match load_subagent_defs_markdown(dir) {
            Ok(custom) => {
                for d in custom {
                    if let Some(existing) = defs.iter_mut().find(|e| e.id == d.id) {
                        tracing::info!(id = %d.id, "markdown sub-agent def overrides existing");
                        *existing = d;
                    } else {
                        defs.push(d);
                    }
                }
            }
            Err(e) => tracing::warn!(dir = %dir.display(), error = %e, "failed to load markdown sub-agent defs"),
        }
    }

    defs
}

/// Load agent configs from a directory of JSON files
pub fn load_agent_configs(dir: &std::path::Path) -> anyhow::Result<Vec<AgentConfig>> {
    let mut agents = Vec::new();

    if !dir.exists() {
        return Ok(agents);
    }

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            let text = std::fs::read_to_string(&path)?;
            match json5::from_str::<AgentConfig>(&text) {
                Ok(config) => {
                    tracing::info!(agent_id = %config.agent_id, path = %path.display(), "loaded agent config");
                    agents.push(config);
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "failed to parse agent config");
                }
            }
        }
    }

    Ok(agents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_exact_match() {
        assert!(tool_pattern_matches("shell_exec", "shell_exec"));
        assert!(!tool_pattern_matches("shell_exec", "shell_exec_sandbox"));
        assert!(!tool_pattern_matches("shell_exec", "http_fetch"));
    }

    #[test]
    fn pattern_glob_prefix() {
        assert!(tool_pattern_matches("mcp_*", "mcp_chrome_screenshot"));
        assert!(tool_pattern_matches("mcp_*", "mcp_relay_feedback"));
        assert!(!tool_pattern_matches("mcp_*", "http_fetch"));
        assert!(tool_pattern_matches(
            "mcp_chrome_*",
            "mcp_chrome_screenshot"
        ));
        assert!(!tool_pattern_matches("mcp_chrome_*", "mcp_relay_feedback"));
    }

    #[test]
    fn pattern_negation() {
        assert!(!tool_pattern_matches("!shell_exec", "shell_exec"));
        assert!(tool_pattern_matches("!shell_exec", "http_fetch"));
        assert!(!tool_pattern_matches("!mcp_*", "mcp_chrome_screenshot"));
        assert!(tool_pattern_matches("!mcp_*", "http_fetch"));
    }

    #[test]
    fn behavior_empty_allows_all() {
        let b = BehaviorConfig::default();
        assert!(b.is_tool_allowed("http_fetch"));
        assert!(b.is_tool_allowed("mcp_chrome_screenshot"));
        assert!(b.is_tool_allowed("shell_exec"));
    }

    #[test]
    fn behavior_deny_exact() {
        let b = BehaviorConfig {
            tools_deny: vec!["shell_exec".into()],
            ..Default::default()
        };
        assert!(!b.is_tool_allowed("shell_exec"));
        assert!(b.is_tool_allowed("http_fetch"));
    }

    #[test]
    fn behavior_deny_glob() {
        let b = BehaviorConfig {
            tools_deny: vec!["mcp_*".into()],
            ..Default::default()
        };
        assert!(!b.is_tool_allowed("mcp_chrome_screenshot"));
        assert!(!b.is_tool_allowed("mcp_relay_feedback"));
        assert!(b.is_tool_allowed("http_fetch"));
        assert!(b.is_tool_allowed("shell_exec"));
    }

    #[test]
    fn behavior_allow_glob_includes_mcp() {
        let b = BehaviorConfig {
            tools_allow: vec!["http_fetch".into(), "web_search".into(), "mcp_*".into()],
            ..Default::default()
        };
        assert!(b.is_tool_allowed("http_fetch"));
        assert!(b.is_tool_allowed("mcp_chrome_screenshot"));
        assert!(!b.is_tool_allowed("shell_exec"));
    }

    #[test]
    fn behavior_allow_with_deny_glob() {
        let b = BehaviorConfig {
            tools_allow: vec!["mcp_*".into(), "web_search".into()],
            tools_deny: vec!["mcp_dangerous_*".into()],
            ..Default::default()
        };
        assert!(b.is_tool_allowed("mcp_chrome_screenshot"));
        assert!(!b.is_tool_allowed("mcp_dangerous_tool"));
        assert!(b.is_tool_allowed("web_search"));
        assert!(!b.is_tool_allowed("shell_exec"));
    }

    #[test]
    fn tools_ask_requires_confirmation() {
        let b = BehaviorConfig {
            tools_ask: vec!["shell_exec".into(), "mcp_dangerous_*".into()],
            ..Default::default()
        };
        assert_eq!(b.tool_permission("shell_exec"), ToolPermission::Ask);
        assert_eq!(b.tool_permission("mcp_dangerous_rm"), ToolPermission::Ask);
        assert_eq!(b.tool_permission("http_fetch"), ToolPermission::Allow);
        assert!(b.is_tool_allowed("shell_exec"));
        assert!(b.requires_confirmation("shell_exec"));
        assert!(!b.requires_confirmation("http_fetch"));
    }

    #[test]
    fn deny_overrides_ask() {
        let b = BehaviorConfig {
            tools_ask: vec!["shell_exec".into()],
            tools_deny: vec!["shell_exec".into()],
            ..Default::default()
        };
        assert_eq!(b.tool_permission("shell_exec"), ToolPermission::Deny);
        assert!(!b.is_tool_allowed("shell_exec"));
    }

    #[test]
    fn file_access_mode_deserialization() {
        // Test that "full" (snake_case) deserializes to Full
        let mode: FileAccessMode = serde_json::from_str("\"full\"").unwrap();
        assert_eq!(mode, FileAccessMode::Full);

        // Test that "workspace" (snake_case) deserializes to Workspace
        let mode: FileAccessMode = serde_json::from_str("\"workspace\"").unwrap();
        assert_eq!(mode, FileAccessMode::Workspace);

        // Test that "none" (snake_case) deserializes to None
        let mode: FileAccessMode = serde_json::from_str("\"none\"").unwrap();
        assert_eq!(mode, FileAccessMode::None);

        // Test serialization round-trip
        assert_eq!(
            serde_json::to_string(&FileAccessMode::Full).unwrap(),
            "\"full\""
        );
    }

    #[test]
    fn behavior_config_file_access_from_camel_case() {
        // This is what the JSON config file contains
        let json =
            r#"{"fileAccess": "full", "maxToolCallsPerTurn": 50, "maxConsecutiveErrors": 3}"#;
        let behavior: BehaviorConfig = json5::from_str(json).unwrap();
        assert_eq!(behavior.file_access, FileAccessMode::Full);

        let json2 =
            r#"{"fileAccess": "workspace", "maxToolCallsPerTurn": 50, "maxConsecutiveErrors": 3}"#;
        let behavior2: BehaviorConfig = json5::from_str(json2).unwrap();
        assert_eq!(behavior2.file_access, FileAccessMode::Workspace);
    }

    #[test]
    fn subagent_tool_filter_empty_allows_all() {
        let f = SubAgentToolFilter::default();
        assert!(f.is_tool_allowed("anything"));
        assert!(f.is_tool_allowed("shell_exec"));
    }

    #[test]
    fn subagent_tool_filter_allowed_only() {
        let f = SubAgentToolFilter {
            allowed: vec!["read_file".into(), "grep".into()],
            denied: vec![],
            profile: None,
        };
        assert!(f.is_tool_allowed("read_file"));
        assert!(f.is_tool_allowed("grep"));
        assert!(!f.is_tool_allowed("write_file"));
    }

    #[test]
    fn subagent_tool_filter_denied_overrides() {
        let f = SubAgentToolFilter {
            allowed: vec!["mcp_*".into()],
            denied: vec!["mcp_dangerous_*".into()],
            profile: None,
        };
        assert!(f.is_tool_allowed("mcp_chrome_screenshot"));
        assert!(!f.is_tool_allowed("mcp_dangerous_tool"));
        assert!(!f.is_tool_allowed("shell_exec"));
    }

    #[test]
    fn subagent_def_json_parse() {
        let json = r#"{
            "id": "test",
            "name": "Test Agent",
            "description": "A test sub-agent",
            "tools": {
                "allowed": ["read_file"],
                "denied": ["shell_exec"]
            },
            "systemPrompt": "You are a test agent.",
            "background": true,
            "concurrencySafe": true
        }"#;
        let def: SubAgentDef = json5::from_str(json).unwrap();
        assert_eq!(def.id, "test");
        assert_eq!(def.name.as_deref(), Some("Test Agent"));
        assert!(def.background);
        assert!(def.concurrency_safe);
        assert!(def.tools.is_tool_allowed("read_file"));
        assert!(!def.tools.is_tool_allowed("shell_exec"));
    }

    #[test]
    fn subagent_def_markdown_parse() {
        let md = r#"---
id: reviewer
name: Code Reviewer
tools:
  allowed:
    - read_file
    - grep
  denied:
    - write_file
---

You are a code review specialist. Analyze the provided code carefully."#;
        let def = parse_markdown_subagent_def(md, std::path::Path::new("test.md")).unwrap();
        assert_eq!(def.id, "reviewer");
        assert_eq!(def.name.as_deref(), Some("Code Reviewer"));
        assert!(def.system_prompt.unwrap().contains("code review specialist"));
        assert!(def.tools.is_tool_allowed("read_file"));
        assert!(!def.tools.is_tool_allowed("write_file"));
    }

    #[test]
    fn builtin_subagent_defs_sanity() {
        let defs = builtin_subagent_defs();
        assert!(defs.len() >= 4);
        let ids: Vec<&str> = defs.iter().map(|d| d.id.as_str()).collect();
        assert!(ids.contains(&"explore"));
        assert!(ids.contains(&"code"));
        assert!(ids.contains(&"shell"));
        assert!(ids.contains(&"research"));

        let explore = defs.iter().find(|d| d.id == "explore").unwrap();
        assert!(explore.concurrency_safe);
        assert!(!explore.background);
        assert!(explore.tools.is_tool_allowed("read_file"));
        assert!(!explore.tools.is_tool_allowed("write_file"));
        assert!(!explore.tools.is_tool_allowed("shell_exec"));
    }

    #[test]
    fn permission_preset_resolve_behavior() {
        let base = BehaviorConfig::default();
        let preset = PermissionPreset {
            id: "auto-edit".into(),
            name: "Auto edit".into(),
            description: "test".into(),
            behavior_override: BehaviorOverride {
                approval_strategy: None,
                file_access: Some(FileAccessMode::Workspace),
                tools_ask: Some(vec!["shell_exec".into()]),
                tools_deny: Some(vec![]),
            },
        };
        let resolved = preset.resolve_behavior(&base);
        assert_eq!(resolved.tools_ask, vec!["shell_exec"]);
        assert!(resolved.tools_deny.is_empty());
        assert_eq!(resolved.file_access, FileAccessMode::Workspace);
        assert!(resolved.approval_strategy.is_none());
    }

    #[test]
    fn permission_preset_full_auto_sets_strategy() {
        let base = BehaviorConfig::default();
        let presets = builtin_permission_presets();
        let full_auto = presets.iter().find(|p| p.id == "full-auto").unwrap();
        let resolved = full_auto.resolve_behavior(&base);
        assert_eq!(resolved.approval_strategy.as_deref(), Some("auto_approve"));
        assert_eq!(resolved.file_access, FileAccessMode::Full);
    }

    #[test]
    fn permission_preset_registry_builtins() {
        let registry = PermissionPresetRegistry::new();
        assert_eq!(registry.list().len(), 4);
        assert!(registry.get("suggest").is_some());
        assert!(registry.get("auto-edit").is_some());
        assert!(registry.get("full-auto").is_some());
        assert!(registry.get("plan-only").is_some());
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn legacy_require_confirmation_for_merges_with_tools_ask() {
        let b = BehaviorConfig {
            require_confirmation_for: vec!["shell_exec".into()],
            tools_ask: vec!["http_fetch".into()],
            ..Default::default()
        };
        assert!(b.requires_confirmation("shell_exec"));
        assert!(b.requires_confirmation("http_fetch"));
        assert!(!b.requires_confirmation("web_search"));
    }
}
