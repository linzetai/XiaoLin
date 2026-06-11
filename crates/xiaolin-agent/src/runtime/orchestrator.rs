use std::path::Path;
use std::sync::Arc;

use xiaolin_core::tool_runtime::{
    ApprovalStrategy, DecisionSource, ExecApprovalRequirement, OrchestratorResult, SandboxAttempt,
    SandboxBackend, ToolExecContext, ToolProgressEvent, ToolRuntimeError,
};

use super::runtimes::ErasedToolRuntime;
use xiaolin_execpolicy::{PolicyDecision, PolicyEngine};
use xiaolin_protocol::{AgentEvent, ApprovalDecision, PendingAction, TurnId};
use xiaolin_session_actor::InteractionHandle;
use tokio::sync::Mutex;

use super::approval_cache::ApprovalCache;
use super::permissions::DenialTracker;
use crate::guardian::GuardianReviewer;


/// Result of policy pre-check before entering the user-approval flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyRequirement {
    Skip,
    NeedsApproval { reason: String },
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
        _ => PendingAction::ShellCommand {
            command: format!("{tool_name}({arguments})"),
            cwd,
        },
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

/// Context for a single orchestrator `run()` invocation.
pub struct OrchestratorContext<'a> {
    pub turn_id: &'a TurnId,
    pub cwd: &'a Path,
    pub call_id: &'a str,
    pub approval_cache: &'a mut ApprovalCache,
    pub approval_strategy: &'a ApprovalStrategy,
    pub interaction_handle: Option<&'a InteractionHandle>,
    pub event_tx: &'a tokio::sync::mpsc::Sender<AgentEvent>,
    pub denial_tracker: &'a mut DenialTracker,
}

/// Centralized tool approval + sandbox + execution pipeline.
///
/// The new unified orchestrator implements a 5-phase pipeline:
/// 1. Requirement — ask the runtime if approval/sandbox is needed
/// 2. Approval — resolve via cache, policy, guardian, or user
/// 3. Sandbox selection — pick the appropriate sandbox backend
/// 4. Execution — run the tool
/// 5. Escalation — retry without sandbox on sandbox denial (if allowed)
pub struct ToolOrchestrator {
    policy: Arc<Mutex<PolicyEngine>>,
    guardian: Option<Arc<GuardianReviewer>>,
}

impl Default for ToolOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolOrchestrator {
    pub fn new() -> Self {
        Self {
            policy: Arc::new(Mutex::new(PolicyEngine::new())),
            guardian: None,
        }
    }

    pub fn with_policy(policy: PolicyEngine) -> Self {
        Self {
            policy: Arc::new(Mutex::new(policy)),
            guardian: None,
        }
    }

    pub fn with_guardian(mut self, guardian: Arc<GuardianReviewer>) -> Self {
        self.guardian = Some(guardian);
        self
    }

    /// New unified 5-phase execution pipeline.
    pub async fn run(
        &self,
        runtime: &dyn ErasedToolRuntime,
        args: &serde_json::Value,
        ctx: &mut OrchestratorContext<'_>,
    ) -> Result<OrchestratorResult, ToolRuntimeError> {
        // Phase 1: Determine approval requirement
        let requirement = runtime.exec_requirement(args, ctx.cwd);

        // Phase 1.5: Check denial tracker — auto-deny previously denied operations
        if ctx.denial_tracker.is_denied(runtime.name(), &format!("{args}")) {
            return Err(ToolRuntimeError::Rejected {
                reason: "previously denied in this session".to_string(),
            });
        }

        // Phase 2: Resolve approval
        let decision_source = match requirement {
            ExecApprovalRequirement::Skip => DecisionSource::NotRequired,
            ExecApprovalRequirement::Forbidden { ref reason } => {
                ctx.denial_tracker
                    .record_denial(runtime.name(), &format!("{args}"));
                return Err(ToolRuntimeError::Rejected {
                    reason: reason.clone(),
                });
            }
            ExecApprovalRequirement::NeedsApproval { reason } => {
                match self.resolve_approval(runtime, args, ctx, &reason).await {
                    Ok(source) => source,
                    Err(e) => {
                        ctx.denial_tracker
                            .record_denial(runtime.name(), &format!("{args}"));
                        return Err(e);
                    }
                }
            }
        };

        // Phase 3: Select sandbox
        let sandbox = self.select_sandbox(runtime, ctx.cwd);

        // Phase 4: Execute with progress forwarding
        let tool_name = runtime.name().to_string();
        let call_id = ctx.call_id.to_string();
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<ToolProgressEvent>(64);

        let exec_ctx = ToolExecContext {
            turn_id: ctx.turn_id.clone(),
            session_id: xiaolin_protocol::SessionId::new(""),
            call_id: call_id.clone(),
            cwd: ctx.cwd.to_path_buf(),
            progress_tx: Some(progress_tx),
        };

        // Forward progress events to the agent event stream
        let progress_fwd = {
            let event_tx = ctx.event_tx.clone();
            let turn_id = ctx.turn_id.clone();
            let tool_name_fwd = tool_name.clone();
            let call_id_fwd = call_id.clone();
            tokio::spawn(async move {
                while let Some(evt) = progress_rx.recv().await {
                    let _ = event_tx.send(AgentEvent::ToolProgress {
                        turn_id: turn_id.clone(),
                        tool_name: tool_name_fwd.clone(),
                        call_id: call_id_fwd.clone(),
                        message: evt.message,
                        progress: evt.progress,
                        partial_output: evt.partial_output,
                    }).await;
                }
            })
        };

        let run_result = runtime.run(args, &sandbox, &exec_ctx).await;
        // Drop the context to close progress_tx, allowing the forwarding task to finish
        drop(exec_ctx);
        let _ = progress_fwd.await;

        match run_result {
            Ok(output) => Ok(OrchestratorResult {
                output,
                decision_source,
                sandbox_used: sandbox.sandbox_type,
            }),
            Err(ToolRuntimeError::SandboxDenied { reason }) => {
                // Phase 5: Escalation
                if runtime.escalate_on_sandbox_failure() {
                    let no_sandbox = SandboxAttempt {
                        sandbox_type: SandboxBackend::None,
                        cwd: ctx.cwd.to_path_buf(),
                    };
                    let escalation_ctx = ToolExecContext {
                        turn_id: ctx.turn_id.clone(),
                        session_id: xiaolin_protocol::SessionId::new(""),
                        call_id: call_id.clone(),
                        cwd: ctx.cwd.to_path_buf(),
                        progress_tx: None,
                    };
                    let output = runtime.run(args, &no_sandbox, &escalation_ctx).await?;
                    Ok(OrchestratorResult {
                        output,
                        decision_source,
                        sandbox_used: SandboxBackend::None,
                    })
                } else {
                    Err(ToolRuntimeError::SandboxDenied { reason })
                }
            }
            Err(e) => Err(e),
        }
    }

    /// Authorization-only pipeline: phases 1–3 (requirement, approval, sandbox
    /// selection) without executing the tool. Returns `Ok(())` if the call is
    /// permitted; callers can then execute the tool through any path they choose,
    /// preserving rich `ToolResult` data that `runtime.run()` would discard.
    pub async fn authorize(
        &self,
        runtime: &dyn ErasedToolRuntime,
        args: &serde_json::Value,
        ctx: &mut OrchestratorContext<'_>,
    ) -> Result<(), ToolRuntimeError> {
        let requirement = runtime.exec_requirement(args, ctx.cwd);

        if ctx.denial_tracker.is_denied(runtime.name(), &format!("{args}")) {
            return Err(ToolRuntimeError::Rejected {
                reason: "previously denied in this session".to_string(),
            });
        }

        match requirement {
            ExecApprovalRequirement::Skip => Ok(()),
            ExecApprovalRequirement::Forbidden { ref reason } => {
                ctx.denial_tracker
                    .record_denial(runtime.name(), &format!("{args}"));
                Err(ToolRuntimeError::Rejected {
                    reason: reason.clone(),
                })
            }
            ExecApprovalRequirement::NeedsApproval { reason } => {
                match self.resolve_approval(runtime, args, ctx, &reason).await {
                    Ok(_source) => Ok(()),
                    Err(e) => {
                        ctx.denial_tracker
                            .record_denial(runtime.name(), &format!("{args}"));
                        Err(e)
                    }
                }
            }
        }
    }

    /// Phase 2 implementation: resolve approval through the pipeline.
    async fn resolve_approval(
        &self,
        runtime: &dyn ErasedToolRuntime,
        args: &serde_json::Value,
        ctx: &mut OrchestratorContext<'_>,
        reason: &str,
    ) -> Result<DecisionSource, ToolRuntimeError> {
        let keys = runtime.approval_keys(args);

        // 2a: Check cache
        if ctx.approval_cache.check(&keys).is_some() {
            return Ok(DecisionSource::Cached);
        }

        // 2b: Check ExecPolicy
        let action = runtime.to_pending_action(args, ctx.cwd);
        let tokens = action_to_command_tokens(&action);
        let token_refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
        let policy_decision = {
            let policy = self.policy.lock().await;
            policy.evaluate(&token_refs).decision
        };

        match policy_decision {
            PolicyDecision::Allow { .. } => return Ok(DecisionSource::PolicyAllowed),
            PolicyDecision::Forbidden { justification, .. } => {
                return Err(ToolRuntimeError::Rejected {
                    reason: justification,
                });
            }
            PolicyDecision::Prompt { .. } => { /* fall through to strategy */ }
        }

        // 2c: Guardian review (if configured)
        if let Some(ref guardian) = self.guardian {
            if !guardian.is_circuit_tripped().await {
                let action_desc = format!("{action:?}");
                match guardian.review(&action_desc).await {
                    Ok(assessment) => {
                        let _ = ctx
                            .event_tx
                            .send(AgentEvent::GuardianAssessment {
                                turn_id: ctx.turn_id.clone(),
                                review_id: assessment.review_id.clone(),
                                risk_level: assessment.risk_level,
                                outcome: assessment.outcome,
                                rationale: assessment.rationale.clone(),
                            })
                            .await;

                        if assessment.outcome == xiaolin_protocol::GuardianOutcome::Allow {
                            ctx.approval_cache
                                .store(&keys, ApprovalDecision::ApprovedForSession);
                            return Ok(DecisionSource::GuardianAllowed);
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "guardian review failed, falling through");
                    }
                }
            }
        }

        // 2d: Strategy-based resolution
        match ctx.approval_strategy {
            ApprovalStrategy::AutoApprove => {
                ctx.approval_cache
                    .store(&keys, ApprovalDecision::ApprovedForSession);
                Ok(DecisionSource::AutoApproved)
            }
            ApprovalStrategy::DenyAll => Err(ToolRuntimeError::Rejected {
                reason: "approval required but strategy is DenyAll".into(),
            }),
            ApprovalStrategy::PolicyBased => Err(ToolRuntimeError::Rejected {
                reason: "tool requires approval but no interactive session available".into(),
            }),
            ApprovalStrategy::Interactive => {
                let ih = ctx.interaction_handle.ok_or(ToolRuntimeError::Internal {
                    message: "Interactive strategy requires an InteractionHandle".into(),
                })?;

                let approval_id = uuid::Uuid::new_v4().to_string();
                let available_decisions = vec![
                    ApprovalDecision::Approved,
                    ApprovalDecision::ApprovedForSession,
                    ApprovalDecision::Denied,
                    ApprovalDecision::Abort,
                ];

                let _ = ctx
                    .event_tx
                    .send(AgentEvent::ApprovalRequired {
                        turn_id: ctx.turn_id.clone(),
                        approval_id: approval_id.clone(),
                        action: action.clone(),
                        reason: reason.to_string(),
                        available_decisions,
                        session_id: None,
                    })
                    .await;

                let rx = ih.request_approval(approval_id.clone(), &action);
                let decision = rx.await.unwrap_or(ApprovalDecision::TimedOut);

                if decision == ApprovalDecision::ApprovedForSession {
                    ctx.approval_cache
                        .store(&keys, ApprovalDecision::ApprovedForSession);
                }

                let _ = ctx
                    .event_tx
                    .send(AgentEvent::ApprovalResolved {
                        turn_id: ctx.turn_id.clone(),
                        approval_id,
                        decision: decision.clone(),
                        source: "user".to_string(),
                    })
                    .await;

                match decision {
                    ApprovalDecision::Approved | ApprovalDecision::ApprovedForSession => {
                        Ok(DecisionSource::UserApproved)
                    }
                    ApprovalDecision::Denied => Err(ToolRuntimeError::Rejected {
                        reason: "user denied".into(),
                    }),
                    ApprovalDecision::Abort => Err(ToolRuntimeError::Rejected {
                        reason: "user aborted".into(),
                    }),
                    ApprovalDecision::TimedOut => Err(ToolRuntimeError::Rejected {
                        reason: "approval timed out".into(),
                    }),
                    _ => Err(ToolRuntimeError::Rejected {
                        reason: "unexpected approval decision".into(),
                    }),
                }
            }
        }
    }

    /// Phase 3: Select sandbox based on runtime preference.
    fn select_sandbox(&self, runtime: &dyn ErasedToolRuntime, cwd: &Path) -> SandboxAttempt {
        use xiaolin_core::tool_runtime::SandboxPreference;
        match runtime.sandbox_preference() {
            SandboxPreference::Skip => SandboxAttempt {
                sandbox_type: SandboxBackend::None,
                cwd: cwd.to_path_buf(),
            },
            SandboxPreference::Auto | SandboxPreference::Required => {
                let backend = Self::detect_platform_sandbox();
                SandboxAttempt {
                    sandbox_type: backend,
                    cwd: cwd.to_path_buf(),
                }
            }
        }
    }

    fn detect_platform_sandbox() -> SandboxBackend {
        #[cfg(target_os = "linux")]
        {
            SandboxBackend::Landlock
        }
        #[cfg(target_os = "macos")]
        {
            SandboxBackend::Seatbelt
        }
        #[cfg(target_os = "windows")]
        {
            SandboxBackend::RestrictedToken
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            SandboxBackend::None
        }
    }

    // ─── Legacy API (kept during migration, used by mod.rs) ───────────────────

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
            PolicyDecision::Prompt { reason, .. } => PolicyRequirement::NeedsApproval { reason },
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaolin_core::tool_runtime::ToolRuntime;

    #[tokio::test]
    async fn check_policy_returns_requirement() {
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

        let orch = ToolOrchestrator::with_policy(engine);
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

    // ─── New pipeline tests ──────────────────────────────────────────────────

    use xiaolin_core::tool_runtime::SandboxPreference as CoreSandboxPref;

    struct MockRuntime {
        requirement: ExecApprovalRequirement,
        sandbox_pref: CoreSandboxPref,
        output: String,
    }

    impl xiaolin_core::tool_runtime::Approvable for MockRuntime {
        fn approval_keys(&self, _args: &serde_json::Value) -> Vec<String> {
            vec!["mock:test".to_string()]
        }
        fn exec_requirement(
            &self,
            _args: &serde_json::Value,
            _cwd: &Path,
        ) -> ExecApprovalRequirement {
            self.requirement.clone()
        }
        fn to_pending_action(
            &self,
            _args: &serde_json::Value,
            cwd: &Path,
        ) -> PendingAction {
            PendingAction::ShellCommand {
                command: "mock".into(),
                cwd: cwd.display().to_string(),
            }
        }
    }

    impl xiaolin_core::tool_runtime::Sandboxable for MockRuntime {
        fn sandbox_preference(&self) -> CoreSandboxPref {
            self.sandbox_pref
        }
    }

    #[async_trait::async_trait]
    impl ToolRuntime for MockRuntime {
        async fn run(
            &self,
            _args: &serde_json::Value,
            _sandbox: &SandboxAttempt,
            _ctx: &ToolExecContext,
        ) -> Result<String, ToolRuntimeError> {
            Ok(self.output.clone())
        }
        fn name(&self) -> &str {
            "mock"
        }
    }

    #[tokio::test]
    async fn new_pipeline_skip_approval_executes() {
        let orch = ToolOrchestrator::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let turn_id = TurnId::new("t1");

        let runtime = MockRuntime {
            requirement: ExecApprovalRequirement::Skip,
            sandbox_pref: CoreSandboxPref::Skip,
            output: "ok".into(),
        };

        let mut cache = ApprovalCache::new();
        let mut tracker = DenialTracker::new();
        let mut ctx = OrchestratorContext {
            turn_id: &turn_id,
            cwd: Path::new("/tmp"),
            call_id: "test-call-1",
            approval_cache: &mut cache,
            approval_strategy: &ApprovalStrategy::DenyAll,
            interaction_handle: None,
            event_tx: &tx,
            denial_tracker: &mut tracker,
        };

        let result = orch.run(&runtime, &serde_json::json!({}), &mut ctx).await;
        assert!(result.is_ok());
        let r = result.unwrap();
        assert_eq!(r.output, "ok");
        assert_eq!(r.decision_source, DecisionSource::NotRequired);
    }

    #[tokio::test]
    async fn new_pipeline_forbidden_rejects() {
        let orch = ToolOrchestrator::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let turn_id = TurnId::new("t1");

        let runtime = MockRuntime {
            requirement: ExecApprovalRequirement::Forbidden {
                reason: "dangerous".into(),
            },
            sandbox_pref: CoreSandboxPref::Skip,
            output: "should not run".into(),
        };

        let mut cache = ApprovalCache::new();
        let mut tracker = DenialTracker::new();
        let mut ctx = OrchestratorContext {
            turn_id: &turn_id,
            cwd: Path::new("/tmp"),
            call_id: "test-call-2",
            approval_cache: &mut cache,
            approval_strategy: &ApprovalStrategy::AutoApprove,
            interaction_handle: None,
            event_tx: &tx,
            denial_tracker: &mut tracker,
        };

        let result = orch.run(&runtime, &serde_json::json!({}), &mut ctx).await;
        assert!(matches!(
            result,
            Err(ToolRuntimeError::Rejected { reason }) if reason == "dangerous"
        ));
        assert_eq!(tracker.count(), 1);
    }

    #[tokio::test]
    async fn new_pipeline_auto_approve_executes() {
        let orch = ToolOrchestrator::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let turn_id = TurnId::new("t1");

        let runtime = MockRuntime {
            requirement: ExecApprovalRequirement::NeedsApproval {
                reason: "needs it".into(),
            },
            sandbox_pref: CoreSandboxPref::Skip,
            output: "executed".into(),
        };

        let mut cache = ApprovalCache::new();
        let mut tracker = DenialTracker::new();
        let mut ctx = OrchestratorContext {
            turn_id: &turn_id,
            cwd: Path::new("/tmp"),
            call_id: "test-call-3",
            approval_cache: &mut cache,
            approval_strategy: &ApprovalStrategy::AutoApprove,
            interaction_handle: None,
            event_tx: &tx,
            denial_tracker: &mut tracker,
        };

        let result = orch.run(&runtime, &serde_json::json!({}), &mut ctx).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().decision_source, DecisionSource::AutoApproved);
    }

    #[tokio::test]
    async fn new_pipeline_deny_all_rejects() {
        let orch = ToolOrchestrator::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let turn_id = TurnId::new("t1");

        let runtime = MockRuntime {
            requirement: ExecApprovalRequirement::NeedsApproval {
                reason: "needs it".into(),
            },
            sandbox_pref: CoreSandboxPref::Skip,
            output: "nope".into(),
        };

        let mut cache = ApprovalCache::new();
        let mut tracker = DenialTracker::new();
        let mut ctx = OrchestratorContext {
            turn_id: &turn_id,
            cwd: Path::new("/tmp"),
            call_id: "test-call-4",
            approval_cache: &mut cache,
            approval_strategy: &ApprovalStrategy::DenyAll,
            interaction_handle: None,
            event_tx: &tx,
            denial_tracker: &mut tracker,
        };

        let result = orch.run(&runtime, &serde_json::json!({}), &mut ctx).await;
        assert!(matches!(result, Err(ToolRuntimeError::Rejected { .. })));
        assert_eq!(tracker.count(), 1);
    }

    #[tokio::test]
    async fn new_pipeline_cache_hit_skips_approval() {
        let orch = ToolOrchestrator::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let turn_id = TurnId::new("t1");

        let runtime = MockRuntime {
            requirement: ExecApprovalRequirement::NeedsApproval {
                reason: "needs it".into(),
            },
            sandbox_pref: CoreSandboxPref::Skip,
            output: "cached".into(),
        };

        let mut cache = ApprovalCache::new();
        cache.store(
            &["mock:test".to_string()],
            ApprovalDecision::ApprovedForSession,
        );
        let mut tracker = DenialTracker::new();

        let mut ctx = OrchestratorContext {
            turn_id: &turn_id,
            cwd: Path::new("/tmp"),
            call_id: "test-call-5",
            approval_cache: &mut cache,
            approval_strategy: &ApprovalStrategy::DenyAll,
            interaction_handle: None,
            event_tx: &tx,
            denial_tracker: &mut tracker,
        };

        let result = orch.run(&runtime, &serde_json::json!({}), &mut ctx).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().decision_source, DecisionSource::Cached);
    }
}
