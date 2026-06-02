use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use fastclaw_core::tool::{Tool, ToolExposure, ToolKind, ToolParameterSchema, ToolResult};
use fastclaw_core::types::ExecutionMode;

use super::plan_file::PlanFileStore;

const MODE_AGENT: u8 = 0;
const MODE_PLAN: u8 = 1;
const MODE_COORDINATOR: u8 = 2;

tokio::task_local! {
    /// Per-session mode state set by the runtime before tool execution.
    /// Plan mode tools read this to mutate the correct session's state.
    static CURRENT_SESSION_MODE: ExecutionModeState;
    /// Per-session plan context (session_id + PlanFileStore) for plan file I/O.
    static PLAN_CONTEXT: PlanContext;
}

/// Plan file context available to plan mode tools via task-local.
#[derive(Clone)]
pub struct PlanContext {
    pub session_id: String,
    pub store: PlanFileStore,
}

/// Wrap a future so plan mode tools can access the session-specific mode state
/// and plan file context.
pub async fn with_session_mode<F, T>(
    mode_state: ExecutionModeState,
    plan_ctx: Option<PlanContext>,
    fut: F,
) -> T
where
    F: std::future::Future<Output = T>,
{
    let with_mode = CURRENT_SESSION_MODE.scope(mode_state, async {
        if let Some(pc) = plan_ctx {
            PLAN_CONTEXT.scope(pc, fut).await
        } else {
            fut.await
        }
    });
    with_mode.await
}

pub fn current_plan_context() -> Option<PlanContext> {
    PLAN_CONTEXT.try_with(|c| c.clone()).ok()
}

fn mode_from_u8(v: u8) -> ExecutionMode {
    match v {
        MODE_PLAN => ExecutionMode::Plan,
        MODE_COORDINATOR => ExecutionMode::Coordinator,
        _ => ExecutionMode::Agent,
    }
}

fn mode_to_u8(m: ExecutionMode) -> u8 {
    match m {
        ExecutionMode::Agent => MODE_AGENT,
        ExecutionMode::Plan => MODE_PLAN,
        ExecutionMode::Coordinator => MODE_COORDINATOR,
    }
}

/// Shared execution mode state. Thread-safe via AtomicU8.
///
/// The runtime and tools share this via `Arc`. The tool executor checks
/// `current_mode()` before executing write/edit/execute tools; if the
/// mode is `Plan`, those tools are blocked with a friendly message.
#[derive(Debug)]
pub struct ExecutionModeState {
    state: Arc<AtomicU8>,
    /// Number of turns spent in Plan mode since last entry (for attachment throttling).
    plan_turn_counter: Arc<AtomicU32>,
    /// Whether the agent has previously exited Plan mode (for reentry detection).
    has_exited_plan: Arc<AtomicBool>,
}

impl Clone for ExecutionModeState {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            plan_turn_counter: Arc::clone(&self.plan_turn_counter),
            has_exited_plan: Arc::clone(&self.has_exited_plan),
        }
    }
}

impl Default for ExecutionModeState {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutionModeState {
    pub fn new() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(MODE_AGENT)),
            plan_turn_counter: Arc::new(AtomicU32::new(0)),
            has_exited_plan: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn current_mode(&self) -> ExecutionMode {
        mode_from_u8(self.state.load(Ordering::Acquire))
    }

    /// Centralized mode transition. Updates all tracking state atomically:
    /// - On entry to Plan: resets `plan_turn_counter`
    /// - On exit from Plan: sets `has_exited_plan` to true
    ///
    /// Returns `(from, to)`. If already in the target mode, `from == to`.
    pub fn transition(&self, target: ExecutionMode) -> (ExecutionMode, ExecutionMode) {
        let new_val = mode_to_u8(target);
        let old_val = self.state.swap(new_val, Ordering::AcqRel);
        let from = mode_from_u8(old_val);

        if from != target {
            match target {
                ExecutionMode::Plan => {
                    self.plan_turn_counter.store(0, Ordering::Release);
                }
                ExecutionMode::Agent if from == ExecutionMode::Plan => {
                    self.has_exited_plan.store(true, Ordering::Release);
                }
                _ => {}
            }
        }

        (from, target)
    }

    /// Current plan turn counter value.
    pub fn plan_turn_count(&self) -> u32 {
        self.plan_turn_counter.load(Ordering::Acquire)
    }

    /// Increment the plan turn counter by 1. Returns the value *before* increment.
    pub fn increment_plan_turn(&self) -> u32 {
        self.plan_turn_counter.fetch_add(1, Ordering::AcqRel)
    }

    /// Whether the agent has previously exited plan mode (for reentry detection).
    pub fn has_exited_plan(&self) -> bool {
        self.has_exited_plan.load(Ordering::Acquire)
    }

    /// Whether the current mode blocks the given tool kind.
    /// Note: shell_exec (ToolKind::Execute) handles its own Plan mode validation
    /// internally via `validate_readonly_command`, so use `is_blocked_for_tool`
    /// when you have the tool name.
    pub fn is_blocked(&self, kind: ToolKind) -> bool {
        if self.current_mode() != ExecutionMode::Plan {
            return false;
        }
        matches!(kind, ToolKind::Edit | ToolKind::Execute)
    }

    /// Whether the current mode blocks the given tool by name and kind.
    /// `shell_exec` is exempted from the blanket Execute block because it
    /// performs its own readonly command classification internally.
    pub fn is_blocked_for_tool(&self, tool_name: &str, kind: ToolKind) -> bool {
        if self.current_mode() != ExecutionMode::Plan {
            return false;
        }
        if tool_name == "shell_exec" {
            return false;
        }
        matches!(kind, ToolKind::Edit | ToolKind::Execute)
    }

    /// Human-readable message when a tool is blocked by plan mode.
    pub fn blocked_message(tool_name: &str) -> String {
        format!(
            "Tool '{tool_name}' is blocked in Plan mode (read-only). \
             Use exit_plan_mode to return to Agent mode before making changes."
        )
    }
}

// ─── SessionModeRegistry ─────────────────────────────────────────────

/// Per-session execution mode registry.
///
/// Each chat session gets its own independent `ExecutionModeState`, so
/// switching one session to Plan mode doesn't affect other sessions.
#[derive(Debug, Clone)]
pub struct SessionModeRegistry {
    modes: Arc<DashMap<String, ExecutionModeState>>,
}

impl Default for SessionModeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionModeRegistry {
    pub fn new() -> Self {
        Self {
            modes: Arc::new(DashMap::new()),
        }
    }

    /// Get (or create) the mode state for a session.
    pub fn get_or_create(&self, session_id: &str) -> ExecutionModeState {
        self.modes
            .entry(session_id.to_string())
            .or_default()
            .clone()
    }

    /// Convenience: transition a session's mode directly.
    pub fn transition(
        &self,
        session_id: &str,
        target: ExecutionMode,
    ) -> (ExecutionMode, ExecutionMode) {
        self.get_or_create(session_id).transition(target)
    }

    /// Current mode for a session (defaults to Agent if unknown).
    pub fn current_mode(&self, session_id: &str) -> ExecutionMode {
        self.modes
            .get(session_id)
            .map(|ms| ms.current_mode())
            .unwrap_or(ExecutionMode::Agent)
    }

    /// Remove a session's mode state (e.g. on session delete).
    pub fn remove(&self, session_id: &str) {
        self.modes.remove(session_id);
    }
}

// ─── EnterPlanModeTool ───────────────────────────────────────────────

/// Switches the agent to plan mode (read-only exploration).
/// Write/edit/execute tools are blocked until `exit_plan_mode` is called.
pub struct EnterPlanModeTool {
    mode_state: ExecutionModeState,
}

impl EnterPlanModeTool {
    pub fn new(mode_state: ExecutionModeState) -> Self {
        Self { mode_state }
    }
}

#[async_trait]
impl Tool for EnterPlanModeTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Think
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn name(&self) -> &str {
        "enter_plan_mode"
    }

    fn description(&self) -> &str {
        "Switch to plan mode for read-only exploration and design. \
         Write/edit/execute tools are blocked until exit_plan_mode is called."
    }

    fn prompt(&self) -> String {
        "Switch to plan mode for collaborative design before implementation.\n\n\
## When to Enter Plan Mode\n\n\
- Task has multiple valid approaches with significant trade-offs\n\
- Architectural decisions needed (caching strategy, data model, API design)\n\
- Large refactors touching many files or systems\n\
- Requirements are unclear and need exploration before committing to a direction\n\
- User asks for a plan, design, or approach discussion\n\n\
## What Changes in Plan Mode\n\n\
**Available tools (read-only):**\n\
- read_file, search_in_files, glob, list_directory (explore code)\n\
- shell_exec with READONLY commands only (ls, cat, grep, git status, cargo check)\n\
- web_search, web_fetch (research)\n\
- todo_write (planning)\n\
- task_create (delegate exploration)\n\n\
**Blocked tools (write/edit/execute):**\n\
- write_file, edit_file, multi_edit, apply_patch (file modifications)\n\
- shell_exec with write commands (rm, mv, git commit, cargo install)\n\n\
## When NOT to Enter Plan Mode\n\n\
- Task is straightforward with an obvious implementation\n\
- You've already gathered enough context and are ready to code\n\
- The task is a simple fix or small change\n\
- User explicitly asked you to implement something\n\n\
## Workflow\n\n\
1. Enter plan mode to explore and understand\n\
2. Read relevant code, search for patterns, check tests\n\
3. Propose approach to user with trade-offs\n\
4. Exit plan mode when ready to implement\n\
5. Execute the agreed-upon plan in agent mode"
            .to_string()
    }

    fn search_hint(&self) -> &str {
        "switch to plan mode design approach before coding"
    }

    fn is_deferred(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: HashMap::new(),
            required: vec![],
        }
    }

    async fn execute(&self, _arguments: &str) -> ToolResult {
        let ms = CURRENT_SESSION_MODE
            .try_with(|s| s.clone())
            .unwrap_or_else(|_| self.mode_state.clone());
        let (from, _to) = ms.transition(ExecutionMode::Plan);

        if from == ExecutionMode::Plan {
            return ToolResult::ok("Already in plan mode.");
        }

        let plan_info = if let Some(pc) = current_plan_context() {
            let path = pc.store.plan_path(&pc.session_id);
            if pc.store.plan_exists(&pc.session_id) {
                format!(
                    "\n\n## Re-entering Plan Mode\n\
                     A plan file already exists at: {}\n\
                     Read it first to understand what was previously planned. Then decide:\n\
                     - **Different task**: Overwrite with a fresh plan\n\
                     - **Same task, continuing**: Update the existing plan incrementally",
                    path.display()
                )
            } else {
                format!("\n\nPlan file will be saved to: {}", path.display())
            }
        } else {
            String::new()
        };

        ToolResult::ok(format!(
            "Entered plan mode (was: {from}).\n\n\
             In plan mode:\n\
             1. Explore the codebase with read/search tools\n\
             2. Identify patterns and approaches\n\
             3. Design an implementation strategy\n\
             4. Write your plan to the plan file\n\
             5. Use exit_plan_mode when ready to start coding\n\n\
             DO NOT write or edit any files except the plan file. This is a read-only phase.\
             {plan_info}"
        ))
    }
}

// ─── ExitPlanModeTool ────────────────────────────────────────────────

/// Exits plan mode, restoring full tool access (Agent mode).
pub struct ExitPlanModeTool {
    mode_state: ExecutionModeState,
}

impl ExitPlanModeTool {
    pub fn new(mode_state: ExecutionModeState) -> Self {
        Self { mode_state }
    }
}

#[async_trait]
impl Tool for ExitPlanModeTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Think
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn name(&self) -> &str {
        "exit_plan_mode"
    }

    fn description(&self) -> &str {
        "Exit plan mode and return to agent mode with full tool access. \
         Optionally verify plan execution by providing plan_summary and \
         all_steps_completed (combines former verify_plan_execution). \
         Call this after designing your approach to start implementation."
    }

    fn prompt(&self) -> String {
        "Exit plan mode and return to agent mode with full tool access.\n\n\
## When to Exit\n\
- You have a clear implementation plan agreed with the user\n\
- Requirements are sufficiently understood to begin coding\n\
- The user explicitly says to proceed with implementation\n\n\
## Before Exiting — Verify\n\
1. You have identified all files that need changes\n\
2. You understand the approach and trade-offs\n\
3. You have a todo list if the task has multiple steps\n\
4. The user has confirmed the approach (if there were choices)\n\n\
## After Exiting\n\
- All write/edit/execute tools become available\n\
- Start implementing the plan immediately\n\
- Reference your plan notes and todo list during implementation\n\n\
## Anti-Patterns\n\
- Don't exit plan mode just because you're impatient\n\
- Don't exit without a clear direction\n\
- Don't ask the user 'should I exit plan mode?' — just do it when ready"
            .to_string()
    }

    fn search_hint(&self) -> &str {
        "exit plan mode start coding implementation"
    }

    fn is_deferred(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "plan_summary".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional summary of the plan being executed. Enables verification."
            }),
        );
        props.insert(
            "all_steps_completed".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "Optional. Whether all planned steps are completed. \
                 If false, warns about incomplete work instead of exiting. \
                 Only relevant when plan_summary is provided."
            }),
        );
        props.insert(
            "verification_notes".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional notes about skipped or changed steps."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec![],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value =
            serde_json::from_str(arguments).unwrap_or(serde_json::json!({}));

        let plan_summary = args.get("plan_summary").and_then(|v| v.as_str());
        let all_completed = args
            .get("all_steps_completed")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let notes = args
            .get("verification_notes")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if let Some(summary) = plan_summary {
            if !all_completed {
                let notes_part = if notes.is_empty() {
                    String::new()
                } else {
                    format!("\n\nVerification notes: {notes}")
                };
                return ToolResult::ok(format!(
                    "⚠ Plan verification: INCOMPLETE\n\n\
                     Plan: {summary}\n\n\
                     Not all steps have been completed. Complete remaining \
                     steps before exiting plan mode.{notes_part}"
                ));
            }
        }

        let ms = CURRENT_SESSION_MODE
            .try_with(|s| s.clone())
            .unwrap_or_else(|_| self.mode_state.clone());

        if ms.current_mode() == ExecutionMode::Agent {
            return ToolResult::ok("Already in agent mode.");
        }

        let verify_msg = if let Some(summary) = plan_summary {
            let notes_part = if notes.is_empty() {
                String::new()
            } else {
                format!("\nVerification notes: {notes}")
            };
            format!("\n✓ Plan verified: {summary}{notes_part}")
        } else {
            String::new()
        };

        let (plan_path_str, plan_exists, plan_preview) =
            if let Some(pc) = current_plan_context() {
                let path = pc.store.plan_path(&pc.session_id);
                let exists = pc.store.plan_exists(&pc.session_id);
                let preview = if exists {
                    let content = pc.store.read_plan(&pc.session_id).unwrap_or_default();
                    if content.len() > 500 {
                        let end = content.floor_char_boundary(500);
                        format!(
                            "{}...\n(truncated, read full file for details)",
                            &content[..end]
                        )
                    } else {
                        content
                    }
                } else {
                    String::new()
                };
                (path.display().to_string(), exists, preview)
            } else {
                (String::new(), false, String::new())
            };

        if !plan_exists {
            ms.transition(ExecutionMode::Agent);
            return ToolResult::ok(format!(
                "Switched back to agent mode — full tool access restored.\
                 {verify_msg}\n\n\
                 No plan file was written during this session. \
                 You now have full tool access to proceed."
            ));
        }

        let plan_ref = if !plan_path_str.is_empty() {
            format!(
                "\n\n## Plan File\nSaved at: {plan_path_str}\n\n{plan_preview}\n\n\
                 The user will review this plan and decide the next step."
            )
        } else {
            String::new()
        };

        let metadata = serde_json::json!({
            "approval_pending": true,
            "plan_path": plan_path_str,
            "plan_exists": plan_exists,
        });

        let mut result = ToolResult::ok(format!(
            "Plan complete — waiting for user approval.\
             {verify_msg}{plan_ref}\n\n\
             The user can choose to start implementation (switch to Agent mode) \
             or continue planning (stay in Plan mode)."
        ));
        result.metadata = Some(metadata);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    // ─── VerifyPlanExecutionTool (test-only, superseded by ExitPlanModeTool) ──

    #[derive(Deserialize)]
    struct VerifyPlanArgs {
        plan_summary: String,
        all_steps_completed: bool,
        #[serde(default)]
        verification_notes: Option<String>,
    }

    struct VerifyPlanExecutionTool {
        mode_state: ExecutionModeState,
    }

    impl VerifyPlanExecutionTool {
        fn new(mode_state: ExecutionModeState) -> Self {
            Self { mode_state }
        }
    }

    #[async_trait]
    impl Tool for VerifyPlanExecutionTool {
        fn kind(&self) -> ToolKind {
            ToolKind::Think
        }
        fn name(&self) -> &str {
            "verify_plan_execution"
        }
        fn description(&self) -> &str {
            "Verify that a plan has been fully executed before exiting plan mode."
        }
        fn parameters_schema(&self) -> ToolParameterSchema {
            let mut props = HashMap::new();
            props.insert(
                "plan_summary".to_string(),
                serde_json::json!({"type": "string"}),
            );
            props.insert(
                "all_steps_completed".to_string(),
                serde_json::json!({"type": "boolean"}),
            );
            props.insert(
                "verification_notes".to_string(),
                serde_json::json!({"type": "string"}),
            );
            ToolParameterSchema {
                schema_type: "object".to_string(),
                properties: props,
                required: vec![
                    "plan_summary".to_string(),
                    "all_steps_completed".to_string(),
                ],
            }
        }
        async fn execute(&self, arguments: &str) -> ToolResult {
            let args: VerifyPlanArgs = match serde_json::from_str(arguments) {
                Ok(v) => v,
                Err(e) => return ToolResult::err(format!("Invalid arguments: {e}")),
            };
            let current = self.mode_state.current_mode();
            let mode_note = if current != ExecutionMode::Plan {
                "\n\nNote: You are not currently in plan mode."
            } else {
                ""
            };
            if !args.all_steps_completed {
                let notes_section = args
                    .verification_notes
                    .as_deref()
                    .filter(|n| !n.is_empty())
                    .map(|n| format!("\n\nVerification notes: {n}"))
                    .unwrap_or_default();
                return ToolResult::ok(format!(
                    "⚠ Plan verification: INCOMPLETE\n\nPlan: {}\n\n\
                     Not all steps have been completed. Please review your plan \
                     and complete the remaining steps before exiting plan mode.\
                     {notes_section}{mode_note}",
                    args.plan_summary
                ));
            }
            let notes_section = args
                .verification_notes
                .as_deref()
                .filter(|n| !n.is_empty())
                .map(|n| format!("\n\nVerification notes: {n}"))
                .unwrap_or_default();
            ToolResult::ok(format!(
                "✓ Plan verification: COMPLETE\n\nPlan: {}\n\n\
                 All steps have been completed. You may now safely call \
                 exit_plan_mode to return to agent mode and begin implementation.\
                 {notes_section}{mode_note}",
                args.plan_summary
            ))
        }
    }

    // ─── SessionModeRegistry tests ───────────────────────────────────

    #[test]
    fn session_mode_registry_independent_sessions() {
        let reg = SessionModeRegistry::new();
        reg.transition("sess-1", ExecutionMode::Plan);
        assert_eq!(reg.current_mode("sess-1"), ExecutionMode::Plan);
        assert_eq!(reg.current_mode("sess-2"), ExecutionMode::Agent);
    }

    #[test]
    fn session_mode_registry_remove() {
        let reg = SessionModeRegistry::new();
        reg.get_or_create("sess-1");
        reg.remove("sess-1");
        assert_eq!(reg.current_mode("sess-1"), ExecutionMode::Agent);
    }

    // ─── ExecutionModeState tests ────────────────────────────────────

    #[test]
    fn mode_state_default_is_agent() {
        let state = ExecutionModeState::new();
        assert_eq!(state.current_mode(), ExecutionMode::Agent);
    }

    #[test]
    fn mode_state_transition_to_plan() {
        let state = ExecutionModeState::new();
        let (from, to) = state.transition(ExecutionMode::Plan);
        assert_eq!(from, ExecutionMode::Agent);
        assert_eq!(to, ExecutionMode::Plan);
        assert_eq!(state.current_mode(), ExecutionMode::Plan);
    }

    #[test]
    fn mode_state_transition_back_to_agent() {
        let state = ExecutionModeState::new();
        state.transition(ExecutionMode::Plan);
        let (from, to) = state.transition(ExecutionMode::Agent);
        assert_eq!(from, ExecutionMode::Plan);
        assert_eq!(to, ExecutionMode::Agent);
    }

    #[test]
    fn mode_state_idempotent_transition() {
        let state = ExecutionModeState::new();
        let (from, to) = state.transition(ExecutionMode::Agent);
        assert_eq!(from, ExecutionMode::Agent);
        assert_eq!(to, ExecutionMode::Agent);
    }

    #[test]
    fn is_blocked_in_plan_mode() {
        let state = ExecutionModeState::new();
        state.transition(ExecutionMode::Plan);

        assert!(state.is_blocked(ToolKind::Edit));
        assert!(state.is_blocked(ToolKind::Execute));
        assert!(!state.is_blocked(ToolKind::Read));
        assert!(!state.is_blocked(ToolKind::Search));
        assert!(!state.is_blocked(ToolKind::Fetch));
        assert!(!state.is_blocked(ToolKind::Think));
    }

    #[test]
    fn is_not_blocked_in_agent_mode() {
        let state = ExecutionModeState::new();
        assert!(!state.is_blocked(ToolKind::Edit));
        assert!(!state.is_blocked(ToolKind::Execute));
    }

    #[tokio::test]
    async fn enter_plan_mode_tool() {
        let state = ExecutionModeState::new();
        let tool = EnterPlanModeTool::new(state.clone());

        let result = tool.execute("{}").await;
        assert!(result.success);
        assert!(result.output.contains("Entered plan mode"));
        assert!(result.output.contains("read-only"));
        assert_eq!(state.current_mode(), ExecutionMode::Plan);
    }

    #[tokio::test]
    async fn enter_plan_mode_already_in_plan() {
        let state = ExecutionModeState::new();
        state.transition(ExecutionMode::Plan);
        let tool = EnterPlanModeTool::new(state.clone());

        let result = tool.execute("{}").await;
        assert!(result.success);
        assert!(result.output.contains("Already in plan mode"));
    }

    #[tokio::test]
    async fn exit_plan_mode_tool_no_plan_file() {
        let state = ExecutionModeState::new();
        state.transition(ExecutionMode::Plan);
        let tool = ExitPlanModeTool::new(state.clone());

        let result = tool.execute("{}").await;
        assert!(result.success);
        assert!(
            result.output.contains("agent mode") && result.output.contains("No plan file"),
            "without plan file, should directly switch to agent mode, got: {}",
            result.output
        );
        assert_eq!(
            state.current_mode(),
            ExecutionMode::Agent,
            "without plan file, should transition directly to Agent mode"
        );
        assert!(
            result.metadata.is_none(),
            "without plan file, should NOT set approval_pending metadata"
        );
    }

    #[tokio::test]
    async fn exit_plan_mode_already_in_agent() {
        let state = ExecutionModeState::new();
        let tool = ExitPlanModeTool::new(state.clone());

        let result = tool.execute("{}").await;
        assert!(result.success);
        assert!(result.output.contains("Already in agent mode"));
    }

    #[tokio::test]
    async fn roundtrip_enter_exit_no_plan() {
        let state = ExecutionModeState::new();
        let enter = EnterPlanModeTool::new(state.clone());
        let exit = ExitPlanModeTool::new(state.clone());

        assert_eq!(state.current_mode(), ExecutionMode::Agent);

        enter.execute("{}").await;
        assert_eq!(state.current_mode(), ExecutionMode::Plan);
        assert!(state.is_blocked(ToolKind::Edit));

        exit.execute("{}").await;
        // Without a plan file, exit_plan_mode transitions directly to Agent
        assert_eq!(
            state.current_mode(),
            ExecutionMode::Agent,
            "without plan file, exit should go directly to Agent mode"
        );
        assert!(!state.is_blocked(ToolKind::Edit));
    }

    #[test]
    fn blocked_message_format() {
        let msg = ExecutionModeState::blocked_message("write_file");
        assert!(msg.contains("write_file"));
        assert!(msg.contains("Plan mode"));
        assert!(msg.contains("exit_plan_mode"));
    }

    // ═══════════════════════════════════════════════════════════════
    // VerifyPlanExecutionTool tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn verify_plan_all_completed() {
        let state = ExecutionModeState::new();
        state.transition(ExecutionMode::Plan);
        let tool = VerifyPlanExecutionTool::new(state);

        let result = tool
            .execute(r#"{"plan_summary":"Refactor auth module","all_steps_completed":true}"#)
            .await;
        assert!(result.success);
        assert!(result.output.contains("COMPLETE"));
        assert!(result.output.contains("Refactor auth module"));
        assert!(result.output.contains("exit_plan_mode"));
        assert!(!result.output.contains("not currently in plan mode"));
    }

    #[tokio::test]
    async fn verify_plan_incomplete() {
        let state = ExecutionModeState::new();
        state.transition(ExecutionMode::Plan);
        let tool = VerifyPlanExecutionTool::new(state);

        let result = tool
            .execute(r#"{"plan_summary":"Add caching","all_steps_completed":false}"#)
            .await;
        assert!(result.success);
        assert!(result.output.contains("INCOMPLETE"));
        assert!(result.output.contains("Add caching"));
        assert!(result.output.contains("remaining steps"));
    }

    #[tokio::test]
    async fn verify_plan_with_notes() {
        let state = ExecutionModeState::new();
        state.transition(ExecutionMode::Plan);
        let tool = VerifyPlanExecutionTool::new(state);

        let result = tool
            .execute(
                r#"{"plan_summary":"DB migration","all_steps_completed":true,"verification_notes":"Skipped index step"}"#,
            )
            .await;
        assert!(result.success);
        assert!(result.output.contains("COMPLETE"));
        assert!(result.output.contains("Skipped index step"));
    }

    #[tokio::test]
    async fn verify_plan_incomplete_with_notes() {
        let state = ExecutionModeState::new();
        state.transition(ExecutionMode::Plan);
        let tool = VerifyPlanExecutionTool::new(state);

        let result = tool
            .execute(
                r#"{"plan_summary":"API redesign","all_steps_completed":false,"verification_notes":"Step 3 blocked by external dep"}"#,
            )
            .await;
        assert!(result.success);
        assert!(result.output.contains("INCOMPLETE"));
        assert!(result.output.contains("Step 3 blocked by external dep"));
    }

    #[tokio::test]
    async fn verify_plan_not_in_plan_mode() {
        let state = ExecutionModeState::new();
        let tool = VerifyPlanExecutionTool::new(state);

        let result = tool
            .execute(r#"{"plan_summary":"Test plan","all_steps_completed":true}"#)
            .await;
        assert!(result.success);
        assert!(result.output.contains("COMPLETE"));
        assert!(result.output.contains("not currently in plan mode"));
    }

    #[tokio::test]
    async fn verify_plan_invalid_args() {
        let state = ExecutionModeState::new();
        let tool = VerifyPlanExecutionTool::new(state);

        let result = tool.execute(r#"{"plan_summary":"missing bool"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("Invalid arguments"));
    }

    #[tokio::test]
    async fn verify_plan_empty_notes_ignored() {
        let state = ExecutionModeState::new();
        state.transition(ExecutionMode::Plan);
        let tool = VerifyPlanExecutionTool::new(state);

        let result = tool
            .execute(
                r#"{"plan_summary":"Cleanup","all_steps_completed":true,"verification_notes":""}"#,
            )
            .await;
        assert!(result.success);
        assert!(result.output.contains("COMPLETE"));
        assert!(!result.output.contains("Verification notes"));
    }
}
