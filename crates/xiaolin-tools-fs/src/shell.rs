use std::collections::HashMap;

use xiaolin_core::tool::{
    Tool, ToolErrorType, ToolGroup, ToolKind, ToolParameterSchema, ToolResult,
};

/// Definition-only stub for ToolRegistry. Execution is handled by RuntimeRegistry.
pub struct ShellDefinitionStub;

#[async_trait::async_trait]
impl Tool for ShellDefinitionStub {
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn name(&self) -> &str {
        "shell_exec"
    }
    fn description(&self) -> &str {
        "Run a shell command (sh -c). Returns exit_code, duration, stdout, stderr. \
         Read-only commands (ls, cat, git status, etc.) execute directly without sandbox. \
         Write commands run inside a sandbox with filesystem isolation. \
         Default timeout: 120s (override with timeout_ms). \
         For long-running processes (dev servers, watchers), use 'nohup cmd > output.log 2>&1 &' \
         and poll with 'cat output.log' to monitor progress."
    }

    fn group(&self) -> ToolGroup {
        ToolGroup::System
    }

    fn prompt(&self) -> String {
        "\
Run a shell command via sh -c. Returns exit_code, duration, stdout, stderr.

## When to Use shell_exec

shell_exec is for system commands that have NO dedicated tool:
- Git operations: commit, push, pull, branch, rebase, stash
- Build & test: cargo build, cargo test, npm run, make
- Package management: cargo add, npm install, pip install
- Process inspection: ps, lsof, netstat
- Environment setup: export, source, creating virtualenvs

## When NOT to Use shell_exec

NEVER use shell_exec for operations that have dedicated tools:
- Reading files → use read_file (handles encoding, line ranges, stale detection)
- Writing files → use write_file (atomic writes, conflict detection)
- Editing files → use edit_file (fuzzy match, multi-occurrence handling)
- Searching content → use search_in_files (structured output, regex)
- Finding files → use glob (gitignore-aware, sorted results)
- Listing dirs → use list_directory (structured output)

NEVER use these shell patterns:
- `cat`, `head`, `tail`, `less` → read_file
- `sed -i`, `awk`, `perl -i` → edit_file
- `echo >`, `cat >`, heredoc → write_file
- `grep`, `rg`, `ag` → search_in_files
- `find`, `fd` → glob
- `ls` → list_directory

## Git Operation Rules

### Commits
- Use descriptive commit messages explaining WHY, not WHAT
- Pass message via heredoc for multi-line:
  git commit -m \"$(cat <<'EOF'\n  Your message here\n  EOF\n  )\"
- NEVER use `git commit --amend` unless the user explicitly requests it AND the commit was made by you in this session AND hasn't been pushed

### Safety
- NEVER run `git push --force` to main/master — warn the user first
- NEVER skip hooks (--no-verify) unless user explicitly requests
- NEVER modify git config (user.name, user.email)
- Before destructive operations (reset --hard, clean -fd), confirm with the user

### Workflow
- Run `git status` and `git diff` in parallel to understand state
- Always verify with `git status` after a commit succeeds

## Command Construction

### Quoting
- Always double-quote paths with spaces: cd \"/path/with spaces\"
- Use single quotes for literal strings: grep 'exact match'

### Chaining
- Use `&&` for dependent commands: mkdir foo && cd foo
- Use `;` only when later commands should run regardless of earlier failures
- NEVER use newlines to separate commands in the command string

### Parallelism
- If you need output from multiple independent commands, make separate shell_exec calls in the same response — they may execute in parallel
- Do NOT chain independent commands with && (forces serial execution)

### Working Directory
- Use the working_dir parameter instead of `cd dir && command`
- Paths are relative to project root or absolute

## Anti-Patterns

NEVER do these:
1. `sleep N && check` polling loops — use appropriate timeouts instead
2. `echo \"message\"` to communicate — write your response text directly
3. Infinite loops or watch commands — they will hit the timeout
4. Here-doc or echo to create files — use write_file
5. `sed -i` to edit files — use edit_file
6. Multi-line scripts — prefer single-line commands; for complex logic, write a script file first
7. `wc -l` to count lines — estimate from what you've already read
8. `pwd` when you already know the working directory
9. `cat file | grep pattern` — use search_in_files directly

## Timeout

Default: 120s. Max: 300s (5min). Override with timeout_ms parameter.
For builds or tests that may take longer, set an appropriate timeout.
If a command times out, do NOT retry with the same timeout — increase it or investigate why it's slow."
            .to_string()
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        shell_parameter_schema(true)
    }
    fn supports_progress(&self) -> bool {
        true
    }
    fn max_result_size_chars(&self) -> usize {
        30_000
    }
    async fn execute(&self, _arguments: &str) -> ToolResult {
        ToolResult::err_with_recovery(
            ToolErrorType::ExecutionFailed,
            "shell_exec execution should go through RuntimeRegistry/orchestrator. \
             This is a definition-only stub.",
            "Do not call this stub directly; shell_exec is executed by the runtime orchestrator.",
        )
    }
}

/// Build common shell parameter schema.
fn shell_parameter_schema(_include_is_background: bool) -> ToolParameterSchema {
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
        "timeout_ms".to_string(),
        serde_json::json!({
            "type": "integer",
            "description": "Timeout in milliseconds. Default 120000 (120s). Max 300000 (5min)."
        }),
    );
    let required = vec!["command".to_string()];
    ToolParameterSchema {
        schema_type: "object".to_string(),
        properties: props,
        required,
    }
}

/// Validate that a full command (with pipes and chains) is entirely readonly.
/// Every segment in pipes (|), AND (&&), OR (||), and semicolons (;) must be readonly.
///
/// Delegates to [`ReadOnlyClassifier`] so Plan mode and sandbox fast-path share one policy.
pub fn validate_readonly_command(command: &str) -> Result<(), String> {
    use crate::shell_readonly::{CommandClassification, ReadOnlyClassifier};

    match ReadOnlyClassifier::classify(command) {
        CommandClassification::ReadOnly => Ok(()),
        CommandClassification::Write { reason } | CommandClassification::Dangerous { reason } => {
            Err(reason)
        }
    }
}

// ─── Path Safety Validation ─────────────────────────────────────────────────
//
// Readonly classification lives in `shell_readonly.rs` (`ReadOnlyClassifier`).
// Write-command path validation is implemented in `shell_path_validation.rs` (`PathValidator`).

/// Validate paths extracted from a command against security rules.
/// Only applies to write commands (rm, mv, cp, touch, etc.) since read commands
/// are bounded by the OS file permissions and the sandbox directory restriction.
pub fn validate_command_paths(command: &str, allowed_dirs: &[String]) -> Result<(), String> {
    use crate::shell_path_validation::{PathValidator, PathVerdict};

    let roots: Vec<std::path::PathBuf> = allowed_dirs.iter().map(std::path::PathBuf::from).collect();
    let validator = PathValidator::new(roots);
    match validator.validate(command) {
        PathVerdict::Safe => Ok(()),
        PathVerdict::Blocked { path, reason } => Err(format!("path '{path}' blocked: {reason}")),
    }
}

// ─── Permission Rule Engine ─────────────────────────────────────────────────

/// Environment variables that indicate binary hijack attempts.
/// These MUST NOT be stripped before rule matching.
const BINARY_HIJACK_VARS: &[&str] = &[
    "PATH",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "DYLD_FRAMEWORK_PATH",
];

/// Wrapper commands that are safe to strip before permission matching.
const SAFE_WRAPPERS: &[&str] = &["timeout", "time", "nice", "nohup", "stdbuf", "env"];

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
                b'*' => {
                    result.push('*');
                    i += 2;
                    continue;
                }
                b'\\' => {
                    result.push('\\');
                    i += 2;
                    continue;
                }
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
            let var_name = stripped[..m.end()].split('=').next().unwrap_or("");
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
                if arg == "--" {
                    i += 1;
                    break;
                }
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
                if arg == "--" {
                    i += 1;
                    break;
                }
                if arg == "-n" {
                    i += 2;
                    continue;
                }
                if arg.starts_with('-') && arg.chars().skip(1).all(|c| c.is_ascii_digit()) {
                    i += 1;
                    continue;
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
                if args[i] == "--" {
                    i += 1;
                    break;
                }
                if args[i].starts_with('-') {
                    i += 1;
                    continue;
                }
                if env_re.is_match(args[i]) {
                    i += 1;
                    continue;
                }
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
            if i < args.len()
                && !args[i].starts_with('-')
                && (args[i].is_empty() || args[i].starts_with('.'))
            {
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
                if expression.is_some() {
                    return None;
                } // multiple expressions not supported
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
    let escaped_pattern = info.pattern.replace('\\', "\\\\").replace('/', "\\/");
    let escaped_replacement = info.replacement.replace('\\', "\\\\").replace('/', "\\/");

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

/// Check for unescaped backtick command substitution.
/// Returns true if the string contains backticks that are not preceded by `\`.
#[allow(dead_code)]
fn contains_unescaped_backticks(s: &str) -> bool {
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'`' && (i == 0 || bytes[i - 1] != b'\\') {
            return true;
        }
    }
    false
}



#[cfg(test)]
mod tests {
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
        assert!(super::validate_readonly_command("cargo tree").is_ok());
    }

    #[test]
    fn readonly_blocks_cargo_test_and_bench() {
        assert!(super::validate_readonly_command("cargo test").is_err());
        assert!(super::validate_readonly_command("cargo bench").is_err());
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
    }

    #[test]
    fn readonly_blocks_npm_test_and_run() {
        assert!(super::validate_readonly_command("npm test").is_err());
        assert!(super::validate_readonly_command("npm run lint").is_err());
    }

    #[test]
    fn readonly_blocks_generic_executors() {
        assert!(super::validate_readonly_command("python3 -c 'print(1)'").is_err());
        assert!(super::validate_readonly_command("node -e '1'").is_err());
        assert!(super::validate_readonly_command("make build").is_err());
        assert!(super::validate_readonly_command("curl -s http://example.com").is_err());
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
        assert!(
            super::validate_command_paths("ls /home/user/project && rm /tmp/evil", &allowed)
                .is_err()
        );
    }

    #[test]
    fn path_no_false_positive_on_quoted_content() {
        assert!(super::validate_command_paths("echo 'rm ~/.ssh/id_rsa'", &[]).is_ok());
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
        assert_eq!(
            super::strip_safe_wrappers("timeout -k 5 10 npm test"),
            "npm test"
        );
    }

    #[test]
    fn strip_wrappers_nice_nohup() {
        assert_eq!(
            super::strip_safe_wrappers("nice -n 10 cargo build"),
            "cargo build"
        );
        assert_eq!(
            super::strip_safe_wrappers("nohup python3 server.py"),
            "python3 server.py"
        );
    }

    #[test]
    fn strip_wrappers_env_vars() {
        assert_eq!(
            super::strip_safe_wrappers("GOOS=linux cargo build"),
            "cargo build"
        );
        assert_eq!(
            super::strip_safe_wrappers("NODE_ENV=test npm test"),
            "npm test"
        );
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

    // --- Namespace isolation tests ---

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
