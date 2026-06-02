//! Post-execution validation pipeline for tool results.
//!
//! After a tool executes, the validation pipeline runs registered validators
//! to check the output quality. Validation results are appended to the tool
//! result so the LLM sees them in the next turn, enabling immediate correction.
//!
//! This is the framework for Plan proposals A1 (output quality checks) and
//! C1 (scenario-based auto-validation). Validators are selected based on
//! tool name, file extension, or output characteristics.

use std::path::Path;

/// Severity of a validation finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationSeverity {
    /// Informational hint, not blocking.
    Info,
    /// Potential issue that the LLM should address.
    Warning,
    /// Definite error that must be fixed.
    Error,
}

/// A single validation finding.
#[derive(Debug, Clone)]
pub struct ValidationFinding {
    pub severity: ValidationSeverity,
    pub message: String,
    /// Optional file path relevant to the finding.
    pub file: Option<String>,
    /// Optional line number.
    pub line: Option<u32>,
}

/// Aggregated result of running the validation pipeline on a tool result.
#[derive(Debug, Clone, Default)]
pub struct ValidationResult {
    pub findings: Vec<ValidationFinding>,
}

impl ValidationResult {
    pub fn is_empty(&self) -> bool {
        self.findings.is_empty()
    }

    pub fn has_errors(&self) -> bool {
        self.findings
            .iter()
            .any(|f| f.severity == ValidationSeverity::Error)
    }

    pub fn error_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == ValidationSeverity::Error)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == ValidationSeverity::Warning)
            .count()
    }

    /// Format findings as a block to append to tool result output.
    pub fn format_for_injection(&self) -> Option<String> {
        if self.findings.is_empty() {
            return None;
        }

        let mut out = String::with_capacity(256);
        out.push_str("\n── Validation ──────────────────────────────────\n");

        for finding in &self.findings {
            let icon = match finding.severity {
                ValidationSeverity::Error => "❌",
                ValidationSeverity::Warning => "⚠️",
                ValidationSeverity::Info => "ℹ️",
            };

            if let (Some(file), Some(line)) = (&finding.file, finding.line) {
                out.push_str(&format!(
                    "{} {}:{} — {}\n",
                    icon, file, line, finding.message
                ));
            } else if let Some(file) = &finding.file {
                out.push_str(&format!("{} {} — {}\n", icon, file, finding.message));
            } else {
                out.push_str(&format!("{} {}\n", icon, finding.message));
            }
        }

        out.push_str("────────────────────────────────────────────────\n");
        Some(out)
    }
}

/// Context provided to validators.
pub struct ValidationContext<'a> {
    pub tool_name: &'a str,
    pub arguments: &'a str,
    pub output: &'a str,
    pub success: bool,
    pub work_dir: &'a Path,
}

/// Trait for implementing validators.
pub trait Validator: Send + Sync {
    /// Whether this validator should run for the given context.
    fn applies_to(&self, ctx: &ValidationContext<'_>) -> bool;

    /// Run validation and return findings.
    fn validate(&self, ctx: &ValidationContext<'_>) -> Vec<ValidationFinding>;
}

/// The main pipeline that runs registered validators.
pub struct ValidationPipeline {
    validators: Vec<Box<dyn Validator>>,
}

impl ValidationPipeline {
    pub fn new() -> Self {
        Self {
            validators: Vec::new(),
        }
    }

    /// Create a pipeline with the default set of built-in validators.
    pub fn with_defaults() -> Self {
        let mut pipeline = Self::new();
        pipeline.register(Box::new(ShellSafetyValidator));
        pipeline.register(Box::new(FileOutputValidator));
        pipeline.register(Box::new(WebFetchValidator));
        pipeline.register(Box::new(DataOutputValidator));
        pipeline
    }

    pub fn register(&mut self, validator: Box<dyn Validator>) {
        self.validators.push(validator);
    }

    /// Run all applicable validators against the given tool execution context.
    pub fn validate(&self, ctx: &ValidationContext<'_>) -> ValidationResult {
        let mut result = ValidationResult::default();
        for validator in &self.validators {
            if validator.applies_to(ctx) {
                let findings = validator.validate(ctx);
                result.findings.extend(findings);
            }
        }
        result
    }
}

impl Default for ValidationPipeline {
    fn default() -> Self {
        Self::with_defaults()
    }
}

// ─── Built-in Validators ─────────────────────────────────────────────────

/// Checks shell commands for dangerous patterns.
pub struct ShellSafetyValidator;

impl Validator for ShellSafetyValidator {
    fn applies_to(&self, ctx: &ValidationContext<'_>) -> bool {
        ctx.tool_name == "shell_exec" || ctx.tool_name == "bash"
    }

    fn validate(&self, ctx: &ValidationContext<'_>) -> Vec<ValidationFinding> {
        let mut findings = Vec::new();
        let cmd = extract_command_from_args(ctx.arguments);

        for pattern in DANGEROUS_PATTERNS {
            if cmd.contains(pattern.pattern) {
                findings.push(ValidationFinding {
                    severity: pattern.severity,
                    message: pattern.message.to_string(),
                    file: None,
                    line: None,
                });
            }
        }

        if !ctx.success && ctx.output.contains("command not found") {
            if let Some(suggestion) = suggest_command_fix(&cmd, ctx.output) {
                findings.push(ValidationFinding {
                    severity: ValidationSeverity::Info,
                    message: suggestion,
                    file: None,
                    line: None,
                });
            }
        }

        findings
    }
}

/// Validates file write/edit operations.
pub struct FileOutputValidator;

impl Validator for FileOutputValidator {
    fn applies_to(&self, ctx: &ValidationContext<'_>) -> bool {
        matches!(
            ctx.tool_name,
            "write_file" | "edit_file" | "create_file" | "str_replace_editor"
        )
    }

    fn validate(&self, ctx: &ValidationContext<'_>) -> Vec<ValidationFinding> {
        let mut findings = Vec::new();

        if !ctx.success {
            return findings;
        }

        if let Some(file_path) = extract_file_path_from_args(ctx.arguments) {
            if file_path.ends_with(".json") && ctx.output.contains("written") {
                if let Some(content) = extract_written_content(ctx.arguments) {
                    if serde_json::from_str::<serde_json::Value>(&content).is_err() {
                        findings.push(ValidationFinding {
                            severity: ValidationSeverity::Error,
                            message: "Written JSON content is malformed".into(),
                            file: Some(file_path.to_string()),
                            line: None,
                        });
                    }
                }
            }

            if is_empty_write(ctx.arguments) {
                findings.push(ValidationFinding {
                    severity: ValidationSeverity::Warning,
                    message: "File was written with empty content".into(),
                    file: Some(file_path.to_string()),
                    line: None,
                });
            }
        }

        findings
    }
}

/// Validates web fetch results for completeness.
pub struct WebFetchValidator;

impl Validator for WebFetchValidator {
    fn applies_to(&self, ctx: &ValidationContext<'_>) -> bool {
        ctx.tool_name == "web_fetch" || ctx.tool_name == "web_search"
    }

    fn validate(&self, ctx: &ValidationContext<'_>) -> Vec<ValidationFinding> {
        let mut findings = Vec::new();

        if ctx.success && ctx.tool_name == "web_fetch" {
            let output_lower = ctx.output.to_lowercase();
            if output_lower.contains("403 forbidden")
                || output_lower.contains("access denied")
                || output_lower.contains("captcha")
                || output_lower.contains("please enable javascript")
            {
                findings.push(ValidationFinding {
                    severity: ValidationSeverity::Warning,
                    message: "Page appears to block automated access (anti-bot/captcha detected)"
                        .into(),
                    file: None,
                    line: None,
                });
            }

            if ctx.output.len() < 100 && !ctx.output.trim().is_empty() {
                findings.push(ValidationFinding {
                    severity: ValidationSeverity::Info,
                    message: "Fetched content is very short — page may not have loaded fully"
                        .into(),
                    file: None,
                    line: None,
                });
            }
        }

        if ctx.success && ctx.tool_name == "web_search" && ctx.output.trim().is_empty() {
            findings.push(ValidationFinding {
                severity: ValidationSeverity::Warning,
                message: "Search returned no results — try different keywords".into(),
                file: None,
                line: None,
            });
        }

        findings
    }
}

/// Validates data processing outputs for sanity.
pub struct DataOutputValidator;

impl Validator for DataOutputValidator {
    fn applies_to(&self, ctx: &ValidationContext<'_>) -> bool {
        if ctx.tool_name != "shell_exec" && ctx.tool_name != "bash" {
            return false;
        }
        let cmd = extract_command_from_args(ctx.arguments);
        cmd.contains("python")
            || cmd.contains("node")
            || cmd.contains("awk")
            || cmd.contains("jq")
            || cmd.contains("csvtool")
            || cmd.contains("wc")
    }

    fn validate(&self, ctx: &ValidationContext<'_>) -> Vec<ValidationFinding> {
        let mut findings = Vec::new();

        if ctx.success && ctx.output.trim().is_empty() {
            findings.push(ValidationFinding {
                severity: ValidationSeverity::Warning,
                message: "Command produced no output — check if filters are too restrictive".into(),
                file: None,
                line: None,
            });
        }

        if ctx.success && ctx.output.lines().count() == 1 {
            let line = ctx.output.trim();
            if line == "0" || line == "0.0" || line == "nan" || line == "NaN" {
                findings.push(ValidationFinding {
                    severity: ValidationSeverity::Info,
                    message: format!("Output is '{}' — verify this is expected", line),
                    file: None,
                    line: None,
                });
            }
        }

        findings
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────

struct DangerousPattern {
    pattern: &'static str,
    severity: ValidationSeverity,
    message: &'static str,
}

const DANGEROUS_PATTERNS: &[DangerousPattern] = &[
    DangerousPattern {
        pattern: "rm -rf /",
        severity: ValidationSeverity::Error,
        message: "DANGEROUS: rm -rf / will delete the entire filesystem",
    },
    DangerousPattern {
        pattern: "rm -rf /*",
        severity: ValidationSeverity::Error,
        message: "DANGEROUS: rm -rf /* will delete all top-level directories",
    },
    DangerousPattern {
        pattern: "git push --force",
        severity: ValidationSeverity::Warning,
        message: "Force push will overwrite remote history — use with caution",
    },
    DangerousPattern {
        pattern: "git push -f",
        severity: ValidationSeverity::Warning,
        message: "Force push will overwrite remote history — use with caution",
    },
    DangerousPattern {
        pattern: "DROP TABLE",
        severity: ValidationSeverity::Warning,
        message: "DROP TABLE will permanently delete data",
    },
    DangerousPattern {
        pattern: "DROP DATABASE",
        severity: ValidationSeverity::Error,
        message: "DROP DATABASE will permanently delete the entire database",
    },
    DangerousPattern {
        pattern: "chmod 777",
        severity: ValidationSeverity::Warning,
        message: "chmod 777 grants world-readable/writable access — security risk",
    },
    DangerousPattern {
        pattern: "curl | sh",
        severity: ValidationSeverity::Warning,
        message: "Piping curl to shell executes unverified code",
    },
    DangerousPattern {
        pattern: "curl | bash",
        severity: ValidationSeverity::Warning,
        message: "Piping curl to bash executes unverified code",
    },
];

fn extract_command_from_args(arguments: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(arguments) {
        v.get("command")
            .or_else(|| v.get("cmd"))
            .and_then(|c| c.as_str())
            .unwrap_or(arguments)
            .to_string()
    } else {
        arguments.to_string()
    }
}

fn extract_file_path_from_args(arguments: &str) -> Option<&str> {
    // Try JSON parsing first
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(arguments) {
        if let Some(path) = v.get("path").and_then(|p| p.as_str()) {
            // Return a static ref is not possible, but we can use the raw JSON slice
            // For simplicity, just search for the path string in the original
            if let Some(start) = arguments.find(path) {
                return Some(&arguments[start..start + path.len()]);
            }
        }
    }
    None
}

fn extract_written_content(arguments: &str) -> Option<String> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(arguments) {
        v.get("content")
            .or_else(|| v.get("contents"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string())
    } else {
        None
    }
}

fn is_empty_write(arguments: &str) -> bool {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(arguments) {
        v.get("content")
            .or_else(|| v.get("contents"))
            .and_then(|c| c.as_str())
            .map(|s| s.trim().is_empty())
            .unwrap_or(false)
    } else {
        false
    }
}

fn suggest_command_fix(cmd: &str, output: &str) -> Option<String> {
    let lower = output.to_lowercase();
    if lower.contains("python: command not found") || lower.contains("python3: command not found") {
        return Some("Try 'python3' instead of 'python', or check if Python is installed".into());
    }
    if lower.contains("node: command not found") {
        return Some("Node.js is not installed or not in PATH".into());
    }
    if lower.contains("cargo: command not found") {
        return Some("Rust/Cargo is not installed — run 'curl --proto =https --tlsv1.2 -sSf https://sh.rustup.rs | sh'".into());
    }
    if cmd.contains("npm") && lower.contains("command not found") {
        return Some("npm is not installed — install Node.js first".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    static TEST_WORK_DIR: std::sync::LazyLock<PathBuf> =
        std::sync::LazyLock::new(|| PathBuf::from("/tmp"));

    fn test_ctx<'a>(
        tool_name: &'a str,
        arguments: &'a str,
        output: &'a str,
        success: bool,
    ) -> ValidationContext<'a> {
        ValidationContext {
            tool_name,
            arguments,
            output,
            success,
            work_dir: &TEST_WORK_DIR,
        }
    }

    #[test]
    fn shell_safety_detects_rm_rf() {
        let v = ShellSafetyValidator;
        let ctx = test_ctx("shell_exec", r#"{"command": "rm -rf /"}"#, "", true);
        let findings = v.validate(&ctx);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, ValidationSeverity::Error);
        assert!(findings[0].message.contains("DANGEROUS"));
    }

    #[test]
    fn shell_safety_detects_force_push() {
        let v = ShellSafetyValidator;
        let ctx = test_ctx(
            "shell_exec",
            r#"{"command": "git push --force origin main"}"#,
            "",
            true,
        );
        let findings = v.validate(&ctx);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, ValidationSeverity::Warning);
    }

    #[test]
    fn shell_safety_ignores_safe_commands() {
        let v = ShellSafetyValidator;
        let ctx = test_ctx(
            "shell_exec",
            r#"{"command": "cargo test --release"}"#,
            "test result: ok",
            true,
        );
        let findings = v.validate(&ctx);
        assert!(findings.is_empty());
    }

    #[test]
    fn file_validator_detects_empty_write() {
        let v = FileOutputValidator;
        let ctx = test_ctx(
            "write_file",
            r#"{"path": "/tmp/test.txt", "content": "  "}"#,
            "written",
            true,
        );
        let findings = v.validate(&ctx);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, ValidationSeverity::Warning);
        assert!(findings[0].message.contains("empty"));
    }

    #[test]
    fn file_validator_detects_bad_json() {
        let v = FileOutputValidator;
        let ctx = test_ctx(
            "write_file",
            r#"{"path": "/tmp/config.json", "content": "{invalid json"}"#,
            "written",
            true,
        );
        let findings = v.validate(&ctx);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, ValidationSeverity::Error);
        assert!(findings[0].message.contains("JSON"));
    }

    #[test]
    fn web_fetch_detects_blocked_page() {
        let v = WebFetchValidator;
        let long_body = format!(
            "403 Forbidden - Access Denied. {}",
            "You do not have permission to access this resource. ".repeat(3)
        );
        let ctx = test_ctx(
            "web_fetch",
            r#"{"url": "https://example.com"}"#,
            &long_body,
            true,
        );
        let findings = v.validate(&ctx);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("anti-bot"));
    }

    #[test]
    fn data_validator_warns_on_empty_output() {
        let v = DataOutputValidator;
        let ctx = test_ctx(
            "shell_exec",
            r#"{"command": "python3 analyze.py"}"#,
            "",
            true,
        );
        let findings = v.validate(&ctx);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("no output"));
    }

    #[test]
    fn pipeline_runs_multiple_validators() {
        let pipeline = ValidationPipeline::with_defaults();
        let ctx = test_ctx(
            "shell_exec",
            r#"{"command": "rm -rf / && python3 script.py"}"#,
            "",
            true,
        );
        let result = pipeline.validate(&ctx);
        // ShellSafetyValidator finds rm -rf, DataOutputValidator finds empty output from python
        assert!(result.has_errors());
    }

    #[test]
    fn format_for_injection_empty_returns_none() {
        let result = ValidationResult::default();
        assert!(result.format_for_injection().is_none());
    }

    #[test]
    fn format_for_injection_produces_readable_output() {
        let result = ValidationResult {
            findings: vec![
                ValidationFinding {
                    severity: ValidationSeverity::Error,
                    message: "Syntax error".into(),
                    file: Some("src/main.rs".into()),
                    line: Some(42),
                },
                ValidationFinding {
                    severity: ValidationSeverity::Warning,
                    message: "Unused import".into(),
                    file: Some("src/lib.rs".into()),
                    line: None,
                },
            ],
        };
        let formatted = result.format_for_injection().unwrap();
        assert!(formatted.contains("❌ src/main.rs:42"));
        assert!(formatted.contains("Syntax error"));
        assert!(formatted.contains("⚠️ src/lib.rs"));
        assert!(formatted.contains("Unused import"));
    }

    #[test]
    fn suggest_command_fix_python() {
        let suggestion = suggest_command_fix("python script.py", "python: command not found");
        assert!(suggestion.is_some());
        assert!(suggestion.unwrap().contains("python3"));
    }
}
