//! Browser-specific network configuration (proxy mode, host mappings).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{HostMapping, validate_host_mapping};

/// How browser WebViews route outbound traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BrowserProxyMode {
    /// Direct connection — no WebView proxy.
    None,
    /// Follow OS / environment proxy settings.
    System,
    /// User-provided proxy URL on the WebView.
    Custom,
    /// Route through XiaoLin built-in loopback proxy (default).
    #[default]
    XiaolinProxy,
}

/// Persisted browser network settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserNetworkConfig {
    #[serde(default)]
    pub proxy_mode: BrowserProxyMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_proxy_url: Option<String>,
    /// Upstream proxy when `proxy_mode` is XiaolinProxy (hot-swappable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_proxy_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub host_mappings: Vec<HostMapping>,
    /// Agent-approved temporary mappings (cleared on session end).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_host_mappings: Vec<HostMapping>,
}

impl Default for BrowserNetworkConfig {
    fn default() -> Self {
        Self {
            proxy_mode: BrowserProxyMode::XiaolinProxy,
            custom_proxy_url: None,
            upstream_proxy_url: None,
            host_mappings: Vec::new(),
            session_host_mappings: Vec::new(),
        }
    }
}

impl BrowserNetworkConfig {
    /// All mappings effective for proxy DNS rewrite (persistent + session).
    pub fn effective_host_mappings(&self) -> Vec<HostMapping> {
        let mut all = self.host_mappings.clone();
        all.extend(self.session_host_mappings.clone());
        all
    }

    pub fn set_persistent_hosts(&mut self, mappings: Vec<HostMapping>) -> Result<(), String> {
        for m in &mappings {
            validate_host_mapping(m)?;
        }
        self.host_mappings = mappings;
        Ok(())
    }

    pub fn set_session_hosts(&mut self, mappings: Vec<HostMapping>) -> Result<(), String> {
        for m in &mappings {
            validate_host_mapping(m)?;
        }
        self.session_host_mappings = mappings;
        Ok(())
    }

    pub fn clear_session_hosts(&mut self) {
        self.session_host_mappings.clear();
    }

    pub fn validate(&self) -> Result<(), String> {
        for m in self.host_mappings.iter().chain(self.session_host_mappings.iter()) {
            validate_host_mapping(m)?;
        }
        if let Some(url) = &self.custom_proxy_url {
            validate_proxy_url(url)?;
        }
        if let Some(url) = &self.upstream_proxy_url {
            validate_proxy_url(url)?;
        }
        Ok(())
    }
}

/// Default config file location: `{data_dir}/xiaolin/browser-network.json`.
pub fn default_config_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("xiaolin")
        .join("browser-network.json")
}

pub fn load_config(path: Option<&Path>) -> BrowserNetworkConfig {
    let default_path = default_config_path();
    let path = path.unwrap_or(&default_path);
    match std::fs::read_to_string(path) {
        Ok(raw) => match serde_json::from_str(&raw) {
            Ok(cfg) => cfg,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "invalid browser network config; using defaults");
                BrowserNetworkConfig::default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => BrowserNetworkConfig::default(),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "failed to read browser network config");
            BrowserNetworkConfig::default()
        }
    }
}

pub fn save_config(config: &BrowserNetworkConfig, path: Option<&Path>) -> Result<(), String> {
    config.validate()?;
    let default_path = default_config_path();
    let path = path.unwrap_or(&default_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("failed to create config directory: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("failed to serialize browser network config: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("failed to write browser network config: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn validate_proxy_url(url: &str) -> Result<(), String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err("proxy URL must not be empty".to_string());
    }
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("http://")
        && !lower.starts_with("https://")
        && !lower.starts_with("socks5://")
        && !lower.starts_with("socks5h://")
    {
        return Err("proxy URL must use http://, https://, or socks5:// scheme".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mode_is_xiaolin_proxy() {
        let cfg = BrowserNetworkConfig::default();
        assert_eq!(cfg.proxy_mode, BrowserProxyMode::XiaolinProxy);
    }

    #[test]
    fn effective_mappings_merge_session() {
        let mut cfg = BrowserNetworkConfig::default();
        cfg.host_mappings = vec![HostMapping::new("a.com", "1.2.3.4")];
        cfg.session_host_mappings = vec![HostMapping::new("b.com", "5.6.7.8")];
        assert_eq!(cfg.effective_host_mappings().len(), 2);
    }

    #[test]
    fn rejects_loopback_mapping() {
        let mut cfg = BrowserNetworkConfig::default();
        let err = cfg
            .set_persistent_hosts(vec![HostMapping::new("evil.com", "127.0.0.1")])
            .unwrap_err();
        assert!(err.contains("loopback"));
    }

    #[test]
    fn json_roundtrip() {
        let mut cfg = BrowserNetworkConfig::default();
        cfg.upstream_proxy_url = Some("http://proxy:8080".into());
        cfg.host_mappings = vec![HostMapping::new("*.dev.local", "10.0.0.1")];
        let json = serde_json::to_string(&cfg).unwrap();
        let back: BrowserNetworkConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cfg);
    }
}
