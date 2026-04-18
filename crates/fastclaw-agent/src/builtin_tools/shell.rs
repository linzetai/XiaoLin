use std::collections::HashMap;

use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolResult};

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
    fn name(&self) -> &str {
        "shell_exec"
    }

    fn description(&self) -> &str {
        "Run one shell snippet via sh -c and return JSON {exit_code, stdout, stderr}. This is the escape hatch for builds (cargo test, npm run), git, package managers, ripgrep/grep, and short glue scripts when no narrower builtin fits. \
         Prefer read_file, write_file, and list_directory for direct file reads, writes, and one-level directory listings—shell_exec is the most powerful builtin and the easiest to misuse: wrong working_dir, fragile quoting (nested quotes in JSON), accidental destructive commands, or unintended network or secret exfiltration. \
         The child inherits the gateway environment and current working directory unless you set working_dir explicitly; never assume sudo, rm -rf on precious paths, credential-bearing env vars, or outbound network access are allowed without explicit user or operator approval. \
         Each call has a hard wall-clock timeout; stdin is not interactive—commands that prompt for passwords will hang until timeout unless you pass non-interactive flags or files. stdout and stderr are truncated around 8192 bytes per stream with a trailing total-byte suffix to protect model context—if output is truncated, rerun with quieter flags, narrower paths, or pipe through head/tail. \
         Non-zero exit_code is returned as a tool error, but the error string still embeds the same JSON payload—read stderr first for the primary compiler/test/git message, then stdout, then fix flags, cwd, missing dependencies, or tests before retrying. \
         Anti-pattern: cat-ing huge logs or binaries—use head/tail, rg with a path, or write_file + read_file for bounded slices. \
         Example when the repo root matters: {\"command\": \"cargo check -p fastclaw-agent\", \"working_dir\": \"/home/you/workspace/FastClaw\"}."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "command".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Single string passed to sh -c (pipes, &&, ; allowed). Examples: 'cargo test -p fastclaw-agent', 'rg -n pattern crates/', 'git status --short', 'npm test -- --runTestsByPath src/foo.test.ts'. Keep commands short; do not embed megabyte payloads—use write_file for large inputs. If stdout/stderr grow too large, add '| tail -n 80', quieter flags, or narrower paths."
            }),
        );
        props.insert(
            "working_dir".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional cwd; the directory must already exist. Examples: absolute repo root '/home/you/FastClaw', or 'crates/fastclaw-agent' for crate-scoped cargo. Omit only when you are sure the gateway cwd is correct—wrong cwd is the most common cause of spurious \"file not found\" and missing Cargo.toml errors in builds."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["command".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "shell_exec arguments are not valid JSON: {e}. \
                 Pass {{\"command\": \"...\"}} with optional \"working_dir\": \"...\", both as strings, then retry."
            )),
        };

        let command = match args.get("command").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolResult::err(
                "shell_exec is missing string field 'command'. \
                 Example: {\"command\": \"ls -la\"}. \
                 working_dir alone is not enough—the shell snippet must be non-empty."
                    .to_string(),
            ),
        };

        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(command);

        if let Some(dir) = args.get("working_dir").and_then(|v| v.as_str()) {
            cmd.current_dir(dir);
        }

        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let timeout = tokio::time::Duration::from_secs(self.timeout_secs);

        match tokio::time::timeout(timeout, cmd.output()).await {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let code = output.status.code().unwrap_or(-1);

                let truncate = |s: &str| -> String {
                    if s.len() > 8192 {
                        let end = s
                            .char_indices()
                            .map(|(i, _)| i)
                            .take_while(|&i| i <= 8192)
                            .last()
                            .unwrap_or(0);
                        format!("{}... [truncated, {} bytes total]", &s[..end], s.len())
                    } else {
                        s.to_string()
                    }
                };

                let result = serde_json::json!({
                    "exit_code": code,
                    "stdout": truncate(&stdout),
                    "stderr": truncate(&stderr),
                });

                if code == 0 {
                    ToolResult::ok(result.to_string())
                } else {
                    ToolResult::err(format!(
                        "shell_exec finished with exit_code={code} (non-zero), which is treated as failure. What went wrong: the shell snippet returned a failing status—often a compiler/test/git error. What to do next: parse the embedded JSON in this message; read stderr first for the primary error line, then stdout for command output; fix flags, install missing binaries, correct working_dir to the repo root, or address failing tests; rerun a narrower command if the output was truncated. JSON payload: {}",
                        result.to_string()
                    ))
                }
            }
            Ok(Err(e)) => ToolResult::err(format!(
                "shell_exec could not spawn the subprocess: {e}. \
                 What went wrong: sh -c never started (bad path to sh, invalid working_dir, resource limits, or OS refusal). \
                 What to do next: confirm /bin/sh is available, working_dir exists and is a directory the gateway user can enter, and the command string is valid; retry with a minimal command like echo ok to isolate the issue."
            )),
            Err(_) => ToolResult::err(format!(
                "shell_exec timed out after {}s—the command was killed. \
                 Narrow output (add head/tail, filters), split into smaller steps, or ask the operator to raise shell_exec timeout if the workload is legitimately long.",
                self.timeout_secs
            )),
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
    /// Deny patterns matched against the full command string (regex-like substring).
    pub denied_patterns: Vec<String>,
    /// Allowed working directories. If empty, any directory is allowed.
    pub allowed_dirs: Vec<String>,
    /// Environment variables to strip before execution.
    pub strip_env_vars: Vec<String>,
    /// Whether to enable Linux namespace isolation (requires unshare permissions).
    pub use_namespace: bool,
    /// Read-only filesystem paths (for namespace mode).
    pub readonly_paths: Vec<String>,
}

impl Default for ShellSandboxConfig {
    fn default() -> Self {
        Self {
            timeout_secs: 30,
            max_output_bytes: 65536,
            denied_commands: vec![
                "rm".into(),
                "rmdir".into(),
                "mkfs".into(),
                "dd".into(),
                "shutdown".into(),
                "reboot".into(),
                "poweroff".into(),
                "halt".into(),
                "kill".into(),
                "killall".into(),
                "pkill".into(),
                "chmod".into(),
                "chown".into(),
                "chgrp".into(),
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
            denied_patterns: vec![
                "> /dev/".into(),
                "| /dev/".into(),
                ":(){ :|:& };:".into(),
                "eval ".into(),
                "/etc/shadow".into(),
                "/etc/passwd".into(),
                "~/.ssh".into(),
                ".ssh/".into(),
            ],
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
    pub fn new(config: ShellSandboxConfig) -> Self {
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

        for pattern in &self.config.denied_patterns {
            if trimmed.contains(pattern.as_str()) {
                return Err(format!(
                    "Sandbox denied_pattern matched substring '{pattern}' in the command text. \
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
    fn name(&self) -> &str {
        "shell_exec"
    }

    fn description(&self) -> &str {
        "Same contract as shell_exec—JSON {command, working_dir?} in, JSON {exit_code, stdout, stderr, sandboxed: true} out—but every command is validated before spawn: risky base binaries and each && / || / ; segment must pass allow/deny rules. \
         Typical enforcement: blocklists (rm, sudo, chmod, …), pattern denials (sensitive paths, fork bombs), optional command allowlists, optional cwd confined to allowed_dirs, secret env vars stripped, optional Linux namespace isolation via unshare when enabled. \
         Prefer read_file, write_file, list_directory for plain file work; use sandboxed shell_exec when operators require bounded automation (git, cargo, npm, rg) instead of arbitrary shell. \
         stdout+stderr share one max_output_bytes ceiling; timeouts still kill hung work; stdin stays non-interactive. \
         SANDBOX BLOCKED means zero execution—adjust the command, cwd, or policy; resubmitting the same blocked command will never succeed. \
         Example: {\"command\": \"git diff --stat\", \"working_dir\": \"/repo\"} with allowed_dirs covering /repo."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "command".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Argument to sh -c (or unshare … sh -c when namespace mode is on). Examples: 'git status -sb', 'cargo check -p mycrate', 'rg -n pattern .'. Must satisfy allowlist, denylist, and pattern rules or the tool returns SANDBOX BLOCKED without spawning."
            }),
        );
        props.insert(
            "working_dir".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional cwd. If the deployment sets allowed_dirs, the canonicalized cwd must lie under one of those roots; otherwise any existing directory the gateway user can access is allowed. The directory must exist before the command runs."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["command".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "sandboxed shell_exec arguments are not valid JSON: {e}. \
                 Pass {{\"command\": \"...\"}} with optional \"working_dir\", then retry."
            )),
        };

        let command = match args.get("command").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolResult::err(
                "sandboxed shell_exec is missing string field 'command'. \
                 Example: {\"command\": \"echo ok\"}. \
                 Ensure the snippet obeys allow/deny policy before retrying."
                    .to_string(),
            ),
        };

        if let Err(reason) = self.validate_command(command) {
            return ToolResult::err(format!(
                "SANDBOX BLOCKED: {reason} \
                 Adjust the command to comply, or use non-shell tools (read_file, write_file, web_fetch) where appropriate."
            ));
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
            let mut c = tokio::process::Command::new("unshare");
            c.args(["--mount", "--pid", "--fork", "--", "sh", "-c", command]);
            c
        } else {
            let mut c = tokio::process::Command::new("sh");
            c.arg("-c").arg(command);
            c
        };

        if let Some(dir) = args.get("working_dir").and_then(|v| v.as_str()) {
            cmd.current_dir(dir);
        }

        for var in &self.config.strip_env_vars {
            cmd.env_remove(var);
        }

        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let timeout = tokio::time::Duration::from_secs(self.config.timeout_secs);
        let max_out = self.config.max_output_bytes;

        match tokio::time::timeout(timeout, cmd.output()).await {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let code = output.status.code().unwrap_or(-1);

                let truncate = |s: &str| -> String {
                    if s.len() > max_out {
                        format!("{}... [truncated, {} bytes total]", &s[..max_out], s.len())
                    } else {
                        s.to_string()
                    }
                };

                let result = serde_json::json!({
                    "exit_code": code,
                    "stdout": truncate(&stdout),
                    "stderr": truncate(&stderr),
                    "sandboxed": true,
                });

                if code == 0 {
                    ToolResult::ok(result.to_string())
                } else {
                    ToolResult::err(format!(
                        "sandboxed shell_exec finished with exit_code={code} (failure). What went wrong: the command ran but returned a non-zero status. What to do next: inspect stderr then stdout inside the JSON payload; fix compiler/test/git errors, correct working_dir, or narrow the command; retry. JSON payload: {}",
                        result.to_string()
                    ))
                }
            }
            Ok(Err(e)) => ToolResult::err(format!(
                "sandboxed shell_exec failed to start the subprocess: {e}. \
                 What went wrong: the process did not launch after sandbox validation (missing sh/unshare, bad working_dir, or OS spawn error). \
                 What to do next: verify sh exists, if use_namespace is on confirm unshare is installed and permitted, ensure working_dir is a real directory, then retry with echo ok; if spawn still fails, escalate to the operator."
            )),
            Err(_) => ToolResult::err(format!(
                "sandboxed shell_exec timed out after {}s—the process was stopped. \
                 Narrow the command, paginate output, or split work across calls; stdout/stderr are capped by max_output_bytes.",
                self.config.timeout_secs
            )),
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
    fn blocks_rm() {
        let tool = default_sandbox();
        assert!(tool.validate_command("rm -rf /").is_err());
    }

    #[test]
    fn blocks_sudo() {
        let tool = default_sandbox();
        assert!(tool.validate_command("sudo apt install foo").is_err());
    }

    #[test]
    fn blocks_chained_dangerous() {
        let tool = default_sandbox();
        assert!(tool.validate_command("echo hello && rm file.txt").is_err());
        assert!(tool.validate_command("ls; kill -9 1234").is_err());
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
        let result = tool.execute(r#"{"command": "echo sandbox_test"}"#).await;
        assert!(result.success);
        assert!(result.output.contains("sandbox_test"));
        assert!(
            result.output.contains("\"sandboxed\":true")
                || result.output.contains("\"sandboxed\": true")
        );
    }

    #[tokio::test]
    async fn rejects_dangerous_at_execute() {
        let tool = default_sandbox();
        let result = tool.execute(r#"{"command": "rm -rf /"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("SANDBOX BLOCKED"));
    }
}
