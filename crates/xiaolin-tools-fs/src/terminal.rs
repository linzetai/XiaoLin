use std::collections::HashMap;
use std::io::SeekFrom;
use std::path::PathBuf;

use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use xiaolin_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolResult};

const DEFAULT_LINES: usize = 50;
const TAIL_READ_CHUNK: usize = 8192;
const INLINE_READ_MAX_BYTES: u64 = 512 * 1024;

/// Strip ANSI escape sequences (CSI, OSC, simple escapes) from text.
fn strip_ansi(input: &str) -> String {
    let re = Regex::new(
        r"\x1b\[[0-9;]*[A-Za-z]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[()#].|\x1b[A-Za-z]",
    )
    .expect("static regex");
    re.replace_all(input, "").into_owned()
}

/// Default directory where ShellTool writes terminal output files.
fn default_terminal_dir() -> PathBuf {
    std::env::temp_dir().join("xiaolin_terminals")
}

/// Read the last `line_count` lines from a file without loading the entire file
/// into memory when it is large. Returns `(tail_lines, total_line_count)`.
async fn read_terminal_tail(
    path: &std::path::Path,
    line_count: usize,
) -> Result<(Vec<String>, usize), String> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|e| format!("Failed to stat terminal file {}: {e}", path.display()))?;

    if metadata.len() <= INLINE_READ_MAX_BYTES {
        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| format!("Failed to read terminal file {}: {e}", path.display()))?;
        let cleaned = strip_ansi(&content);
        let all_lines: Vec<&str> = cleaned.lines().collect();
        let total = all_lines.len();
        let start = total.saturating_sub(line_count);
        let tail: Vec<String> = all_lines[start..].iter().map(|s| s.to_string()).collect();
        return Ok((tail, total));
    }

    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| format!("Failed to open terminal file {}: {e}", path.display()))?;
    let file_len = metadata.len();
    let mut pos = file_len;
    let mut buffer = Vec::new();
    let mut reached_start = false;

    while pos > 0 {
        let read_size = TAIL_READ_CHUNK.min(pos as usize);
        pos -= read_size as u64;
        file.seek(SeekFrom::Start(pos))
            .await
            .map_err(|e| format!("Failed to seek terminal file {}: {e}", path.display()))?;
        let mut chunk = vec![0u8; read_size];
        file.read_exact(&mut chunk)
            .await
            .map_err(|e| format!("Failed to read terminal file {}: {e}", path.display()))?;
        chunk.extend_from_slice(&buffer);
        buffer = chunk;

        if pos == 0 {
            reached_start = true;
        }

        let cleaned = strip_ansi(&String::from_utf8_lossy(&buffer));
        if cleaned.lines().count() > line_count || reached_start {
            break;
        }
    }

    let cleaned = strip_ansi(&String::from_utf8_lossy(&buffer));
    let all_lines: Vec<&str> = cleaned.lines().collect();
    let total = if reached_start {
        all_lines.len()
    } else {
        count_lines_in_file(path).await?
    };
    let start = all_lines.len().saturating_sub(line_count);
    let tail: Vec<String> = all_lines[start..].iter().map(|s| s.to_string()).collect();
    Ok((tail, total))
}

/// Count lines using Rust `lines()` semantics without loading the whole file.
async fn count_lines_in_file(path: &std::path::Path) -> Result<usize, String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let mut file = std::fs::File::open(&path)
            .map_err(|e| format!("Failed to open terminal file {}: {e}", path.display()))?;
        let mut buf = [0u8; TAIL_READ_CHUNK];
        let mut count = 0usize;
        let mut pending_line = false;
        loop {
            let n = file
                .read(&mut buf)
                .map_err(|e| format!("Failed to read terminal file {}: {e}", path.display()))?;
            if n == 0 {
                break;
            }
            for &byte in &buf[..n] {
                if byte == b'\n' {
                    count += 1;
                    pending_line = false;
                } else {
                    pending_line = true;
                }
            }
        }
        if pending_line {
            count += 1;
        }
        Ok(count)
    })
    .await
    .map_err(|e| format!("Failed to count terminal lines: {e}"))?
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

        let (tail, total) = match read_terminal_tail(&target_path, line_count).await {
            Ok(result) => result,
            Err(e) => return ToolResult::err(e),
        };

        let panel_name = target_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let start = total.saturating_sub(line_count);
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
