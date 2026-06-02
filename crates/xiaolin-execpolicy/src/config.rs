use serde::{Deserialize, Serialize};

/// Top-level TOML policy configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyConfig {
    /// Default settings and shortcuts.
    #[serde(default)]
    pub defaults: Option<Defaults>,
    /// Prefix-based command rules.
    #[serde(default, rename = "rules")]
    pub rules: Vec<PrefixRule>,
    /// Network access rules.
    #[serde(default)]
    pub network: Vec<NetworkRule>,
    /// Inline validation tests.
    #[serde(default)]
    pub tests: Vec<PolicyTest>,
}

/// Default configuration shortcuts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Defaults {
    /// Commands that are always allowed (read-only operations).
    /// Each entry generates an "allow" prefix rule for that command.
    #[serde(default)]
    pub allow_readonly: Vec<String>,
    /// Default decision when no rule matches.
    #[serde(default = "default_fallback")]
    pub fallback: String,
}

fn default_fallback() -> String {
    "prompt".to_string()
}

/// A prefix-based command execution rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefixRule {
    /// Optional unique identifier for the rule.
    #[serde(default)]
    pub id: Option<String>,
    /// Command prefix pattern. Each element is either a string (exact match)
    /// or an array of strings (alternatives).
    pub pattern: Vec<PatternElement>,
    /// Decision: "allow", "forbidden", or "prompt".
    pub decision: String,
    /// Human-readable justification for the decision.
    #[serde(default)]
    pub justification: Option<String>,
}

/// An element in a prefix pattern: either an exact string or alternatives.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PatternElement {
    /// Exact match for a single token.
    Exact(String),
    /// Match any one of the alternatives.
    Alternatives(Vec<String>),
}

/// Type-safe network protocol for network rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkRuleProtocol {
    Https,
    Http,
    Ssh,
    /// SOCKS5 over TCP.
    #[serde(alias = "socks5_tcp")]
    Socks5Tcp,
    /// SOCKS5 over UDP.
    #[serde(alias = "socks5_udp")]
    Socks5Udp,
    /// Match any protocol.
    #[serde(alias = "*")]
    Any,
}

impl NetworkRuleProtocol {
    /// Check if this protocol matches a given protocol string.
    pub fn matches(&self, protocol: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Https => protocol.eq_ignore_ascii_case("https"),
            Self::Http => protocol.eq_ignore_ascii_case("http"),
            Self::Ssh => protocol.eq_ignore_ascii_case("ssh"),
            Self::Socks5Tcp => protocol.eq_ignore_ascii_case("socks5_tcp"),
            Self::Socks5Udp => protocol.eq_ignore_ascii_case("socks5_udp"),
        }
    }
}

impl std::fmt::Display for NetworkRuleProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Https => write!(f, "https"),
            Self::Http => write!(f, "http"),
            Self::Ssh => write!(f, "ssh"),
            Self::Socks5Tcp => write!(f, "socks5_tcp"),
            Self::Socks5Udp => write!(f, "socks5_udp"),
            Self::Any => write!(f, "*"),
        }
    }
}

/// Normalize a network rule host value (strict mode).
///
/// - Rejects inputs containing `://`, `/`, `?`, `#`, or the wildcard `*`.
/// - Supports IPv6 bracket notation: `[::1]:8080` -> `::1`.
/// - Strips port numbers, trailing dots, and whitespace; lowercases the result.
///
/// Returns `Err` if the input is malformed or empty after normalization.
pub fn normalize_network_rule_host(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();

    if trimmed.is_empty() {
        return Err("host is empty".to_string());
    }

    if trimmed.contains("://") {
        return Err(format!("host must not contain a scheme ('://'): '{trimmed}'"));
    }
    if trimmed.contains('/') {
        return Err(format!("host must not contain a path ('/'): '{trimmed}'"));
    }
    if trimmed.contains('?') {
        return Err(format!("host must not contain '?': '{trimmed}'"));
    }
    if trimmed.contains('#') {
        return Err(format!("host must not contain '#': '{trimmed}'"));
    }
    if trimmed == "*" || trimmed.contains('*') {
        return Err(format!(
            "wildcard '*' not allowed in host; use a catch-all rule instead: '{trimmed}'"
        ));
    }

    let mut host = trimmed.to_string();

    // IPv6 bracket notation: [::1]:8080 -> ::1
    if host.starts_with('[') {
        if let Some(bracket_end) = host.find(']') {
            host = host[1..bracket_end].to_string();
        } else {
            return Err(format!("unclosed IPv6 bracket: '{trimmed}'"));
        }
    } else {
        // Strip port number (only for non-IPv6)
        if let Some(pos) = host.rfind(':') {
            let after = &host[pos + 1..];
            if !after.is_empty() && after.chars().all(|c| c.is_ascii_digit()) {
                host.truncate(pos);
            }
        }
    }

    // Strip trailing dots
    while host.ends_with('.') {
        host.pop();
    }

    host = host.to_lowercase();

    if host.is_empty() {
        return Err("host is empty after normalization".to_string());
    }

    if host.contains(' ') {
        return Err(format!("invalid host (contains spaces): '{host}'"));
    }

    Ok(host)
}

/// Network access rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkRule {
    /// Optional unique identifier.
    #[serde(default)]
    pub id: Option<String>,
    /// Host name or IP address. Use "*" for catch-all.
    pub host: String,
    /// Protocol filter. `None` means match any protocol.
    #[serde(default)]
    pub protocol: Option<NetworkRuleProtocol>,
    /// Decision: "allow", "forbidden", or "prompt".
    pub decision: String,
}

/// Inline test for policy validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyTest {
    /// Command string to test (will be split on whitespace).
    pub command: String,
    /// Expected decision: "allow", "forbidden", or "prompt".
    pub expect: String,
    /// If true, asserts that no rules matched (negative test case).
    #[serde(default)]
    pub not_match: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_config() {
        let toml_str = r#"
[[rules]]
pattern = ["echo"]
decision = "allow"
"#;
        let config: PolicyConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.rules.len(), 1);
        assert!(config.defaults.is_none());
    }

    #[test]
    fn parse_full_config() {
        let toml_str = r#"
[defaults]
allow_readonly = ["ls", "cat"]
fallback = "prompt"

[[rules]]
id = "forbid-sudo"
pattern = ["sudo"]
decision = "forbidden"
justification = "Never run as root"

[[rules]]
id = "allow-git-ops"
pattern = ["git", ["status", "diff", "log"]]
decision = "allow"

[[network]]
id = "allow-npm"
host = "registry.npmjs.org"
protocol = "https"
decision = "allow"

[[tests]]
command = "ls -la"
expect = "allow"
"#;
        let config: PolicyConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.rules.len(), 2);
        assert!(config.defaults.is_some());
        assert_eq!(config.defaults.as_ref().unwrap().allow_readonly.len(), 2);
        assert_eq!(config.network.len(), 1);
        assert_eq!(config.tests.len(), 1);
    }

    #[test]
    fn pattern_element_alternatives() {
        let toml_str = r#"
[[rules]]
pattern = ["git", ["merge", "rebase", "cherry-pick"]]
decision = "prompt"
"#;
        let config: PolicyConfig = toml::from_str(toml_str).unwrap();
        match &config.rules[0].pattern[1] {
            PatternElement::Alternatives(alts) => {
                assert_eq!(alts.len(), 3);
                assert!(alts.contains(&"merge".to_string()));
                assert!(alts.contains(&"rebase".to_string()));
            }
            _ => panic!("expected Alternatives"),
        }
    }

    #[test]
    fn network_rule_protocol_deserialize() {
        let toml_str = r#"
[[network]]
host = "example.com"
protocol = "https"
decision = "allow"

[[network]]
host = "git.example.com"
protocol = "ssh"
decision = "allow"

[[network]]
host = "internal.corp"
protocol = "any"
decision = "allow"

[[network]]
host = "other.com"
decision = "forbidden"
"#;
        let config: PolicyConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.network.len(), 4);
        assert_eq!(config.network[0].protocol, Some(NetworkRuleProtocol::Https));
        assert_eq!(config.network[1].protocol, Some(NetworkRuleProtocol::Ssh));
        assert_eq!(config.network[2].protocol, Some(NetworkRuleProtocol::Any));
        assert_eq!(config.network[3].protocol, None);
    }

    #[test]
    fn network_rule_protocol_matches() {
        assert!(NetworkRuleProtocol::Https.matches("https"));
        assert!(NetworkRuleProtocol::Https.matches("HTTPS"));
        assert!(!NetworkRuleProtocol::Https.matches("http"));
        assert!(NetworkRuleProtocol::Http.matches("http"));
        assert!(NetworkRuleProtocol::Ssh.matches("ssh"));
        assert!(NetworkRuleProtocol::Any.matches("https"));
        assert!(NetworkRuleProtocol::Any.matches("whatever"));
    }

    #[test]
    fn normalize_host_strips_port() {
        assert_eq!(
            normalize_network_rule_host("api.github.com:443").unwrap(),
            "api.github.com"
        );
    }

    #[test]
    fn normalize_host_lowercases() {
        assert_eq!(
            normalize_network_rule_host("API.GITHUB.COM").unwrap(),
            "api.github.com"
        );
    }

    #[test]
    fn normalize_host_strips_trailing_dot() {
        assert_eq!(
            normalize_network_rule_host("example.com.").unwrap(),
            "example.com"
        );
    }

    #[test]
    fn normalize_host_rejects_empty() {
        assert!(normalize_network_rule_host("").is_err());
        assert!(normalize_network_rule_host("   ").is_err());
    }

    #[test]
    fn normalize_host_rejects_scheme() {
        assert!(normalize_network_rule_host("https://example.com").is_err());
        assert!(normalize_network_rule_host("http://example.com").is_err());
    }

    #[test]
    fn normalize_host_rejects_path_and_query() {
        assert!(normalize_network_rule_host("example.com/path").is_err());
        assert!(normalize_network_rule_host("example.com?q=1").is_err());
        assert!(normalize_network_rule_host("example.com#frag").is_err());
    }

    #[test]
    fn normalize_host_rejects_wildcard() {
        assert!(normalize_network_rule_host("*").is_err());
        assert!(normalize_network_rule_host("*.example.com").is_err());
    }

    #[test]
    fn normalize_host_ipv6_bracket() {
        assert_eq!(
            normalize_network_rule_host("[::1]:8080").unwrap(),
            "::1"
        );
        assert_eq!(
            normalize_network_rule_host("[2001:db8::1]").unwrap(),
            "2001:db8::1"
        );
    }

    #[test]
    fn normalize_host_ipv6_unclosed_bracket() {
        assert!(normalize_network_rule_host("[::1").is_err());
    }

    #[test]
    fn socks5_protocol_deserialize_and_match() {
        let toml_str = r#"
[[network]]
host = "proxy.example.com"
protocol = "socks5tcp"
decision = "allow"

[[network]]
host = "proxy2.example.com"
protocol = "socks5udp"
decision = "allow"
"#;
        let config: PolicyConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.network[0].protocol,
            Some(NetworkRuleProtocol::Socks5Tcp)
        );
        assert_eq!(
            config.network[1].protocol,
            Some(NetworkRuleProtocol::Socks5Udp)
        );
        assert!(NetworkRuleProtocol::Socks5Tcp.matches("socks5_tcp"));
        assert!(NetworkRuleProtocol::Socks5Udp.matches("SOCKS5_UDP"));
        assert!(!NetworkRuleProtocol::Socks5Tcp.matches("socks5_udp"));
    }

    #[test]
    fn not_match_field_deserializes() {
        let toml_str = r#"
[[tests]]
command = "unknown-cmd"
expect = "prompt"
not_match = true
"#;
        let config: PolicyConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.tests[0].not_match, Some(true));
    }
}
