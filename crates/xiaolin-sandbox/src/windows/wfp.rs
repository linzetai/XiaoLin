//! Windows Filtering Platform (WFP) network firewall rules for sandboxing.
//!
//! Aligned with Codex `windows-sandbox-rs/src/wfp.rs` + `wfp_setup.rs`.
//! Uses WFP to block or redirect network traffic from sandboxed processes.

use std::net::IpAddr;

/// A WFP filter rule for network access control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WfpRule {
    /// Rule name for identification.
    pub name: String,
    /// The action to take when the rule matches.
    pub action: WfpAction,
    /// Match condition.
    pub condition: WfpCondition,
    /// Weight (higher = evaluated first).
    pub weight: u16,
}

/// Action for a WFP rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WfpAction {
    /// Allow the connection.
    Permit,
    /// Block the connection.
    Block,
    /// Redirect to a local proxy port.
    Redirect { port: u16 },
}

/// Match condition for a WFP rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WfpCondition {
    /// Match any traffic.
    Any,
    /// Match traffic to a specific remote port.
    RemotePort(u16),
    /// Match traffic to a specific remote IP.
    RemoteAddress(IpAddr),
    /// Match traffic from a specific process ID.
    ProcessId(u32),
    /// Match traffic to localhost.
    Loopback,
    /// Compound condition (all must match).
    All(Vec<WfpCondition>),
}

/// A complete WFP ruleset for a sandboxed process.
#[derive(Debug, Clone)]
pub struct WfpRuleset {
    /// Rules to apply (ordered by weight descending).
    pub rules: Vec<WfpRule>,
    /// The process ID this ruleset targets.
    pub target_pid: Option<u32>,
}

impl WfpRuleset {
    /// Create an empty ruleset.
    pub fn empty() -> Self {
        Self {
            rules: Vec::new(),
            target_pid: None,
        }
    }

    /// Whether this ruleset has any rules.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

/// Build a WFP ruleset for sandboxed network access.
///
/// - `allow_loopback`: whether to permit localhost traffic
/// - `proxy_port`: if set, redirect HTTP/HTTPS to this local port
/// - `block_all_outbound`: block all outbound connections except allowed ones
pub fn build_wfp_ruleset(
    allow_loopback: bool,
    proxy_port: Option<u16>,
    block_all_outbound: bool,
    pid: Option<u32>,
) -> WfpRuleset {
    let mut rules = Vec::new();
    let mut weight: u16 = 1000;

    // Allow loopback if permitted
    if allow_loopback {
        rules.push(WfpRule {
            name: "allow-loopback".to_string(),
            action: WfpAction::Permit,
            condition: WfpCondition::Loopback,
            weight,
        });
        weight -= 10;
    }

    // Redirect HTTP/HTTPS to proxy
    if let Some(port) = proxy_port {
        rules.push(WfpRule {
            name: "redirect-http-to-proxy".to_string(),
            action: WfpAction::Redirect { port },
            condition: WfpCondition::RemotePort(80),
            weight,
        });
        weight -= 10;

        rules.push(WfpRule {
            name: "redirect-https-to-proxy".to_string(),
            action: WfpAction::Redirect { port },
            condition: WfpCondition::RemotePort(443),
            weight,
        });
        weight -= 10;

        // Allow traffic to the proxy itself
        rules.push(WfpRule {
            name: "allow-proxy-port".to_string(),
            action: WfpAction::Permit,
            condition: WfpCondition::All(vec![
                WfpCondition::Loopback,
                WfpCondition::RemotePort(port),
            ]),
            weight,
        });
        weight -= 10;

        // Allow DNS
        rules.push(WfpRule {
            name: "allow-dns".to_string(),
            action: WfpAction::Permit,
            condition: WfpCondition::RemotePort(53),
            weight,
        });
    }

    // Block all outbound if requested
    if block_all_outbound {
        rules.push(WfpRule {
            name: "block-all-outbound".to_string(),
            action: WfpAction::Block,
            condition: WfpCondition::Any,
            weight: 1, // lowest weight
        });
    }

    WfpRuleset {
        rules,
        target_pid: pid,
    }
}

/// Apply WFP rules to the system.
///
/// On non-Windows platforms, this is a no-op.
#[cfg(target_os = "windows")]
pub fn apply_wfp_rules(ruleset: &WfpRuleset) -> Result<WfpSession, std::io::Error> {
    // Windows implementation would use:
    // - FwpmEngineOpen0 to open the WFP engine
    // - FwpmTransactionBegin0 to start a transaction
    // - FwpmSubLayerAdd0 to add a sublayer
    // - FwpmFilterAdd0 for each rule
    // - FwpmTransactionCommit0 to commit
    todo!("WFP engine implementation")
}

#[cfg(not(target_os = "windows"))]
pub fn apply_wfp_rules(_ruleset: &WfpRuleset) -> Result<WfpSession, std::io::Error> {
    Ok(WfpSession { _private: () })
}

/// Handle to an active WFP session. Dropping this removes the rules.
pub struct WfpSession {
    _private: (),
}

impl WfpSession {
    /// Remove all WFP rules from this session.
    pub fn cleanup(self) -> Result<(), std::io::Error> {
        // On Windows, would close the engine handle, removing all filters in the sublayer.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_ruleset() {
        let ruleset = WfpRuleset::empty();
        assert!(ruleset.is_empty());
        assert!(ruleset.target_pid.is_none());
    }

    #[test]
    fn build_ruleset_loopback_only() {
        let ruleset = build_wfp_ruleset(true, None, true, Some(1234));
        assert!(!ruleset.is_empty());
        assert_eq!(ruleset.target_pid, Some(1234));

        let allow = ruleset.rules.iter().find(|r| r.name == "allow-loopback");
        assert!(allow.is_some());
        assert_eq!(allow.unwrap().action, WfpAction::Permit);
        assert_eq!(allow.unwrap().condition, WfpCondition::Loopback);

        let block = ruleset.rules.iter().find(|r| r.name == "block-all-outbound");
        assert!(block.is_some());
        assert_eq!(block.unwrap().action, WfpAction::Block);
    }

    #[test]
    fn build_ruleset_with_proxy() {
        let ruleset = build_wfp_ruleset(true, Some(8080), true, None);
        assert!(ruleset.rules.len() >= 5);

        let redirect_http = ruleset.rules.iter().find(|r| r.name == "redirect-http-to-proxy");
        assert!(redirect_http.is_some());
        assert_eq!(redirect_http.unwrap().action, WfpAction::Redirect { port: 8080 });

        let redirect_https = ruleset.rules.iter().find(|r| r.name == "redirect-https-to-proxy");
        assert!(redirect_https.is_some());

        let allow_dns = ruleset.rules.iter().find(|r| r.name == "allow-dns");
        assert!(allow_dns.is_some());
    }

    #[test]
    fn build_ruleset_no_block() {
        let ruleset = build_wfp_ruleset(false, None, false, None);
        assert!(ruleset.is_empty());
    }

    #[test]
    fn build_ruleset_proxy_without_loopback() {
        let ruleset = build_wfp_ruleset(false, Some(3128), true, None);
        let loopback = ruleset.rules.iter().find(|r| r.name == "allow-loopback");
        assert!(loopback.is_none());

        let redirect = ruleset.rules.iter().find(|r| r.name == "redirect-http-to-proxy");
        assert!(redirect.is_some());
    }

    #[test]
    fn rule_weights_are_descending() {
        let ruleset = build_wfp_ruleset(true, Some(8080), true, None);
        for window in ruleset.rules.windows(2) {
            if window[1].name != "block-all-outbound" {
                assert!(window[0].weight >= window[1].weight);
            }
        }
    }

    #[test]
    fn apply_wfp_noop_on_non_windows() {
        let ruleset = build_wfp_ruleset(true, Some(8080), true, Some(999));
        let session = apply_wfp_rules(&ruleset);
        assert!(session.is_ok());
    }

    #[test]
    fn wfp_session_cleanup() {
        let session = WfpSession { _private: () };
        assert!(session.cleanup().is_ok());
    }

    #[test]
    fn wfp_condition_all_compound() {
        let cond = WfpCondition::All(vec![
            WfpCondition::Loopback,
            WfpCondition::RemotePort(8080),
        ]);
        match cond {
            WfpCondition::All(inner) => assert_eq!(inner.len(), 2),
            _ => panic!("expected All"),
        }
    }

    #[test]
    fn wfp_action_redirect_port() {
        let action = WfpAction::Redirect { port: 3128 };
        match action {
            WfpAction::Redirect { port } => assert_eq!(port, 3128),
            _ => panic!("expected Redirect"),
        }
    }
}
