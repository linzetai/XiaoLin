//! Host mapping lookup and DNS resolution for proxy connect paths.
//!
//! Applies custom host→IP mappings before DNS, and reuses a single resolved
//! address for policy check + TCP connect (rule #41 — anti DNS rebinding).

use std::net::{IpAddr, SocketAddr};

use anyhow::{anyhow, ensure, Context, Result};

use crate::config::HostMapping;

/// Result of resolving a connect target (mapped IP or DNS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConnectTarget {
    pub host: String,
    pub port: u16,
    pub address: SocketAddr,
    /// True when the address came from a host mapping (DNS skipped).
    pub mapped: bool,
}

/// Resolve `host:port` for outbound TCP connect.
///
/// 1. Check host mappings (exact patterns win over wildcards).
/// 2. Otherwise perform a single DNS lookup and reuse the result.
pub async fn resolve_connect_target(
    host: &str,
    port: u16,
    mappings: &[HostMapping],
) -> Result<ResolvedConnectTarget> {
    let host = host.trim();
    ensure!(!host.is_empty(), "host is empty");
    ensure!(port > 0, "port must be non-zero");

    if let Some(mapping) = HostMapping::lookup(mappings, host) {
        let ip: IpAddr = mapping
            .target_ip
            .parse()
            .with_context(|| format!("invalid mapped IP for host {host}"))?;
        return Ok(ResolvedConnectTarget {
            host: host.to_string(),
            port,
            address: SocketAddr::new(ip, port),
            mapped: true,
        });
    }

    let address = lookup_host_once(host, port).await?;
    Ok(ResolvedConnectTarget {
        host: host.to_string(),
        port,
        address,
        mapped: false,
    })
}

async fn lookup_host_once(host: &str, port: u16) -> Result<SocketAddr> {
    tokio::net::lookup_host(format!("{host}:{port}"))
        .await
        .with_context(|| format!("failed to resolve {host}:{port}"))?
        .next()
        .ok_or_else(|| anyhow!("DNS lookup returned no addresses for {host}:{port}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mapping_skips_dns() {
        let mappings = vec![HostMapping {
            pattern: "api.dev.com".into(),
            target_ip: "192.168.1.100".into(),
        }];
        let resolved = resolve_connect_target("api.dev.com", 443, &mappings)
            .await
            .unwrap();
        assert!(resolved.mapped);
        assert_eq!(resolved.address, "192.168.1.100:443".parse().unwrap());
    }

    #[tokio::test]
    async fn wildcard_mapping_applies_to_subdomain() {
        let mappings = vec![HostMapping {
            pattern: "*.internal.corp".into(),
            target_ip: "172.16.0.1".into(),
        }];
        let resolved = resolve_connect_target("a.internal.corp", 80, &mappings)
            .await
            .unwrap();
        assert!(resolved.mapped);
        assert_eq!(resolved.address.ip().to_string(), "172.16.0.1");
    }

    #[tokio::test]
    async fn dns_resolves_public_host() {
        let resolved = resolve_connect_target("example.com", 443, &[])
            .await
            .unwrap();
        assert!(!resolved.mapped);
        assert_eq!(resolved.address.port(), 443);
    }
}
