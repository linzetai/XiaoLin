//! Stop hook evaluation: when the LLM finishes (no tool_calls), decide
//! whether the agent should continue for another iteration or truly stop.
//!
//! Each hook is a pure function that inspects the conversation state and
//! returns a [`StopHookResult`].  The first hook that says `should_continue`
//! wins — the agent injects its `continuation_message` and re-enters the loop.

use crate::builtin_tools::{
    ContinuationActivityResult, GoalStatus, GoalStore, TodoItem, TodoStatus, TodoStore,
};
use xiaolin_protocol::ExecutionMode;

/// Outcome of evaluating stop hooks after the LLM finishes a turn.
#[derive(Debug, Clone)]
pub(crate) struct StopHookResult {
    /// If true the agent loop should continue with another LLM call.
    pub should_continue: bool,
    /// Message injected into the conversation to guide the next iteration.
    pub continuation_message: Option<String>,
    /// Which hook triggered the continuation (for logging).
    pub reason: &'static str,
}

impl StopHookResult {
    pub fn stop() -> Self {
        Self {
            should_continue: false,
            continuation_message: None,
            reason: "none",
        }
    }

    fn cont(reason: &'static str, message: String) -> Self {
        Self {
            should_continue: true,
            continuation_message: Some(message),
            reason,
        }
    }
}

/// Recovery state flags passed from QueryLoopState to stop hooks.
///
/// When the agent has exhausted its recovery budget (max-output retries,
/// reactive compact), continuation hooks like truncation should be
/// suppressed to avoid infinite retry loops.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RecoveryState {
    pub max_output_recovery_exhausted: bool,
}

/// Evaluate all stop hooks in priority order.
///
/// Returns as soon as any hook says `should_continue = true`.
/// If no hook fires, returns a "stop" result.
///
/// # Arguments
/// * `last_assistant_text` — the text content of the assistant's final message
/// * `finish_reason` — the LLM's finish_reason (e.g. "stop", "length")
/// * `todo_store` — optional reference to the session's todo store
/// * `queued_slash_commands` — any pending slash commands from the user
/// * `goal_store` — optional reference to the session's goal store
/// * `execution_mode` — current execution mode; goal hook is skipped in Plan mode
/// * `recovery` — recovery state from `QueryLoopState`
#[allow(clippy::too_many_arguments)]
pub(crate) async fn evaluate_stop_hooks(
    _last_assistant_text: &str,
    finish_reason: Option<&str>,
    todo_store: Option<&TodoStore>,
    queued_slash_commands: &[String],
    goal_store: Option<&GoalStore>,
    execution_mode: Option<ExecutionMode>,
    had_tool_calls: bool,
    had_progress: bool,
    recovery: RecoveryState,
) -> StopHookResult {
    // Hook 0: Active goal continuation (highest priority for long-running tasks)
    // Skipped in Plan mode — goal continuation should not override plan-mode read-only behavior
    let is_plan_mode = execution_mode == Some(ExecutionMode::Plan);
    if !is_plan_mode {
        if let Some(store) = goal_store {
            if let Some(result) = check_active_goal(store, had_tool_calls, had_progress).await {
                return result;
            }
        }
    }

    // Hook 1: Incomplete todo items
    if let Some(store) = todo_store {
        if let Some(result) = check_incomplete_todos(store).await {
            return result;
        }
    }

    // Hook 2: Output truncation (finish_reason=length)
    // Skip when max_output recovery is exhausted to prevent infinite retry loops.
    if !recovery.max_output_recovery_exhausted {
        if let Some(result) = check_truncation(finish_reason) {
            return result;
        }
    }

    // Hook 3: Queued slash commands
    if let Some(result) = check_queued_commands(queued_slash_commands) {
        return result;
    }

    StopHookResult::stop()
}

/// Hook 0: If there is an active/budget-limited goal, inject a continuation prompt.
/// Uses `get_current()` to also catch `BudgetLimited` goals that `get_active()` would miss.
/// Enforces a max-round safety limit and idle detection to prevent runaway loops.
async fn check_active_goal(
    store: &GoalStore,
    had_tool_calls: bool,
    had_progress: bool,
) -> Option<StopHookResult> {
    let goal = store.get_current().await?;

    match goal.status {
        GoalStatus::Active => {
            if store.increment_rounds(&goal.id).await {
                tracing::warn!(goal_id = %goal.id, "goal hit max continuation rounds, pausing");
                let _ = store
                    .update_status(&goal.id, GoalStatus::Paused, Some("max_rounds"))
                    .await;
                return Some(StopHookResult::cont(
                    "goal_max_rounds",
                    "<goal_context>\n\
                     [GOAL PAUSED — MAX ROUNDS]\n\n\
                     The goal has been automatically paused after reaching the maximum \
                     continuation round limit. Please summarize progress so far and let \
                     the user know they can resume the goal.\n\
                     </goal_context>"
                        .to_string(),
                ));
            }
            match store
                .record_continuation_activity(had_tool_calls, had_progress)
                .await
            {
                ContinuationActivityResult::IdleLimitReached => {
                    tracing::warn!(goal_id = %goal.id, "goal idle for too many rounds, pausing");
                    let _ = store
                        .update_status(&goal.id, GoalStatus::Paused, Some("idle_rounds"))
                        .await;
                    return Some(StopHookResult::cont(
                        "goal_idle",
                        "<goal_context>\n\
                         [GOAL PAUSED — NO PROGRESS]\n\n\
                         The goal has been automatically paused because no tool calls \
                         were made for several consecutive rounds. The agent may be stuck. \
                         Please summarize what was accomplished and explain any blockers \
                         to the user.\n\
                         </goal_context>"
                            .to_string(),
                    ));
                }
                ContinuationActivityResult::StagnationLimitReached => {
                    tracing::warn!(goal_id = %goal.id, "goal stagnating (read-only loops), pausing");
                    let _ = store
                        .update_status(&goal.id, GoalStatus::Paused, Some("stagnation"))
                        .await;
                    return Some(StopHookResult::cont(
                        "goal_stagnation",
                        "<goal_context>\n\
                         [GOAL PAUSED — STAGNATION DETECTED]\n\n\
                         The goal has been automatically paused because multiple consecutive \
                         rounds only performed read operations (glob, read_file, list_directory) \
                         without making any progress (no file writes, no commands executed). \
                         This usually means the agent is stuck in a verification loop.\n\n\
                         Please either:\n\
                         1. Mark the goal as completed if the work is actually done\n\
                         2. Identify what's blocking progress and take action\n\
                         3. Ask the user for guidance\n\
                         </goal_context>"
                            .to_string(),
                    ));
                }
                ContinuationActivityResult::Normal => {}
            }
            let prompt = if store.take_objective_updated().await {
                crate::runtime::goal_prompts::render_objective_updated_prompt(&goal)
            } else {
                crate::runtime::goal_prompts::render_continuation_prompt(&goal)
            };
            Some(StopHookResult::cont("active_goal", prompt))
        }
        GoalStatus::BudgetLimited => {
            let prompt = crate::runtime::goal_prompts::render_budget_limit_prompt(&goal);
            let _ = store
                .update_status(&goal.id, GoalStatus::Paused, Some("budget_exhausted"))
                .await;
            Some(StopHookResult::cont("goal_budget_limit", prompt))
        }
        _ => None,
    }
}

/// Hook 1: If there are pending or in-progress todo items, the agent
/// should continue working on them.
async fn check_incomplete_todos(store: &TodoStore) -> Option<StopHookResult> {
    let items = store.snapshot().await;
    let incomplete: Vec<&TodoItem> = items
        .iter()
        .filter(|t| matches!(t.status, TodoStatus::Pending | TodoStatus::InProgress))
        .collect();

    if incomplete.is_empty() {
        return None;
    }

    let summary: Vec<String> = incomplete
        .iter()
        .take(5)
        .map(|t| format!("- [{}] {}: {}", t.status, t.id, t.content))
        .collect();

    let remaining = if incomplete.len() > 5 {
        format!("\n  ... and {} more", incomplete.len() - 5)
    } else {
        String::new()
    };

    let subagent_hint = if incomplete.len() >= 3 {
        "\n\nParallelization hint: you have 3+ pending tasks. If they are independent \
         (no data dependency), use `spawn_subagent` to run them concurrently instead of \
         doing them one by one."
    } else {
        ""
    };

    Some(StopHookResult::cont(
        "incomplete_todos",
        format!(
            "[STOP HOOK: incomplete todos] You still have {} unfinished task(s):\n{}{}\n\n\
             Please continue working on the remaining tasks before ending your turn.{subagent_hint}",
            incomplete.len(),
            summary.join("\n"),
            remaining,
        ),
    ))
}

/// Hook 2: If the LLM's output was truncated (finish_reason=length),
/// it likely has more to say.
fn check_truncation(finish_reason: Option<&str>) -> Option<StopHookResult> {
    if finish_reason == Some("length") {
        Some(StopHookResult::cont(
            "output_truncated",
            "[STOP HOOK: output truncated] Your previous response was cut short \
             (finish_reason=length). Please continue from where you left off."
                .to_string(),
        ))
    } else {
        None
    }
}

/// Hook 3: If there are queued slash commands, execute them before stopping.
fn check_queued_commands(commands: &[String]) -> Option<StopHookResult> {
    if commands.is_empty() {
        return None;
    }

    Some(StopHookResult::cont(
        "queued_commands",
        format!(
            "[STOP HOOK: queued commands] There are {} pending command(s): {}. \
             Please process them before ending your turn.",
            commands.len(),
            commands.join(", "),
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stop_when_no_hooks_fire() {
        let result = evaluate_stop_hooks(
            "done",
            Some("stop"),
            None,
            &[],
            None,
            None,
            false,
            false,
            RecoveryState::default(),
        )
        .await;
        assert!(!result.should_continue);
        assert!(result.continuation_message.is_none());
        assert_eq!(result.reason, "none");
    }

    #[tokio::test]
    async fn continue_on_incomplete_todos() {
        let store = TodoStore::new();
        store
            .replace_all(vec![
                TodoItem {
                    id: "1".into(),
                    content: "Fix bug".into(),
                    status: TodoStatus::Pending,
                    created_at: String::new(),
                    completed_at: None,
                },
                TodoItem {
                    id: "2".into(),
                    content: "Write tests".into(),
                    status: TodoStatus::InProgress,
                    created_at: String::new(),
                    completed_at: None,
                },
                TodoItem {
                    id: "3".into(),
                    content: "Deploy".into(),
                    status: TodoStatus::Completed,
                    created_at: String::new(),
                    completed_at: None,
                },
            ])
            .await;

        let result = evaluate_stop_hooks(
            "done",
            Some("stop"),
            Some(&store),
            &[],
            None,
            None,
            false,
            false,
            RecoveryState::default(),
        )
        .await;
        assert!(result.should_continue);
        assert_eq!(result.reason, "incomplete_todos");
        let msg = result.continuation_message.unwrap();
        assert!(msg.contains("2 unfinished"));
        assert!(msg.contains("Fix bug"));
        assert!(msg.contains("Write tests"));
        assert!(!msg.contains("Deploy"));
    }

    #[tokio::test]
    async fn stop_when_all_todos_completed() {
        let store = TodoStore::new();
        store
            .replace_all(vec![TodoItem {
                id: "1".into(),
                content: "Done task".into(),
                status: TodoStatus::Completed,
                created_at: String::new(),
                completed_at: None,
            }])
            .await;

        let result = evaluate_stop_hooks(
            "done",
            Some("stop"),
            Some(&store),
            &[],
            None,
            None,
            false,
            false,
            RecoveryState::default(),
        )
        .await;
        assert!(!result.should_continue);
    }

    #[tokio::test]
    async fn continue_on_output_truncation() {
        let result = evaluate_stop_hooks(
            "partial output...",
            Some("length"),
            None,
            &[],
            None,
            None,
            false,
            false,
            RecoveryState::default(),
        )
        .await;
        assert!(result.should_continue);
        assert_eq!(result.reason, "output_truncated");
        assert!(result.continuation_message.unwrap().contains("cut short"));
    }

    #[tokio::test]
    async fn skip_truncation_when_recovery_exhausted() {
        let recovery = RecoveryState {
            max_output_recovery_exhausted: true,
        };
        let result = evaluate_stop_hooks(
            "partial output...",
            Some("length"),
            None,
            &[],
            None,
            None,
            false,
            false,
            recovery,
        )
        .await;
        assert!(
            !result.should_continue,
            "should stop when recovery exhausted"
        );
    }

    #[tokio::test]
    async fn continue_on_queued_slash_commands() {
        let commands = vec!["/compact".to_string(), "/help".to_string()];
        let result = evaluate_stop_hooks(
            "done",
            Some("stop"),
            None,
            &commands,
            None,
            None,
            false,
            false,
            RecoveryState::default(),
        )
        .await;
        assert!(result.should_continue);
        assert_eq!(result.reason, "queued_commands");
        let msg = result.continuation_message.unwrap();
        assert!(msg.contains("2 pending"));
        assert!(msg.contains("/compact"));
    }
}
