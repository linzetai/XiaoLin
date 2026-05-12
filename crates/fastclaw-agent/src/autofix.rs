//! Auto-fix loop: detect compiler errors → parse → plan fix → inject guidance.
//!
//! When a `shell_exec` (or similar build tool) returns a non-zero exit code
//! with compiler/linter output, this module:
//!
//! 1. Parses the output into structured [`CompilerDiagnostic`]s
//! 2. Deduplicates and prioritizes the diagnostics
//! 3. Generates an [`AutoFixGuide`] with concrete instructions for the LLM
//!
//! The runtime injects the guide as a system-message annotation so the LLM
//! can immediately act on the errors without manual intervention.

use std::path::Path;

// ── Structured compiler diagnostic ──────────────────────────────────────

/// Severity of a compiler/linter diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[allow(dead_code)]
pub(crate) enum Severity {
    Error,
    Warning,
    Note,
    Help,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Error => write!(f, "error"),
            Self::Warning => write!(f, "warning"),
            Self::Note => write!(f, "note"),
            Self::Help => write!(f, "help"),
        }
    }
}

/// A single compiler/linter diagnostic parsed from build output.
#[derive(Debug, Clone)]
pub(crate) struct CompilerDiagnostic {
    pub file: String,
    pub line: usize,
    pub col: Option<usize>,
    pub severity: Severity,
    pub code: Option<String>,
    pub message: String,
    /// Compiler-suggested fix text (e.g. rustc "help: try ...", tsc quick fix).
    pub suggestion: Option<String>,
}

/// Which compiler/language produced the diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompilerKind {
    Rustc,
    Typescript,
    Gcc,
    Python,
    Go,
    Eslint,
    Unknown,
}

// ── Multi-language error parser ─────────────────────────────────────────

/// Detect the compiler kind from output heuristics and parse diagnostics.
pub(crate) fn parse_compiler_output(output: &str) -> (CompilerKind, Vec<CompilerDiagnostic>) {
    // Try each parser in priority order; the first one that produces results wins.
    if let Some(diags) = try_parse_rustc(output) {
        if !diags.is_empty() {
            return (CompilerKind::Rustc, diags);
        }
    }
    if let Some(diags) = try_parse_typescript(output) {
        if !diags.is_empty() {
            return (CompilerKind::Typescript, diags);
        }
    }
    if let Some(diags) = try_parse_eslint(output) {
        if !diags.is_empty() {
            return (CompilerKind::Eslint, diags);
        }
    }
    if let Some(diags) = try_parse_gcc(output) {
        if !diags.is_empty() {
            return (CompilerKind::Gcc, diags);
        }
    }
    if let Some(diags) = try_parse_python(output) {
        if !diags.is_empty() {
            return (CompilerKind::Python, diags);
        }
    }
    if let Some(diags) = try_parse_go(output) {
        if !diags.is_empty() {
            return (CompilerKind::Go, diags);
        }
    }

    (CompilerKind::Unknown, Vec::new())
}

// ── Rustc parser ────────────────────────────────────────────────────────

/// Matches: `error[E0308]: mismatched types`
///          `warning: unused variable`
///          ` --> src/main.rs:42:5`
///          `help: consider ...`
fn try_parse_rustc(output: &str) -> Option<Vec<CompilerDiagnostic>> {
    let lines: Vec<&str> = output.lines().collect();
    let mut diags = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // Match "error[E0308]: message" or "warning: message" or "error: message"
        let (severity, code, message) = if let Some(rest) = line.strip_prefix("error[") {
            let bracket_end = rest.find(']').unwrap_or(rest.len());
            let code = &rest[..bracket_end];
            let msg = rest[bracket_end..].strip_prefix("]: ").unwrap_or("");
            (Severity::Error, Some(code.to_string()), msg.to_string())
        } else if let Some(msg) = line.strip_prefix("error: ") {
            (Severity::Error, None, msg.to_string())
        } else if let Some(msg) = line.strip_prefix("warning: ") {
            (Severity::Warning, None, msg.to_string())
        } else {
            i += 1;
            continue;
        };

        // Look for " --> file:line:col" on the next few lines
        let mut file = String::new();
        let mut line_num = 0;
        let mut col = None;
        let mut suggestion = None;

        for candidate_line in &lines[(i + 1)..lines.len().min(i + 8)] {
            let candidate = candidate_line.trim();
            if file.is_empty() {
                if let Some(loc) = candidate.strip_prefix("--> ") {
                    if let Some((f, lc)) = parse_file_line_col(loc) {
                        file = f;
                        line_num = lc.0;
                        col = lc.1;
                    }
                }
            }
            if candidate.starts_with("help: ") || candidate.starts_with("= help: ") {
                let help_text = candidate
                    .strip_prefix("help: ")
                    .or_else(|| candidate.strip_prefix("= help: "))
                    .unwrap_or(candidate);
                suggestion = Some(help_text.to_string());
            }
        }

        if !file.is_empty() && line_num > 0 {
            diags.push(CompilerDiagnostic {
                file,
                line: line_num,
                col,
                severity,
                code,
                message,
                suggestion,
            });
        }

        i += 1;
    }

    if diags.is_empty() {
        None
    } else {
        Some(diags)
    }
}

// ── TypeScript / tsc parser ─────────────────────────────────────────────

/// Matches: `src/app.ts(42,5): error TS2304: Cannot find name 'foo'.`
///          `src/app.ts:42:5 - error TS2304: Cannot find name 'foo'.`
fn try_parse_typescript(output: &str) -> Option<Vec<CompilerDiagnostic>> {
    let mut diags = Vec::new();

    for line in output.lines() {
        // Format 1: file(line,col): error TSxxxx: message
        if let Some(d) = parse_tsc_paren_format(line) {
            diags.push(d);
            continue;
        }
        // Format 2: file:line:col - error TSxxxx: message
        if let Some(d) = parse_tsc_colon_format(line) {
            diags.push(d);
        }
    }

    if diags.is_empty() {
        None
    } else {
        Some(diags)
    }
}

fn parse_tsc_paren_format(line: &str) -> Option<CompilerDiagnostic> {
    // file(line,col): error TSxxxx: message
    let paren_start = line.find('(')?;
    let paren_end = line[paren_start..].find(')')? + paren_start;
    let file = &line[..paren_start];
    let coords = &line[paren_start + 1..paren_end];
    let rest = line[paren_end + 1..].trim().strip_prefix(": ")?;

    let (line_num, col) = parse_comma_coords(coords)?;
    let (severity, code, message) = parse_ts_severity_and_message(rest)?;

    Some(CompilerDiagnostic {
        file: file.to_string(),
        line: line_num,
        col: Some(col),
        severity,
        code: Some(code),
        message,
        suggestion: None,
    })
}

fn parse_tsc_colon_format(line: &str) -> Option<CompilerDiagnostic> {
    // file:line:col - error TSxxxx: message
    let dash_pos = line.find(" - ")?;
    let path_part = &line[..dash_pos];
    let rest = &line[dash_pos + 3..];

    let (file, (line_num, col_opt)) = parse_file_line_col(path_part)?;
    let (severity, code, message) = parse_ts_severity_and_message(rest)?;

    Some(CompilerDiagnostic {
        file,
        line: line_num,
        col: col_opt,
        severity,
        code: Some(code),
        message,
        suggestion: None,
    })
}

fn parse_ts_severity_and_message(text: &str) -> Option<(Severity, String, String)> {
    let (severity, rest) = if let Some(r) = text.strip_prefix("error ") {
        (Severity::Error, r)
    } else if let Some(r) = text.strip_prefix("warning ") {
        (Severity::Warning, r)
    } else {
        return None;
    };
    // TSxxxx: message
    let colon_pos = rest.find(": ")?;
    let code = rest[..colon_pos].to_string();
    let message = rest[colon_pos + 2..].to_string();
    Some((severity, code, message))
}

// ── GCC / Clang parser ──────────────────────────────────────────────────

/// Matches: `src/main.c:42:5: error: expected ';' before '}'`
fn try_parse_gcc(output: &str) -> Option<Vec<CompilerDiagnostic>> {
    let mut diags = Vec::new();

    for line in output.lines() {
        // file:line:col: severity: message
        let parts: Vec<&str> = line.splitn(5, ':').collect();
        if parts.len() < 5 {
            continue;
        }
        let file = parts[0].trim();
        let line_num: usize = match parts[1].trim().parse() {
            Ok(n) => n,
            Err(_) => continue,
        };
        let col: usize = match parts[2].trim().parse() {
            Ok(n) => n,
            Err(_) => continue,
        };
        let sev_str = parts[3].trim();
        let severity = match sev_str {
            "error" | "fatal error" => Severity::Error,
            "warning" => Severity::Warning,
            "note" => Severity::Note,
            _ => continue,
        };
        let message = parts[4].trim().to_string();

        if !file.is_empty() && looks_like_path(file) {
            diags.push(CompilerDiagnostic {
                file: file.to_string(),
                line: line_num,
                col: Some(col),
                severity,
                code: None,
                message,
                suggestion: None,
            });
        }
    }

    if diags.is_empty() {
        None
    } else {
        Some(diags)
    }
}

// ── Python parser ───────────────────────────────────────────────────────

/// Matches:
///   `File "src/app.py", line 42`
///   `SyntaxError: invalid syntax`
///   or pytest-style: `src/app.py:42: error: ...`
fn try_parse_python(output: &str) -> Option<Vec<CompilerDiagnostic>> {
    let lines: Vec<&str> = output.lines().collect();
    let mut diags = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        // Python traceback: File "path", line N
        if line.starts_with("File \"") {
            if let Some(d) = parse_python_traceback_line(line, &lines, i) {
                diags.push(d);
            }
        }
        // mypy/pyright style: path:line: error: message
        else if let Some(d) = parse_python_tool_line(line) {
            diags.push(d);
        }

        i += 1;
    }

    if diags.is_empty() {
        None
    } else {
        Some(diags)
    }
}

fn parse_python_traceback_line(
    line: &str,
    all_lines: &[&str],
    idx: usize,
) -> Option<CompilerDiagnostic> {
    // File "path/to/file.py", line 42
    let file_start = line.find('"')? + 1;
    let file_end = line[file_start..].find('"')? + file_start;
    let file = &line[file_start..file_end];

    let line_marker = ", line ";
    let line_pos = line.find(line_marker)?;
    let num_start = line_pos + line_marker.len();
    let num_str = line[num_start..]
        .split(|c: char| !c.is_ascii_digit())
        .next()?;
    let line_num: usize = num_str.parse().ok()?;

    // Next non-empty line after the code line is usually the error
    let mut message = String::new();
    for candidate_line in &all_lines[(idx + 1)..all_lines.len().min(idx + 4)] {
        let candidate = candidate_line.trim();
        if candidate.contains("Error:") || candidate.contains("Exception:") {
            message = candidate.to_string();
            break;
        }
    }
    if message.is_empty() {
        message = "Python error".to_string();
    }

    Some(CompilerDiagnostic {
        file: file.to_string(),
        line: line_num,
        col: None,
        severity: Severity::Error,
        code: None,
        message,
        suggestion: None,
    })
}

fn parse_python_tool_line(line: &str) -> Option<CompilerDiagnostic> {
    // mypy/pyright: path.py:42: error: message
    let parts: Vec<&str> = line.splitn(4, ':').collect();
    if parts.len() < 4 {
        return None;
    }
    let file = parts[0].trim();
    if !file.ends_with(".py") {
        return None;
    }
    let line_num: usize = parts[1].trim().parse().ok()?;
    let sev_str = parts[2].trim();
    let severity = match sev_str {
        "error" => Severity::Error,
        "warning" => Severity::Warning,
        "note" => Severity::Note,
        _ => return None,
    };
    let message = parts[3].trim().to_string();

    Some(CompilerDiagnostic {
        file: file.to_string(),
        line: line_num,
        col: None,
        severity,
        code: None,
        message,
        suggestion: None,
    })
}

// ── Go parser ───────────────────────────────────────────────────────────

/// Matches: `./main.go:42:5: undefined: foo`
fn try_parse_go(output: &str) -> Option<Vec<CompilerDiagnostic>> {
    let mut diags = Vec::new();

    for line in output.lines() {
        let parts: Vec<&str> = line.splitn(4, ':').collect();
        if parts.len() < 4 {
            continue;
        }
        let file = parts[0].trim();
        if !(file.ends_with(".go") || file.starts_with("./")) {
            continue;
        }
        let line_num: usize = match parts[1].trim().parse() {
            Ok(n) => n,
            Err(_) => continue,
        };
        let col: Option<usize> = parts[2].trim().parse().ok();
        let message = if col.is_some() && parts.len() > 3 {
            parts[3].trim().to_string()
        } else {
            parts[2..].join(":").trim().to_string()
        };

        if !message.is_empty() {
            diags.push(CompilerDiagnostic {
                file: file.to_string(),
                line: line_num,
                col,
                severity: Severity::Error,
                code: None,
                message,
                suggestion: None,
            });
        }
    }

    if diags.is_empty() {
        None
    } else {
        Some(diags)
    }
}

// ── ESLint parser ───────────────────────────────────────────────────────

/// Matches ESLint default formatter:
///   `/path/to/file.ts`
///   `  42:5  error  message  rule-name`
fn try_parse_eslint(output: &str) -> Option<Vec<CompilerDiagnostic>> {
    let lines: Vec<&str> = output.lines().collect();
    let mut diags = Vec::new();
    let mut current_file = String::new();

    for line in &lines {
        let trimmed = line.trim();

        // File header: an absolute or relative path with an extension
        if !trimmed.is_empty()
            && !trimmed.starts_with(char::is_whitespace)
            && looks_like_path(trimmed)
            && !trimmed.contains("  ")
        {
            current_file = trimmed.to_string();
            continue;
        }

        // Diagnostic line: "  line:col  severity  message  rule-name"
        if current_file.is_empty() {
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }
        let (line_num, col) = match parse_colon_pair(parts[0]) {
            Some(v) => v,
            None => continue,
        };
        let severity = match parts[1] {
            "error" => Severity::Error,
            "warning" => Severity::Warning,
            _ => continue,
        };
        // Everything between severity and the last token (rule name) is the message.
        let rule = if parts.len() > 3 {
            Some(parts[parts.len() - 1].to_string())
        } else {
            None
        };
        let msg_end = if rule.is_some() {
            parts.len() - 1
        } else {
            parts.len()
        };
        let message = parts[2..msg_end].join(" ");

        diags.push(CompilerDiagnostic {
            file: current_file.clone(),
            line: line_num,
            col: Some(col),
            severity,
            code: rule,
            message,
            suggestion: None,
        });
    }

    if diags.is_empty() {
        None
    } else {
        Some(diags)
    }
}

// ── Utility helpers ─────────────────────────────────────────────────────

/// Parse "file:line:col" into (file, (line, Some(col))) or "file:line" into (file, (line, None)).
fn parse_file_line_col(s: &str) -> Option<(String, (usize, Option<usize>))> {
    let parts: Vec<&str> = s.rsplitn(3, ':').collect();
    match parts.len() {
        // col:line:file (reversed)
        3 => {
            let col: usize = parts[0].trim().parse().ok()?;
            let line: usize = parts[1].trim().parse().ok()?;
            let file = parts[2].trim().to_string();
            Some((file, (line, Some(col))))
        }
        2 => {
            let line_or_col: usize = parts[0].trim().parse().ok()?;
            let file = parts[1].trim().to_string();
            Some((file, (line_or_col, None)))
        }
        _ => None,
    }
}

/// Parse "line,col" as used by tsc parenthesized format.
fn parse_comma_coords(s: &str) -> Option<(usize, usize)> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 2 {
        return None;
    }
    let line: usize = parts[0].trim().parse().ok()?;
    let col: usize = parts[1].trim().parse().ok()?;
    Some((line, col))
}

/// Parse "line:col" into (line, col).
fn parse_colon_pair(s: &str) -> Option<(usize, usize)> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let a: usize = parts[0].parse().ok()?;
    let b: usize = parts[1].parse().ok()?;
    Some((a, b))
}

/// Heuristic: does this string look like a file path?
fn looks_like_path(s: &str) -> bool {
    (s.contains('/') || s.contains('\\') || s.contains('.')) && !s.contains(' ') && s.len() < 300
}

// ── Auto-fix guidance generator ─────────────────────────────────────────

/// Maximum number of diagnostics to include in fix guidance.
const MAX_DIAG_IN_GUIDE: usize = 10;

/// Maximum auto-fix loop iterations to prevent infinite cycles.
pub(crate) const MAX_AUTOFIX_ITERATIONS: u32 = 5;

/// Structured auto-fix guidance to inject into the LLM system prompt.
#[derive(Debug, Clone)]
pub(crate) struct AutoFixGuide {
    pub compiler: CompilerKind,
    pub diagnostics: Vec<CompilerDiagnostic>,
    pub iteration: u32,
    pub formatted: String,
}

/// Detect if shell output contains compiler errors and generate fix guidance.
///
/// Returns `Some(guide)` when actionable errors are found, `None` otherwise.
pub(crate) fn detect_and_plan(
    command: &str,
    output: &str,
    exit_code: i32,
    iteration: u32,
) -> Option<AutoFixGuide> {
    if exit_code == 0 {
        return None;
    }
    if iteration >= MAX_AUTOFIX_ITERATIONS {
        return None;
    }
    if !is_likely_build_command(command) {
        return None;
    }

    let (compiler, mut diags) = parse_compiler_output(output);
    if diags.is_empty() {
        return None;
    }

    // Prioritize: errors first, then warnings. Deduplicate by (file, line).
    diags.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
    });
    dedup_diagnostics(&mut diags);

    let error_count = diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    let warning_count = diags
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .count();

    let display_diags: Vec<&CompilerDiagnostic> = diags.iter().take(MAX_DIAG_IN_GUIDE).collect();

    let mut formatted = String::new();
    formatted.push_str(&format!(
        "\n\n─── AUTO-FIX ({} compiler, iteration {}/{}) ───\n",
        compiler_name(compiler),
        iteration + 1,
        MAX_AUTOFIX_ITERATIONS
    ));
    formatted.push_str(&format!(
        "Build failed: {error_count} error(s), {warning_count} warning(s).\n\n"
    ));

    // Group by file for efficient reading
    let mut by_file: std::collections::BTreeMap<&str, Vec<&CompilerDiagnostic>> =
        std::collections::BTreeMap::new();
    for d in &display_diags {
        by_file.entry(&d.file).or_default().push(d);
    }

    formatted.push_str("FIX PLAN (execute in order):\n");
    let mut step = 1;

    for (file, file_diags) in &by_file {
        let line_ranges: Vec<String> = file_diags
            .iter()
            .map(|d| {
                let start = d.line.saturating_sub(3);
                let end = d.line + 3;
                format!("{start}-{end}")
            })
            .collect();

        formatted.push_str(&format!(
            "\n  Step {step}: Read `{file}` around lines {}\n",
            line_ranges.join(", ")
        ));
        step += 1;

        for d in file_diags {
            let loc = match d.col {
                Some(c) => format!("{}:{c}", d.line),
                None => format!("{}", d.line),
            };
            let code_tag = d
                .code
                .as_deref()
                .map(|c| format!(" [{c}]"))
                .unwrap_or_default();
            formatted.push_str(&format!(
                "    → L{loc}: {}{code_tag}: {}\n",
                d.severity, d.message
            ));
            if let Some(ref sug) = d.suggestion {
                formatted.push_str(&format!("      hint: {sug}\n"));
            }
        }

        formatted.push_str(&format!("  Step {step}: Apply fix with edit_file\n"));
        step += 1;
    }

    formatted.push_str(&format!(
        "\n  Step {step}: Re-run the build command: `{command}`\n"
    ));

    if diags.len() > MAX_DIAG_IN_GUIDE {
        formatted.push_str(&format!(
            "\n  [+{} more diagnostics not shown — fix the above first]\n",
            diags.len() - MAX_DIAG_IN_GUIDE
        ));
    }

    formatted.push_str("\nINSTRUCTIONS:\n");
    formatted.push_str("- Fix ALL errors shown above before re-running the build.\n");
    formatted.push_str("- Read the exact lines indicated, apply the fix, then rebuild.\n");
    formatted.push_str("- If a compiler hint is provided, prefer using it.\n");
    formatted.push_str("- Do NOT explain or ask — just fix and rebuild.\n");
    formatted.push_str("───────────────────────────────────────────\n");

    Some(AutoFixGuide {
        compiler,
        diagnostics: diags,
        iteration,
        formatted,
    })
}

/// Heuristic: is this command likely a build/compile/lint invocation?
fn is_likely_build_command(command: &str) -> bool {
    let lower = command.to_lowercase();
    let build_patterns = [
        "cargo check",
        "cargo build",
        "cargo test",
        "cargo clippy",
        "rustc ",
        "tsc ",
        "tsc\n",
        "npx tsc",
        "npm run build",
        "npm run lint",
        "npm test",
        "yarn build",
        "yarn lint",
        "yarn test",
        "pnpm build",
        "pnpm lint",
        "pnpm test",
        "bun build",
        "bun test",
        "eslint ",
        "gcc ",
        "g++ ",
        "clang ",
        "clang++ ",
        "make ",
        "cmake ",
        "python ",
        "python3 ",
        "pytest",
        "mypy ",
        "pyright ",
        "go build",
        "go test",
        "go vet",
        "javac ",
        "gradle ",
        "mvn ",
        "dotnet build",
        "dotnet test",
        "swift build",
        "swiftc ",
    ];
    build_patterns.iter().any(|p| lower.contains(p))
}

pub(crate) fn compiler_name(kind: CompilerKind) -> &'static str {
    match kind {
        CompilerKind::Rustc => "rustc",
        CompilerKind::Typescript => "tsc",
        CompilerKind::Gcc => "gcc/clang",
        CompilerKind::Python => "python",
        CompilerKind::Go => "go",
        CompilerKind::Eslint => "eslint",
        CompilerKind::Unknown => "unknown",
    }
}

/// Remove duplicate diagnostics at the same (file, line) location,
/// keeping the first (highest severity) occurrence.
fn dedup_diagnostics(diags: &mut Vec<CompilerDiagnostic>) {
    let mut seen = std::collections::HashSet::new();
    diags.retain(|d| seen.insert((d.file.clone(), d.line)));
}

// ── Auto-fix loop state ─────────────────────────────────────────────────

/// Tracks the auto-fix loop state across agent iterations.
#[derive(Debug, Default)]
pub(crate) struct AutoFixState {
    /// Current iteration count within the auto-fix loop.
    pub iteration: u32,
    /// The last build command that was executed (for re-running).
    pub last_build_command: Option<String>,
    /// Number of errors in the previous iteration (to detect progress).
    pub prev_error_count: usize,
    /// Whether the last build succeeded (exit_code == 0).
    pub last_build_succeeded: bool,
}

impl AutoFixState {
    pub fn reset(&mut self) {
        self.iteration = 0;
        self.last_build_command = None;
        self.prev_error_count = 0;
        self.last_build_succeeded = false;
    }

    /// Check if we should continue the auto-fix loop.
    #[allow(dead_code)]
    pub fn should_continue(&self) -> bool {
        !self.last_build_succeeded && self.iteration < MAX_AUTOFIX_ITERATIONS
    }

    /// Update state after a build attempt.
    pub fn record_build_result(&mut self, command: &str, exit_code: i32, error_count: usize) {
        self.last_build_command = Some(command.to_string());
        self.last_build_succeeded = exit_code == 0;
        if exit_code != 0 {
            self.prev_error_count = error_count;
            self.iteration += 1;
        } else {
            self.iteration = 0;
            self.prev_error_count = 0;
        }
    }
}

// ── Integration helper: detect build commands in tool results ───────────

/// Check if a shell_exec tool call looks like a build command based on its arguments.
pub(crate) fn extract_build_command(tool_name: &str, arguments: &str) -> Option<String> {
    if tool_name != "shell_exec" && tool_name != "shell" && tool_name != "run_command" {
        return None;
    }
    let args: serde_json::Value = serde_json::from_str(arguments).ok()?;
    let command = args.get("command")?.as_str()?;
    if is_likely_build_command(command) {
        Some(command.to_string())
    } else {
        None
    }
}

/// Extract the exit code from a shell_exec result output.
#[allow(dead_code)]
pub(crate) fn extract_exit_code(output: &str) -> Option<i32> {
    // Terminal-file compact summary format: "exit_code=N, ..."
    if let Some(rest) = output.strip_prefix("exit_code=") {
        let num_str = rest
            .split(|c: char| !c.is_ascii_digit() && c != '-')
            .next()?;
        return num_str.parse().ok();
    }
    // JSON format: {"exit_code": N, ...}
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(output) {
        return val
            .get("exit_code")
            .and_then(|v| v.as_i64())
            .map(|n| n as i32);
    }
    // Look for "exit_code" or "Exit code" anywhere in the output
    for line in output.lines() {
        let lower = line.to_lowercase();
        if lower.contains("exit_code=") || lower.contains("exit code:") {
            let nums: String = line
                .chars()
                .skip_while(|c| !c.is_ascii_digit() && *c != '-')
                .take_while(|c| c.is_ascii_digit() || *c == '-')
                .collect();
            if let Ok(n) = nums.parse::<i32>() {
                return Some(n);
            }
        }
    }
    None
}

/// Check if a file path looks like it's within the workspace (relative or under cwd).
#[allow(dead_code)]
pub(crate) fn normalize_diag_path(file: &str, workspace_root: &Path) -> std::path::PathBuf {
    let p = Path::new(file);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace_root.join(p)
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rustc_error_with_code() {
        let output = r#"error[E0308]: mismatched types
 --> src/main.rs:42:5
  |
42 |     let x: i32 = "hello";
  |                  ^^^^^^^ expected `i32`, found `&str`

error: aborting due to previous error
"#;
        let (kind, diags) = parse_compiler_output(output);
        assert_eq!(kind, CompilerKind::Rustc);
        assert!(!diags.is_empty(), "should parse rustc errors");
        assert_eq!(diags[0].file, "src/main.rs");
        assert_eq!(diags[0].line, 42);
        assert_eq!(diags[0].col, Some(5));
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].code.as_deref(), Some("E0308"));
        assert!(diags[0].message.contains("mismatched types"));
    }

    #[test]
    fn parse_rustc_warning_with_help() {
        let output = r#"warning: unused variable: `x`
 --> src/lib.rs:10:9
  |
10 |     let x = 42;
  |         ^ help: if this is intentional, prefix it with an underscore: `_x`
  |
  = help: consider using `_x` instead
"#;
        let (kind, diags) = parse_compiler_output(output);
        assert_eq!(kind, CompilerKind::Rustc);
        assert!(!diags.is_empty());
        assert_eq!(diags[0].severity, Severity::Warning);
        assert!(
            diags[0].suggestion.is_some(),
            "should extract help suggestion"
        );
    }

    #[test]
    fn parse_tsc_paren_format() {
        let output = "src/app.ts(42,5): error TS2304: Cannot find name 'foo'.";
        let (kind, diags) = parse_compiler_output(output);
        assert_eq!(kind, CompilerKind::Typescript);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].file, "src/app.ts");
        assert_eq!(diags[0].line, 42);
        assert_eq!(diags[0].col, Some(5));
        assert_eq!(diags[0].code.as_deref(), Some("TS2304"));
    }

    #[test]
    fn parse_tsc_colon_format() {
        let output = "src/index.ts:10:3 - error TS2551: Property 'nme' does not exist.";
        let (kind, diags) = parse_compiler_output(output);
        assert_eq!(kind, CompilerKind::Typescript);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].line, 10);
    }

    #[test]
    fn parse_gcc_error() {
        let output = "src/main.c:42:5: error: expected ';' before '}'";
        let (kind, diags) = parse_compiler_output(output);
        assert_eq!(kind, CompilerKind::Gcc);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].file, "src/main.c");
        assert_eq!(diags[0].line, 42);
    }

    #[test]
    fn parse_python_traceback() {
        let output = r#"Traceback (most recent call last):
  File "src/app.py", line 42, in main
    result = compute(x)
NameError: name 'compute' is not defined
"#;
        let (kind, diags) = parse_compiler_output(output);
        assert_eq!(kind, CompilerKind::Python);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].file, "src/app.py");
        assert_eq!(diags[0].line, 42);
        assert!(diags[0].message.contains("NameError"));
    }

    #[test]
    fn parse_python_mypy_error() {
        let output = "src/main.py:15: error: Incompatible return value type";
        let (kind, diags) = parse_compiler_output(output);
        assert_eq!(kind, CompilerKind::Python);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].line, 15);
    }

    #[test]
    fn parse_go_error() {
        let output = "./main.go:42:5: undefined: foo";
        let (kind, diags) = parse_compiler_output(output);
        assert_eq!(kind, CompilerKind::Go);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].file, "./main.go");
        assert_eq!(diags[0].line, 42);
    }

    #[test]
    fn parse_eslint_output() {
        let output = r#"/home/user/project/src/app.ts
  10:5  error  Unexpected console statement  no-console
  25:1  warning  Missing return type  @typescript-eslint/explicit-function-return-type
"#;
        let (kind, diags) = parse_compiler_output(output);
        assert_eq!(kind, CompilerKind::Eslint);
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].line, 10);
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[1].severity, Severity::Warning);
    }

    #[test]
    fn detect_and_plan_produces_guide() {
        let output = r#"error[E0308]: mismatched types
 --> src/main.rs:42:5
  |
42 |     let x: i32 = "hello";
  |                  ^^^^^^^ expected `i32`, found `&str`
"#;
        let guide = detect_and_plan("cargo check", output, 1, 0);
        assert!(guide.is_some(), "should produce fix guide");
        let guide = guide.unwrap();
        assert_eq!(guide.compiler, CompilerKind::Rustc);
        assert!(!guide.diagnostics.is_empty());
        assert!(guide.formatted.contains("AUTO-FIX"));
        assert!(guide.formatted.contains("src/main.rs"));
        assert!(guide.formatted.contains("cargo check"));
    }

    #[test]
    fn detect_and_plan_skips_success() {
        assert!(detect_and_plan("cargo check", "Compiling...\nFinished", 0, 0).is_none());
    }

    #[test]
    fn detect_and_plan_skips_non_build_commands() {
        let output = "error: something went wrong";
        assert!(detect_and_plan("ls -la", output, 1, 0).is_none());
    }

    #[test]
    fn detect_and_plan_respects_max_iterations() {
        let output = "error[E0308]: x\n --> src/main.rs:1:1";
        assert!(detect_and_plan("cargo check", output, 1, MAX_AUTOFIX_ITERATIONS).is_none());
    }

    #[test]
    fn is_likely_build_command_matches() {
        assert!(is_likely_build_command("cargo check"));
        assert!(is_likely_build_command("cargo build --release"));
        assert!(is_likely_build_command("npm run build"));
        assert!(is_likely_build_command("npx tsc --noEmit"));
        assert!(is_likely_build_command("gcc -o main main.c"));
        assert!(is_likely_build_command("go build ./..."));
        assert!(is_likely_build_command("python -m pytest"));
        assert!(is_likely_build_command("eslint src/"));
        assert!(!is_likely_build_command("ls -la"));
        assert!(!is_likely_build_command("cat file.txt"));
    }

    #[test]
    fn extract_build_command_from_shell_exec() {
        let args = r#"{"command": "cargo check", "is_background": false}"#;
        assert_eq!(
            extract_build_command("shell_exec", args),
            Some("cargo check".into())
        );
        assert!(extract_build_command("shell_exec", r#"{"command": "ls"}"#).is_none());
        assert!(extract_build_command("read_file", args).is_none());
    }

    #[test]
    fn autofix_state_tracks_iterations() {
        let mut state = AutoFixState::default();
        assert!(state.should_continue()); // last_build_succeeded is false initially

        state.record_build_result("cargo check", 1, 3);
        assert!(state.should_continue());
        assert_eq!(state.iteration, 1);

        state.record_build_result("cargo check", 0, 0);
        assert!(!state.should_continue());
        assert_eq!(state.iteration, 0);
    }

    #[test]
    fn autofix_state_caps_iterations() {
        let mut state = AutoFixState::default();
        for i in 0..MAX_AUTOFIX_ITERATIONS {
            state.record_build_result("cargo check", 1, 3 - (i as usize).min(2));
        }
        assert!(!state.should_continue());
    }

    #[test]
    fn dedup_diagnostics_removes_duplicates() {
        let mut diags = vec![
            CompilerDiagnostic {
                file: "a.rs".into(),
                line: 10,
                col: None,
                severity: Severity::Error,
                code: None,
                message: "first".into(),
                suggestion: None,
            },
            CompilerDiagnostic {
                file: "a.rs".into(),
                line: 10,
                col: Some(5),
                severity: Severity::Warning,
                code: None,
                message: "duplicate".into(),
                suggestion: None,
            },
            CompilerDiagnostic {
                file: "b.rs".into(),
                line: 10,
                col: None,
                severity: Severity::Error,
                code: None,
                message: "different file".into(),
                suggestion: None,
            },
        ];
        dedup_diagnostics(&mut diags);
        assert_eq!(diags.len(), 2, "should dedup same file+line");
    }

    #[test]
    fn extract_exit_code_from_various_formats() {
        assert_eq!(extract_exit_code("exit_code=1, 42 lines"), Some(1));
        assert_eq!(extract_exit_code(r#"{"exit_code": 2}"#), Some(2));
        assert_eq!(extract_exit_code("something exit_code=127 end"), Some(127));
        assert_eq!(extract_exit_code("no code here"), None);
    }

    #[test]
    fn normalize_diag_path_relative() {
        let root = Path::new("/workspace");
        let p = normalize_diag_path("src/main.rs", root);
        assert_eq!(p, Path::new("/workspace/src/main.rs"));
    }

    #[test]
    fn normalize_diag_path_absolute() {
        let root = Path::new("/workspace");
        let p = normalize_diag_path("/other/main.rs", root);
        assert_eq!(p, Path::new("/other/main.rs"));
    }

    #[test]
    fn guide_contains_all_steps() {
        let output = r#"error[E0599]: no method named `foo`
 --> src/lib.rs:100:5
  |
  = help: consider importing `Foo`

error[E0308]: wrong type
 --> src/main.rs:50:10
"#;
        let guide = detect_and_plan("cargo check", output, 1, 0).unwrap();
        assert!(guide.formatted.contains("Step 1"));
        assert!(guide.formatted.contains("src/lib.rs"));
        assert!(guide.formatted.contains("src/main.rs"));
        assert!(guide.formatted.contains("cargo check"));
        assert!(guide.formatted.contains("Re-run the build"));
    }
}
