use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;
use xiaolin_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolResult};
use regex::Regex;
use serde::Deserialize;

const DEFAULT_LINES: usize = 50;

/// Default directory where ShellTool writes terminal output files.
fn default_terminal_dir() -> PathBuf {
    std::env::temp_dir().join("xiaolin_terminals")
}

/// Strip ANSI escape sequences (CSI, OSC, simple escapes) from text.
fn strip_ansi(input: &str) -> String {
    let re = Regex::new(
        r"\x1b\[[0-9;]*[A-Za-z]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[()#].|\x1b[A-Za-z]",
    )
    .expect("static regex");
    re.replace_all(input, "").into_owned()
}

/// List available terminal panel files, sorted by modification time (newest first).
fn list_panels(dir: &std::path::Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut panels: Vec<(String, PathBuf, std::time::SystemTime)> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "txt")
                .unwrap_or(false)
        })
        .filter_map(|e| {
            let path = e.path();
            let mtime = e.metadata().ok()?.modified().ok()?;
            let name = path.file_stem()?.to_string_lossy().to_string();
            Some((name, path, mtime))
        })
        .collect();

    panels.sort_by(|a, b| b.2.cmp(&a.2));
    panels
        .into_iter()
        .map(|(name, path, _)| (name, path))
        .collect()
}

#[derive(Deserialize)]
struct TerminalCaptureArgs {
    #[serde(default)]
    lines: Option<usize>,
    #[serde(default)]
    panel_id: Option<String>,
}

/// Reads output from terminal panel files written by ShellTool.
///
/// Returns the last N lines (default 50) from the specified panel,
/// or the most recent panel if no panel_id is given.
/// ANSI escape codes are stripped from the output.
pub struct TerminalCaptureTool {
    terminal_dir: PathBuf,
}

impl TerminalCaptureTool {
    pub fn new() -> Self {
        Self {
            terminal_dir: default_terminal_dir(),
        }
    }

    #[cfg(test)]
    fn with_dir(dir: PathBuf) -> Self {
        Self { terminal_dir: dir }
    }
}

impl Default for TerminalCaptureTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for TerminalCaptureTool {
    fn supports_parallel(&self) -> bool {
        true
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }

    fn name(&self) -> &str {
        "terminal_capture"
    }

    fn description(&self) -> &str {
        "Read the last N lines from a terminal panel. \
         Returns stripped output without ANSI escape codes. \
         If panel_id is omitted, reads the most recent terminal."
    }

    fn search_hint(&self) -> &str {
        "read terminal output capture panel last lines"
    }

    fn is_deferred(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "lines".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Number of lines to return from the end of the terminal output. Default: 50."
            }),
        );
        props.insert(
            "panel_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Terminal panel identifier (e.g. 'shell_1719000000000'). \
                                If omitted, the most recent terminal is used."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec![],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: TerminalCaptureArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::err(format!(
                    "Invalid arguments: {e}. Expected \
                     {{\"lines\": 50, \"panel_id\": \"shell_...\"}}"
                ))
            }
        };

        let line_count = args.lines.unwrap_or(DEFAULT_LINES).max(1);

        let panels = list_panels(&self.terminal_dir);
        if panels.is_empty() {
            return ToolResult::err(
                "No terminal panels found. Run a shell command first to create terminal output.",
            );
        }

        let target_path = if let Some(ref id) = args.panel_id {
            let found = panels.iter().find(|(name, _)| name == id);
            match found {
                Some((_, path)) => path.clone(),
                None => {
                    let available: Vec<&str> =
                        panels.iter().map(|(name, _)| name.as_str()).collect();
                    let list = if available.len() <= 10 {
                        available.join(", ")
                    } else {
                        format!(
                            "{} (and {} more)",
                            available[..10].join(", "),
                            available.len() - 10
                        )
                    };
                    return ToolResult::err(format!(
                        "Terminal panel '{id}' not found. Available panels: {list}"
                    ));
                }
            }
        } else {
            panels[0].1.clone()
        };

        let content = match std::fs::read_to_string(&target_path) {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::err(format!(
                    "Failed to read terminal file {}: {e}",
                    target_path.display()
                ))
            }
        };

        let cleaned = strip_ansi(&content);
        let all_lines: Vec<&str> = cleaned.lines().collect();
        let total = all_lines.len();
        let start = total.saturating_sub(line_count);
        let tail: Vec<&str> = all_lines[start..].to_vec();

        let panel_name = target_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let header = if start > 0 {
            format!("Terminal: {panel_name} (showing last {line_count} of {total} lines)\n\n")
        } else {
            format!("Terminal: {panel_name} ({total} lines)\n\n")
        };

        ToolResult::ok(format!("{header}{}", tail.join("\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_test_dir() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("xiaolin_terminals");
        std::fs::create_dir_all(&dir).unwrap();
        (tmp, dir)
    }

    fn write_panel(dir: &std::path::Path, name: &str, content: &str) {
        let path = dir.join(format!("{name}.txt"));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn strip_ansi_removes_csi() {
        let input = "\x1b[31mhello\x1b[0m world";
        assert_eq!(strip_ansi(input), "hello world");
    }

    #[test]
    fn strip_ansi_removes_osc() {
        let input = "\x1b]0;title\x07content";
        assert_eq!(strip_ansi(input), "content");
    }

    #[test]
    fn strip_ansi_preserves_plain() {
        let input = "plain text\nwith lines";
        assert_eq!(strip_ansi(input), input);
    }

    #[test]
    fn strip_ansi_cursor_movement() {
        let input = "\x1b[2Jhello\x1b[1Aworld";
        assert_eq!(strip_ansi(input), "helloworld");
    }

    #[tokio::test]
    async fn capture_most_recent_panel() {
        let (_tmp, dir) = make_test_dir();
        write_panel(&dir, "shell_001", "line1\nline2\nline3\n");
        std::thread::sleep(std::time::Duration::from_millis(50));
        write_panel(&dir, "shell_002", "alpha\nbeta\ngamma\n");

        let tool = TerminalCaptureTool::with_dir(dir);
        let result = tool.execute(r#"{}"#).await;
        assert!(result.success);
        assert!(result.output.contains("shell_002"));
        assert!(result.output.contains("gamma"));
    }

    #[tokio::test]
    async fn capture_specific_panel() {
        let (_tmp, dir) = make_test_dir();
        write_panel(&dir, "shell_100", "data_a\n");
        write_panel(&dir, "shell_200", "data_b\n");

        let tool = TerminalCaptureTool::with_dir(dir);
        let result = tool.execute(r#"{"panel_id":"shell_100"}"#).await;
        assert!(result.success);
        assert!(result.output.contains("shell_100"));
        assert!(result.output.contains("data_a"));
    }

    #[tokio::test]
    async fn capture_panel_not_found() {
        let (_tmp, dir) = make_test_dir();
        write_panel(&dir, "shell_001", "content\n");

        let tool = TerminalCaptureTool::with_dir(dir);
        let result = tool.execute(r#"{"panel_id":"nonexistent"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("not found"));
        assert!(result.output.contains("shell_001"));
    }

    #[tokio::test]
    async fn capture_no_panels() {
        let (_tmp, dir) = make_test_dir();

        let tool = TerminalCaptureTool::with_dir(dir);
        let result = tool.execute(r#"{}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("No terminal panels"));
    }

    #[tokio::test]
    async fn capture_tail_lines() {
        let (_tmp, dir) = make_test_dir();
        let lines: Vec<String> = (1..=100).map(|i| format!("line_{i}")).collect();
        write_panel(&dir, "shell_tail", &lines.join("\n"));

        let tool = TerminalCaptureTool::with_dir(dir);
        let result = tool.execute(r#"{"lines":5}"#).await;
        assert!(result.success);
        assert!(result.output.contains("line_96"));
        assert!(result.output.contains("line_100"));
        assert!(!result.output.contains("line_95"));
        assert!(result.output.contains("last 5 of 100"));
    }

    #[tokio::test]
    async fn capture_strips_ansi() {
        let (_tmp, dir) = make_test_dir();
        write_panel(
            &dir,
            "shell_ansi",
            "\x1b[32mgreen\x1b[0m text\n\x1b[1mbold\x1b[0m end",
        );

        let tool = TerminalCaptureTool::with_dir(dir);
        let result = tool.execute(r#"{}"#).await;
        assert!(result.success);
        assert!(result.output.contains("green text"));
        assert!(result.output.contains("bold end"));
        assert!(!result.output.contains("\x1b"));
    }

    #[tokio::test]
    async fn capture_invalid_args() {
        let tool = TerminalCaptureTool::new();
        let result = tool.execute(r#"{"lines":"not_a_number"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("Invalid arguments"));
    }
}
