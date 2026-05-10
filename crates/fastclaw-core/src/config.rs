use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use crate::error::{FastClawError, FastClawResult};
use crate::types::ModelCapabilities;

/// Selects which state directory and config search paths to use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigMode {
    /// Production: uses `~/.fastclaw/`
    Production,
    /// Development: uses `~/.fastclaw-dev/`
    Development,
    /// Named profile: uses `~/.fastclaw-<name>/`
    Profile(String),
}

impl ConfigMode {
    /// Construct from the legacy `(dev, profile)` pair used by the CLI.
    pub fn from_flags(dev: bool, profile: Option<&str>) -> Self {
        match (dev, profile) {
            (true, _) => Self::Development,
            (_, Some(name)) => Self::Profile(name.to_string()),
            _ => {
                // 如果没有明确指定 dev 标志，则根据编译模式决定
                if cfg!(debug_assertions) {
                    Self::Development
                } else {
                    Self::Production
                }
            }
        }
    }

    pub fn is_dev(&self) -> bool {
        matches!(self, Self::Development)
    }

    pub fn profile_name(&self) -> Option<&str> {
        match self {
            Self::Profile(name) => Some(name),
            _ => None,
        }
    }
}

/// Per-key model / provider hints from the top-level `models` config object.
///
/// Known fields map to typed options; any additional keys are preserved in [`Self::extra`]
/// for forward compatibility with evolving configs.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelProviderConfig {
    #[serde(default, alias = "providerType")]
    pub provider: String,
    #[serde(default, alias = "defaultModel")]
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default)]
    pub temperature: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Maximum context window size (tokens) the model supports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_vision: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_tool_calling: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_reasoning: Option<bool>,
    /// Structured capability declaration. Takes precedence over the individual
    /// `supports_*` booleans above when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<ModelCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_per_1k_input: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_per_1k_output: Option<f64>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FastClawConfig {
    #[serde(default)]
    pub gateway: GatewayConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub session: SessionConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub models: HashMap<String, ModelProviderConfig>,
    #[serde(default)]
    pub security: SecurityConfig,
    #[serde(default)]
    pub channels: HashMap<String, ChannelConfig>,
    #[serde(default)]
    pub agents: AgentsConfig,
    #[serde(default)]
    pub bindings: Vec<BindingConfig>,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub skills: SkillsConfig,
    #[serde(default)]
    pub paths: PathsConfig,
    /// When true, a missing `$include` file fails config load instead of logging a warning.
    #[serde(default)]
    pub strict_includes: bool,
    #[serde(default)]
    pub credentials: CredentialsConfig,
    #[serde(default)]
    pub web_search: WebSearchConfig,
    #[serde(default, rename = "modelRouter")]
    pub model_router: ModelRouterConfig,
    #[serde(default, rename = "promptRouter")]
    pub prompt_router: PromptRouterConfig,
    /// Background evolution jobs (skill extraction, skill store maintenance).
    #[serde(default)]
    pub evolution: EvolutionRuntimeConfig,
    /// Global MCP servers available to all agents.
    #[serde(default, rename = "mcpServers")]
    pub mcp_servers: Vec<crate::agent_config::McpServerConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onboarding: Option<serde_json::Value>,
    #[serde(default)]
    pub tracing: TracingConfig,
    /// LLM provider plugin system configuration.
    #[serde(default, rename = "llmPlugins")]
    pub llm_plugins: crate::llm_plugin::LlmPluginsConfig,
    /// Channel plugin system configuration.
    #[serde(default, rename = "channelPlugins")]
    pub channel_plugins: crate::channel_plugin::ChannelPluginsConfig,
}

/// Controls conversation tracing for the harness / eval subsystem.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TracingConfig {
    /// When true, every chat turn is recorded as a `ConversationTrace` in the session database.
    #[serde(default)]
    pub conversation_trace: bool,
}

/// Intervals for gateway-hosted evolution background tasks ([`FastClawConfig::evolution`]).
///
/// Set either interval to `0` to disable that task.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvolutionRuntimeConfig {
    /// How often to scan recent trajectories and extract candidate skills (seconds). Zero disables.
    #[serde(default = "default_skill_extraction_interval_secs")]
    pub skill_extraction_interval_secs: u64,
    /// How often to run skill store maintenance (promote / retire) (seconds). Zero disables.
    #[serde(default = "default_skill_maintenance_interval_secs")]
    pub skill_maintenance_interval_secs: u64,
}

fn default_skill_extraction_interval_secs() -> u64 {
    600
}

fn default_skill_maintenance_interval_secs() -> u64 {
    300
}

impl Default for EvolutionRuntimeConfig {
    fn default() -> Self {
        Self {
            skill_extraction_interval_secs: default_skill_extraction_interval_secs(),
            skill_maintenance_interval_secs: default_skill_maintenance_interval_secs(),
        }
    }
}

/// Skills configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsConfig {
    /// How skills are injected into the system prompt.
    /// - "full":    Inject complete SKILL.md content (highest accuracy, most tokens)
    /// - "compact": Inject name + one-line description only (~50% token savings)
    /// - "lazy":    Inject minimal list, provide list_skills/read_skill tools (~95% savings)
    #[serde(default = "default_prompt_mode")]
    pub prompt_mode: SkillPromptMode,
    /// Explicit allowlist of skill IDs. Empty = allow all.
    #[serde(default)]
    pub allow: Vec<String>,
    /// Denylist of skill IDs.
    #[serde(default)]
    pub deny: Vec<String>,
}

fn default_prompt_mode() -> SkillPromptMode {
    SkillPromptMode::Full
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            prompt_mode: SkillPromptMode::Full,
            allow: Vec::new(),
            deny: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillPromptMode {
    Full,
    Compact,
    Lazy,
}

/// Web search backend configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchConfig {
    /// Backend: "tavily", "searxng", "builtin", or "" (unconfigured).
    #[serde(default = "default_search_backend")]
    pub backend: String,
    /// API key for Tavily backend. Also checked in credentials.tavily.apiKey.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Base URL for SearXNG instance.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Enabled built-in engine IDs (e.g. ["google", "baidu", "bing", "sogou", "360"]).
    /// Only used when backend = "builtin".
    #[serde(default)]
    pub engines: Option<Vec<String>>,
}

fn default_search_backend() -> String {
    "builtin".to_string()
}

/// Model router configuration for intelligent model selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRouterConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Strategy: "fixed", "cost_optimized", "fallback".
    /// Deprecated: "quality_first" / "latency_optimized" are accepted but mapped to "fallback" with a warning.
    #[serde(default = "default_routing_strategy")]
    pub strategy: String,
    /// Maximum daily spend (USD). None = no limit.
    #[serde(default)]
    pub daily_budget: Option<f64>,
    /// Fallback chain of model names (used with "fallback" strategy).
    #[serde(default)]
    pub fallback_chain: Vec<String>,
}

fn default_routing_strategy() -> String {
    "fixed".to_string()
}

impl Default for ModelRouterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            strategy: default_routing_strategy(),
            daily_budget: None,
            fallback_chain: Vec::new(),
        }
    }
}

/// Prompt router configuration for intent-based dynamic role prompt selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptRouterConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_prompt_router_profile")]
    pub default_profile: String,
    #[serde(default)]
    pub profiles: HashMap<String, PromptProfileConfig>,
    #[serde(default)]
    pub rules: Vec<PromptRuleConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptProfileConfig {
    pub role_prompt_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptRuleConfig {
    pub profile: String,
    #[serde(default)]
    pub keywords: Vec<String>,
}

fn default_prompt_router_profile() -> String {
    "default".to_string()
}

impl Default for PromptRouterConfig {
    fn default() -> Self {
        let mut profiles = HashMap::new();
        profiles.insert(
            "default".to_string(),
            PromptProfileConfig {
                role_prompt_id: "main".to_string(),
            },
        );
        Self {
            enabled: false,
            default_profile: default_prompt_router_profile(),
            profiles,
            rules: Vec::new(),
        }
    }
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            backend: default_search_backend(),
            api_key: None,
            base_url: None,
            engines: None,
        }
    }
}

/// Per-channel configuration loaded from the main JSON config.
/// Values here override environment variables.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChannelConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub app_secret: Option<String>,
    #[serde(default)]
    pub verification_token: Option<String>,
    #[serde(default)]
    pub encrypt_key: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub connection_mode: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub reply_mode: Option<String>,
    /// User OAuth access token for user-scoped channel APIs (e.g. Feishu tasks, docs, calendar).
    #[serde(default)]
    pub user_access_token: Option<String>,
    /// Per-account overrides. Each key is an account ID used in bindings.
    #[serde(default)]
    pub accounts: std::collections::HashMap<String, ChannelAccountConfig>,
    /// Which account to use when none is specified in binding.
    #[serde(default)]
    pub default_account: Option<String>,
}

/// Account-specific overrides for a channel (all fields optional, merge with top-level).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChannelAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub app_secret: Option<String>,
    #[serde(default)]
    pub verification_token: Option<String>,
    #[serde(default)]
    pub encrypt_key: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub reply_mode: Option<String>,
}

impl ChannelConfig {
    /// Fill `None` fields with sensible defaults so they survive a
    /// serialize→deserialize round-trip (e.g. when persisted to `default.json`).
    pub fn fill_defaults(&mut self) {
        if self.enabled.is_none() {
            self.enabled = Some(true);
        }
        if self.connection_mode.is_none() {
            self.connection_mode = Some("websocket".to_string());
        }
        if self.domain.is_none() {
            self.domain = Some("https://open.feishu.cn".to_string());
        }
        if self.reply_mode.is_none() {
            self.reply_mode = Some("mention_only".to_string());
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum BindMode {
    #[default]
    Loopback,
    Lan,
    Custom,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub bind: BindMode,
    #[serde(default)]
    pub custom_bind_host: Option<String>,
    #[serde(default)]
    pub max_connections: usize,
    #[serde(default)]
    pub rate_limit: RateLimitCfg,
    /// Allowed CORS origins. Empty = same-origin only (no cross-origin CORS headers).
    /// Use `["*"]` for permissive development mode.
    #[serde(default)]
    pub cors_origins: Vec<String>,
}

fn default_port() -> u16 {
    18789
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            bind: BindMode::Loopback,
            custom_bind_host: None,
            max_connections: 1024,
            rate_limit: RateLimitCfg::default(),
            cors_origins: Vec::new(),
        }
    }
}

impl GatewayConfig {
    pub fn bind_addr(&self) -> SocketAddr {
        match &self.bind {
            BindMode::Loopback => SocketAddr::from(([127, 0, 0, 1], self.port)),
            BindMode::Lan => SocketAddr::from(([0, 0, 0, 0], self.port)),
            BindMode::Custom => {
                if let Some(ref host) = self.custom_bind_host {
                    if let Ok(addr) = host.parse::<std::net::IpAddr>() {
                        return SocketAddr::from((addr, self.port));
                    }
                    tracing::warn!(host, "custom_bind_host is not a valid IP, falling back to loopback");
                }
                SocketAddr::from(([127, 0, 0, 1], self.port))
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitCfg {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_rate_max")]
    pub max_requests: u32,
    #[serde(default = "default_rate_window")]
    pub window_secs: u64,
    #[serde(default)]
    pub trusted_proxies: Vec<IpAddr>,
}

fn default_rate_max() -> u32 {
    60
}
fn default_rate_window() -> u64 {
    60
}

impl Default for RateLimitCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            max_requests: default_rate_max(),
            window_secs: default_rate_window(),
            trusted_proxies: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default = "default_log_format")]
    pub format: String,
}

fn default_log_level() -> String {
    "info".to_string()
}
fn default_log_format() -> String {
    "json".to_string()
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: default_log_format(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfig {
    #[serde(default)]
    pub ttl_hours: Option<u64>,
    #[serde(default)]
    pub dm_scope: Option<DmScope>,
    #[serde(default)]
    pub reset: Option<SessionResetConfig>,
    #[serde(default)]
    pub identity_links: Vec<IdentityLink>,
}

/// DM session isolation scope (from OpenClaw).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum DmScope {
    /// All DMs share one session (default, single-user).
    #[default]
    Main,
    /// Isolate by sender across channels.
    PerPeer,
    /// Isolate by channel + sender (recommended for multi-user).
    PerChannelPeer,
    /// Isolate by account + channel + sender.
    PerAccountChannelPeer,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionResetConfig {
    #[serde(default)]
    pub daily_hour: Option<u32>,
    #[serde(default)]
    pub idle_minutes: Option<u64>,
}

/// Link identities across channels so the same person shares one session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityLink {
    pub ids: Vec<String>,
}

/// Multi-agent configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentsConfig {
    #[serde(default)]
    pub defaults: AgentDefaults,
    #[serde(default)]
    pub list: Vec<AgentEntry>,
}

impl AgentsConfig {
    /// Return agent entries with defaults merged in (agent-level overrides take precedence).
    pub fn resolved_list(&self) -> Vec<AgentEntry> {
        self.list
            .iter()
            .map(|entry| {
                let mut e = entry.clone();
                if e.workspace.is_none() {
                    e.workspace = self.defaults.workspace.clone();
                }
                if e.model.is_none() {
                    e.model = self.defaults.model.clone();
                }
                if e.skills.is_none() {
                    e.skills = self.defaults.skills.clone();
                }
                e
            })
            .collect()
    }
}

/// Default settings applied to all agents unless overridden.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefaults {
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub skills: Option<Vec<String>>,
}

/// A single agent entry in the multi-agent config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEntry {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub agent_dir: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default = "default_false")]
    pub default: bool,
    #[serde(default)]
    pub identity: Option<AgentIdentity>,
    #[serde(default)]
    pub group_chat: Option<GroupChatConfig>,
    #[serde(default)]
    pub tools: Option<AgentToolsConfig>,
    #[serde(default)]
    pub skills: Option<Vec<String>>,
}

fn default_false() -> bool {
    false
}

/// Agent public identity for channel display.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentIdentity {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

/// Per-agent group chat behavior.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GroupChatConfig {
    #[serde(default)]
    pub mention_patterns: Vec<String>,
    #[serde(default)]
    pub require_mention: Option<bool>,
}

/// Per-agent tool allow/deny lists.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolsConfig {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default)]
    pub profile: Option<String>,
}

/// Binding: routes inbound messages to an agent by match criteria.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingConfig {
    pub agent_id: String,
    #[serde(rename = "match")]
    pub match_rule: BindingMatch,
}

/// Match criteria for a binding.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BindingMatch {
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub peer: Option<PeerMatch>,
}

/// Peer match (direct DM or group).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerMatch {
    pub kind: String,
    pub id: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub embedding: EmbeddingConfig,
    /// Seconds between automatic dream cycles over episodic memory (0 = disabled).
    #[serde(default = "default_dreaming_interval_secs")]
    pub dreaming_interval_secs: u64,
    /// Importance scoring weights for auto-consolidation and episode recording.
    #[serde(default)]
    pub importance: ImportanceScoringConfig,
    /// Minimum non-system messages before auto-consolidation fires (default 6).
    #[serde(default = "default_consolidation_min_messages")]
    pub consolidation_min_messages: usize,
    /// Optional model override for the consolidation summarisation call (fast/cheap model).
    #[serde(default)]
    pub consolidation_model: Option<String>,
}

fn default_dreaming_interval_secs() -> u64 {
    3600
}

fn default_consolidation_min_messages() -> usize {
    6
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            embedding: EmbeddingConfig::default(),
            dreaming_interval_secs: default_dreaming_interval_secs(),
            importance: ImportanceScoringConfig::default(),
            consolidation_min_messages: default_consolidation_min_messages(),
            consolidation_model: None,
        }
    }
}

/// Importance scoring weights — mirrors `fastclaw_memory::ImportanceScorer` fields
/// so users can override via JSON config without depending on the memory crate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportanceScoringConfig {
    #[serde(default = "default_w_015")]
    pub weight_length: f32,
    #[serde(default = "default_w_025")]
    pub weight_tool_calls: f32,
    #[serde(default = "default_w_030")]
    pub weight_keywords: f32,
    #[serde(default = "default_w_015")]
    pub weight_depth: f32,
    #[serde(default = "default_w_015")]
    pub weight_corrections: f32,
    #[serde(default = "default_min_threshold")]
    pub min_threshold: f32,
}

fn default_w_015() -> f32 {
    0.15
}
fn default_w_025() -> f32 {
    0.25
}
fn default_w_030() -> f32 {
    0.30
}
fn default_min_threshold() -> f32 {
    0.3
}

impl Default for ImportanceScoringConfig {
    fn default() -> Self {
        Self {
            weight_length: default_w_015(),
            weight_tool_calls: default_w_025(),
            weight_keywords: default_w_030(),
            weight_depth: default_w_015(),
            weight_corrections: default_w_015(),
            min_threshold: default_min_threshold(),
        }
    }
}

/// Embedding model configuration for memory vector search.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingConfig {
    /// Provider type: "local" (hypembed, pure Rust, default) or "remote" (OpenAI-compatible API).
    #[serde(default = "default_embedding_provider")]
    pub provider: String,
    /// Model name. For local: HuggingFace model ID (e.g. "sentence-transformers/all-MiniLM-L6-v2").
    /// For remote: API model name (e.g. "text-embedding-3-small").
    #[serde(default = "default_embedding_model")]
    pub model: String,
    /// Base URL for remote embedding API.
    #[serde(default)]
    pub base_url: Option<String>,
    /// API key for remote embedding. Falls back to credentials config.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Override embedding dimensions (default: auto-detected from model).
    #[serde(default)]
    pub dimensions: Option<u32>,
}

fn default_embedding_provider() -> String {
    "local".to_string()
}
fn default_embedding_model() -> String {
    "sentence-transformers/all-MiniLM-L6-v2".to_string()
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: default_embedding_provider(),
            model: default_embedding_model(),
            base_url: None,
            api_key: None,
            dimensions: None,
        }
    }
}

/// Policy for destructive / dangerous operations (file deletion, dangerous shell commands, etc.).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DangerousOpsPolicy {
    /// Block all dangerous operations outright.
    Deny,
    /// Allow without any confirmation.
    Allow,
    /// Pause and ask the user for confirmation before executing.
    #[default]
    Confirm,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SecurityConfig {
    #[serde(default)]
    pub prompt_injection_detection: bool,
    #[serde(default)]
    pub api_keys: Vec<String>,
    /// Hostnames (and optional :port) that bypass SSRF private-IP checks.
    /// Useful for local SearXNG, internal APIs, or MCP servers on localhost.
    /// Example: `["localhost:8888", "searxng.internal", "127.0.0.1:3000"]`
    #[serde(default)]
    pub ssrf_allowed_hosts: Vec<String>,
    /// How to handle destructive operations (rm, rmdir, chmod, etc.).
    /// `deny` = block outright, `allow` = let through, `confirm` = ask user first.
    #[serde(default)]
    pub dangerous_ops_policy: DangerousOpsPolicy,
    /// Regex patterns for shell commands considered "dangerous" (checked in `confirm`/`deny` modes).
    /// Defaults to patterns matching rm, rmdir, chmod, chown, mkfs, dd, etc.
    #[serde(default = "default_dangerous_patterns")]
    pub dangerous_patterns: Vec<String>,
}

fn default_dangerous_patterns() -> Vec<String> {
    vec![
        r"\brm\s".to_string(),
        r"\brm$".to_string(),
        r"\brmdir\b".to_string(),
        r"\bchmod\b".to_string(),
        r"\bchown\b".to_string(),
        r"\bmkfs\b".to_string(),
        r"\bdd\b".to_string(),
        r"\bshred\b".to_string(),
        r">\s*/dev/".to_string(),
    ]
}

/// Directory path overrides. All fields are optional;
/// when absent, the system uses built-in defaults.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PathsConfig {
    pub state_dir: Option<String>,
    pub db_path: Option<String>,
    pub plugins_dir: Option<String>,
    pub extensions_dir: Option<String>,
    pub skills_dir: Option<String>,
    pub agents_dir: Option<String>,
}

/// Centralized credential store: provider name → credential.
/// Replaces all environment variable-based API key lookups.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CredentialsConfig {
    #[serde(flatten)]
    pub providers: HashMap<String, ProviderCredential>,
}

impl CredentialsConfig {
    pub fn get_api_key(&self, provider: &str) -> Option<&str> {
        self.providers
            .get(provider)
            .and_then(|c| c.api_key.as_deref())
            .filter(|s| !s.is_empty())
    }

    pub fn get_base_url(&self, provider: &str) -> Option<&str> {
        self.providers
            .get(provider)
            .and_then(|c| c.base_url.as_deref())
            .filter(|s| !s.is_empty())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCredential {
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
}

/// Load configuration by deep-merging all found config layers.
///
/// Search order (highest priority first): project `config/` > `~/.fastclaw/` > `~/.openclaw/`.
/// Higher-priority values win; lower-priority layers fill in missing/empty fields (e.g.
/// credentials left blank in the project config are populated from the user-level config).
///
/// All configuration is loaded from JSON5 config files only — environment
/// variables are **not** used for configuration values.
pub fn load_config(mode: &ConfigMode) -> FastClawResult<FastClawConfig> {
    let (fastclaw_paths, legacy_paths) = build_search_paths(mode);

    // Deep-merge all fastclaw-native config layers (project + user-level).
    // Iterate in reverse so highest-priority paths overlay lower ones.
    let mut base: Option<serde_json::Value> = None;
    for path in fastclaw_paths.iter().rev() {
        if !path.exists() {
            continue;
        }
        tracing::info!(path = %path.display(), "loading config");
        let text = std::fs::read_to_string(path)?;
        let raw: serde_json::Value = match json5::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "config parse failed, attempting auto-repair");
                match try_repair_config(path) {
                    Ok(repaired) => {
                        let val: serde_json::Value = json5::from_str(&repaired)
                            .map_err(FastClawError::json5)?;
                        let pretty = serde_json::to_string_pretty(&val).unwrap_or(repaired);
                        let backup = path.with_extension("json.bak");
                        let _ = std::fs::copy(path, &backup);
                        if std::fs::write(path, &pretty).is_ok() {
                            tracing::info!(
                                path = %path.display(),
                                backup = %backup.display(),
                                "auto-repaired config (backup saved)"
                            );
                        }
                        val
                    }
                    Err(_) => return Err(FastClawError::json5(e)),
                }
            }
        };
        let strict_includes = raw
            .get("strictIncludes")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let processed = process_includes(raw, path.parent(), strict_includes)?;
        base = Some(match base {
            Some(existing) => deep_merge(existing, processed),
            None => processed,
        });
    }

    if let Some(merged) = base {
        warn_unknown_keys(&merged);
        let config: FastClawConfig = serde_json::from_value(merged)?;
        return Ok(config);
    }

    // Fallback: try legacy config paths (not merged, used as-is).
    for path in &legacy_paths {
        if !path.exists() {
            continue;
        }
        tracing::info!(path = %path.display(), "loading legacy config (fallback)");
        let loaded = (|| -> FastClawResult<FastClawConfig> {
            let text = std::fs::read_to_string(path)?;
            let raw: serde_json::Value = json5::from_str(&text).map_err(FastClawError::json5)?;
            let strict_includes = raw
                .get("strictIncludes")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let merged = process_includes(raw, path.parent(), strict_includes)?;
            warn_unknown_keys(&merged);
            let config: FastClawConfig = serde_json::from_value(merged)?;
            Ok(config)
        })();
        match loaded {
            Ok(config) => return Ok(config),
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "legacy config ignored because it is incompatible with FastClaw schema"
                );
            }
        }
    }

    tracing::info!("no config file found, using built-in defaults");
    Ok(FastClawConfig::default())
}

/// Attempt to repair a config file with common issues.
/// Returns `Ok(repaired_text)` if fixable, `Err` if not salvageable.
pub fn try_repair_config(path: &std::path::Path) -> Result<String, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;

    if json5::from_str::<serde_json::Value>(&text).is_ok() {
        return Err("config already valid, no repair needed".into());
    }

    let mut repaired = text.clone();

    // Strip BOM
    repaired = repaired.strip_prefix('\u{feff}').unwrap_or(&repaired).to_string();

    // Remove trailing commas before closing braces/brackets: ,\s*} or ,\s*]
    let trailing_comma_re = regex::Regex::new(r",(\s*[}\]])").unwrap();
    repaired = trailing_comma_re.replace_all(&repaired, "$1").to_string();

    // Try to fix unquoted keys by converting to strict JSON then back
    // (json5 is lenient about keys; this handles cases where the file is
    // almost-JSON but has issues json5 can't parse)
    if json5::from_str::<serde_json::Value>(&repaired).is_ok() {
        return Ok(repaired);
    }

    // Try wrapping bare content in braces if it looks like key-value pairs
    if !repaired.trim_start().starts_with('{') {
        let wrapped = format!("{{{repaired}}}");
        if json5::from_str::<serde_json::Value>(&wrapped).is_ok() {
            return Ok(wrapped);
        }
    }

    // Last resort: try serde_json strict parse after trailing-comma fix
    if serde_json::from_str::<serde_json::Value>(&repaired).is_ok() {
        return Ok(repaired);
    }

    Err(format!(
        "cannot auto-repair config at {}; manual editing required",
        path.display()
    ))
}

/// Attempt to repair and rewrite a config file in place.
/// Returns a description of what was fixed, or an error.
pub fn repair_config_file(path: &std::path::Path) -> Result<String, String> {
    let repaired = try_repair_config(path)?;

    // Validate the repaired content parses into a valid config
    let val: serde_json::Value = json5::from_str(&repaired)
        .map_err(|e| format!("repaired text still invalid: {e}"))?;

    // Re-serialize as pretty JSON for clean output
    let pretty = serde_json::to_string_pretty(&val)
        .map_err(|e| format!("failed to re-serialize: {e}"))?;

    // Back up the original
    let backup = path.with_extension("json.bak");
    let _ = std::fs::copy(path, &backup);

    std::fs::write(path, &pretty)
        .map_err(|e| format!("failed to write repaired config: {e}"))?;

    Ok(format!(
        "Config repaired and written to {}. Backup saved to {}.",
        path.display(),
        backup.display()
    ))
}

/// Known serde alias pairs: (canonical, alias). When both exist in a merged
/// object, keep only the canonical form to avoid "duplicate field" errors.
const ALIAS_PAIRS: &[(&str, &str)] = &[
    ("model", "defaultModel"),
    ("provider", "providerType"),
];

/// Recursively merge `overlay` on top of `base`. For objects, keys from `overlay`
/// override `base` unless the overlay value is null or an empty string.
fn deep_merge(base: serde_json::Value, overlay: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match (base, overlay) {
        (Value::Object(mut b), Value::Object(o)) => {
            for (key, o_val) in o {
                let merged = if let Some(b_val) = b.remove(&key) {
                    deep_merge(b_val, o_val)
                } else {
                    o_val
                };
                b.insert(key, merged);
            }
            deduplicate_aliases(&mut b);
            Value::Object(b)
        }
        (base, Value::Null) => base,
        (base, Value::String(ref s)) if s.is_empty() => base,
        (_base, overlay) => overlay,
    }
}

/// If both a canonical key and its alias exist in the same object, keep only
/// the canonical one (which is the overlay / higher-priority value).
fn deduplicate_aliases(map: &mut serde_json::Map<String, serde_json::Value>) {
    for &(canonical, alias) in ALIAS_PAIRS {
        if map.contains_key(canonical) && map.contains_key(alias) {
            map.remove(alias);
        }
    }
}

const MAX_INCLUDE_DEPTH: usize = 8;

fn process_includes(
    root: serde_json::Value,
    base_dir: Option<&std::path::Path>,
    strict_includes: bool,
) -> FastClawResult<serde_json::Value> {
    let mut seen = std::collections::HashSet::new();
    process_includes_inner(root, base_dir, strict_includes, 0, &mut seen)
}

fn process_includes_inner(
    mut root: serde_json::Value,
    base_dir: Option<&std::path::Path>,
    strict_includes: bool,
    depth: usize,
    seen_paths: &mut std::collections::HashSet<PathBuf>,
) -> FastClawResult<serde_json::Value> {
    if depth > MAX_INCLUDE_DEPTH {
        return Err(FastClawError::config(format!(
            "$include nesting too deep (max {MAX_INCLUDE_DEPTH})"
        )));
    }

    if let Some(inc) = root.get("$include").cloned() {
        let paths: Vec<String> = match inc {
            serde_json::Value::String(s) => vec![s],
            serde_json::Value::Array(arr) => arr
                .into_iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            _ => vec![],
        };
        root.as_object_mut().map(|o| o.remove("$include"));
        for p in paths {
            if p.contains("..") {
                tracing::error!(
                    path = %p,
                    depth = depth,
                    "$include: path traversal attempt blocked"
                );
                return Err(FastClawError::config(format!(
                    "$include path must not contain '..': {p}"
                )));
            }
            let inc_path = if let Some(base) = base_dir {
                base.join(&p)
            } else {
                PathBuf::from(&p)
            };
            if let Ok(canonical) = inc_path.canonicalize() {
                if let Some(base) = base_dir {
                    if let Ok(canon_base) = base.canonicalize() {
                        if !canonical.starts_with(&canon_base) {
                            tracing::error!(
                                path = %inc_path.display(),
                                canonical = %canonical.display(),
                                base = %canon_base.display(),
                                "$include: path escapes base directory — blocked"
                            );
                            return Err(FastClawError::config(format!(
                                "$include path escapes base directory: {}",
                                inc_path.display()
                            )));
                        }
                    }
                }
                if !seen_paths.insert(canonical.clone()) {
                    tracing::error!(
                        path = %canonical.display(),
                        depth = depth,
                        "$include: circular dependency detected"
                    );
                    return Err(FastClawError::config(format!(
                        "$include circular dependency detected: {}",
                        canonical.display()
                    )));
                }
            }
            if inc_path.exists() {
                let inc_text = std::fs::read_to_string(&inc_path)?;
                let inc_val: serde_json::Value =
                    json5::from_str(&inc_text).map_err(FastClawError::json5)?;
                let inc_base = inc_path.parent();
                let inc_val =
                    process_includes_inner(inc_val, inc_base, strict_includes, depth + 1, seen_paths)?;
                merge_json(&mut root, inc_val);
                tracing::info!(path = %inc_path.display(), depth, "merged $include config");
            } else if strict_includes {
                return Err(FastClawError::config(format!(
                    "$include file not found: {}",
                    inc_path.display()
                )));
            } else {
                tracing::warn!(path = %inc_path.display(), "$include file not found");
            }
        }
    }

    if let Some(obj) = root.as_object_mut() {
        let keys: Vec<String> = obj.keys().cloned().collect();
        for key in keys {
            if let Some(val) = obj.remove(&key) {
                let processed = process_includes_inner(
                    val, base_dir, strict_includes, depth, seen_paths,
                )?;
                obj.insert(key, processed);
            }
        }
    }

    Ok(root)
}

fn merge_json(base: &mut serde_json::Value, overlay: serde_json::Value) {
    match (base, overlay) {
        (serde_json::Value::Object(base_map), serde_json::Value::Object(overlay_map)) => {
            for (k, v) in overlay_map {
                merge_json(base_map.entry(k).or_insert(serde_json::Value::Null), v);
            }
        }
        (base, overlay) => *base = overlay,
    }
}

const KNOWN_TOP_KEYS: &[&str] = &[
    "$schema",
    "meta",
    "gateway",
    "logging",
    "session",
    "memory",
    "models",
    "modelRouter",
    "promptRouter",
    "plugins",
    "evolution",
    "channels",
    "security",
    "agents",
    "bindings",
    "tools",
    "workspace",
    "skills",
    "paths",
    "credentials",
    "webSearch",
    "strictIncludes",
    "mcpServers",
    "$include",
];

fn warn_unknown_keys(val: &serde_json::Value) {
    if let Some(obj) = val.as_object() {
        for key in obj.keys() {
            if !KNOWN_TOP_KEYS.contains(&key.as_str()) {
                tracing::warn!(key, "unknown config key (will be ignored)");
            }
        }
    }
}

/// Returns (fastclaw_paths, legacy_paths). Fastclaw paths are deep-merged;
/// legacy paths are used only as a standalone fallback.
fn build_search_paths(mode: &ConfigMode) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut fastclaw = Vec::new();
    let mut legacy = Vec::new();

    // 1. Local project config (highest priority)
    fastclaw.push(PathBuf::from("config/default.json"));

    // 2. Home directory user config
    if let Some(home) = dirs::home_dir() {
        let state_dir = match mode {
            ConfigMode::Development => home.join(".fastclaw-dev"),
            ConfigMode::Profile(name) => home.join(format!(".fastclaw-{name}")),
            ConfigMode::Production => home.join(".fastclaw"),
        };
        fastclaw.push(state_dir.join("config/default.json"));
    }

    // 3. OpenClaw compatibility (standalone fallback only — different schema)
    if let Some(home) = dirs::home_dir() {
        legacy.push(home.join(".openclaw/openclaw.json"));
    }

    (fastclaw, legacy)
}

#[cfg(test)]
mod model_provider_config_tests {
    use super::*;

    #[test]
    fn models_object_deserializes_to_typed_map_with_extra_fields() {
        let j = serde_json::json!({
            "models": {
                "openai": {
                    "providerType": "openai_compatible",
                    "baseUrl": "https://api.openai.com/v1",
                    "defaultModel": "gpt-4o",
                    "maxConcurrent": 10,
                    "timeoutSecs": 120
                }
            }
        });
        let cfg: FastClawConfig = serde_json::from_value(j).expect("parse config");
        let openai = cfg.models.get("openai").expect("openai entry");
        assert_eq!(openai.provider, "openai_compatible");
        assert_eq!(openai.model, "gpt-4o");
        assert_eq!(
            openai.base_url.as_deref(),
            Some("https://api.openai.com/v1")
        );
        assert_eq!(
            openai.extra.get("maxConcurrent").and_then(|v| v.as_u64()),
            Some(10)
        );
        assert_eq!(
            openai.extra.get("timeoutSecs").and_then(|v| v.as_u64()),
            Some(120)
        );
    }

    #[test]
    fn models_empty_object_defaults() {
        let j = serde_json::json!({ "models": {} });
        let cfg: FastClawConfig = serde_json::from_value(j).unwrap();
        assert!(cfg.models.is_empty());
    }
}

#[cfg(test)]
mod memory_config_tests {
    use super::*;

    #[test]
    fn dreaming_interval_defaults() {
        let j = serde_json::json!({ "memory": { "enabled": true } });
        let cfg: FastClawConfig = serde_json::from_value(j).unwrap();
        assert!(cfg.memory.enabled);
        assert_eq!(cfg.memory.dreaming_interval_secs, 3600);
    }

    #[test]
    fn dreaming_interval_override() {
        let j = serde_json::json!({ "memory": { "dreamingIntervalSecs": 120 } });
        let cfg: FastClawConfig = serde_json::from_value(j).unwrap();
        assert_eq!(cfg.memory.dreaming_interval_secs, 120);
    }
}
