use std::sync::Arc;

use dashmap::DashMap;
use fastclaw_execpolicy::{PolicyDecision, PolicyEngine};
use fastclaw_protocol::{AgentEvent, ApprovalDecision, PendingAction, TurnId};
use tokio::sync::Mutex;

use crate::guardian::GuardianReviewer;

/// Result of policy pre-check before entering the user-approval flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyRequirement {
    /// Policy explicitly allows — skip user approval entirely.
    Skip,
    /// Policy says "prompt" — fall through to user/Guardian approval.
    NeedsApproval { reason: String },
    /// Policy explicitly forbids — reject without asking the user.
    Forbidden { reason: String },
}

/// Map a tool call to a PendingAction for the approval pipeline.
pub fn map_tool_to_pending_action(
    tool_name: &str,
    arguments: &str,
    work_dir: Option<&str>,
) -> PendingAction {
    let args: serde_json::Value = serde_json::from_str(arguments).unwrap_or_default();
    let cwd = work_dir.unwrap_or(".").to_string();

    match tool_name {
        "shell_exec" | "sandboxed_shell_exec" => {
            let command = args
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or(arguments)
                .to_string();
            PendingAction::ShellCommand { command, cwd }
        }
        "write_file" | "create_file" => {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            PendingAction::FileWrite { path }
        }
        "edit_file" | "apply_diff" => {
            let path = args
                .get("path")
                .or_else(|| args.get("file_path"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            PendingAction::ApplyPatch {
                paths: vec![path],
            }
        }
        _ => {
            PendingAction::ShellCommand {
                command: format!("{tool_name}({arguments})"),
                cwd,
            }
        }
    }
}

/// Extract command tokens from a `PendingAction` for policy evaluation.
fn action_to_command_tokens(action: &PendingAction) -> Vec<String> {
    match action {
        PendingAction::ShellCommand { command, .. } => {
            shell_words::split(command).unwrap_or_else(|_| vec![command.clone()])
        }
        PendingAction::FileWrite { path } => vec!["write_file".into(), path.clone()],
        PendingAction::ApplyPatch { paths } => {
            let mut tokens = vec!["apply_patch".into()];
            tokens.extend(paths.iter().cloned());
            tokens
        }
        PendingAction::NetworkAccess { host, port } => {
            vec!["network".into(), host.clone(), port.to_string()]
        }
        _ => vec!["unknown".into()],
    }
}

/// Centralized tool approval pipeline.
///
/// Flow: PolicyCheck → Hooks → Guardian → User → Execute
///
/// This provides a unified decision pathway for tool calls that require
/// approval, replacing the inline `needs_confirmation` + re-exec pattern.
pub struct ToolOrchestrator {
    pending_approvals: Arc<DashMap<String, tokio::sync::oneshot::Sender<ApprovalDecision>>>,
    session_approvals: Arc<DashMap<String, ApprovalDecision>>,
    policy: Arc<Mutex<PolicyEngine>>,
    guardian: Option<Arc<GuardianReviewer>>,
}

impl ToolOrchestrator {
    pub fn new(
        pending_approvals: Arc<DashMap<String, tokio::sync::oneshot::Sender<ApprovalDecision>>>,
    ) -> Self {
        Self {
            pending_approvals,
            session_approvals: Arc::new(DashMap::new()),
            policy: Arc::new(Mutex::new(PolicyEngine::new())),
            guardian: None,
        }
    }

    /// Create an orchestrator with a pre-configured policy engine.
    pub fn with_policy(
        pending_approvals: Arc<DashMap<String, tokio::sync::oneshot::Sender<ApprovalDecision>>>,
        policy: PolicyEngine,
    ) -> Self {
        Self {
            pending_approvals,
            session_approvals: Arc::new(DashMap::new()),
            policy: Arc::new(Mutex::new(policy)),
            guardian: None,
        }
    }

    /// Attach a Guardian reviewer for automatic LLM-based safety review.
    pub fn with_guardian(mut self, guardian: Arc<GuardianReviewer>) -> Self {
        self.guardian = Some(guardian);
        self
    }

    /// Check policy for an action without entering the approval flow.
    pub async fn check_policy(&self, action: &PendingAction) -> PolicyRequirement {
        let tokens = action_to_command_tokens(action);
        let token_refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
        let policy = self.policy.lock().await;
        let eval = policy.evaluate(&token_refs);

        match eval.decision {
            PolicyDecision::Allow { .. } => PolicyRequirement::Skip,
            PolicyDecision::Forbidden { justification, .. } => {
                PolicyRequirement::Forbidden { reason: justification }
            }
            PolicyDecision::Prompt { reason, .. } => {
                PolicyRequirement::NeedsApproval { reason }
            }
        }
    }

    /// Run the approval pipeline for a pending action.
    ///
    /// Steps:
    /// 1. Check ExecPolicy — `Allow` skips approval, `Forbidden` rejects immediately
    /// 2. Check session-level cache
    /// 3. Otherwise request user confirmation
    pub async fn request_approval(
        &self,
        turn_id: &TurnId,
        action: PendingAction,
        reason: String,
        tx: &tokio::sync::mpsc::Sender<AgentEvent>,
    ) -> ApprovalDecision {
        // Step 1: ExecPolicy pre-check
        match self.check_policy(&action).await {
            PolicyRequirement::Skip => return ApprovalDecision::Approved,
            PolicyRequirement::Forbidden { reason } => {
                let _ = tx
                    .send(AgentEvent::ApprovalResolved {
                        turn_id: turn_id.clone(),
                        approval_id: String::new(),
                        decision: ApprovalDecision::Denied,
                        source: format!("policy: {reason}"),
                    })
                    .await;
                return ApprovalDecision::Denied;
            }
            PolicyRequirement::NeedsApproval { .. } => {}
        }

        // Step 1b: Guardian LLM review (if configured)
        if let Some(ref guardian) = self.guardian {
            if guardian.is_circuit_tripped().await {
                let _ = tx
                    .send(AgentEvent::GuardianWarning {
                        turn_id: turn_id.clone(),
                        message: "Guardian circuit breaker tripped: too many denials".into(),
                    })
                    .await;
                return ApprovalDecision::Denied;
            }

            let action_desc = format!("{action:?}");
            match guardian.review(&action_desc).await {
                Ok(assessment) => {
                    let _ = tx
                        .send(AgentEvent::GuardianAssessment {
                            turn_id: turn_id.clone(),
                            review_id: assessment.review_id.clone(),
                            risk_level: assessment.risk_level,
                            outcome: assessment.outcome,
                            rationale: assessment.rationale.clone(),
                        })
                        .await;

                    if assessment.outcome == fastclaw_protocol::GuardianOutcome::Allow {
                        return ApprovalDecision::Approved;
                    }
                    // Deny → fall through to user approval (fail-closed)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "guardian review failed, falling through to user approval");
                }
            }
        }

        let action_key = self.action_cache_key(&action);

        // Step 2: Check session-level cached approvals
        if let Some(cached) = self.session_approvals.get(&action_key) {
            return cached.clone();
        }

        // Step 3: Ask the user
        let approval_id = uuid::Uuid::new_v4().to_string();
        let (answer_tx, answer_rx) = tokio::sync::oneshot::channel::<ApprovalDecision>();
        self.pending_approvals.insert(approval_id.clone(), answer_tx);

        let available_decisions = vec![
            ApprovalDecision::Approved,
            ApprovalDecision::ApprovedForSession,
            ApprovalDecision::Denied,
            ApprovalDecision::Abort,
        ];

        let _ = tx
            .send(AgentEvent::ApprovalRequired {
                turn_id: turn_id.clone(),
                approval_id: approval_id.clone(),
                action: action.clone(),
                reason,
                available_decisions,
            })
            .await;

        let decision = match answer_rx.await {
            Ok(d) => d,
            Err(_) => ApprovalDecision::TimedOut,
        };

        self.pending_approvals.remove(&approval_id);

        if decision == ApprovalDecision::ApprovedForSession {
            // Cache for future calls and amend the policy with an allow rule
            self.session_approvals
                .insert(action_key, ApprovalDecision::Approved);

            let tokens = action_to_command_tokens(&action);
            let pattern_elements: Vec<fastclaw_execpolicy::PatternElement> = tokens
                .iter()
                .map(|t| fastclaw_execpolicy::PatternElement::Exact(t.clone()))
                .collect();
            let mut policy = self.policy.lock().await;
            policy.add_session_rule(fastclaw_execpolicy::PrefixRule {
                id: Some(format!("session_approved_{}", uuid::Uuid::new_v4())),
                pattern: pattern_elements,
                decision: "allow".into(),
                justification: Some("approved for session by user".into()),
            });
        }

        let _ = tx
            .send(AgentEvent::ApprovalResolved {
                turn_id: turn_id.clone(),
                approval_id,
                decision: decision.clone(),
                source: "user".to_string(),
            })
            .await;

        decision
    }

    /// Resolve a pending approval from the gateway (user input).
    pub fn resolve(&self, approval_id: &str, decision: ApprovalDecision) -> bool {
        if let Some((_, tx)) = self.pending_approvals.remove(approval_id) {
            let _ = tx.send(decision);
            true
        } else {
            false
        }
    }

    fn action_cache_key(&self, action: &PendingAction) -> String {
        match action {
            PendingAction::ShellCommand { command, cwd } => {
                format!("shell:{}:{}", cwd, command)
            }
            PendingAction::FileWrite { path } => format!("file_write:{path}"),
            PendingAction::ApplyPatch { paths } => format!("patch:{}", paths.join(",")),
            PendingAction::NetworkAccess { host, port } => format!("net:{host}:{port}"),
            _ => format!("unknown:{action:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn request_approval_resolves_via_gateway() {
        let pending = Arc::new(DashMap::new());
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let turn_id = TurnId::new("turn-1");

        let action = PendingAction::FileWrite {
            path: "/tmp/test.txt".into(),
        };

        let pending_for_task = pending.clone();
        let tx_for_task = tx.clone();
        let approval_task = tokio::spawn(async move {
            let orch = ToolOrchestrator::new(pending_for_task);
            orch.request_approval(
                &turn_id,
                action,
                "write file".into(),
                &tx_for_task,
            )
            .await
        });

        let required = rx.recv().await.unwrap();
        let approval_id = match required {
            AgentEvent::ApprovalRequired { approval_id, .. } => approval_id,
            other => panic!("expected ApprovalRequired, got {other:?}"),
        };

        let orch = ToolOrchestrator::new(pending);
        assert!(orch.resolve(&approval_id, ApprovalDecision::Approved));

        let decision = approval_task.await.unwrap();
        assert_eq!(decision, ApprovalDecision::Approved);

        let resolved = rx.recv().await.unwrap();
        match resolved {
            AgentEvent::ApprovalResolved { decision, .. } => {
                assert_eq!(decision, ApprovalDecision::Approved);
            }
            other => panic!("expected ApprovalResolved, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn approved_for_session_caches_subsequent_requests() {
        let pending = Arc::new(DashMap::new());
        let orchestrator = Arc::new(ToolOrchestrator::new(pending));
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let turn_id = TurnId::new("turn-2");

        let action = PendingAction::ShellCommand {
            command: "echo hi".into(),
            cwd: "/tmp".into(),
        };

        let first = tokio::spawn({
            let orchestrator = orchestrator.clone();
            let tx = tx.clone();
            let turn_id = turn_id.clone();
            let action = action.clone();
            async move {
                orchestrator
                    .request_approval(&turn_id, action, "run shell".into(), &tx)
                    .await
            }
        });

        let approval_id = match rx.recv().await.unwrap() {
            AgentEvent::ApprovalRequired { approval_id, .. } => approval_id,
            other => panic!("expected ApprovalRequired, got {other:?}"),
        };
        assert!(orchestrator.resolve(
            &approval_id,
            ApprovalDecision::ApprovedForSession,
        ));
        assert_eq!(first.await.unwrap(), ApprovalDecision::ApprovedForSession);
        let _ = rx.recv().await;

        let second = orchestrator
            .request_approval(&turn_id, action, "run shell again".into(), &tx)
            .await;
        assert_eq!(second, ApprovalDecision::Approved);
    }

    #[test]
    fn resolve_unknown_approval_returns_false() {
        let pending = Arc::new(DashMap::new());
        let orchestrator = ToolOrchestrator::new(pending);
        assert!(!orchestrator.resolve("missing", ApprovalDecision::Denied));
    }

    #[tokio::test]
    async fn policy_allow_skips_user_approval() {
        let pending = Arc::new(DashMap::new());
        let mut engine = PolicyEngine::new();
        engine
            .load_str(
                r#"
[[rules]]
pattern = ["echo"]
decision = "allow"
"#,
                "test",
            )
            .unwrap();

        let orch = ToolOrchestrator::with_policy(pending, engine);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let turn_id = TurnId::new("turn-1");
        let action = PendingAction::ShellCommand {
            command: "echo hello".into(),
            cwd: ".".into(),
        };

        let decision = orch
            .request_approval(&turn_id, action, "run echo".into(), &tx)
            .await;
        assert_eq!(decision, ApprovalDecision::Approved);
    }

    #[tokio::test]
    async fn policy_forbidden_denies_without_user_prompt() {
        let pending = Arc::new(DashMap::new());
        let mut engine = PolicyEngine::new();
        engine
            .load_str(
                r#"
[[rules]]
pattern = ["rm", "-rf", "/"]
decision = "forbidden"
justification = "destructive"
"#,
                "test",
            )
            .unwrap();

        let orch = ToolOrchestrator::with_policy(pending, engine);
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let turn_id = TurnId::new("turn-1");
        let action = PendingAction::ShellCommand {
            command: "rm -rf /".into(),
            cwd: ".".into(),
        };

        let decision = orch
            .request_approval(&turn_id, action, "danger".into(), &tx)
            .await;
        assert_eq!(decision, ApprovalDecision::Denied);

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, AgentEvent::ApprovalResolved { source, .. } if source.contains("policy")));
    }

    #[tokio::test]
    async fn check_policy_returns_requirement() {
        let pending = Arc::new(DashMap::new());
        let mut engine = PolicyEngine::new();
        engine
            .load_str(
                r#"
[[rules]]
pattern = ["ls"]
decision = "allow"
"#,
                "test",
            )
            .unwrap();

        let orch = ToolOrchestrator::with_policy(pending, engine);
        let action = PendingAction::ShellCommand {
            command: "ls -la".into(),
            cwd: ".".into(),
        };
        assert_eq!(orch.check_policy(&action).await, PolicyRequirement::Skip);

        let unknown = PendingAction::ShellCommand {
            command: "unknown-cmd".into(),
            cwd: ".".into(),
        };
        assert!(matches!(
            orch.check_policy(&unknown).await,
            PolicyRequirement::NeedsApproval { .. }
        ));
    }
}
