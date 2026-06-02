use crate::network_policy::{NetworkDecisionSource, NetworkPolicyDecision, NetworkProtocol};
use crate::reasons::*;
use serde::Serialize;

pub struct PolicyDecisionDetails<'a> {
    pub decision: NetworkPolicyDecision,
    pub reason: &'a str,
    pub source: NetworkDecisionSource,
    pub protocol: NetworkProtocol,
    pub host: &'a str,
    pub port: u16,
}

pub fn blocked_header_value(reason: &str) -> &'static str {
    match reason {
        REASON_CONNECT_BLOCKED => "blocked-by-allowlist",
        REASON_DENIED => "blocked-by-denylist",
        REASON_MODE_GUARD => "blocked-by-method-policy",
        REASON_BLOCKED_IP | REASON_LOOPBACK_BLOCKED => "blocked-by-local-binding",
        REASON_UNIX_SOCKET_BLOCKED => "blocked-by-unix-socket",
        REASON_NOT_ENABLED => "blocked-proxy-disabled",
        _ => "blocked-by-policy",
    }
}

pub fn blocked_message(details: &PolicyDecisionDetails<'_>) -> String {
    let base = match details.reason {
        REASON_CONNECT_BLOCKED => "Domain not in allowlist.",
        REASON_DENIED => "Domain denied by the sandbox policy.",
        REASON_MODE_GUARD => "Method not allowed in limited mode.",
        REASON_BLOCKED_IP | REASON_LOOPBACK_BLOCKED => {
            "Sandbox policy blocks local/private network addresses."
        }
        REASON_UNIX_SOCKET_BLOCKED => "Unix socket access not allowed.",
        REASON_NOT_ENABLED => "Network proxy is disabled.",
        REASON_NO_PROXY => "Request not proxied.",
        _ => "Request blocked by network policy.",
    };
    format!(
        "{base} (host={}, port={}, protocol={}, source={}, decision={})",
        details.host,
        details.port,
        details.protocol.as_policy_protocol(),
        details.source.as_str(),
        details.decision.as_str(),
    )
}

/// Serialize a blocked response as JSON.
pub fn json_blocked_response(details: &PolicyDecisionDetails<'_>) -> String {
    #[derive(Serialize)]
    struct BlockedJson<'a> {
        status: &'a str,
        host: &'a str,
        port: u16,
        reason: &'a str,
        decision: &'a str,
        source: &'a str,
        protocol: &'a str,
    }
    let payload = BlockedJson {
        status: "blocked",
        host: details.host,
        port: details.port,
        reason: details.reason,
        decision: details.decision.as_str(),
        source: details.source.as_str(),
        protocol: details.protocol.as_policy_protocol(),
    };
    serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_details<'a>(reason: &'a str, host: &'a str) -> PolicyDecisionDetails<'a> {
        PolicyDecisionDetails {
            decision: NetworkPolicyDecision::Deny,
            reason,
            source: NetworkDecisionSource::BaselinePolicy,
            protocol: NetworkProtocol::Http,
            host,
            port: 80,
        }
    }

    #[test]
    fn blocked_header_value_mapping() {
        assert_eq!(blocked_header_value(REASON_DENIED), "blocked-by-denylist");
        assert_eq!(
            blocked_header_value(REASON_CONNECT_BLOCKED),
            "blocked-by-allowlist"
        );
        assert_eq!(
            blocked_header_value(REASON_MODE_GUARD),
            "blocked-by-method-policy"
        );
        assert_eq!(
            blocked_header_value(REASON_NOT_ENABLED),
            "blocked-proxy-disabled"
        );
        assert_eq!(
            blocked_header_value("unknown_reason"),
            "blocked-by-policy"
        );
    }

    #[test]
    fn blocked_message_contains_host_and_reason() {
        let details = make_details(REASON_DENIED, "evil.com");
        let msg = blocked_message(&details);
        assert!(msg.contains("evil.com"));
        assert!(msg.contains("Domain denied"));
    }

    #[test]
    fn json_blocked_response_roundtrip() {
        let details = make_details(REASON_DENIED, "bad.com");
        let json = json_blocked_response(&details);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["status"], "blocked");
        assert_eq!(v["host"], "bad.com");
        assert_eq!(v["reason"], REASON_DENIED);
        assert_eq!(v["decision"], "deny");
        assert_eq!(v["source"], "baseline_policy");
        assert_eq!(v["protocol"], "http");
        assert_eq!(v["port"], 80);
    }
}
