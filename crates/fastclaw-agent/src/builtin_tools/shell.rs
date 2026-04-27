use std::collections::HashMap;

use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolProgressUpdate, ToolResult, ProgressSender};

/// Default output truncation limit (raised from 8KB to 64KB to capture more useful output).
const DEFAULT_MAX_OUTPUT_BYTES: usize = 65536;

/// Truncate output string at a char boundary, adding a note about total size.
fn truncate_output(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let end = s
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|&i| i <= max_bytes)
        .last()
        .unwrap_or(0);
    format!("{}... [truncated, {} bytes total]", &s[..end], s.len())
}

/// Detect the preferred shell on Unix (bash if available, else sh).
#[cfg(not(windows))]
fn preferred_shell() -> &'static str {
    use std::sync::OnceLock;
    static SHELL: OnceLock<&str> = OnceLock::new();
    *SHELL.get_or_init(|| {
        if std::path::Path::new("/bin/bash").exists()
            || std::path::Path::new("/usr/bin/bash").exists()
        {
            "bash"
        } else {
            "sh"
        }
    })
}

/// Build common shell parameter schema.
fn shell_parameter_schema(include_is_background: bool) -> ToolParameterSchema {
    let mut props = HashMap::new();
    props.insert(
        "command".to_string(),
        serde_json::json!({
            "type": "string",
            "description": "The shell command to execute."
        }),
    );
    props.insert(
        "description".to_string(),
        serde_json::json!({
            "type": "string",
            "description": "Optional brief description of the command's purpose, shown to the user."
        }),
    );
    props.insert(
        "working_dir".to_string(),
        serde_json::json!({
            "type": "string",
            "description": "Optional working directory (relative to project root or absolute). Must exist."
        }),
    );
    if include_is_background {
        props.insert(
            "is_background".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "Whether to run the command in background. Required. \
                 Set true for long-running processes (dev servers, watchers, daemons). \
                 Set false for one-time commands that should complete before proceeding."
            }),
        );
    }
    let mut required = vec!["command".to_string()];
    if include_is_background {
        required.push("is_background".to_string());
    }
    ToolParameterSchema {
        schema_type: "object".to_string(),
        properties: props,
        required,
    }
}

/// Execute a shell command and return stdout/stderr.
pub struct ShellTool {
    timeout_secs: u64,
}

impl ShellTool {
    pub fn new(timeout_secs: u64) -> Self {
        Self { timeout_secs }
    }
}

#[async_trait]
impl Tool for ShellTool {
    fn kind(&self) -> ToolKind { ToolKind::Execute }
    fn name(&self) -> &str {
        "shell_exec"
    }

    fn description(&self) -> &str {
        "Run a shell command. Returns exit_code, stdout, stderr, and signal info. \
         Uses bash -c (Unix) or cmd.exe /C (Windows). \
         Set is_background=true for long-running processes (dev servers, watchers); \
         set is_background=false for one-time commands. \
         stdout/stderr truncated at ~64KB. Non-zero exit_code returned as tool error."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        shell_parameter_schema(true)
    }

    fn supports_progress(&self) -> bool { true }

    async fn execute(&self, arguments: &str) -> ToolResult {
        self.execute_shell(arguments, None).await
    }

    async fn execute_with_progress(
        &self,
        arguments: &str,
        progress: ProgressSender,
    ) -> ToolResult {
        self.execute_shell(arguments, Some(progress)).await
    }
}

impl ShellTool {
    async fn execute_shell(&self, arguments: &str, progress: Option<ProgressSender>) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "shell_exec arguments are not valid JSON: {e}. \
                 Pass {{\"command\": \"...\", \"is_background\": false}} with optional \"working_dir\", \"description\"."
            )),
        };

        let command = match args.get("command").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolResult::err(
                "shell_exec is missing string field 'command'. \
                 Example: {\"command\": \"ls -la\", \"is_background\": false}."
                    .to_string(),
            ),
        };

        let is_background = args
            .get("is_background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let user_confirmed = args
            .get("confirmed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !user_confirmed {
            match fastclaw_security::dangerous_ops::check_dangerous_command(command) {
                Ok(()) => {}
                Err(fastclaw_security::dangerous_ops::CheckResult::Denied(msg)) => {
                    return ToolResult::err(format!(
                        "BLOCKED by dangerous-ops policy (deny): {msg}. \
                         Change the command or ask an admin to adjust security.dangerousOpsPolicy."
                    ));
                }
                Err(fastclaw_security::dangerous_ops::CheckResult::NeedsConfirmation(msg)) => {
                    return ToolResult::needs_confirm(format!(
                        "This command requires user confirmation: {msg}"
                    ));
                }
            }
        }

        #[cfg(windows)]
        let mut cmd = {
            let mut c = tokio::process::Command::new("cmd.exe");
            c.args(["/C", command]);
            c.creation_flags(0x08000000); // CREATE_NO_WINDOW
            c
        };
        #[cfg(not(windows))]
        let mut cmd = {
            let shell = preferred_shell();
            let mut c = tokio::process::Command::new(shell);
            c.arg("-c").arg(command);
            c
        };

        if let Some(dir) = args.get("working_dir").and_then(|v| v.as_str()) {
            cmd.current_dir(dir);
        }

        cmd.env("FASTCLAW_AGENT", "1");

        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let timeout = tokio::time::Duration::from_secs(self.timeout_secs);

        if is_background {
            match cmd.spawn() {
                Ok(mut child) => {
                    let pid = child.id();
                    let desc = args.get("description").and_then(|v| v.as_str()).unwrap_or(command);
                    let cmd_for_log = command.to_string();
                    tokio::spawn(async move {
                        match child.wait().await {
                            Ok(status) => tracing::debug!(
                                pid, command = %cmd_for_log,
                                exit_code = status.code(),
                                "background shell command exited"
                            ),
                            Err(e) => tracing::warn!(
                                pid, command = %cmd_for_log,
                                error = %e,
                                "background shell command wait failed"
                            ),
                        }
                    });
                    ToolResult::ok(
                        serde_json::json!({
                            "background": true,
                            "pid": pid,
                            "command": command,
                            "description": desc,
                        })
                        .to_string(),
                    )
                }
                Err(e) => ToolResult::err(format!("shell_exec spawn failed: {e}")),
            }
        } else if progress.is_some() {
            self.execute_with_streaming(cmd, command, timeout, progress.unwrap()).await
        } else {
            match tokio::time::timeout(timeout, cmd.output()).await {
                Ok(Ok(output)) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let code = output.status.code().unwrap_or(-1);

                    #[cfg(unix)]
                    let signal = {
                        use std::os::unix::process::ExitStatusExt;
                        output.status.signal()
                    };
                    #[cfg(not(unix))]
                    let signal: Option<i32> = None;

                    let result = serde_json::json!({
                        "exit_code": code,
                        "stdout": truncate_output(&stdout, DEFAULT_MAX_OUTPUT_BYTES),
                        "stderr": truncate_output(&stderr, DEFAULT_MAX_OUTPUT_BYTES),
                        "signal": signal,
                    });

                    let full_display = result.to_string();
                    if code == 0 {
                        let llm_summary = format!(
                            "{{\"exit_code\":0,\"stdout\":{},\"stderr\":{}}}",
                            serde_json::Value::String(truncate_output(&stdout, DEFAULT_MAX_OUTPUT_BYTES)),
                            serde_json::Value::String(truncate_output(&stderr, DEFAULT_MAX_OUTPUT_BYTES)),
                        );
                        ToolResult::ok_split(llm_summary, full_display)
                    } else {
                        ToolResult::err(format!(
                            "exit_code={code}: {}", result
                        ))
                    }
                }
                Ok(Err(e)) => ToolResult::err(format!(
                    "shell_exec spawn failed: {e}"
                )),
                Err(_) => ToolResult::err(format!(
                    "shell_exec timed out after {}s, command killed.",
                    self.timeout_secs
                )),
            }
        }
    }

    async fn execute_with_streaming(
        &self,
        mut cmd: tokio::process::Command,
        _command: &str,
        timeout: tokio::time::Duration,
        progress: ProgressSender,
    ) -> ToolResult {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("shell_exec spawn failed: {e}")),
        };

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let progress_clone = progress.clone();
        let stdout_task = tokio::spawn(async move {
            let mut lines = Vec::new();
            if let Some(out) = stdout {
                let mut reader = BufReader::new(out).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    lines.push(line.clone());
                    let count = lines.len();
                    if count % 10 == 0 || count <= 5 {
                        let _ = progress_clone.send(ToolProgressUpdate {
                            message: format!("stdout: {} lines", count),
                            progress: None,
                            partial_output: Some(
                                lines.iter().rev().take(5).rev()
                                    .cloned().collect::<Vec<_>>().join("\n")
                            ),
                        }).await;
                    }
                }
            }
            lines.join("\n")
        });

        let stderr_task = tokio::spawn(async move {
            let mut lines = Vec::new();
            if let Some(err) = stderr {
                let mut reader = BufReader::new(err).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    lines.push(line);
                }
            }
            lines.join("\n")
        });

        let wait_result = tokio::time::timeout(timeout, async {
            let stdout_out = stdout_task.await.unwrap_or_default();
            let stderr_out = stderr_task.await.unwrap_or_default();
            let status = child.wait().await;
            (stdout_out, stderr_out, status)
        }).await;

        match wait_result {
            Ok((stdout_out, stderr_out, Ok(status))) => {
                let code = status.code().unwrap_or(-1);
                let total_lines = stdout_out.lines().count() + stderr_out.lines().count();

                let _ = progress.send(ToolProgressUpdate {
                    message: format!("completed: exit_code={code}, {total_lines} total lines"),
                    progress: Some(1.0),
                    partial_output: None,
                }).await;

                #[cfg(unix)]
                let signal = {
                    use std::os::unix::process::ExitStatusExt;
                    status.signal()
                };
                #[cfg(not(unix))]
                let signal: Option<i32> = None;

                let result = serde_json::json!({
                    "exit_code": code,
                    "stdout": truncate_output(&stdout_out, DEFAULT_MAX_OUTPUT_BYTES),
                    "stderr": truncate_output(&stderr_out, DEFAULT_MAX_OUTPUT_BYTES),
                    "signal": signal,
                });

                let full_display = result.to_string();
                if code == 0 {
                    let llm_summary = format!(
                        "{{\"exit_code\":0,\"stdout\":{},\"stderr\":{}}}",
                        serde_json::Value::String(truncate_output(&stdout_out, DEFAULT_MAX_OUTPUT_BYTES)),
                        serde_json::Value::String(truncate_output(&stderr_out, DEFAULT_MAX_OUTPUT_BYTES)),
                    );
                    ToolResult::ok_split(llm_summary, full_display)
                } else {
                    ToolResult::err(format!("exit_code={code}: {}", result))
                }
            }
            Ok((_, _, Err(e))) => ToolResult::err(format!("shell_exec wait failed: {e}")),
            Err(_) => {
                let _ = child.kill().await;
                ToolResult::err(format!(
                    "shell_exec timed out after {}s, command killed.",
                    self.timeout_secs
                ))
            }
        }
    }
}
// --- Sandboxed Shell Execution ---

/// Policy-driven sandbox configuration for shell command execution.
#[derive(Debug, Clone)]
pub struct ShellSandboxConfig {
    /// Maximum execution time in seconds.
    pub timeout_secs: u64,
    /// Maximum output size in bytes before truncation.
    pub max_output_bytes: usize,
    /// Commands that are always blocked (matched against the first token).
    pub denied_commands: Vec<String>,
    /// If non-empty, only these commands are allowed (whitelist mode).
    pub allowed_commands: Vec<String>,
    /// Deny patterns: each is a regex applied to the full command string.
    /// Use `\b` word boundaries to avoid false positives like `eval ` matching `evaluate`.
    pub denied_patterns: Vec<String>,
    /// Compiled deny-pattern regexes (built from `denied_patterns`).
    denied_regexes: Vec<(regex::Regex, String)>,
    /// Allowed working directories. If empty, any directory is allowed.
    pub allowed_dirs: Vec<String>,
    /// Environment variables to strip before execution.
    pub strip_env_vars: Vec<String>,
    /// Whether to enable Linux namespace isolation (requires unshare permissions).
    pub use_namespace: bool,
    /// Read-only filesystem paths (for namespace mode).
    pub readonly_paths: Vec<String>,
}

impl ShellSandboxConfig {
    fn compile_denied_regexes(patterns: &[String]) -> Vec<(regex::Regex, String)> {
        patterns
            .iter()
            .filter_map(|p| {
                regex::Regex::new(p)
                    .map(|re| (re, p.clone()))
                    .map_err(|e| {
                        tracing::warn!(
                            pattern = %p,
                            error = %e,
                            "sandbox: skipped invalid denied_pattern regex"
                        );
                    })
                    .ok()
            })
            .collect()
    }
}

fn default_denied_patterns() -> Vec<String> {
    vec![
        r"> /dev/".into(),
        r"\| /dev/".into(),
        r":\(\)\{ :\|:& \};:".into(),
        r"\beval\b".into(),
        r"/etc/shadow".into(),
        r"/etc/passwd".into(),
        r"~/\.ssh\b".into(),
        r"\.ssh/".into(),
    ]
}

impl Default for ShellSandboxConfig {
    fn default() -> Self {
        let patterns = default_denied_patterns();
        let denied_regexes = Self::compile_denied_regexes(&patterns);
        Self {
            timeout_secs: 30,
            max_output_bytes: 65536,
            denied_commands: vec![
                // rm, rmdir, chmod, chown, chgrp are handled by the dangerous_ops policy
                // (deny/confirm/allow) rather than hard-blocked here.
                "mkfs".into(),
                "dd".into(),
                "shutdown".into(),
                "reboot".into(),
                "poweroff".into(),
                "halt".into(),
                // kill/killall/pkill removed — allow process management for dev workflows
                "mount".into(),
                "umount".into(),
                "iptables".into(),
                "ip6tables".into(),
                "nft".into(),
                "useradd".into(),
                "userdel".into(),
                "usermod".into(),
                "passwd".into(),
                "su".into(),
                "sudo".into(),
                "crontab".into(),
                "nc".into(),
                "ncat".into(),
                "socat".into(),
            ],
            allowed_commands: Vec::new(),
            denied_patterns: patterns,
            denied_regexes,
            allowed_dirs: Vec::new(),
            strip_env_vars: vec![
                "AWS_SECRET_ACCESS_KEY".into(),
                "AWS_SESSION_TOKEN".into(),
                "GITHUB_TOKEN".into(),
                "GH_TOKEN".into(),
                "NPM_TOKEN".into(),
                "DATABASE_URL".into(),
                "PRIVATE_KEY".into(),
            ],
            use_namespace: false,
            readonly_paths: Vec::new(),
        }
    }
}

/// Sandboxed shell execution tool with policy enforcement.
pub struct SandboxedShellTool {
    config: ShellSandboxConfig,
}

impl SandboxedShellTool {
    pub fn new(mut config: ShellSandboxConfig) -> Self {
        if config.denied_regexes.is_empty() && !config.denied_patterns.is_empty() {
            config.denied_regexes =
                ShellSandboxConfig::compile_denied_regexes(&config.denied_patterns);
        }
        Self { config }
    }

    fn validate_command(&self, command: &str) -> Result<(), String> {
        let trimmed = command.trim();
        if trimmed.is_empty() {
            return Err(
                "Sandbox shell command is empty after trimming whitespace. \
                 Provide a non-empty sh -c snippet, e.g. {\"command\": \"echo ok\"}."
                    .into(),
            );
        }

        let first_token = trimmed.split_whitespace().next().unwrap_or("");
        let base_cmd = first_token.rsplit('/').next().unwrap_or(first_token);

        if !self.config.allowed_commands.is_empty() {
            if !self.config.allowed_commands.iter().any(|c| c == base_cmd) {
                return Err(format!(
                    "Sandbox allowlist rejects first command '{base_cmd}'. \
                     Allowed base commands: {}. \
                     Rewrite the pipeline using only those binaries, or ask the operator to widen allowed_commands.",
                    self.config.allowed_commands.join(", ")
                ));
            }
        }

        if self.config.denied_commands.iter().any(|c| c == base_cmd) {
            return Err(format!(
                "Sandbox policy blocks base command '{base_cmd}' (classified as high-risk). \
                 Use read_file/write_file/list_directory or narrower tools instead of shell for that action, or ask the operator to change denied_commands if this block is a false positive."
            ));
        }

        for (re, pattern) in &self.config.denied_regexes {
            if re.is_match(trimmed) {
                return Err(format!(
                    "Sandbox denied_pattern matched regex '{pattern}' in the command text. \
                     Remove that construct (often sensitive paths or shell tricks) and use approved tools; if this is a false positive, ask the operator to tune denied_patterns."
                ));
            }
        }

        for part in trimmed
            .split("&&")
            .chain(trimmed.split("||"))
            .chain(trimmed.split(';'))
        {
            let sub = part.trim();
            let sub_cmd = sub.split_whitespace().next().unwrap_or("");
            let sub_base = sub_cmd.rsplit('/').next().unwrap_or(sub_cmd);
            if sub_base.is_empty() {
                continue;
            }
            if self.config.denied_commands.iter().any(|c| c == sub_base) {
                return Err(format!(
                    "Sandbox detected blocked base command '{sub_base}' inside a chained segment (split on &&, ||, or ;). \
                     Every segment must pass the same allow/deny rules—remove or replace the risky segment, or split into separate shell_exec calls."
                ));
            }
        }

        Ok(())
    }

    fn validate_dir(&self, dir: &str) -> Result<(), String> {
        if self.config.allowed_dirs.is_empty() {
            return Ok(());
        }
        let canonical = std::path::Path::new(dir)
            .canonicalize()
            .map_err(|e| {
                format!(
                    "Sandbox working_dir '{dir}' could not be canonicalized: {e}. \
                     Pass an existing directory the gateway user can stat, or omit working_dir if policy permits."
                )
            })?;
        let is_allowed = self.config.allowed_dirs.iter().any(|d| {
            let allowed = std::path::Path::new(d.as_str());
            let allowed_canonical = allowed.canonicalize().unwrap_or_else(|_| allowed.to_path_buf());
            canonical.starts_with(&allowed_canonical)
        });
        if is_allowed {
            Ok(())
        } else {
            Err(format!(
                "Sandbox working_dir '{dir}' resolves outside allowed directory roots: {}. \
                 Choose a cwd inside one of those roots, or omit working_dir when configuration allows any cwd.",
                self.config.allowed_dirs.join(", ")
            ))
        }
    }
}

#[async_trait]
impl Tool for SandboxedShellTool {
    fn kind(&self) -> ToolKind { ToolKind::Execute }
    fn name(&self) -> &str {
        "shell_exec"
    }

    fn description(&self) -> &str {
        "Sandboxed shell_exec — commands validated against allow/deny rules before execution. \
         Uses bash -c (Unix) or cmd.exe /C (Windows). \
         Set is_background=true for long-running processes (dev servers, watchers); \
         set is_background=false for one-time commands. \
         Blocked commands (sudo, mkfs, dd, etc.) return SANDBOX BLOCKED. \
         Destructive ops (rm, chmod) follow the dangerous_ops security policy."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        shell_parameter_schema(true)
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "sandboxed shell_exec arguments are not valid JSON: {e}. \
                 Pass {{\"command\": \"...\", \"is_background\": false}} with optional \"working_dir\", \"description\"."
            )),
        };

        let command = match args.get("command").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolResult::err(
                "sandboxed shell_exec is missing string field 'command'. \
                 Example: {\"command\": \"echo ok\", \"is_background\": false}."
                    .to_string(),
            ),
        };

        let is_background = args
            .get("is_background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if let Err(reason) = self.validate_command(command) {
            return ToolResult::err(format!(
                "SANDBOX BLOCKED: {reason} \
                 Adjust the command to comply, or use non-shell tools (read_file, write_file, web_fetch) where appropriate."
            ));
        }

        let user_confirmed = args
            .get("confirmed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !user_confirmed {
            match fastclaw_security::dangerous_ops::check_dangerous_command(command) {
                Ok(()) => {}
                Err(fastclaw_security::dangerous_ops::CheckResult::Denied(msg)) => {
                    return ToolResult::err(format!(
                        "BLOCKED by dangerous-ops policy (deny): {msg}. \
                         Change the command or ask an admin to adjust security.dangerousOpsPolicy."
                    ));
                }
                Err(fastclaw_security::dangerous_ops::CheckResult::NeedsConfirmation(msg)) => {
                    return ToolResult::needs_confirm(format!(
                        "This command requires user confirmation: {msg}"
                    ));
                }
            }
        }

        if let Some(dir) = args.get("working_dir").and_then(|v| v.as_str()) {
            if let Err(reason) = self.validate_dir(dir) {
                return ToolResult::err(format!(
                    "SANDBOX BLOCKED: {reason} \
                     Choose a working_dir under an allowed root or omit working_dir if permitted."
                ));
            }
        }

        let mut cmd = if self.config.use_namespace {
            #[cfg(not(windows))]
            {
                let shell = preferred_shell();
                let mut c = tokio::process::Command::new("unshare");
                c.args(["--mount", "--pid", "--fork", "--", shell, "-c", command]);
                c
            }
            #[cfg(windows)]
            {
                // Namespace sandboxing via unshare is Linux-only; fallback to cmd on Windows.
                let mut c = tokio::process::Command::new("cmd.exe");
                c.args(["/C", command]);
                c.creation_flags(0x08000000); // CREATE_NO_WINDOW
                c
            }
        } else {
            #[cfg(windows)]
            {
                let mut c = tokio::process::Command::new("cmd.exe");
                c.args(["/C", command]);
                c.creation_flags(0x08000000); // CREATE_NO_WINDOW
                c
            }
            #[cfg(not(windows))]
            {
                let shell = preferred_shell();
                let mut c = tokio::process::Command::new(shell);
                c.arg("-c").arg(command);
                c
            }
        };

        if let Some(dir) = args.get("working_dir").and_then(|v| v.as_str()) {
            cmd.current_dir(dir);
        }

        for var in &self.config.strip_env_vars {
            cmd.env_remove(var);
        }
        cmd.env("FASTCLAW_AGENT", "1");

        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let timeout = tokio::time::Duration::from_secs(self.config.timeout_secs);
        let max_out = self.config.max_output_bytes;

        if is_background {
            match cmd.spawn() {
                Ok(child) => {
                    let pid = child.id();
                    let desc = args.get("description").and_then(|v| v.as_str()).unwrap_or(command);
                    ToolResult::ok(
                        serde_json::json!({
                            "background": true,
                            "pid": pid,
                            "command": command,
                            "description": desc,
                            "sandboxed": true,
                        })
                        .to_string(),
                    )
                }
                Err(e) => ToolResult::err(format!("sandboxed shell_exec spawn failed: {e}")),
            }
        } else {
            match tokio::time::timeout(timeout, cmd.output()).await {
                Ok(Ok(output)) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let code = output.status.code().unwrap_or(-1);

                    #[cfg(unix)]
                    let signal = {
                        use std::os::unix::process::ExitStatusExt;
                        output.status.signal()
                    };
                    #[cfg(not(unix))]
                    let signal: Option<i32> = None;

                    let result = serde_json::json!({
                        "exit_code": code,
                        "stdout": truncate_output(&stdout, max_out),
                        "stderr": truncate_output(&stderr, max_out),
                        "signal": signal,
                        "sandboxed": true,
                    });

                    let full_display = result.to_string();
                    if code == 0 {
                        let llm_summary = format!(
                            "{{\"exit_code\":0,\"stdout\":{},\"stderr\":{}}}",
                            serde_json::Value::String(truncate_output(&stdout, max_out)),
                            serde_json::Value::String(truncate_output(&stderr, max_out)),
                        );
                        ToolResult::ok_split(llm_summary, full_display)
                    } else {
                        ToolResult::err(format!(
                            "exit_code={code}: {}", result
                        ))
                    }
                }
                Ok(Err(e)) => ToolResult::err(format!(
                    "sandboxed shell_exec spawn failed: {e}"
                )),
                Err(_) => ToolResult::err(format!(
                    "sandboxed shell_exec timed out after {}s, command killed.",
                    self.config.timeout_secs
                )),
            }
        }
    }
}

#[cfg(test)]
mod sandbox_tests {
    use super::*;

    fn default_sandbox() -> SandboxedShellTool {
        SandboxedShellTool::new(ShellSandboxConfig::default())
    }

    #[test]
    fn rm_not_blocked_by_sandbox() {
        // rm is now handled by dangerous_ops policy, not the sandbox
        let tool = default_sandbox();
        assert!(tool.validate_command("rm -rf /").is_ok());
    }

    #[test]
    fn blocks_sudo() {
        let tool = default_sandbox();
        assert!(tool.validate_command("sudo apt install foo").is_err());
    }

    #[test]
    fn blocks_chained_dangerous() {
        let tool = default_sandbox();
        // rm and kill are no longer blocked by sandbox (handled by dangerous_ops policy)
        assert!(tool.validate_command("echo hello && rm file.txt").is_ok());
        // sudo is still blocked
        assert!(tool.validate_command("ls; sudo rm -rf /").is_err());
    }

    #[test]
    fn blocks_denied_patterns() {
        let tool = default_sandbox();
        assert!(tool.validate_command("cat /etc/shadow").is_err());
        assert!(tool.validate_command("cat ~/.ssh/id_rsa").is_err());
    }

    #[test]
    fn allows_safe_commands() {
        let tool = default_sandbox();
        assert!(tool.validate_command("echo hello").is_ok());
        assert!(tool.validate_command("ls -la").is_ok());
        assert!(tool.validate_command("cat README.md").is_ok());
        assert!(tool.validate_command("python3 script.py").is_ok());
        assert!(tool.validate_command("git status").is_ok());
        assert!(tool.validate_command("cargo build").is_ok());
    }

    #[test]
    fn allowlist_mode() {
        let config = ShellSandboxConfig {
            allowed_commands: vec!["ls".into(), "echo".into(), "cat".into()],
            ..Default::default()
        };
        let tool = SandboxedShellTool::new(config);
        assert!(tool.validate_command("ls -la").is_ok());
        assert!(tool.validate_command("echo hi").is_ok());
        assert!(tool.validate_command("python3 foo.py").is_err());
    }

    #[test]
    fn dir_restriction() {
        let config = ShellSandboxConfig {
            allowed_dirs: vec!["/tmp".into()],
            ..Default::default()
        };
        let tool = SandboxedShellTool::new(config);
        assert!(tool.validate_dir("/tmp").is_ok());
        assert!(tool.validate_dir("/etc").is_err());
    }

    #[tokio::test]
    async fn executes_safe_command() {
        let tool = default_sandbox();
        let result = tool.execute(r#"{"command": "echo sandbox_test", "is_background": false}"#).await;
        assert!(result.success, "command should succeed: {}", result.output);
        assert!(result.output.contains("sandbox_test"));
        // "sandboxed" flag is in display_output (richer UI representation)
        if let Some(ref display) = result.display_output {
            assert!(
                display.contains("\"sandboxed\":true") || display.contains("\"sandboxed\": true"),
                "display_output should contain sandboxed flag: {}", display
            );
        }
    }

    #[tokio::test]
    async fn rejects_mkfs_at_execute() {
        let tool = default_sandbox();
        let result = tool.execute(r#"{"command": "mkfs /dev/sda"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("SANDBOX BLOCKED"));
    }

    #[test]
    fn eval_regex_blocks_eval_command() {
        let tool = default_sandbox();
        assert!(tool.validate_command("eval $(cat script.sh)").is_err());
        assert!(tool.validate_command(r#"bash -c "eval something""#).is_err());
    }

    #[test]
    fn eval_regex_allows_evaluate_and_similar() {
        let tool = default_sandbox();
        assert!(tool.validate_command("python3 evaluate_model.py").is_ok());
        assert!(tool.validate_command("cargo test -- test_evaluate").is_ok());
        assert!(tool.validate_command("echo 'retrieval system'").is_ok());
        assert!(tool.validate_command("node evaluation.js").is_ok());
    }

    #[test]
    fn ssh_regex_blocks_actual_ssh_paths() {
        let tool = default_sandbox();
        assert!(tool.validate_command("cat ~/.ssh/id_rsa").is_err());
        assert!(tool.validate_command("ls .ssh/config").is_err());
    }

    #[test]
    fn ssh_regex_allows_non_ssh_paths() {
        let tool = default_sandbox();
        assert!(tool.validate_command("cat docs/openssh-guide.md").is_ok());
    }

    #[tokio::test]
    async fn background_command_returns_pid() {
        let tool = default_sandbox();
        let result = tool.execute(r#"{"command": "sleep 0.1", "is_background": true}"#).await;
        assert!(result.success, "background command should succeed: {}", result.output);
        assert!(
            result.output.contains("\"background\":true") || result.output.contains("\"background\": true"),
            "output should indicate background: {}", result.output
        );
        assert!(
            result.output.contains("\"pid\""),
            "output should include PID: {}", result.output
        );
    }

    #[tokio::test]
    async fn foreground_command_returns_signal_field() {
        let tool = default_sandbox();
        let result = tool.execute(r#"{"command": "echo signal_test", "is_background": false}"#).await;
        assert!(result.success, "command should succeed: {}", result.output);
        // display_output should contain signal field
        if let Some(ref display) = result.display_output {
            assert!(
                display.contains("\"signal\""),
                "display output should include signal field: {}", display
            );
        }
    }

    #[tokio::test]
    async fn description_field_in_background_output() {
        let tool = default_sandbox();
        let result = tool.execute(
            r#"{"command": "sleep 0.1", "is_background": true, "description": "test bg process"}"#
        ).await;
        assert!(result.success, "should succeed: {}", result.output);
        assert!(
            result.output.contains("test bg process"),
            "should include description: {}", result.output
        );
    }
}
