use serde::{Deserialize, Serialize};

#[cfg(feature = "ts")]
use ts_rs::TS;

/// Risk level inferred from the action content via rule-based classification.
///
/// Separate from Guardian's `RiskLevel` (which is LLM-assessed with 4 levels).
/// This is a quick, zero-latency heuristic for the approval UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "snake_case")]
pub enum ActionRiskLevel {
    /// Read-only or known-safe operations.
    Low,
    /// File write in workspace, known tools.
    Medium,
    /// rm -rf, sudo, write outside workspace, etc.
    High,
}

/// Decision returned by user, guardian, or hook for a pending action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "decision", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ApprovalDecision {
    Approved,
    ApprovedForSession,
    /// Approve ALL tool types for the remainder of this session/turn.
    /// Unlike `ApprovedForSession` (which is per-tool-type), this sets a
    /// global flag in the approval cache so no further prompts appear.
    ApprovedAllForSession,
    /// Approve and persist the command prefix as an ExecPolicy rule.
    /// Future commands matching this prefix are auto-approved.
    ApprovedWithPolicyAmend {
        prefix: Vec<String>,
    },
    Denied,
    TimedOut,
    Abort,
}

/// An action awaiting approval before execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "action_type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum PendingAction {
    ShellCommand {
        command: String,
        cwd: String,
    },
    FileWrite {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content: Option<String>,
    },
    ApplyPatch {
        paths: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diff: Option<String>,
    },
    NetworkAccess {
        host: String,
        port: u16,
    },
    McpToolCall {
        server_id: String,
        tool_name: String,
        arguments_summary: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_decision_roundtrip() {
        let decision = ApprovalDecision::ApprovedForSession;
        let json = serde_json::to_string(&decision).unwrap();
        let back: ApprovalDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ApprovalDecision::ApprovedForSession);
    }

    #[test]
    fn pending_action_shell_roundtrip() {
        let action = PendingAction::ShellCommand {
            command: "ls".into(),
            cwd: "/tmp".into(),
        };
        let json = serde_json::to_string(&action).unwrap();
        let back: PendingAction = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back,
            PendingAction::ShellCommand {
                command: "ls".into(),
                cwd: "/tmp".into(),
            }
        );
    }

    #[test]
    fn action_risk_level_roundtrip() {
        let level = ActionRiskLevel::High;
        let json = serde_json::to_string(&level).unwrap();
        assert_eq!(json, r#""high""#);
        let back: ActionRiskLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ActionRiskLevel::High);

        let low: ActionRiskLevel = serde_json::from_str(r#""low""#).unwrap();
        assert_eq!(low, ActionRiskLevel::Low);
    }

    #[test]
    fn pending_action_file_write_with_content() {
        let action = PendingAction::FileWrite {
            path: "/tmp/foo.txt".into(),
            content: Some("hello world".into()),
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains(r#""content":"hello world""#));
        let back: PendingAction = serde_json::from_str(&json).unwrap();
        assert_eq!(back, action);
    }

    #[test]
    fn pending_action_file_write_without_content_omits_field() {
        let action = PendingAction::FileWrite {
            path: "/tmp/foo.txt".into(),
            content: None,
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(!json.contains("content"));
        let back: PendingAction = serde_json::from_str(&json).unwrap();
        assert_eq!(back, action);
    }

    #[test]
    fn pending_action_apply_patch_with_diff() {
        let action = PendingAction::ApplyPatch {
            paths: vec!["src/main.rs".into()],
            diff: Some("-foo\n+bar".into()),
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("diff"));
        let back: PendingAction = serde_json::from_str(&json).unwrap();
        assert_eq!(back, action);
    }

    #[test]
    fn approved_with_policy_amend_roundtrip() {
        let decision = ApprovalDecision::ApprovedWithPolicyAmend {
            prefix: vec!["npm".into()],
        };
        let json = serde_json::to_string(&decision).unwrap();
        assert!(json.contains(r#""decision":"approved_with_policy_amend""#));
        assert!(json.contains(r#""prefix":["npm"]"#));
        let back: ApprovalDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(back, decision);
    }

    #[test]
    fn approved_with_policy_amend_multi_token() {
        let decision = ApprovalDecision::ApprovedWithPolicyAmend {
            prefix: vec!["cargo".into(), "build".into()],
        };
        let json = serde_json::to_string(&decision).unwrap();
        let back: ApprovalDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(back, decision);
    }
}
