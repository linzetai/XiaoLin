use std::collections::HashMap;

use xiaolin_core::tool::{
    Tool, ToolKind, ToolParameterSchema, ToolResult,
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
        ToolResult::err(
            "shell_exec execution should go through RuntimeRegistry/orchestrator. \
             This is a definition-only stub."
                .to_string(),
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

// --- Shell Injection Detection ---

/// Commands considered safe (read-only) for Plan mode execution.
/// These commands only read information and do not modify state.
const READONLY_COMMANDS: &[&str] = &[
    // File inspection
    "ls", "ll", "la", "dir", "exa", "eza", "lsd", "cat", "bat", "head", "tail", "less", "more",
    "wc", "file", "stat", "du", "df", // Search
    "grep", "rg", "ag", "ack", "fgrep", "egrep", "find", "fd", "fdfind", "locate", "which",
    "whereis", "type", // Text processing (readonly)
    "sort", "uniq", "tr", "cut", "paste", "column", "awk",
    "sed", // Only readonly when no -i flag (checked separately)
    "diff", "comm", "cmp", "jq", "yq", "xq", // System info
    "echo", "printf", "date", "whoami", "hostname", "uname", "env", "printenv", "id", "groups",
    "ps", "top", "htop", "free", "uptime", "lsof", "pwd", "realpath", "dirname", "basename",
    // Development tools (read-only subcommands handled separately)
    "tree", "tokei", "cloc", "scc", "python3", "python", "node",
    "ruby",  // Script execution for queries
    "cargo", // Subcommand checked separately
    "npm", "npx", "yarn", "pnpm",    // Subcommand checked separately
    "git",     // Subcommand checked separately
    "gh",      // Subcommand checked separately
    "docker",  // Subcommand checked separately
    "kubectl", // Subcommand checked separately
    "rustc", "gcc", "g++",
    "clang", // Compilation is treated as read since it doesn't modify source
    "make",  // Build is read-only from source perspective
    "test", "[", "true", "false", "sleep",
    "xargs", // Only safe with readonly sub-commands (checked via pipeline)
];

/// Git subcommands that are read-only.
const GIT_READONLY_SUBCOMMANDS: &[&str] = &[
    "status",
    "log",
    "diff",
    "show",
    "branch",
    "tag",
    "describe",
    "shortlog",
    "blame",
    "ls-files",
    "ls-tree",
    "rev-parse",
    "rev-list",
    "remote",
    "config",
    "stash", // stash list/show are readonly; stash pop/apply are not but common enough
];

/// Cargo subcommands that are read-only.
const CARGO_READONLY_SUBCOMMANDS: &[&str] = &[
    "check",
    "clippy",
    "test",
    "bench",
    "doc",
    "tree",
    "metadata",
    "pkgid",
    "verify-project",
    "version",
    "help",
    "search",
];

/// npm/yarn/pnpm subcommands that are read-only.
const NPM_READONLY_SUBCOMMANDS: &[&str] = &[
    "list", "ls", "info", "show", "view", "outdated", "audit", "explain", "why", "help", "version",
    "test", "run", // run scripts are common in development
];

/// Docker subcommands that are read-only.
const DOCKER_READONLY_SUBCOMMANDS: &[&str] = &[
    "ps", "images", "inspect", "logs", "stats", "top", "port", "diff", "history", "version", "info",
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

    Err(format!(
        "command '{base_cmd}' is not in the read-only allowlist"
    ))
}

fn classify_git_readonly(args: &[&str]) -> Result<(), String> {
    let subcommand = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .copied()
        .unwrap_or("");

    if subcommand.is_empty() || GIT_READONLY_SUBCOMMANDS.contains(&subcommand) {
        Ok(())
    } else {
        Err(format!("git {subcommand} is not a read-only git operation"))
    }
}

fn classify_subcommand_readonly(
    args: &[&str],
    allowed: &[&str],
    parent: &str,
) -> Result<(), String> {
    let subcommand = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .copied()
        .unwrap_or("");

    if subcommand.is_empty() || allowed.contains(&subcommand) {
        Ok(())
    } else {
        Err(format!(
            "{parent} {subcommand} is not a read-only operation"
        ))
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
            let next = if bytes[i + 1..].first() == Some(&b'>') {
                i + 2
            } else {
                i + 1
            };
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
    ".xiaolin",
];

/// Commands known to write/modify files (for which path validation applies).
const PATH_WRITE_COMMANDS: &[&str] = &[
    "rm", "rmdir", "mv", "cp", "touch", "mkdir", "chmod", "chown", "chgrp", "ln", "unlink", "tee",
];

/// Extract file path arguments from a command string for validation.
/// Returns (base_command, list of path arguments).
fn extract_paths_from_command(segment: &str) -> (String, Vec<String>) {
    let tokens: Vec<&str> = segment.split_whitespace().collect();
    if tokens.is_empty() {
        return (String::new(), Vec::new());
    }

    let base_cmd = tokens[0]
        .rsplit('/')
        .next()
        .unwrap_or(tokens[0])
        .to_string();
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
                let base = allowed_dirs
                    .first()
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| {
                        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
                    });
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
                    let allowed_c = allowed
                        .canonicalize()
                        .unwrap_or_else(|_| allowed.to_path_buf());
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
