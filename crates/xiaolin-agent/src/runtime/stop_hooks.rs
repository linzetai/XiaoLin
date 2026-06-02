//! Stop hook evaluation: when the LLM finishes (no tool_calls), decide
//! whether the agent should continue for another iteration or truly stop.
//!
//! Each hook is a pure function that inspects the conversation state and
//! returns a [`StopHookResult`].  The first hook that says `should_continue`
//! wins — the agent injects its `continuation_message` and re-enters the loop.

use crate::builtin_tools::{TodoItem, TodoStatus, TodoStore};

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
pub(crate) async fn evaluate_stop_hooks(
    _last_assistant_text: &str,
    finish_reason: Option<&str>,
    todo_store: Option<&TodoStore>,
    queued_slash_commands: &[String],
) -> StopHookResult {
    // Hook 1: Incomplete todo items
    if let Some(store) = todo_store {
        if let Some(result) = check_incomplete_todos(store).await {
            return result;
        }
    }

    // Hook 2: Output truncation (finish_reason=length)
    if let Some(result) = check_truncation(finish_reason) {
        return result;
    }

    // Hook 3: Queued slash commands
    if let Some(result) = check_queued_commands(queued_slash_commands) {
        return result;
    }

    // Hook 4: Custom hook placeholder (extensible via config in the future)
    // Currently a no-op; the hook system can be extended to support
    // user-defined stop conditions.

    StopHookResult::stop()
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

    Some(StopHookResult::cont(
        "incomplete_todos",
        format!(
            "[STOP HOOK: incomplete todos] You still have {} unfinished task(s):\n{}{}\n\n\
             Please continue working on the remaining tasks before ending your turn.",
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
        let result = evaluate_stop_hooks("done", Some("stop"), None, &[]).await;
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

        let result = evaluate_stop_hooks("done", Some("stop"), Some(&store), &[]).await;
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

        let result = evaluate_stop_hooks("done", Some("stop"), Some(&store), &[]).await;
        assert!(!result.should_continue);
    }

    #[tokio::test]
    async fn continue_on_output_truncation() {
        let result = evaluate_stop_hooks("partial output...", Some("length"), None, &[]).await;
        assert!(result.should_continue);
        assert_eq!(result.reason, "output_truncated");
        assert!(result.continuation_message.unwrap().contains("cut short"));
    }

    #[tokio::test]
    async fn continue_on_queued_slash_commands() {
        let commands = vec!["/compact".to_string(), "/help".to_string()];
        let result = evaluate_stop_hooks("done", Some("stop"), None, &commands).await;
        assert!(result.should_continue);
        assert_eq!(result.reason, "queued_commands");
        let msg = result.continuation_message.unwrap();
        assert!(msg.contains("2 pending"));
        assert!(msg.contains("/compact"));
    }
}
