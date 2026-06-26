use std::env;

pub const PROXY_URL_ENV_KEYS: &[&str] = &["http_proxy", "HTTP_PROXY", "https_proxy", "HTTPS_PROXY"];

pub const ALL_PROXY_ENV_KEYS: &[&str] = &["all_proxy", "ALL_PROXY"];

pub const PROXY_ACTIVE_ENV_KEY: &str = "XIAOLIN_PROXY_ACTIVE";

pub const ALLOW_LOCAL_BINDING_ENV_KEY: &str = "XIAOLIN_ALLOW_LOCAL_BINDING";

pub const PROXY_ENV_KEYS: &[&str] = &[
    "http_proxy",
    "HTTP_PROXY",
    "https_proxy",
    "HTTPS_PROXY",
    "all_proxy",
    "ALL_PROXY",
    "ftp_proxy",
    "FTP_PROXY",
];

pub const NO_PROXY_ENV_KEYS: &[&str] = &["no_proxy", "NO_PROXY"];

pub const DEFAULT_NO_PROXY_VALUE: &str = "localhost,127.0.0.0/8,::1";

pub const PROXY_GIT_SSH_COMMAND_MARKER: &str = "XIAOLIN_PROXY_GIT_SSH";

/// Read the first defined proxy URL from environment variables.
pub fn proxy_url_env_value() -> Option<String> {
    for key in PROXY_URL_ENV_KEYS.iter().chain(ALL_PROXY_ENV_KEYS) {
        if let Ok(val) = env::var(key) {
            if !val.is_empty() {
                return Some(val);
            }
        }
    }
    None
}

/// Check whether any proxy URL environment variable is set.
pub fn has_proxy_url_env_vars() -> bool {
    proxy_url_env_value().is_some()
}

/// Extract loopback proxy ports from environment variables.
/// Returns `(http_port, socks_port)` if the proxy is on 127.0.0.1.
pub fn proxy_loopback_ports_from_env() -> (Option<u16>, Option<u16>) {
    let http_port = proxy_url_env_value().and_then(|url_str| parse_loopback_port(&url_str));

    let socks_port = env::var("all_proxy")
        .or_else(|_| env::var("ALL_PROXY"))
        .ok()
        .and_then(|url_str| parse_loopback_port(&url_str));

    (http_port, socks_port)
}

fn parse_loopback_port(url_str: &str) -> Option<u16> {
    let parsed = url::Url::parse(url_str).ok()?;
    let host = parsed.host_str()?;
    if host == "127.0.0.1" || host == "localhost" || host == "::1" {
        parsed.port()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_env_keys_are_defined() {
        assert!(!PROXY_URL_ENV_KEYS.is_empty());
        assert!(!PROXY_ENV_KEYS.is_empty());
        assert!(!NO_PROXY_ENV_KEYS.is_empty());
    }

    #[test]
    fn parse_loopback_port_from_localhost() {
        assert_eq!(parse_loopback_port("http://127.0.0.1:8080"), Some(8080));
        assert_eq!(parse_loopback_port("socks5://localhost:1080"), Some(1080));
    }

    #[test]
    fn parse_loopback_port_non_loopback() {
        assert_eq!(parse_loopback_port("http://proxy.example.com:3128"), None);
    }

    #[test]
    fn parse_loopback_port_invalid_url() {
        assert_eq!(parse_loopback_port("not-a-url"), None);
    }

    #[test]
    fn default_no_proxy_value_is_sane() {
        assert!(DEFAULT_NO_PROXY_VALUE.contains("localhost"));
        assert!(DEFAULT_NO_PROXY_VALUE.contains("127.0.0.0"));
    }
}
