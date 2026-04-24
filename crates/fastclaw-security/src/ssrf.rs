use std::net::{IpAddr, ToSocketAddrs};
use std::sync::RwLock;

/// Global set of hostnames (with optional :port) that bypass private-IP SSRF checks.
/// Uses `RwLock` so the list can be hot-reloaded without restarting the gateway.
static ALLOWED_HOSTS: RwLock<Vec<String>> = RwLock::new(Vec::new());

/// Register (or replace) hosts that should bypass SSRF private-IP checks.
/// Safe to call multiple times; each call replaces the previous list.
/// Each entry is a hostname or `hostname:port` pair (compared case-insensitively).
pub fn set_ssrf_allowed_hosts(hosts: Vec<String>) {
    let normalized: Vec<String> = hosts.iter().map(|h| h.to_lowercase()).collect();
    if let Ok(mut guard) = ALLOWED_HOSTS.write() {
        *guard = normalized;
    }
}

fn is_host_allowed(host: &str, port: u16) -> bool {
    let allowed = match ALLOWED_HOSTS.read() {
        Ok(guard) => guard,
        Err(_) => return false,
    };
    if allowed.is_empty() {
        return false;
    }
    let host_lower = host.to_lowercase();
    let host_port = format!("{}:{}", host_lower, port);
    allowed.iter().any(|entry| {
        *entry == host_lower || *entry == host_port
    })
}

pub fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_ipv4(v4),
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_private_ipv4(&mapped);
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xFE00) == 0xFC00 // ULA fc00::/7
                || (v6.segments()[0] == 0xFE80)           // link-local fe80::/10
        }
    }
}

pub fn is_private_ipv4(v4: &std::net::Ipv4Addr) -> bool {
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_broadcast()
        || v4.is_unspecified()
        || v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64 // 100.64.0.0/10 (CGN)
        || v4.octets()[0] == 169 && v4.octets()[1] == 254        // 169.254.0.0/16
        || v4.octets()[0] == 0                                    // 0.0.0.0/8
}

pub fn ssrf_check_url(url_str: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url_str).map_err(|e| format!("invalid URL: {e}"))?;
    ssrf_check_parsed_url(&parsed)
}

pub fn ssrf_check_parsed_url(parsed: &url::Url) -> Result<(), String> {
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        tracing::warn!(
            url = %parsed,
            scheme = %scheme,
            "SSRF: blocked non-HTTP scheme"
        );
        return Err(format!("scheme '{scheme}' not allowed; only http/https"));
    }
    let host = parsed.host_str().ok_or("URL has no host")?;
    let port = parsed.port_or_known_default().unwrap_or(80);

    if is_host_allowed(host, port) {
        tracing::debug!(
            url = %parsed,
            host = %host,
            "SSRF: host is in ssrfAllowedHosts, skipping private-IP check"
        );
        return Ok(());
    }

    let socket_addr_str = format!("{host}:{port}");
    let addrs: Vec<_> = socket_addr_str
        .to_socket_addrs()
        .map_err(|e| format!("DNS resolution failed for '{host}': {e}"))?
        .collect();
    if addrs.is_empty() {
        return Err(format!("DNS resolution returned no addresses for '{host}'"));
    }
    for addr in &addrs {
        if is_private_ip(&addr.ip()) {
            tracing::warn!(
                url = %parsed,
                host = %host,
                resolved_ip = %addr.ip(),
                "SSRF: blocked request to private/reserved address"
            );
            return Err(format!(
                "URL resolves to private/reserved address {} — request blocked (SSRF protection). \
                 To allow this host, add it to security.ssrfAllowedHosts in config.",
                addr.ip()
            ));
        }
    }
    Ok(())
}

pub fn ssrf_safe_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 5 {
            tracing::warn!(
                redirect_target = %attempt.url(),
                hop_count = attempt.previous().len(),
                "SSRF: too many redirects"
            );
            attempt.error("too many redirects")
        } else {
            let url = attempt.url();
            match ssrf_check_parsed_url(url) {
                Ok(()) => {
                    tracing::debug!(
                        redirect_target = %url,
                        hop = attempt.previous().len(),
                        "SSRF: following redirect after validation"
                    );
                    attempt.follow()
                }
                Err(msg) => {
                    tracing::warn!(
                        redirect_target = %url,
                        from = %attempt.previous().last().map(|u| u.as_str()).unwrap_or("?"),
                        hop = attempt.previous().len(),
                        reason = %msg,
                        "SSRF: blocked redirect to private/disallowed target"
                    );
                    attempt.error(format!("redirect blocked by SSRF protection: {msg}"))
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_non_http_scheme() {
        assert!(ssrf_check_url("file:///etc/passwd").is_err());
        assert!(ssrf_check_url("ftp://mirror.example.com/data").is_err());
    }

    #[test]
    fn allows_public_https() {
        assert!(ssrf_check_url("https://api.github.com/zen").is_ok());
    }

    #[test]
    fn allowed_hosts_and_localhost_blocking() {
        // Start with empty list — localhost should be blocked
        set_ssrf_allowed_hosts(vec![]);
        assert!(ssrf_check_url("http://localhost:8080/api").is_err());
        assert!(ssrf_check_url("http://127.0.0.1:3000/").is_err());

        // Set allowed hosts — verify matching logic
        set_ssrf_allowed_hosts(vec![
            "localhost".to_string(),
            "searxng.local:8888".to_string(),
            "MyHost".to_string(),
        ]);

        assert!(is_host_allowed("localhost", 8080));
        assert!(is_host_allowed("localhost", 3000));
        assert!(!is_host_allowed("evilhost", 80));

        assert!(is_host_allowed("searxng.local", 8888));
        assert!(!is_host_allowed("searxng.local", 9999));

        assert!(is_host_allowed("myhost", 80));
        assert!(is_host_allowed("MYHOST", 80));

        // Reset to empty — localhost should be blocked again
        set_ssrf_allowed_hosts(vec![]);
        assert!(!is_host_allowed("localhost", 8080));
    }

    #[test]
    fn private_ip_detection() {
        use std::net::Ipv4Addr;
        assert!(is_private_ipv4(&Ipv4Addr::new(127, 0, 0, 1)));
        assert!(is_private_ipv4(&Ipv4Addr::new(10, 0, 0, 1)));
        assert!(is_private_ipv4(&Ipv4Addr::new(192, 168, 1, 1)));
        assert!(is_private_ipv4(&Ipv4Addr::new(172, 16, 0, 1)));
        assert!(!is_private_ipv4(&Ipv4Addr::new(8, 8, 8, 8)));
        assert!(!is_private_ipv4(&Ipv4Addr::new(1, 1, 1, 1)));
    }
}
