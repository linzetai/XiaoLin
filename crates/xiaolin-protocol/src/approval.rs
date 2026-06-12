use serde::{Deserialize, Serialize};

#[cfg(feature = "ts")]
use ts_rs::TS;

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
    ShellCommand { command: String, cwd: String },
    FileWrite { path: String },
    ApplyPatch { paths: Vec<String> },
    NetworkAccess { host: String, port: u16 },
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
}
