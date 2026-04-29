use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolResult};
use fastclaw_core::types::ExecutionMode;
use serde::Deserialize;

const MODE_AGENT: u8 = 0;
const MODE_PLAN: u8 = 1;

fn mode_from_u8(v: u8) -> ExecutionMode {
    if v == MODE_PLAN {
        ExecutionMode::Plan
    } else {
        ExecutionMode::Agent
    }
}

fn mode_to_u8(m: ExecutionMode) -> u8 {
    match m {
        ExecutionMode::Agent => MODE_AGENT,
        ExecutionMode::Plan => MODE_PLAN,
    }
}

/// Shared execution mode state. Thread-safe via AtomicU8.
///
/// The runtime and tools share this via `Arc`. The tool executor checks
/// `current_mode()` before executing write/edit/execute tools; if the
/// mode is `Plan`, those tools are blocked with a friendly message.
#[derive(Debug, Clone)]
pub struct ExecutionModeState {
    state: Arc<AtomicU8>,
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
        }
    }

    pub fn current_mode(&self) -> ExecutionMode {
        mode_from_u8(self.state.load(Ordering::Acquire))
    }

    /// Try to transition to the given mode. Returns `(from, to)`.
    /// If already in the target mode, `from == to`.
    pub fn transition(&self, target: ExecutionMode) -> (ExecutionMode, ExecutionMode) {
        let new_val = mode_to_u8(target);
        let old_val = self.state.swap(new_val, Ordering::AcqRel);
        (mode_from_u8(old_val), target)
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
5. Execute the agreed-upon plan in agent mode".to_string()
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
        let (from, _to) = self.mode_state.transition(ExecutionMode::Plan);

        if from == ExecutionMode::Plan {
            return ToolResult::ok("Already in plan mode.");
        }

        ToolResult::ok(format!(
            "Entered plan mode (was: {from}).\n\n\
             In plan mode:\n\
             1. Explore the codebase with read/search tools\n\
             2. Identify patterns and approaches\n\
             3. Design an implementation strategy\n\
             4. Use exit_plan_mode when ready to start coding\n\n\
             DO NOT write or edit any files. This is a read-only phase."
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

    fn name(&self) -> &str {
        "exit_plan_mode"
    }

    fn description(&self) -> &str {
        "Exit plan mode and return to agent mode with full tool access. \
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
- Don't ask the user 'should I exit plan mode?' — just do it when ready".to_string()
    }

    fn search_hint(&self) -> &str {
        "exit plan mode start coding implementation"
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
        let (from, _to) = self.mode_state.transition(ExecutionMode::Agent);

        if from == ExecutionMode::Agent {
            return ToolResult::ok("Already in agent mode.");
        }

        ToolResult::ok(
            "Exited plan mode → agent mode. All tools are now available.\n\
             You can proceed with implementation."
                .to_string(),
        )
    }
}

// ─── VerifyPlanExecutionTool ─────────────────────────────────────────

#[derive(Deserialize)]
struct VerifyPlanArgs {
    plan_summary: String,
    all_steps_completed: bool,
    #[serde(default)]
    verification_notes: Option<String>,
}

/// Validates that a plan has been fully executed before exiting plan mode.
///
/// The agent should call this tool before `exit_plan_mode` to confirm
/// all planned steps have been carried out. When `all_steps_completed`
/// is false, the tool warns the agent about incomplete work.
pub struct VerifyPlanExecutionTool {
    mode_state: ExecutionModeState,
}

impl VerifyPlanExecutionTool {
    pub fn new(mode_state: ExecutionModeState) -> Self {
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
        "Verify that a plan has been fully executed before exiting plan mode. \
         Provide a summary, whether all steps are completed, and optional notes."
    }

    fn search_hint(&self) -> &str {
        "verify plan execution check steps completed before exit"
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
                "description": "A brief summary of the plan that was being executed."
            }),
        );
        props.insert(
            "all_steps_completed".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "Whether all planned steps have been completed."
            }),
        );
        props.insert(
            "verification_notes".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional notes about the verification (e.g. what was skipped or changed)."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["plan_summary".to_string(), "all_steps_completed".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: VerifyPlanArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::err(format!(
                    "Invalid arguments: {e}. Expected \
                     {{\"plan_summary\": \"...\", \"all_steps_completed\": true/false, \
                     \"verification_notes\": \"...\"}}"
                ))
            }
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
                "⚠ Plan verification: INCOMPLETE\n\n\
                 Plan: {summary}\n\n\
                 Not all steps have been completed. Please review your plan \
                 and complete the remaining steps before exiting plan mode.\
                 {notes_section}{mode_note}"
                , summary = args.plan_summary
            ));
        }

        let notes_section = args
            .verification_notes
            .as_deref()
            .filter(|n| !n.is_empty())
            .map(|n| format!("\n\nVerification notes: {n}"))
            .unwrap_or_default();

        ToolResult::ok(format!(
            "✓ Plan verification: COMPLETE\n\n\
             Plan: {summary}\n\n\
             All steps have been completed. You may now safely call \
             exit_plan_mode to return to agent mode and begin implementation.\
             {notes_section}{mode_note}"
            , summary = args.plan_summary
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    async fn exit_plan_mode_tool() {
        let state = ExecutionModeState::new();
        state.transition(ExecutionMode::Plan);
        let tool = ExitPlanModeTool::new(state.clone());

        let result = tool.execute("{}").await;
        assert!(result.success);
        assert!(result.output.contains("agent mode"));
        assert!(result.output.contains("All tools are now available"));
        assert_eq!(state.current_mode(), ExecutionMode::Agent);
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
    async fn roundtrip_enter_exit() {
        let state = ExecutionModeState::new();
        let enter = EnterPlanModeTool::new(state.clone());
        let exit = ExitPlanModeTool::new(state.clone());

        assert_eq!(state.current_mode(), ExecutionMode::Agent);

        enter.execute("{}").await;
        assert_eq!(state.current_mode(), ExecutionMode::Plan);
        assert!(state.is_blocked(ToolKind::Edit));

        exit.execute("{}").await;
        assert_eq!(state.current_mode(), ExecutionMode::Agent);
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
