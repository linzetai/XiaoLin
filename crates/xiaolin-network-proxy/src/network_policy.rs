use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::sync::Arc;

const REASON_POLICY_DENIED: &str = "policy denied";

/// Network-level protocol for policy decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkProtocol {
    Http,
    HttpsConnect,
    Socks5Tcp,
    Socks5Udp,
}

impl NetworkProtocol {
    pub const fn as_policy_protocol(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::HttpsConnect => "https_connect",
            Self::Socks5Tcp => "socks5_tcp",
            Self::Socks5Udp => "socks5_udp",
        }
    }
}

/// The decision made by a network policy evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkPolicyDecision {
    Deny,
    Ask,
}

impl NetworkPolicyDecision {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::Ask => "ask",
        }
    }

    pub fn is_denied(self) -> bool {
        matches!(self, Self::Deny)
    }

    pub fn is_ask(self) -> bool {
        matches!(self, Self::Ask)
    }
}

/// The source of a network policy decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkDecisionSource {
    BaselinePolicy,
    ModeGuard,
    ProxyState,
    Decider,
}

impl NetworkDecisionSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BaselinePolicy => "baseline_policy",
            Self::ModeGuard => "mode_guard",
            Self::ProxyState => "proxy_state",
            Self::Decider => "decider",
        }
    }
}

/// Arguments for constructing a `NetworkPolicyRequest`.
pub struct NetworkPolicyRequestArgs {
    pub protocol: NetworkProtocol,
    pub host: String,
    pub port: u16,
    pub client_addr: Option<String>,
    pub method: Option<String>,
    pub command: Option<String>,
    pub exec_policy_hint: Option<String>,
}

/// A request for a network policy decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkPolicyRequest {
    pub protocol: NetworkProtocol,
    pub host: String,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_addr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exec_policy_hint: Option<String>,
}

impl NetworkPolicyRequest {
    pub fn new(args: NetworkPolicyRequestArgs) -> Self {
        Self {
            protocol: args.protocol,
            host: args.host,
            port: args.port,
            client_addr: args.client_addr,
            method: args.method,
            command: args.command,
            exec_policy_hint: args.exec_policy_hint,
        }
    }

    pub fn host_port(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub fn is_loopback(&self) -> bool {
        self.host == "127.0.0.1"
            || self.host == "::1"
            || self.host.eq_ignore_ascii_case("localhost")
    }
}

/// The result of a network policy evaluation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum NetworkDecision {
    Allow,
    Deny {
        reason: String,
        source: NetworkDecisionSource,
        decision: NetworkPolicyDecision,
    },
}

impl NetworkDecision {
    pub fn allow() -> Self {
        Self::Allow
    }

    pub fn deny(reason: impl Into<String>) -> Self {
        Self::deny_with_source(reason, NetworkDecisionSource::Decider)
    }

    pub fn ask(reason: impl Into<String>) -> Self {
        Self::ask_with_source(reason, NetworkDecisionSource::Decider)
    }

    pub fn deny_with_source(
        reason: impl Into<String>,
        source: NetworkDecisionSource,
    ) -> Self {
        let reason = reason.into();
        let reason = if reason.is_empty() {
            REASON_POLICY_DENIED.to_string()
        } else {
            reason
        };
        Self::Deny {
            reason,
            source,
            decision: NetworkPolicyDecision::Deny,
        }
    }

    pub fn ask_with_source(
        reason: impl Into<String>,
        source: NetworkDecisionSource,
    ) -> Self {
        let reason = reason.into();
        let reason = if reason.is_empty() {
            REASON_POLICY_DENIED.to_string()
        } else {
            reason
        };
        Self::Deny {
            reason,
            source,
            decision: NetworkPolicyDecision::Ask,
        }
    }

    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow)
    }

    pub fn is_denied(&self) -> bool {
        matches!(
            self,
            Self::Deny {
                decision: NetworkPolicyDecision::Deny,
                ..
            }
        )
    }

    pub fn is_ask(&self) -> bool {
        matches!(
            self,
            Self::Deny {
                decision: NetworkPolicyDecision::Ask,
                ..
            }
        )
    }
}

/// Pluggable async policy decision trait.
///
/// Implementations may consult external services, user prompts, or
/// local rule engines to decide whether a network request should
/// be allowed.
#[async_trait]
pub trait NetworkPolicyDecider: Send + Sync + 'static {
    async fn decide(&self, req: NetworkPolicyRequest) -> NetworkDecision;
}

#[async_trait]
impl<D: NetworkPolicyDecider + ?Sized> NetworkPolicyDecider for Arc<D> {
    async fn decide(&self, req: NetworkPolicyRequest) -> NetworkDecision {
        (**self).decide(req).await
    }
}

#[async_trait]
impl<F, Fut> NetworkPolicyDecider for F
where
    F: Fn(NetworkPolicyRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = NetworkDecision> + Send,
{
    async fn decide(&self, req: NetworkPolicyRequest) -> NetworkDecision {
        (self)(req).await
    }
}

/// Evaluate host-level policy using the runtime state and optional external decider.
pub async fn evaluate_host_policy(
    host_decision: HostBlockDecision,
    decider: Option<&Arc<dyn NetworkPolicyDecider>>,
    request: &NetworkPolicyRequest,
) -> NetworkDecision {
    match host_decision {
        HostBlockDecision::Allowed => NetworkDecision::Allow,
        HostBlockDecision::Blocked(HostBlockReason::NotAllowed) => {
            if let Some(decider) = decider {
                let decider_decision = decider.decide(request.clone()).await;
                map_decider_decision(decider_decision)
            } else {
                NetworkDecision::deny_with_source(
                    HostBlockReason::NotAllowed.as_str(),
                    NetworkDecisionSource::BaselinePolicy,
                )
            }
        }
        HostBlockDecision::Blocked(reason) => NetworkDecision::deny_with_source(
            reason.as_str(),
            NetworkDecisionSource::BaselinePolicy,
        ),
    }
}

fn map_decider_decision(decision: NetworkDecision) -> NetworkDecision {
    match decision {
        NetworkDecision::Allow => NetworkDecision::Allow,
        NetworkDecision::Deny {
            reason, decision, ..
        } => NetworkDecision::Deny {
            reason,
            source: NetworkDecisionSource::Decider,
            decision,
        },
    }
}

/// The result of checking whether a host is blocked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostBlockDecision {
    Allowed,
    Blocked(HostBlockReason),
}

/// Reason a host was blocked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostBlockReason {
    NotAllowed,
    ExplicitlyDenied,
    LoopbackBlocked,
    DnsResolutionFailed,
    ResolvedToLoopback,
    PatternGuard(String),
}

impl HostBlockReason {
    pub fn as_str(&self) -> &str {
        match self {
            Self::NotAllowed => "not_allowed",
            Self::ExplicitlyDenied => "explicitly_denied",
            Self::LoopbackBlocked => "loopback_blocked",
            Self::DnsResolutionFailed => "dns_resolution_failed",
            Self::ResolvedToLoopback => "resolved_to_loopback",
            Self::PatternGuard(pattern) => pattern.as_str(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_as_policy_protocol() {
        assert_eq!(NetworkProtocol::Http.as_policy_protocol(), "http");
        assert_eq!(
            NetworkProtocol::HttpsConnect.as_policy_protocol(),
            "https_connect"
        );
    }

    #[test]
    fn decision_as_str() {
        assert_eq!(NetworkPolicyDecision::Deny.as_str(), "deny");
        assert_eq!(NetworkPolicyDecision::Ask.as_str(), "ask");
    }

    #[test]
    fn decision_is_methods() {
        assert!(NetworkPolicyDecision::Deny.is_denied());
        assert!(!NetworkPolicyDecision::Deny.is_ask());
        assert!(NetworkPolicyDecision::Ask.is_ask());
        assert!(!NetworkPolicyDecision::Ask.is_denied());
    }

    #[test]
    fn source_as_str() {
        assert_eq!(
            NetworkDecisionSource::BaselinePolicy.as_str(),
            "baseline_policy"
        );
        assert_eq!(NetworkDecisionSource::Decider.as_str(), "decider");
    }

    #[test]
    fn network_decision_allow() {
        let d = NetworkDecision::allow();
        assert!(d.is_allowed());
        assert!(!d.is_denied());
        assert!(!d.is_ask());
    }

    #[test]
    fn network_decision_deny() {
        let d = NetworkDecision::deny("blocked");
        assert!(!d.is_allowed());
        assert!(d.is_denied());
        assert!(!d.is_ask());
    }

    #[test]
    fn network_decision_ask() {
        let d = NetworkDecision::ask("need approval");
        assert!(!d.is_allowed());
        assert!(!d.is_denied());
        assert!(d.is_ask());
    }

    #[test]
    fn network_decision_deny_with_source() {
        let d =
            NetworkDecision::deny_with_source("blocked", NetworkDecisionSource::BaselinePolicy);
        assert!(d.is_denied());
    }

    #[test]
    fn network_decision_ask_with_source() {
        let d = NetworkDecision::ask_with_source("need approval", NetworkDecisionSource::ModeGuard);
        assert!(d.is_ask());
    }

    #[test]
    fn network_decision_empty_reason_defaults() {
        let d = NetworkDecision::deny("");
        match d {
            NetworkDecision::Deny { reason, .. } => {
                assert_eq!(reason, "policy denied");
            }
            _ => panic!("expected deny"),
        }
    }

    #[test]
    fn network_decision_json_roundtrip() {
        let d = NetworkDecision::deny_with_source(
            "test",
            NetworkDecisionSource::ModeGuard,
        );
        let json = serde_json::to_string(&d).unwrap();
        let deserialized: NetworkDecision = serde_json::from_str(&json).unwrap();
        assert!(!deserialized.is_allowed());
    }

    #[test]
    fn request_new_from_args() {
        let req = NetworkPolicyRequest::new(NetworkPolicyRequestArgs {
            protocol: NetworkProtocol::Http,
            host: "example.com".to_string(),
            port: 443,
            client_addr: None,
            method: Some("GET".to_string()),
            command: None,
            exec_policy_hint: None,
        });
        assert_eq!(req.host_port(), "example.com:443");
        assert!(!req.is_loopback());
    }

    #[test]
    fn request_loopback_detection() {
        let req = NetworkPolicyRequest::new(NetworkPolicyRequestArgs {
            protocol: NetworkProtocol::Http,
            host: "127.0.0.1".to_string(),
            port: 80,
            client_addr: None,
            method: None,
            command: None,
            exec_policy_hint: None,
        });
        assert!(req.is_loopback());
    }

    #[test]
    fn host_block_reason_as_str() {
        assert_eq!(HostBlockReason::NotAllowed.as_str(), "not_allowed");
        assert_eq!(
            HostBlockReason::ExplicitlyDenied.as_str(),
            "explicitly_denied"
        );
        assert_eq!(
            HostBlockReason::DnsResolutionFailed.as_str(),
            "dns_resolution_failed"
        );
    }

    #[tokio::test]
    async fn evaluate_host_policy_allowed() {
        let req = NetworkPolicyRequest::new(NetworkPolicyRequestArgs {
            protocol: NetworkProtocol::Http,
            host: "example.com".into(),
            port: 443,
            client_addr: None,
            method: None,
            command: None,
            exec_policy_hint: None,
        });
        let decision = evaluate_host_policy(HostBlockDecision::Allowed, None, &req).await;
        assert!(decision.is_allowed());
    }

    #[tokio::test]
    async fn evaluate_host_policy_blocked_no_decider() {
        let req = NetworkPolicyRequest::new(NetworkPolicyRequestArgs {
            protocol: NetworkProtocol::Http,
            host: "evil.com".into(),
            port: 80,
            client_addr: None,
            method: None,
            command: None,
            exec_policy_hint: None,
        });
        let decision = evaluate_host_policy(
            HostBlockDecision::Blocked(HostBlockReason::NotAllowed),
            None,
            &req,
        )
        .await;
        assert!(decision.is_denied());
    }

    #[tokio::test]
    async fn evaluate_host_policy_blocked_with_decider_override() {
        let decider: Arc<dyn NetworkPolicyDecider> =
            Arc::new(|_req: NetworkPolicyRequest| async { NetworkDecision::allow() });
        let req = NetworkPolicyRequest::new(NetworkPolicyRequestArgs {
            protocol: NetworkProtocol::Http,
            host: "example.com".into(),
            port: 443,
            client_addr: None,
            method: None,
            command: None,
            exec_policy_hint: None,
        });
        let decision = evaluate_host_policy(
            HostBlockDecision::Blocked(HostBlockReason::NotAllowed),
            Some(&decider),
            &req,
        )
        .await;
        assert!(decision.is_allowed());
    }
}
