use std::path::Path;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, BufReader};
use xiaolin_core::tool_runtime::{
    Approvable, ExecApprovalRequirement, SandboxAttempt, SandboxBackend, SandboxPreference,
    Sandboxable, ToolExecContext, ToolProgressEvent, ToolRuntime, ToolRuntimeError,
};
use xiaolin_protocol::approval::PendingAction;
use xiaolin_sandbox::SandboxManager;
use xiaolin_security::dangerous_ops::{self, CheckResult};

use xiaolin_tools_fs::shell::validate_readonly_command;

/// Unified shell execution runtime.
///
/// Replaces both `ShellTool` and `SandboxedShellTool` by combining:
/// - `dangerous_ops` check → determines if approval is needed / forbidden
/// - ExecPolicy integration (via orchestrator Phase 2)
/// - Sandbox preference (Auto with escalation)
pub struct ShellRuntime;

impl Approvable for ShellRuntime {
    fn approval_keys(&self, args: &serde_json::Value) -> Vec<String> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let cwd = args
            .get("working_dir")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("cwd").and_then(|v| v.as_str()))
            .unwrap_or(".");
        vec![format!("shell:{}:{}", cwd, command)]
    }

    fn exec_requirement(
        &self,
        args: &serde_json::Value,
        _cwd: &Path,
    ) -> ExecApprovalRequirement {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        match dangerous_ops::check_dangerous_command(command) {
            Ok(()) => ExecApprovalRequirement::NeedsApproval {
                reason: "shell command execution".into(),
            },
            Err(CheckResult::Denied(msg)) => ExecApprovalRequirement::Forbidden { reason: msg },
            Err(CheckResult::NeedsConfirmation(msg)) => {
                ExecApprovalRequirement::NeedsApproval { reason: msg }
            }
        }
    }

    fn to_pending_action(&self, args: &serde_json::Value, cwd: &Path) -> PendingAction {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        PendingAction::ShellCommand {
            command,
            cwd: cwd.display().to_string(),
        }
    }
}

impl Sandboxable for ShellRuntime {
    fn sandbox_preference(&self) -> SandboxPreference {
        SandboxPreference::Auto
    }

    fn escalate_on_sandbox_failure(&self) -> bool {
        true
    }

    fn bypass_approval_on_escalation(&self) -> bool {
        true
    }
}

#[async_trait]
impl ToolRuntime for ShellRuntime {
    async fn run(
        &self,
        args: &serde_json::Value,
        sandbox: &SandboxAttempt,
        ctx: &ToolExecContext,
    ) -> Result<String, ToolRuntimeError> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolRuntimeError::Internal {
                message: "missing 'command' argument".into(),
            })?;

        let mut timeout_ms = args
            .get("timeout_ms")
            .or_else(|| args.get("timeout"))
            .and_then(|v| v.as_u64())
            .unwrap_or(120_000);

        // Cap timeout for dev server commands that are likely to run indefinitely
        if is_dev_server_command(command) && timeout_ms > 30_000 {
            tracing::info!(command = %command, "detected dev server command, capping timeout to 30s");
            timeout_ms = 30_000;
        }

        let cwd = args
            .get("working_dir")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("cwd").and_then(|v| v.as_str()))
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| ctx.cwd.clone());

        let is_readonly = validate_readonly_command(command).is_ok();

        let effective_sandbox = if is_readonly {
            SandboxBackend::None
        } else {
            sandbox.sandbox_type
        };

        let mut cmd = match effective_sandbox {
            SandboxBackend::None => {
                build_plain_command(command, &cwd)
            }
            _ => {
                let sandbox_type = map_backend_to_sandbox_type(effective_sandbox);
                let mgr = SandboxManager::with_type(sandbox_type);
                if mgr.is_available() {
                    let fs_policy = build_fs_policy(&cwd);
                    let net_policy = xiaolin_security::NetworkSandboxPolicy::Enabled;
                    let shell = preferred_shell();
                    let sandboxed = mgr.transform(command, shell, &fs_policy, net_policy, &cwd);
                    tracing::debug!(
                        sandbox = %effective_sandbox,
                        command = %command,
                        "executing shell command in sandbox"
                    );
                    sandboxed.into_tokio_command()
                } else {
                    tracing::warn!(
                        sandbox = %effective_sandbox,
                        "sandbox requested but not available, falling back to plain execution"
                    );
                    build_plain_command(command, &cwd)
                }
            }
        };

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let start = std::time::Instant::now();

        let mut child = cmd.spawn().map_err(|e| ToolRuntimeError::Internal {
            message: format!("failed to spawn shell: {e}"),
        })?;

        let (stdout_str, stderr_str, status) = if let Some(ref progress_tx) = ctx.progress_tx {
            // Streaming mode: read stdout/stderr line by line and emit progress
            let stdout_pipe = child.stdout.take();
            let stderr_pipe = child.stderr.take();

            let stdout_task = tokio::spawn({
                let tx = progress_tx.clone();
                async move {
                    let mut lines = Vec::new();
                    if let Some(pipe) = stdout_pipe {
                        let mut reader = BufReader::new(pipe).lines();
                        while let Ok(Some(line)) = reader.next_line().await {
                            let _ = tx.send(ToolProgressEvent {
                                message: String::new(),
                                partial_output: Some(format!("{line}\n")),
                                progress: None,
                            }).await;
                            lines.push(line);
                        }
                    }
                    lines
                }
            });

            let stderr_task = tokio::spawn({
                let tx = progress_tx.clone();
                async move {
                    let mut lines = Vec::new();
                    if let Some(pipe) = stderr_pipe {
                        let mut reader = BufReader::new(pipe).lines();
                        while let Ok(Some(line)) = reader.next_line().await {
                            let _ = tx.send(ToolProgressEvent {
                                message: String::new(),
                                partial_output: Some(format!("{line}\n")),
                                progress: None,
                            }).await;
                            lines.push(line);
                        }
                    }
                    lines
                }
            });

            let exit_status = match tokio::time::timeout(
                std::time::Duration::from_millis(timeout_ms),
                child.wait(),
            )
            .await
            {
                Ok(Ok(status)) => status,
                Ok(Err(e)) => {
                    return Err(ToolRuntimeError::Internal {
                        message: format!("process error: {e}"),
                    });
                }
                Err(_elapsed) => {
                    // Timeout: kill the child process and its process group
                    let _ = child.kill().await;
                    stdout_task.abort();
                    stderr_task.abort();
                    return Err(ToolRuntimeError::Timeout { elapsed_ms: timeout_ms });
                }
            };

            let stdout_lines = stdout_task.await.unwrap_or_default();
            let stderr_lines = stderr_task.await.unwrap_or_default();

            (stdout_lines.join("\n"), stderr_lines.join("\n"), exit_status)
        } else {
            // Batch mode: capture PID for kill-on-timeout, then wait_with_output
            let child_id = child.id();
            match tokio::time::timeout(
                std::time::Duration::from_millis(timeout_ms),
                child.wait_with_output(),
            )
            .await
            {
                Ok(Ok(output)) => (
                    String::from_utf8_lossy(&output.stdout).into_owned(),
                    String::from_utf8_lossy(&output.stderr).into_owned(),
                    output.status,
                ),
                Ok(Err(e)) => {
                    return Err(ToolRuntimeError::Internal {
                        message: format!("process error: {e}"),
                    });
                }
                Err(_elapsed) => {
                    // Kill via PID since child was moved into wait_with_output
                    if let Some(pid) = child_id {
                        #[cfg(unix)]
                        {
                            let _ = std::process::Command::new("kill")
                                .args(["-9", &format!("-{pid}")])
                                .output();
                        }
                    }
                    return Err(ToolRuntimeError::Timeout { elapsed_ms: timeout_ms });
                }
            }
        };

        let duration_ms = start.elapsed().as_millis();

        #[cfg(unix)]
        let signal = std::os::unix::process::ExitStatusExt::signal(&status);
        #[cfg(not(unix))]
        let signal: Option<i32> = None;

        if let Some(sig) = signal {
            let sandbox_active = effective_sandbox != SandboxBackend::None;
            let is_sandbox_signal = matches!(sig, 6 | 9 | 31);
            if sandbox_active && is_sandbox_signal && stdout_str.is_empty() {
                return Err(ToolRuntimeError::SandboxDenied {
                    reason: format!(
                        "process killed by signal {sig} (sandbox policy violation suspected)"
                    ),
                });
            }
        }

        let exit_code_str = match (status.code(), signal) {
            (Some(code), _) => format!("{code}"),
            (None, Some(sig)) => format!("SIGNAL({sig})"),
            (None, None) => "-1".to_string(),
        };

        let cwd_display = cwd.display();
        let mut result = format!(
            "exit_code={exit_code_str}\nduration_ms={duration_ms}\ncwd={cwd_display}\n---\n"
        );

        if stderr_str.is_empty() {
            result.push_str(&stdout_str);
        } else {
            result.push_str("stdout:\n");
            result.push_str(&stdout_str);
            result.push_str("\nstderr:\n");
            result.push_str(&stderr_str);
        }

        Ok(result)
    }

    fn name(&self) -> &str {
        "shell_exec"
    }
}

fn build_plain_command(command: &str, cwd: &Path) -> tokio::process::Command {
    let shell = preferred_shell();
    let flag = if cfg!(windows) { "/C" } else { "-c" };
    let mut cmd = tokio::process::Command::new(shell);
    cmd.arg(flag).arg(command).current_dir(cwd);
    cmd
}

fn preferred_shell() -> &'static str {
    use std::sync::OnceLock;
    static SHELL: OnceLock<String> = OnceLock::new();
    SHELL.get_or_init(|| {
        if cfg!(windows) {
            "cmd".to_string()
        } else {
            std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string())
        }
    })
}

/// Detect commands that are likely dev servers (long-running by nature).
fn is_dev_server_command(command: &str) -> bool {
    let lower = command.to_lowercase();
    let patterns = [
        "vite", "next dev", "next start", "webpack serve", "webpack-dev-server",
        "ng serve", "npm start", "npm run dev", "npm run serve",
        "yarn dev", "yarn start", "pnpm dev", "pnpm start",
        "flask run", "uvicorn", "gunicorn", "python -m http.server",
        "live-server", "http-server", "nodemon",
    ];
    // Only match if the command appears to run a server synchronously (no & or nohup)
    let has_background = command.trim().ends_with('&')
        || lower.contains("nohup")
        || lower.contains("> /dev/null");
    if has_background {
        return false;
    }
    patterns.iter().any(|p| lower.contains(p))
}

fn map_backend_to_sandbox_type(backend: SandboxBackend) -> xiaolin_sandbox::SandboxType {
    match backend {
        SandboxBackend::Landlock => xiaolin_sandbox::SandboxType::Landlock,
        SandboxBackend::ExternalBinary => xiaolin_sandbox::SandboxType::ExternalBinary,
        SandboxBackend::Seatbelt => xiaolin_sandbox::SandboxType::Seatbelt,
        SandboxBackend::RestrictedToken => xiaolin_sandbox::SandboxType::RestrictedToken,
        SandboxBackend::None => xiaolin_sandbox::SandboxType::Noop,
    }
}

fn build_fs_policy(cwd: &Path) -> xiaolin_security::FileSystemSandboxPolicy {
    use std::convert::TryFrom;
    use xiaolin_security::{
        FileSystemAccessMode, FileSystemSandboxEntry, FileSystemSandboxKind,
        FileSystemSandboxPolicy, FileSystemPath, FileSystemSpecialPath,
    };

    let abs_cwd = std::fs::canonicalize(cwd)
        .unwrap_or_else(|_| cwd.to_path_buf());
    let temp_dir = std::env::temp_dir();

    let mut entries = vec![
        FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::Root,
            },
            access: FileSystemAccessMode::Read,
        },
    ];

    if let Ok(p) = xiaolin_core::path::AbsolutePathBuf::try_from(abs_cwd) {
        entries.push(FileSystemSandboxEntry {
            path: FileSystemPath::Path { path: p },
            access: FileSystemAccessMode::Write,
        });
    }
    if let Ok(p) = xiaolin_core::path::AbsolutePathBuf::try_from(temp_dir) {
        entries.push(FileSystemSandboxEntry {
            path: FileSystemPath::Path { path: p },
            access: FileSystemAccessMode::Write,
        });
    }

    FileSystemSandboxPolicy {
        kind: FileSystemSandboxKind::Restricted,
        glob_scan_max_depth: None,
        entries,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_command_needs_approval() {
        let rt = ShellRuntime;
        let args = serde_json::json!({"command": "ls -la"});
        let req = rt.exec_requirement(&args, Path::new("/tmp"));
        assert!(matches!(req, ExecApprovalRequirement::NeedsApproval { .. }));
    }

    #[test]
    fn approval_keys_include_command_and_cwd() {
        let rt = ShellRuntime;
        let args = serde_json::json!({"command": "echo hi", "cwd": "/home"});
        let keys = rt.approval_keys(&args);
        assert_eq!(keys, vec!["shell:/home:echo hi"]);
    }

    #[test]
    fn different_cwd_different_keys() {
        let rt = ShellRuntime;
        let args1 = serde_json::json!({"command": "ls", "cwd": "/a"});
        let args2 = serde_json::json!({"command": "ls", "cwd": "/b"});
        assert_ne!(rt.approval_keys(&args1), rt.approval_keys(&args2));
    }

    #[test]
    fn sandbox_preference_is_auto() {
        let rt = ShellRuntime;
        assert_eq!(rt.sandbox_preference(), SandboxPreference::Auto);
        assert!(rt.escalate_on_sandbox_failure());
    }

    #[tokio::test]
    async fn run_simple_echo() {
        let rt = ShellRuntime;
        let args = serde_json::json!({"command": "echo hello"});
        let sandbox = SandboxAttempt {
            sandbox_type: SandboxBackend::None,
            cwd: std::path::PathBuf::from("/tmp"),
        };
        let ctx = ToolExecContext {
            turn_id: xiaolin_protocol::TurnId::new("t1"),
            session_id: xiaolin_protocol::SessionId::new("s1"),
            call_id: "c1".into(),
            cwd: std::path::PathBuf::from("/tmp"),
            progress_tx: None,
        };
        let result = rt.run(&args, &sandbox, &ctx).await.unwrap();
        assert!(result.contains("hello"));
        assert!(result.contains("exit_code=0"));
        assert!(result.contains("duration_ms="));
        assert!(result.contains("cwd=/tmp"));
    }

    #[tokio::test]
    async fn run_readonly_bypasses_sandbox() {
        let rt = ShellRuntime;
        let args = serde_json::json!({"command": "echo bypass_test"});
        // Even with Seatbelt requested, readonly commands should bypass it
        let sandbox = SandboxAttempt {
            sandbox_type: SandboxBackend::Seatbelt,
            cwd: std::path::PathBuf::from("/tmp"),
        };
        let ctx = ToolExecContext {
            turn_id: xiaolin_protocol::TurnId::new("t1"),
            session_id: xiaolin_protocol::SessionId::new("s1"),
            call_id: "c1".into(),
            cwd: std::path::PathBuf::from("/tmp"),
            progress_tx: None,
        };
        let result = rt.run(&args, &sandbox, &ctx).await.unwrap();
        assert!(result.contains("bypass_test"));
        assert!(result.contains("exit_code=0"));
    }

    #[tokio::test]
    async fn output_format_includes_metadata() {
        let rt = ShellRuntime;
        let args = serde_json::json!({"command": "echo metadata_test"});
        let sandbox = SandboxAttempt {
            sandbox_type: SandboxBackend::None,
            cwd: std::path::PathBuf::from("/tmp"),
        };
        let ctx = ToolExecContext {
            turn_id: xiaolin_protocol::TurnId::new("t1"),
            session_id: xiaolin_protocol::SessionId::new("s1"),
            call_id: "c1".into(),
            cwd: std::path::PathBuf::from("/tmp"),
            progress_tx: None,
        };
        let result = rt.run(&args, &sandbox, &ctx).await.unwrap();
        let lines: Vec<&str> = result.lines().collect();
        assert!(lines[0].starts_with("exit_code="));
        assert!(lines[1].starts_with("duration_ms="));
        assert!(lines[2].starts_with("cwd="));
        assert_eq!(lines[3], "---");
    }

    #[tokio::test]
    async fn timeout_ms_parameter_works() {
        let rt = ShellRuntime;
        let args = serde_json::json!({"command": "sleep 10", "timeout_ms": 500});
        let sandbox = SandboxAttempt {
            sandbox_type: SandboxBackend::None,
            cwd: std::path::PathBuf::from("/tmp"),
        };
        let ctx = ToolExecContext {
            turn_id: xiaolin_protocol::TurnId::new("t1"),
            session_id: xiaolin_protocol::SessionId::new("s1"),
            call_id: "c1".into(),
            cwd: std::path::PathBuf::from("/tmp"),
            progress_tx: None,
        };
        let result = rt.run(&args, &sandbox, &ctx).await;
        assert!(matches!(result, Err(ToolRuntimeError::Timeout { .. })));
    }
}
