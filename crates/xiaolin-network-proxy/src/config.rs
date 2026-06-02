use std::net::SocketAddr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Top-level proxy configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkProxyConfig {
    pub settings: NetworkProxySettings,
}

/// Permission for a single domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkDomainPermission {
    None,
    Allow,
    Deny,
}

/// A domain-permission entry: a host pattern paired with a permission.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NetworkDomainPermissionEntry {
    pub host: String,
    pub permission: NetworkDomainPermission,
}

/// Collection of per-domain network permissions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkDomainPermissions {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<NetworkDomainPermissionEntry>,
}

impl NetworkDomainPermissions {
    pub fn effective_entries(&self) -> Vec<&NetworkDomainPermissionEntry> {
        self.entries
            .iter()
            .filter(|e| e.permission != NetworkDomainPermission::None)
            .collect()
    }
}

/// Permission for Unix socket access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkUnixSocketPermission {
    None,
    Allow,
    Deny,
}

/// Collection of Unix socket permissions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkUnixSocketPermissions {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<UnixSocketPermissionEntry>,
}

/// A single Unix socket permission entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UnixSocketPermissionEntry {
    pub path: PathBuf,
    pub permission: NetworkUnixSocketPermission,
}

/// Network operation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkMode {
    Limited,
    Full,
    Audit,
    Off,
}

impl NetworkMode {
    pub fn allows_method(&self, method: &str) -> bool {
        match self {
            Self::Full | Self::Audit | Self::Off => true,
            Self::Limited => matches!(
                method.to_uppercase().as_str(),
                "GET" | "HEAD" | "OPTIONS"
            ),
        }
    }
}

/// User-facing proxy settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkProxySettings {
    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,

    #[serde(default)]
    pub enable_socks5: bool,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socks_url: Option<String>,

    #[serde(default)]
    pub mode: Option<NetworkMode>,

    #[serde(default)]
    pub domains: NetworkDomainPermissions,

    #[serde(default)]
    pub unix_sockets: NetworkUnixSocketPermissions,

    #[serde(default)]
    pub allow_local_binding: bool,

    #[serde(default)]
    pub mitm: bool,
}

fn default_true() -> bool {
    true
}

impl Default for NetworkProxySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            proxy_url: None,
            enable_socks5: false,
            socks_url: None,
            mode: None,
            domains: NetworkDomainPermissions::default(),
            unix_sockets: NetworkUnixSocketPermissions::default(),
            allow_local_binding: false,
            mitm: false,
        }
    }
}

impl NetworkProxySettings {
    pub fn allowed_domains(&self) -> Vec<&str> {
        self.domains
            .entries
            .iter()
            .filter(|e| e.permission == NetworkDomainPermission::Allow)
            .map(|e| e.host.as_str())
            .collect()
    }

    pub fn denied_domains(&self) -> Vec<&str> {
        self.domains
            .entries
            .iter()
            .filter(|e| e.permission == NetworkDomainPermission::Deny)
            .map(|e| e.host.as_str())
            .collect()
    }

    pub fn allow_unix_sockets(&self) -> Vec<&PathBuf> {
        self.unix_sockets
            .entries
            .iter()
            .filter(|e| e.permission == NetworkUnixSocketPermission::Allow)
            .map(|e| &e.path)
            .collect()
    }

    pub fn set_allowed_domains(&mut self, hosts: Vec<String>) {
        self.domains.entries.retain(|e| e.permission != NetworkDomainPermission::Allow);
        for host in hosts {
            self.domains.entries.push(NetworkDomainPermissionEntry {
                host,
                permission: NetworkDomainPermission::Allow,
            });
        }
    }

    pub fn set_denied_domains(&mut self, hosts: Vec<String>) {
        self.domains.entries.retain(|e| e.permission != NetworkDomainPermission::Deny);
        for host in hosts {
            self.domains.entries.push(NetworkDomainPermissionEntry {
                host,
                permission: NetworkDomainPermission::Deny,
            });
        }
    }

    pub fn upsert_domain_permission(&mut self, host: String, permission: NetworkDomainPermission) {
        if let Some(entry) = self.domains.entries.iter_mut().find(|e| e.host == host) {
            entry.permission = permission;
        } else {
            self.domains.entries.push(NetworkDomainPermissionEntry {
                host,
                permission,
            });
        }
    }

    pub fn set_allow_unix_sockets(&mut self, paths: Vec<PathBuf>) {
        self.unix_sockets
            .entries
            .retain(|e| e.permission != NetworkUnixSocketPermission::Allow);
        for path in paths {
            self.unix_sockets.entries.push(UnixSocketPermissionEntry {
                path,
                permission: NetworkUnixSocketPermission::Allow,
            });
        }
    }
}

/// Resolved runtime address information.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub http_addr: Option<SocketAddr>,
    pub socks_addr: Option<SocketAddr>,
}

/// Extract host and port from a network address string.
///
/// Handles formats like `host:port`, `[ipv6]:port`, and plain hostnames.
pub fn host_and_port_from_network_addr(addr: &str) -> Option<(String, u16)> {
    parse_host_port(addr).or_else(|| parse_host_port_fallback(addr))
}

fn parse_host_port(addr: &str) -> Option<(String, u16)> {
    if let Ok(url) = url::Url::parse(&format!("scheme://{addr}")) {
        let host = url.host_str()?;
        let host = host.trim_start_matches('[').trim_end_matches(']').to_string();
        let port = url.port()?;
        return Some((host, port));
    }
    None
}

fn parse_host_port_fallback(addr: &str) -> Option<(String, u16)> {
    if let Some((host, port_str)) = addr.rsplit_once(':') {
        if let Ok(port) = port_str.parse::<u16>() {
            let host = host.trim_start_matches('[').trim_end_matches(']');
            return Some((host.to_string(), port));
        }
    }
    None
}

/// Clamp a list of bind addresses to loopback only.
pub fn clamp_bind_addrs(addrs: &[SocketAddr]) -> Vec<SocketAddr> {
    addrs
        .iter()
        .filter(|a| a.ip().is_loopback())
        .copied()
        .collect()
}

/// A validated Unix socket path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ValidatedUnixSocketPath {
    pub path: PathBuf,
}

impl ValidatedUnixSocketPath {
    pub fn new(path: PathBuf) -> Result<Self, String> {
        if !path.is_absolute() {
            return Err(format!(
                "unix socket path must be absolute: {}",
                path.display()
            ));
        }
        Ok(Self { path })
    }
}

/// Validate a list of Unix socket paths.
pub fn validate_unix_socket_allowlist_paths(
    paths: &[PathBuf],
) -> Result<Vec<ValidatedUnixSocketPath>, String> {
    paths.iter().map(|p| ValidatedUnixSocketPath::new(p.clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings() {
        let settings = NetworkProxySettings::default();
        assert!(settings.enabled);
        assert!(settings.proxy_url.is_none());
        assert!(!settings.enable_socks5);
        assert!(!settings.allow_local_binding);
    }

    #[test]
    fn network_mode_allows_method() {
        assert!(NetworkMode::Full.allows_method("POST"));
        assert!(NetworkMode::Full.allows_method("DELETE"));
        assert!(NetworkMode::Limited.allows_method("GET"));
        assert!(NetworkMode::Limited.allows_method("HEAD"));
        assert!(!NetworkMode::Limited.allows_method("POST"));
        assert!(!NetworkMode::Limited.allows_method("PUT"));
    }

    #[test]
    fn host_and_port_parsing() {
        assert_eq!(
            host_and_port_from_network_addr("example.com:8080"),
            Some(("example.com".into(), 8080))
        );
        assert_eq!(
            host_and_port_from_network_addr("[::1]:443"),
            Some(("::1".into(), 443))
        );
        assert_eq!(host_and_port_from_network_addr("no-port"), None);
    }

    #[test]
    fn clamp_bind_addrs_filters_non_loopback() {
        let addrs = vec![
            "127.0.0.1:8080".parse().unwrap(),
            "192.168.1.1:8080".parse().unwrap(),
            "[::1]:8080".parse().unwrap(),
        ];
        let clamped = clamp_bind_addrs(&addrs);
        assert_eq!(clamped.len(), 2);
        assert!(clamped.iter().all(|a| a.ip().is_loopback()));
    }

    #[test]
    fn validated_unix_socket_path_rejects_relative() {
        assert!(ValidatedUnixSocketPath::new(PathBuf::from("relative/path")).is_err());
    }

    #[test]
    fn validated_unix_socket_path_accepts_absolute() {
        assert!(ValidatedUnixSocketPath::new(PathBuf::from("/tmp/socket.sock")).is_ok());
    }

    #[test]
    fn domain_permissions_effective_entries() {
        let perms = NetworkDomainPermissions {
            entries: vec![
                NetworkDomainPermissionEntry {
                    host: "allow.com".into(),
                    permission: NetworkDomainPermission::Allow,
                },
                NetworkDomainPermissionEntry {
                    host: "none.com".into(),
                    permission: NetworkDomainPermission::None,
                },
                NetworkDomainPermissionEntry {
                    host: "deny.com".into(),
                    permission: NetworkDomainPermission::Deny,
                },
            ],
        };
        let effective = perms.effective_entries();
        assert_eq!(effective.len(), 2);
    }

    #[test]
    fn settings_domain_manipulation() {
        let mut settings = NetworkProxySettings::default();
        settings.set_allowed_domains(vec!["a.com".into(), "b.com".into()]);
        assert_eq!(settings.allowed_domains(), vec!["a.com", "b.com"]);

        settings.set_denied_domains(vec!["c.com".into()]);
        assert_eq!(settings.denied_domains(), vec!["c.com"]);

        settings.upsert_domain_permission("a.com".into(), NetworkDomainPermission::Deny);
        assert_eq!(settings.allowed_domains(), vec!["b.com"]);
        assert!(settings.denied_domains().contains(&"a.com"));
    }

    #[test]
    fn settings_json_roundtrip() {
        let mut settings = NetworkProxySettings::default();
        settings.proxy_url = Some("http://proxy:8080".into());
        settings.set_allowed_domains(vec!["example.com".into()]);
        let json = serde_json::to_string(&settings).unwrap();
        let deserialized: NetworkProxySettings = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.proxy_url, Some("http://proxy:8080".into()));
        assert_eq!(deserialized.allowed_domains(), vec!["example.com"]);
    }
}
