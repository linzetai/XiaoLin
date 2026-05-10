//! Channel plugin configuration and loader.
//!
//! Channel plugins allow FastClaw to integrate with external messaging platforms
//! (Feishu, Slack, Discord, etc.) via external processes communicating over
//! JSON-RPC. This module provides the configuration schema and loader for
//! process-based channel plugins.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Channel Plugin Configuration
// ---------------------------------------------------------------------------

/// Top-level configuration for a channel plugin.
///
/// Plugins live as individual JSON files inside the plugins directory
/// (e.g. `~/.fastclaw/plugins/channel/<id>.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelPluginConfig {
    /// Unique channel identifier (e.g., "feishu", "slack").
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Optional version string.
    #[serde(default)]
    pub version: Option<String>,
    /// Optional description.
    #[serde(default)]
    pub description: Option<String>,
    /// Whether this plugin is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(rename = "type")]
    pub plugin_type: ChannelPluginType,

    /// Present when `plugin_type == Process`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<ProcessChannelConfig>,

    /// Tools this channel exposes (declarative, for discovery).
    #[serde(default)]
    pub tools: Vec<ChannelToolDef>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelPluginType {
    Process,
}

// ---------------------------------------------------------------------------
// Process Channel Configuration
// ---------------------------------------------------------------------------

/// Configuration for process-mode channel plugins that run an external executable
/// implementing the channel plugin protocol over stdio or HTTP.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessChannelConfig {
    /// Command to spawn (e.g., "node", "python3").
    pub command: String,
    /// Arguments to pass to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables to set for the process.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Transport protocol.
    #[serde(default)]
    pub transport: ProcessTransport,
    /// Request timeout in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// Maximum memory the process can use (MB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_memory_mb: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProcessTransport {
    #[default]
    Stdio,
    Http,
}

// ---------------------------------------------------------------------------
// Tool Definition
// ---------------------------------------------------------------------------

/// A tool exposed by a channel plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelToolDef {
    /// Tool name (e.g., "feishu_send_message").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema for parameters.
    pub parameters: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Config Section
// ---------------------------------------------------------------------------

/// Section inside `FastClawConfig` governing channel plugins.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelPluginsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Override for the plugins directory path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugins_dir: Option<String>,
}

impl Default for ChannelPluginsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            plugins_dir: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Loader
// ---------------------------------------------------------------------------

/// Scan `dir` for `*.json` files and parse each as a `ChannelPluginConfig`.
/// Invalid files are logged and skipped.
pub fn load_channel_plugins(dir: &Path) -> Vec<ChannelPluginConfig> {
    let mut plugins = Vec::new();

    if !dir.exists() {
        tracing::debug!(path = %dir.display(), "Channel plugins directory does not exist, skipping");
        return plugins;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(path = %dir.display(), error = %e, "failed to read channel plugins directory");
            return plugins;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "failed to read directory entry in channel plugins dir");
                continue;
            }
        };
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            match std::fs::read_to_string(&path) {
                Ok(text) => match json5::from_str::<ChannelPluginConfig>(&text) {
                    Ok(cfg) => {
                        if cfg.id.is_empty() {
                            tracing::warn!(path = %path.display(), "Channel plugin has empty id, skipping");
                            continue;
                        }
                        tracing::info!(
                            plugin_id = %cfg.id,
                            plugin_type = ?cfg.plugin_type,
                            enabled = cfg.enabled,
                            tools = cfg.tools.len(),
                            path = %path.display(),
                            "loaded channel plugin"
                        );
                        plugins.push(cfg);
                    }
                    Err(e) => {
                        tracing::warn!(path = %path.display(), error = %e, "failed to parse channel plugin config");
                    }
                },
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "failed to read channel plugin file");
                }
            }
        }
    }

    plugins
}

/// Resolve the channel plugins directory, preferring the config override.
pub fn resolve_channel_plugins_dir(
    plugins_config: &ChannelPluginsConfig,
    paths_config: &crate::config::PathsConfig,
) -> std::path::PathBuf {
    if let Some(ref dir) = plugins_config.plugins_dir {
        return std::path::PathBuf::from(dir);
    }
    if let Some(ref plugins_dir) = paths_config.plugins_dir {
        return std::path::PathBuf::from(plugins_dir).join("channel");
    }
    let state_dir = crate::paths::resolve_state_dir_from(Some(paths_config));
    state_dir.join("plugins").join("channel")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_process_channel_plugin() {
        let json = r#"{
            "id": "feishu",
            "name": "Feishu/Lark",
            "version": "0.1.0",
            "type": "process",
            "process": {
                "command": "node",
                "args": ["dist/index.js"],
                "env": { "LOG_LEVEL": "info" },
                "timeoutSecs": 30
            },
            "tools": [
                {
                    "name": "feishu_send_message",
                    "description": "Send a message",
                    "parameters": { "type": "object" }
                }
            ]
        }"#;

        let cfg: ChannelPluginConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.id, "feishu");
        assert_eq!(cfg.plugin_type, ChannelPluginType::Process);
        assert!(cfg.enabled);
        let proc = cfg.process.unwrap();
        assert_eq!(proc.command, "node");
        assert_eq!(proc.args, vec!["dist/index.js"]);
        assert_eq!(cfg.tools.len(), 1);
        assert_eq!(cfg.tools[0].name, "feishu_send_message");
    }

    #[test]
    fn load_channel_plugins_skips_missing_dir() {
        let result = load_channel_plugins(Path::new("/nonexistent/path"));
        assert!(result.is_empty());
    }
}
