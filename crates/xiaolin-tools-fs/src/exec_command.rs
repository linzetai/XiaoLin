//! PTY interactive terminal: `exec_command` starts a persistent session,
//! `write_stdin` sends input to it and polls output.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use xiaolin_core::tool::{Tool, ToolGroup, ToolKind, ToolParameterSchema, ToolResult};

use crate::filesystem::ensure_within_workspace;
use crate::shell_path_validation::has_traversal;
use crate::shell_security::{SecurityVerdict, ShellSecurityChecker};

pub use self::pty_session::PtySessionManager;

/// Reject commands that fail shell injection / substitution checks.
fn reject_unsafe_command(cmd: &str) -> Option<String> {
    match ShellSecurityChecker::check(cmd) {
        SecurityVerdict::Safe => None,
        SecurityVerdict::Blocked { pattern, reason }
        | SecurityVerdict::NeedsConfirmation { pattern, reason } => Some(format!(
            "Command rejected by shell security check ({pattern}): {reason}"
        )),
    }
}

fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| {
        if cfg!(target_os = "windows") {
            "powershell.exe".to_string()
        } else {
            "/bin/bash".to_string()
        }
    })
}

/// Starts a command in a PTY session. Returns initial output and a `session_id`
/// for subsequent interaction via `write_stdin`.
pub struct ExecCommandTool {
    session_manager: Arc<PtySessionManager>,
}

impl ExecCommandTool {
    pub fn new(session_manager: Arc<PtySessionManager>) -> Self {
        Self { session_manager }
    }
}

#[async_trait]
impl Tool for ExecCommandTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }

    fn supports_parallel(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "exec_command"
    }

    fn description(&self) -> &str {
        "[DEPRECATED: use terminal_open + terminal_input instead] \
         Start a command in an interactive PTY session. Returns the initial output and a \
         session_id for follow-up interaction via write_stdin. Use for REPLs, interactive \
         processes, or long-running commands where you need to send additional input."
    }

    fn group(&self) -> ToolGroup {
        ToolGroup::System
    }

    fn is_deferred(&self) -> bool {
        true
    }

    fn search_hint(&self) -> &str {
        "pty terminal interactive repl session exec"
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "cmd".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The command to execute in the PTY."
            }),
        );
        props.insert(
            "workdir".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Working directory for the command. Defaults to project root."
            }),
        );
        props.insert(
            "shell".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["bash", "sh", "zsh"],
                "description": "Shell to use. Defaults to $SHELL (or /bin/bash if unset)."
            }),
        );
        props.insert(
            "yield_time_ms".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Time in ms to wait for output after starting the command. Default: 2000."
            }),
        );
        props.insert(
            "max_output_chars".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Maximum characters of output to return. Default: 16000."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["cmd".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("Invalid JSON: {e}")),
        };

        let cmd = match args.get("cmd").and_then(|v| v.as_str()) {
            Some(c) => c.to_string(),
            None => return ToolResult::err("Missing required parameter: cmd"),
        };

        if let Some(reason) = reject_unsafe_command(&cmd) {
            return ToolResult::err(reason);
        }

        let workdir = match args.get("workdir").and_then(|v| v.as_str()) {
            Some(dir) => {
                if has_traversal(dir) {
                    return ToolResult::err(
                        "Invalid workdir: path contains directory traversal (..)",
                    );
                }
                match ensure_within_workspace(Path::new(dir), true) {
                    Ok(p) => Some(p.to_string_lossy().into_owned()),
                    Err(e) => return ToolResult::err(format!("Invalid workdir: {e}")),
                }
            }
            None => None,
        };
        let shell = args
            .get("shell")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(default_shell);
        let yield_time_ms = args
            .get("yield_time_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(2000);
        let max_output_chars = args
            .get("max_output_chars")
            .and_then(|v| v.as_u64())
            .unwrap_or(16000) as usize;

        match self
            .session_manager
            .create_session(&cmd, workdir.as_deref(), &shell)
            .await
        {
            Ok(session_id) => {
                tokio::time::sleep(tokio::time::Duration::from_millis(yield_time_ms)).await;
                let output = self
                    .session_manager
                    .read_output(&session_id, max_output_chars)
                    .await;
                ToolResult::ok(format!(
                    "session_id: {session_id}\n---\n{}",
                    output.unwrap_or_default()
                ))
            }
            Err(e) => ToolResult::err(format!("Failed to start PTY session: {e}")),
        }
    }
}

/// Sends input to an existing PTY session and returns new output.
pub struct WriteStdinTool {
    session_manager: Arc<PtySessionManager>,
}

impl WriteStdinTool {
    pub fn new(session_manager: Arc<PtySessionManager>) -> Self {
        Self { session_manager }
    }
}

#[async_trait]
impl Tool for WriteStdinTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }

    fn name(&self) -> &str {
        "write_stdin"
    }

    fn description(&self) -> &str {
        "[DEPRECATED: use terminal_input instead] \
         Write text to an existing PTY session's stdin and return new output. \
         Use to interact with REPLs, respond to prompts, or send commands to \
         a running interactive process started with exec_command."
    }

    fn group(&self) -> ToolGroup {
        ToolGroup::System
    }

    fn is_deferred(&self) -> bool {
        true
    }

    fn search_hint(&self) -> &str {
        "pty stdin input session interactive write"
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "session_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The session_id returned by exec_command."
            }),
        );
        props.insert(
            "input".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Text to write to the session's stdin. Include newline (\\n) to submit."
            }),
        );
        props.insert(
            "yield_time_ms".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Time in ms to wait for output after writing. Default: 1000."
            }),
        );
        props.insert(
            "max_output_chars".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Maximum characters of output to return. Default: 16000."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["session_id".to_string(), "input".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("Invalid JSON: {e}")),
        };

        let session_id = match args.get("session_id").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => return ToolResult::err("Missing required parameter: session_id"),
        };

        let input = match args.get("input").and_then(|v| v.as_str()) {
            Some(i) => i.to_string(),
            None => return ToolResult::err("Missing required parameter: input"),
        };

        let yield_time_ms = args
            .get("yield_time_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(1000);
        let max_output_chars = args
            .get("max_output_chars")
            .and_then(|v| v.as_u64())
            .unwrap_or(16000) as usize;

        if let Err(e) = self.session_manager.write_input(&session_id, &input).await {
            return ToolResult::err(format!("Failed to write to session: {e}"));
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(yield_time_ms)).await;

        match self
            .session_manager
            .read_output(&session_id, max_output_chars)
            .await
        {
            Some(output) => ToolResult::ok(output),
            None => ToolResult::err(format!("Session '{session_id}' not found or closed")),
        }
    }
}

/// Manages the lifecycle of PTY sessions with automatic timeout cleanup.
pub mod pty_session {
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::{Duration, Instant};

    use tokio::sync::Mutex;

    pub struct PtySession {
        child: std::process::Child,
        output_buffer: Vec<u8>,
        stderr_buf: Arc<StdMutex<Vec<u8>>>,
        stderr_thread: Option<std::thread::JoinHandle<()>>,
        #[allow(dead_code)]
        created_at: Instant,
        last_activity: Instant,
    }

    impl Drop for PtySession {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            if let Some(handle) = self.stderr_thread.take() {
                let _ = handle.join();
            }
        }
    }

    pub struct PtySessionManager {
        sessions: Mutex<HashMap<String, PtySession>>,
        timeout: Duration,
    }

    /// Max concurrent exec_command PTY sessions before rejecting new ones.
    const MAX_PTY_SESSIONS: usize = 50;
    const CLEANUP_INTERVAL: Duration = Duration::from_secs(60);

    impl PtySessionManager {
        pub fn new(timeout: Duration) -> Self {
            Self {
                sessions: Mutex::new(HashMap::new()),
                timeout,
            }
        }

        pub fn with_default_timeout() -> Self {
            Self::new(Duration::from_secs(300))
        }

        pub async fn create_session(
            &self,
            cmd: &str,
            workdir: Option<&str>,
            shell: &str,
        ) -> Result<String, String> {
            {
                let mut sessions = self.sessions.lock().await;
                if sessions.len() >= MAX_PTY_SESSIONS {
                    let before = sessions.len();
                    let expired: Vec<String> = sessions
                        .iter()
                        .filter(|(_, s)| s.last_activity.elapsed() >= self.timeout)
                        .map(|(id, _)| id.clone())
                        .collect();
                    for id in expired {
                        sessions.remove(&id);
                    }
                    if before != sessions.len() {
                        tracing::warn!(
                            max = MAX_PTY_SESSIONS,
                            expired_removed = before - sessions.len(),
                            "exec_command evicted expired PTY sessions before create"
                        );
                    }
                    if sessions.len() >= MAX_PTY_SESSIONS {
                        tracing::warn!(
                            max = MAX_PTY_SESSIONS,
                            "exec_command PTY session table at capacity"
                        );
                        return Err(format!(
                            "Maximum number of PTY sessions ({MAX_PTY_SESSIONS}) reached. \
                             Close an existing session before starting a new one."
                        ));
                    }
                }
            }

            let session_id = format!("pty_{}", uuid::Uuid::new_v4().to_string().split('-').next().unwrap_or("0"));

            let mut command = Command::new(shell);
            command.arg("-c").arg(cmd);
            command.stdin(Stdio::piped());
            command.stdout(Stdio::piped());
            command.stderr(Stdio::piped());

            if let Some(dir) = workdir {
                command.current_dir(dir);
            }

            let mut child = command
                .spawn()
                .map_err(|e| format!("Failed to spawn process: {e}"))?;

            let stderr_buf = Arc::new(StdMutex::new(Vec::<u8>::new()));
            let stderr_thread = if let Some(mut stderr) = child.stderr.take() {
                let buf = Arc::clone(&stderr_buf);
                Some(std::thread::spawn(move || {
                    let mut tmp = [0u8; 4096];
                    loop {
                        match stderr.read(&mut tmp) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                if let Ok(mut b) = buf.lock() {
                                    const MAX_STDERR: usize = 256 * 1024;
                                    if b.len() + n > MAX_STDERR {
                                        let drain = (b.len() + n).saturating_sub(MAX_STDERR);
                                        b.drain(..drain);
                                    }
                                    b.extend_from_slice(&tmp[..n]);
                                }
                            }
                        }
                    }
                }))
            } else {
                None
            };

            let session = PtySession {
                child,
                output_buffer: Vec::new(),
                stderr_buf,
                stderr_thread,
                created_at: Instant::now(),
                last_activity: Instant::now(),
            };

            let mut sessions = self.sessions.lock().await;
            sessions.insert(session_id.clone(), session);

            Ok(session_id)
        }

        pub async fn write_input(&self, session_id: &str, input: &str) -> Result<(), String> {
            let mut sessions = self.sessions.lock().await;
            let session = sessions
                .get_mut(session_id)
                .ok_or_else(|| format!("Session '{session_id}' not found"))?;

            session.last_activity = Instant::now();

            if let Some(stdin) = session.child.stdin.as_mut() {
                stdin
                    .write_all(input.as_bytes())
                    .map_err(|e| format!("Write failed: {e}"))?;
                stdin.flush().map_err(|e| format!("Flush failed: {e}"))?;
                Ok(())
            } else {
                Err("Session stdin not available".to_string())
            }
        }

        pub async fn read_output(&self, session_id: &str, max_chars: usize) -> Option<String> {
            let mut sessions = self.sessions.lock().await;
            let session = sessions.get_mut(session_id)?;

            session.last_activity = Instant::now();

            let mut buf = vec![0u8; max_chars.min(65536)];
            if let Some(stdout) = session.child.stdout.as_mut() {
                let n = stdout.read(&mut buf).unwrap_or(0);
                session.output_buffer.extend_from_slice(&buf[..n]);
            }

            if let Ok(mut stderr) = session.stderr_buf.lock() {
                if !stderr.is_empty() {
                    session.output_buffer.extend_from_slice(&stderr);
                    stderr.clear();
                }
            }

            let output = String::from_utf8_lossy(&session.output_buffer).to_string();
            session.output_buffer.clear();

            let truncated = if output.chars().count() > max_chars {
                output.chars().take(max_chars).collect::<String>()
            } else {
                output
            };

            Some(truncated)
        }

        pub async fn close_session(&self, session_id: &str) -> bool {
            let mut sessions = self.sessions.lock().await;
            sessions.remove(session_id).is_some()
        }

        pub async fn cleanup_expired(&self) -> usize {
            let mut sessions = self.sessions.lock().await;
            let before = sessions.len();
            let expired: Vec<String> = sessions
                .iter()
                .filter(|(_, s)| s.last_activity.elapsed() >= self.timeout)
                .map(|(id, _)| id.clone())
                .collect();

            for id in &expired {
                if sessions.remove(id).is_some() {
                    tracing::info!(
                        session_id = %id,
                        "exec_command session cleaned up (expired)"
                    );
                }
            }

            before - sessions.len()
        }

        pub fn start_cleanup_task(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
            let mgr = Arc::clone(self);
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(CLEANUP_INTERVAL);
                loop {
                    interval.tick().await;
                    let cleaned = mgr.cleanup_expired().await;
                    if cleaned > 0 {
                        tracing::info!(
                            count = cleaned,
                            "legacy exec_command PTY sessions cleaned up"
                        );
                    }
                }
            })
        }

        pub async fn session_count(&self) -> usize {
            self.sessions.lock().await.len()
        }
    }
}
