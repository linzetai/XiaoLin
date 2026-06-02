use std::path::PathBuf;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::hook_events::{HookEvent, HookResult};

/// Filter for matching hook events.
#[derive(Debug, Clone)]
pub enum HookEventFilter {
    /// Matches all events of a given type.
    EventType(&'static str),
    /// Matches pre/post tool events for a specific tool name.
    ToolName(String),
    /// Matches pre/post tool events matching a glob pattern (e.g. "file_*").
    ToolPattern(String),
    /// Matches all events.
    All,
}

impl HookEventFilter {
    pub fn matches(&self, event: &HookEvent) -> bool {
        match self {
            Self::All => true,
            Self::EventType(ty) => event.event_type() == *ty,
            Self::ToolName(name) => event.tool_name() == Some(name.as_str()),
            Self::ToolPattern(pattern) => {
                if let Some(tool) = event.tool_name() {
                    glob_match(pattern, tool)
                } else {
                    false
                }
            }
        }
    }
}

/// Handler types for hook execution.
pub enum HookHandler {
    /// Execute a shell command. The event is passed as JSON via stdin.
    Shell {
        command: String,
        working_dir: Option<PathBuf>,
    },
}

impl std::fmt::Debug for HookHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Shell { command, .. } => {
                f.debug_struct("Shell").field("command", command).finish()
            }
        }
    }
}

/// A registered hook with its filter, handler, and timeout.
pub struct RegisteredHook {
    pub filter: HookEventFilter,
    pub handler: HookHandler,
    pub timeout: Duration,
    pub blocking: bool,
}

/// Executor that manages and dispatches hook events.
pub struct HookExecutor {
    hooks: Vec<RegisteredHook>,
}

impl Default for HookExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl HookExecutor {
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    pub fn register(&mut self, hook: RegisteredHook) {
        self.hooks.push(hook);
    }

    pub fn hook_count(&self) -> usize {
        self.hooks.len()
    }

    /// Execute all hooks matching a pre_tool_use event.
    /// Returns results from blocking hooks only.
    pub async fn execute_pre_tool_hooks(
        &self,
        event: &HookEvent,
        abort: &CancellationToken,
    ) -> Vec<HookResult> {
        self.execute_matching(event, abort, true).await
    }

    /// Execute all hooks matching a post_tool_use event.
    pub async fn execute_post_tool_hooks(
        &self,
        event: &HookEvent,
        abort: &CancellationToken,
    ) -> Vec<HookResult> {
        self.execute_matching(event, abort, false).await
    }

    /// Execute all hooks matching a stop event.
    pub async fn execute_stop_hooks(
        &self,
        event: &HookEvent,
        abort: &CancellationToken,
    ) -> Vec<HookResult> {
        self.execute_matching(event, abort, false).await
    }

    async fn execute_matching(
        &self,
        event: &HookEvent,
        abort: &CancellationToken,
        blocking_only: bool,
    ) -> Vec<HookResult> {
        let mut results = Vec::new();

        for hook in &self.hooks {
            if !hook.filter.matches(event) {
                continue;
            }
            if blocking_only && !hook.blocking {
                continue;
            }

            let result = self.execute_single(hook, event, abort).await;
            results.push(result);

            if abort.is_cancelled() {
                break;
            }
        }

        results
    }

    async fn execute_single(
        &self,
        hook: &RegisteredHook,
        event: &HookEvent,
        abort: &CancellationToken,
    ) -> HookResult {
        let timeout = hook.timeout;

        let result = tokio::select! {
            r = self.run_handler(&hook.handler, event.clone()) => r,
            _ = tokio::time::sleep(timeout) => {
                HookResult::allow()
            }
            _ = abort.cancelled() => {
                HookResult::allow()
            }
        };

        result
    }

    async fn run_handler(&self, handler: &HookHandler, event: HookEvent) -> HookResult {
        let HookHandler::Shell {
            command,
            working_dir,
        } = handler;
        run_shell_hook(command, working_dir.as_deref(), &event).await
    }
}

async fn run_shell_hook(
    command: &str,
    working_dir: Option<&std::path::Path>,
    event: &HookEvent,
) -> HookResult {
    let event_json = match serde_json::to_string(event) {
        Ok(j) => j,
        Err(_) => return HookResult::allow(),
    };

    let mut cmd = tokio::process::Command::new("sh");
    cmd.args(["-c", command]);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.env("HOOK_EVENT", &event_json);

    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return HookResult::allow(),
    };

    let output = match child.wait_with_output().await {
        Ok(o) => o,
        Err(_) => return HookResult::allow(),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return HookResult::block(format!("hook command failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        return HookResult::allow();
    }

    serde_json::from_str(stdout.trim()).unwrap_or_else(|_| HookResult::allow())
}

/// Simple glob matching supporting `*` and `?`.
pub(crate) fn glob_match(pattern: &str, text: &str) -> bool {
    let mut pat_chars = pattern.chars().peekable();
    let mut text_chars = text.chars().peekable();

    while pat_chars.peek().is_some() || text_chars.peek().is_some() {
        match pat_chars.peek() {
            Some('*') => {
                pat_chars.next();
                if pat_chars.peek().is_none() {
                    return true;
                }
                while text_chars.peek().is_some() {
                    let remaining_pat: String = pat_chars.clone().collect();
                    let remaining_text: String = text_chars.clone().collect();
                    if glob_match(&remaining_pat, &remaining_text) {
                        return true;
                    }
                    text_chars.next();
                }
                return false;
            }
            Some('?') => {
                pat_chars.next();
                if text_chars.next().is_none() {
                    return false;
                }
            }
            Some(pc) => {
                let pc = *pc;
                pat_chars.next();
                match text_chars.next() {
                    Some(tc) if tc == pc => {}
                    _ => return false,
                }
            }
            None => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pre_tool_event(tool: &str) -> HookEvent {
        HookEvent::PreToolUse {
            tool_name: tool.into(),
            tool_use_id: "call_1".into(),
            input: serde_json::json!({}),
        }
    }

    fn make_post_tool_event(tool: &str) -> HookEvent {
        HookEvent::PostToolUse {
            tool_name: tool.into(),
            tool_use_id: "call_1".into(),
            input: serde_json::json!({}),
            output: serde_json::json!("done"),
            duration: Duration::from_millis(100),
        }
    }

    fn make_stop_event() -> HookEvent {
        HookEvent::Stop {
            messages: vec![],
            assistant_messages: vec![],
        }
    }

    fn shell_hook(command: &str) -> HookHandler {
        HookHandler::Shell {
            command: command.into(),
            working_dir: None,
        }
    }

    fn blocking_hook(filter: HookEventFilter, command: &str) -> RegisteredHook {
        RegisteredHook {
            filter,
            handler: shell_hook(command),
            timeout: Duration::from_secs(10),
            blocking: true,
        }
    }

    fn non_blocking_hook(filter: HookEventFilter) -> RegisteredHook {
        RegisteredHook {
            filter,
            handler: shell_hook("true"),
            timeout: Duration::from_secs(10),
            blocking: false,
        }
    }

    #[tokio::test]
    async fn execute_pre_tool_hooks_calls_matching() {
        let mut executor = HookExecutor::new();
        executor.register(blocking_hook(
            HookEventFilter::ToolName("shell_exec".into()),
            "exit 1",
        ));
        executor.register(blocking_hook(
            HookEventFilter::ToolName("read_file".into()),
            "true",
        ));

        let event = make_pre_tool_event("shell_exec");
        let token = CancellationToken::new();
        let results = executor.execute_pre_tool_hooks(&event, &token).await;

        assert_eq!(results.len(), 1);
        assert!(results[0].is_blocked());
    }

    #[tokio::test]
    async fn execute_pre_tool_hooks_skips_non_blocking() {
        let mut executor = HookExecutor::new();
        executor.register(non_blocking_hook(HookEventFilter::All));

        let event = make_pre_tool_event("any_tool");
        let token = CancellationToken::new();
        let results = executor.execute_pre_tool_hooks(&event, &token).await;

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn execute_post_tool_hooks_includes_non_blocking() {
        let mut executor = HookExecutor::new();
        executor.register(non_blocking_hook(HookEventFilter::All));
        executor.register(blocking_hook(HookEventFilter::All, "true"));

        let event = make_post_tool_event("test_tool");
        let token = CancellationToken::new();
        let results = executor.execute_post_tool_hooks(&event, &token).await;

        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn execute_stop_hooks_matches_event_type() {
        let mut executor = HookExecutor::new();
        executor.register(blocking_hook(
            HookEventFilter::EventType("stop"),
            r#"echo '{"prevent_continuation":true}'"#,
        ));
        executor.register(blocking_hook(
            HookEventFilter::EventType("pre_tool_use"),
            "exit 1",
        ));

        let event = make_stop_event();
        let token = CancellationToken::new();
        let results = executor.execute_stop_hooks(&event, &token).await;

        assert_eq!(results.len(), 1);
        assert!(results[0].should_stop());
    }

    #[tokio::test]
    async fn timeout_returns_allow() {
        let mut executor = HookExecutor::new();
        executor.register(RegisteredHook {
            filter: HookEventFilter::All,
            handler: shell_hook("sleep 5"),
            timeout: Duration::from_millis(50),
            blocking: true,
        });

        let event = make_pre_tool_event("test");
        let token = CancellationToken::new();
        let results = executor.execute_pre_tool_hooks(&event, &token).await;

        assert_eq!(results.len(), 1);
        assert!(!results[0].is_blocked());
    }

    #[tokio::test]
    async fn abort_cancels_remaining_hooks() {
        let token = CancellationToken::new();
        token.cancel();

        let mut executor = HookExecutor::new();
        executor.register(blocking_hook(HookEventFilter::All, "true"));
        executor.register(blocking_hook(HookEventFilter::All, "exit 1"));

        let event = make_pre_tool_event("test");
        let results = executor.execute_pre_tool_hooks(&event, &token).await;

        assert_eq!(results.len(), 1);
        assert!(!results[0].is_blocked());
    }

    #[tokio::test]
    async fn shell_hook_success_returns_allow() {
        let result = run_shell_hook("echo '{}'", None, &make_pre_tool_event("test")).await;
        assert!(!result.is_blocked());
    }

    #[tokio::test]
    async fn shell_hook_failure_blocks() {
        let result = run_shell_hook("exit 1", None, &make_pre_tool_event("test")).await;
        assert!(result.is_blocked());
    }

    #[test]
    fn glob_match_patterns() {
        assert!(glob_match("file_*", "file_read"));
        assert!(glob_match("file_*", "file_write"));
        assert!(!glob_match("file_*", "shell_exec"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("shell_?", "shell_x"));
        assert!(!glob_match("shell_?", "shell_xy"));
        assert!(glob_match("read_*_file", "read_large_file"));
    }

    #[test]
    fn event_filter_tool_pattern() {
        let filter = HookEventFilter::ToolPattern("file_*".into());
        let event = make_pre_tool_event("file_read");
        assert!(filter.matches(&event));

        let event = make_pre_tool_event("shell_exec");
        assert!(!filter.matches(&event));
    }

}
