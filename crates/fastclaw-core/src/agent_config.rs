use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::complexity::ComplexityTier;
use crate::config::ChannelConfig;
use crate::types::AgentId;

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
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
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
    "bailian".to_string()
}
fn default_model() -> String {
    "qwen3.5-plus".to_string()
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BehaviorConfig {
    #[serde(default = "default_max_tool_calls")]
    pub max_tool_calls_per_turn: u32,
    #[serde(default = "default_max_errors")]
    pub max_consecutive_errors: u32,
    #[serde(default)]
    pub require_confirmation_for: Vec<String>,
    #[serde(default)]
    pub tools_allow: Vec<String>,
    #[serde(default)]
    pub tools_deny: Vec<String>,
    #[serde(default)]
    pub file_access: FileAccessMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileAccessMode {
    None,
    Workspace,
    Full,
}

impl Default for FileAccessMode {
    fn default() -> Self {
        Self::Workspace
    }
}

fn default_max_tool_calls() -> u32 {
    50
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
            tools_allow: Vec::new(),
            tools_deny: Vec::new(),
            file_access: FileAccessMode::default(),
        }
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
    if negated { !base_match } else { base_match }
}

impl BehaviorConfig {
    /// Check whether a tool is allowed by this agent's allow/deny policy.
    /// Supports glob patterns (`mcp_*`) and negation (`!shell_exec`).
    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        if !self.tools_deny.is_empty()
            && self.tools_deny.iter().any(|d| tool_pattern_matches(d, tool_name))
        {
            return false;
        }
        if !self.tools_allow.is_empty()
            && !self.tools_allow.iter().any(|a| tool_pattern_matches(a, tool_name))
        {
            return false;
        }
        true
    }
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
        if path.extension().map_or(false, |e| e == "json") {
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
        assert!(tool_pattern_matches("mcp_chrome_*", "mcp_chrome_screenshot"));
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
            tools_allow: vec![
                "http_fetch".into(),
                "web_search".into(),
                "mcp_*".into(),
            ],
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
}
