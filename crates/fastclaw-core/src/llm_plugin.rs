use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::types::ModelCapabilities;

/// Top-level configuration for an LLM provider plugin.
///
/// Plugins live as individual JSON files inside the plugins directory
/// (e.g. `~/.fastclaw/plugins/llm/<id>.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmPluginConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "type")]
    pub plugin_type: LlmPluginType,
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Present when `plugin_type == Middleware`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub middleware: Option<MiddlewareConfig>,

    /// Present when `plugin_type == Process`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<ProcessPluginConfig>,

    /// Models this plugin exposes. Shown in the frontend model selector.
    #[serde(default)]
    pub models: Vec<LlmPluginModelEntry>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmPluginType {
    Middleware,
    Process,
}

// ---------------------------------------------------------------------------
// Middleware plugin
// ---------------------------------------------------------------------------

/// Configuration for middleware-mode plugins that wrap an existing
/// OpenAI/Anthropic-compatible endpoint with custom headers, auth, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MiddlewareConfig {
    pub base_url: String,

    /// Which wire protocol the upstream speaks.
    #[serde(default)]
    pub protocol: LlmProtocol,

    /// Static headers injected into every request.
    #[serde(default)]
    pub headers: HashMap<String, String>,

    /// Authentication strategy.
    #[serde(default)]
    pub auth: AuthConfig,

    /// Model name remapping: key = local name → value = upstream name.
    #[serde(default)]
    pub model_mapping: HashMap<String, String>,

    /// Retry / timeout overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LlmProtocol {
    #[default]
    Openai,
    Anthropic,
}

// ---------------------------------------------------------------------------
// Auth strategies
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthConfig {
    /// No authentication.
    None,
    /// Static Bearer token in the Authorization header.
    #[serde(rename_all = "camelCase")]
    BearerToken {
        token: String,
    },
    /// A single custom header with a static value.
    #[serde(rename_all = "camelCase")]
    CustomHeader {
        header: String,
        value: String,
    },
    /// OAuth2 Client Credentials grant — acquires an access_token from
    /// `token_endpoint` and caches it until expiry.
    #[serde(rename = "oauth2_client_credentials", rename_all = "camelCase")]
    OAuth2ClientCredentials {
        token_endpoint: String,
        client_id: String,
        client_secret: String,
        #[serde(default)]
        scope: Option<String>,
        /// Header name for the token (default: `Authorization`).
        #[serde(default = "default_auth_header")]
        token_header: String,
        /// Prefix before the token value (default: `Bearer`).
        #[serde(default = "default_token_prefix")]
        token_prefix: String,
    },
    /// Call an HTTP endpoint before each LLM request and extract a token
    /// from the JSON response (e.g. internal auth services).
    #[serde(rename_all = "camelCase")]
    PreRequestHook {
        url: String,
        #[serde(default = "default_post")]
        method: String,
        /// Optional static JSON body to send.
        #[serde(default)]
        body: Option<serde_json::Value>,
        /// Optional extra headers for the auth request.
        #[serde(default)]
        headers: HashMap<String, String>,
        /// JSONPath-like dot-separated key to extract from the response.
        /// e.g. `"data.accessToken"` extracts `response["data"]["accessToken"]`.
        #[serde(default = "default_extract_path")]
        extract_path: String,
        /// Header name to inject the extracted value into.
        #[serde(default = "default_auth_header")]
        token_header: String,
        /// Prefix before the extracted value (default: `Bearer`).
        #[serde(default = "default_token_prefix")]
        token_prefix: String,
        /// Cache TTL in seconds for the extracted token. 0 = no caching.
        #[serde(default)]
        cache_ttl_secs: u64,
    },
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self::None
    }
}

fn default_auth_header() -> String {
    "Authorization".to_string()
}
fn default_token_prefix() -> String {
    "Bearer".to_string()
}
fn default_post() -> String {
    "POST".to_string()
}
fn default_extract_path() -> String {
    "access_token".to_string()
}

// ---------------------------------------------------------------------------
// Process plugin
// ---------------------------------------------------------------------------

/// Configuration for process-mode plugins that run an external executable
/// implementing the full LLM provider protocol over stdio or HTTP.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessPluginConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub transport: ProcessTransport,
    /// For HTTP transport: the base URL the process listens on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProcessTransport {
    #[default]
    Stdio,
    Http,
}

// ---------------------------------------------------------------------------
// Model entries
// ---------------------------------------------------------------------------

/// A model exposed by the plugin, surfaced in the frontend model selector.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmPluginModelEntry {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub context_window: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<ModelCapabilities>,
}

// ---------------------------------------------------------------------------
// Top-level config section
// ---------------------------------------------------------------------------

/// Section inside `FastClawConfig` governing LLM provider plugins.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmPluginsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Override for the plugins directory path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugins_dir: Option<String>,
}

impl Default for LlmPluginsConfig {
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

/// Scan `dir` for `*.json` files and parse each as an `LlmPluginConfig`.
/// Invalid files are logged and skipped.
pub fn load_llm_plugins(dir: &Path) -> Vec<LlmPluginConfig> {
    let mut plugins = Vec::new();

    if !dir.exists() {
        tracing::debug!(path = %dir.display(), "LLM plugins directory does not exist, skipping");
        return plugins;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(path = %dir.display(), error = %e, "failed to read LLM plugins directory");
            return plugins;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "failed to read directory entry in LLM plugins dir");
                continue;
            }
        };
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            match std::fs::read_to_string(&path) {
                Ok(text) => match json5::from_str::<LlmPluginConfig>(&text) {
                    Ok(cfg) => {
                        if cfg.id.is_empty() {
                            tracing::warn!(path = %path.display(), "LLM plugin has empty id, skipping");
                            continue;
                        }
                        tracing::info!(
                            plugin_id = %cfg.id,
                            plugin_type = ?cfg.plugin_type,
                            enabled = cfg.enabled,
                            models = cfg.models.len(),
                            path = %path.display(),
                            "loaded LLM provider plugin"
                        );
                        plugins.push(cfg);
                    }
                    Err(e) => {
                        tracing::warn!(path = %path.display(), error = %e, "failed to parse LLM plugin config");
                    }
                },
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "failed to read LLM plugin file");
                }
            }
        }
    }

    plugins
}

/// Resolve the LLM plugins directory, preferring the config override.
pub fn resolve_plugins_dir(
    plugins_config: &LlmPluginsConfig,
    paths_config: &crate::config::PathsConfig,
) -> std::path::PathBuf {
    if let Some(ref dir) = plugins_config.plugins_dir {
        return std::path::PathBuf::from(dir);
    }
    if let Some(ref plugins_dir) = paths_config.plugins_dir {
        return std::path::PathBuf::from(plugins_dir).join("llm");
    }
    let state_dir = crate::paths::resolve_state_dir_from(Some(paths_config));
    state_dir.join("plugins").join("llm")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_middleware_plugin() {
        let json = r#"{
            "id": "corp-gw",
            "name": "Corp Gateway",
            "version": "1.0",
            "type": "middleware",
            "middleware": {
                "baseUrl": "https://gw.corp.example.com/v1",
                "protocol": "openai",
                "headers": { "x-app": "fastclaw" },
                "auth": {
                    "type": "oauth2_client_credentials",
                    "tokenEndpoint": "https://auth.corp.example.com/token",
                    "clientId": "cid",
                    "clientSecret": "csec",
                    "scope": "llm:invoke"
                },
                "modelMapping": { "gpt-4o": "corp-gpt4" }
            },
            "models": [
                { "id": "corp-gpt4", "name": "Corp GPT-4", "contextWindow": 128000 }
            ]
        }"#;

        let cfg: LlmPluginConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.id, "corp-gw");
        assert_eq!(cfg.plugin_type, LlmPluginType::Middleware);
        assert!(cfg.enabled);
        let mw = cfg.middleware.unwrap();
        assert_eq!(mw.base_url, "https://gw.corp.example.com/v1");
        assert_eq!(mw.protocol, LlmProtocol::Openai);
        assert_eq!(mw.headers.get("x-app").unwrap(), "fastclaw");
        assert_eq!(mw.model_mapping.get("gpt-4o").unwrap(), "corp-gpt4");
        match mw.auth {
            AuthConfig::OAuth2ClientCredentials { ref client_id, .. } => {
                assert_eq!(client_id, "cid");
            }
            _ => panic!("expected OAuth2ClientCredentials"),
        }
        assert_eq!(cfg.models.len(), 1);
        assert_eq!(cfg.models[0].id, "corp-gpt4");
    }

    #[test]
    fn deserialize_process_plugin() {
        let json = r#"{
            "id": "custom-llm",
            "name": "Custom Provider",
            "type": "process",
            "process": {
                "command": "python3",
                "args": ["provider.py"],
                "transport": "stdio"
            },
            "models": []
        }"#;

        let cfg: LlmPluginConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.plugin_type, LlmPluginType::Process);
        let proc = cfg.process.unwrap();
        assert_eq!(proc.command, "python3");
        assert_eq!(proc.args, vec!["provider.py"]);
        assert_eq!(proc.transport, ProcessTransport::Stdio);
    }

    #[test]
    fn deserialize_custom_header_auth() {
        let json = r#"{
            "id": "hdr",
            "name": "Header Auth",
            "type": "middleware",
            "middleware": {
                "baseUrl": "https://api.example.com",
                "auth": {
                    "type": "custom_header",
                    "header": "x-api-key",
                    "value": "secret123"
                }
            }
        }"#;

        let cfg: LlmPluginConfig = serde_json::from_str(json).unwrap();
        let mw = cfg.middleware.unwrap();
        match mw.auth {
            AuthConfig::CustomHeader { ref header, ref value } => {
                assert_eq!(header, "x-api-key");
                assert_eq!(value, "secret123");
            }
            _ => panic!("expected CustomHeader"),
        }
    }

    #[test]
    fn deserialize_pre_request_hook_auth() {
        let json = r#"{
            "id": "hook",
            "name": "Pre-request Hook",
            "type": "middleware",
            "middleware": {
                "baseUrl": "https://api.example.com",
                "auth": {
                    "type": "pre_request_hook",
                    "url": "https://auth.internal/token",
                    "method": "POST",
                    "body": {"grant_type": "client_credentials"},
                    "extractPath": "data.token",
                    "cacheTtlSecs": 300
                }
            }
        }"#;

        let cfg: LlmPluginConfig = serde_json::from_str(json).unwrap();
        let mw = cfg.middleware.unwrap();
        match mw.auth {
            AuthConfig::PreRequestHook { ref url, cache_ttl_secs, ref extract_path, .. } => {
                assert_eq!(url, "https://auth.internal/token");
                assert_eq!(cache_ttl_secs, 300);
                assert_eq!(extract_path, "data.token");
            }
            _ => panic!("expected PreRequestHook"),
        }
    }

    #[test]
    fn load_llm_plugins_skips_missing_dir() {
        let result = load_llm_plugins(Path::new("/nonexistent/path"));
        assert!(result.is_empty());
    }

    #[test]
    fn default_auth_is_none() {
        let json = r#"{
            "id": "no-auth",
            "name": "No Auth",
            "type": "middleware",
            "middleware": { "baseUrl": "https://api.example.com" }
        }"#;
        let cfg: LlmPluginConfig = serde_json::from_str(json).unwrap();
        let mw = cfg.middleware.unwrap();
        assert!(matches!(mw.auth, AuthConfig::None));
    }
}
