//! Batch file read tool — reads multiple files in one call by delegating to [`ReadFileTool`].

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use serde::Deserialize;
use xiaolin_core::tool::{
    no_retry_recovery_hint, Tool, ToolErrorType, ToolExposure, ToolKind, ToolParameterSchema,
    ToolResult,
};

use crate::filesystem::ReadFileTool;

/// Maximum number of distinct paths per `read_files` call.
pub const MAX_READ_FILES_BATCH: usize = 20;

#[derive(Debug, Deserialize)]
struct ReadFilesArgs {
    paths: Vec<String>,
}

/// Split a message that already embeds a `What to do next:` recovery suffix.
fn split_embedded_recovery_hint(full: &str) -> (&str, String) {
    if let Some(idx) = full.find("What to do next:") {
        (full[..idx].trim_end(), full[idx..].trim().to_string())
    } else {
        (full.trim(), String::new())
    }
}

fn default_recovery_hint(error_type: ToolErrorType, path: &str) -> String {
    match error_type {
        ToolErrorType::FileNotFound => no_retry_recovery_hint(format!(
            "Verify the path exists with list_directory or glob; fix spelling for '{path}'."
        )),
        ToolErrorType::PathNotInWorkspace => no_retry_recovery_hint(
            "Pick a path inside the workspace or an allowed skills directory.",
        ),
        ToolErrorType::PermissionDenied => no_retry_recovery_hint(
            "Check execution mode in Settings → Security or ask the user to adjust permissions.",
        ),
        ToolErrorType::FileTooLarge => no_retry_recovery_hint(
            "Read a portion with read_file offset/limit instead of batch-reading this file.",
        ),
        ToolErrorType::ReadContentFailure => no_retry_recovery_hint(
            "Skip binary or unsupported files; use read_file on text files only.",
        ),
        _ => no_retry_recovery_hint(
            "Fix the path or read the file individually with read_file to diagnose.",
        ),
    }
}

fn dedup_paths(paths: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(paths.len());
    for path in paths {
        if seen.insert(path.clone()) {
            out.push(path);
        }
    }
    out
}

fn format_file_boundary(path: &str, content: &str) -> String {
    format!("=== {path} ===\n{content}")
}

fn failure_entry(path: &str, result: &ToolResult) -> serde_json::Value {
    let (error, mut recovery_hint) = split_embedded_recovery_hint(&result.output);
    let error_type = result.error_type.unwrap_or(ToolErrorType::Unknown);
    if recovery_hint.is_empty() {
        recovery_hint = default_recovery_hint(error_type, path);
    }
    serde_json::json!({
        "path": path,
        "success": false,
        "error_type": error_type,
        "error": error,
        "recovery_hint": recovery_hint,
    })
}

fn success_entry(path: &str, result: &ToolResult) -> serde_json::Value {
    let content = format_file_boundary(path, &result.output);
    let mut entry = serde_json::json!({
        "path": path,
        "success": true,
        "content": content,
    });
    if let Some(meta) = &result.metadata {
        entry["metadata"] = meta.clone();
    }
    entry
}

/// Read multiple files in one tool call, reusing single-file validation and caching.
pub struct ReadFilesTool;

#[async_trait]
impl Tool for ReadFilesTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }

    fn supports_parallel(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "read_files"
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn search_hint(&self) -> &str {
        "batch read multiple files context parallel paths"
    }

    // Use default max_result_size_chars (100k) so oversized combined output is
    // persisted by the runtime instead of silently truncated.
    fn description(&self) -> &str {
        "Read multiple text files in one call. Each file is validated and formatted like read_file. \
         Partial failures return structured errors per path without aborting the rest."
    }

    fn prompt(&self) -> String {
        "Read several known file paths in a single call when you need cross-file context.\n\n\
When to use:\n\
- You already know 2–20 specific paths (e.g. after file_outline, lsp, or search_in_files)\n\
- You need related modules/types together before editing or explaining architecture\n\
- Pair with file_outline on large files first, then read_files on the sections you need\n\n\
Parameters:\n\
- `paths`: array of absolute file paths (max 20, duplicates removed)\n\n\
Behavior:\n\
- Each path is processed by the same logic as read_file (workspace checks, size limits, line numbers, dedup cache)\n\
- Successful files are labeled with `=== /path ===` headers so content boundaries are obvious\n\
- If one path fails, others still return; check per-file `success`, `error_type`, and `recovery_hint`\n\n\
Anti-patterns:\n\
- Do NOT pass directories or glob patterns — use list_directory/glob first, then read_files\n\
- Do NOT batch-read huge trees or every file in a repo; pick the minimum set\n\
- Do NOT use read_files when you only need one file — use read_file instead\n\
- For a single large file, use file_outline + read_file with lines/offset rather than read_files\n\n\
Per-file limits:\n\
- Each path still uses read_file line/character limits; large batches may exceed the context budget and trigger disk persistence of the combined output"
            .to_string()
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "paths".to_string(),
            serde_json::json!({
                "type": "array",
                "items": { "type": "string" },
                "description": "Absolute paths to read (max 20). Duplicates are ignored."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["paths".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: ReadFilesArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::err_with_recovery(
                    ToolErrorType::InvalidToolParams,
                    format!("read_files arguments are not valid JSON: {e}"),
                    "Pass {{\"paths\": [\"/abs/path/a.rs\", \"/abs/path/b.rs\"]}}.",
                );
            }
        };

        if args.paths.is_empty() {
            return ToolResult::err_with_recovery(
                ToolErrorType::InvalidToolParams,
                "read_files requires at least one path in the paths array.",
                "Provide one or more absolute file paths.",
            );
        }

        let paths = dedup_paths(args.paths);
        if paths.len() > MAX_READ_FILES_BATCH {
            return ToolResult::err_with_recovery(
                ToolErrorType::InvalidToolParams,
                format!(
                    "read_files accepts at most {MAX_READ_FILES_BATCH} paths (got {}).",
                    paths.len()
                ),
                format!(
                    "Split into multiple calls or narrow the set to {MAX_READ_FILES_BATCH} files."
                ),
            );
        }

        let read_tool = ReadFileTool;
        let mut files = Vec::with_capacity(paths.len());
        let mut succeeded = 0usize;
        let mut failed = 0usize;
        let mut all_images = Vec::new();

        for path in &paths {
            let read_args = serde_json::json!({ "file_path": path }).to_string();
            let result = read_tool.execute(&read_args).await;

            if result.success {
                succeeded += 1;
                files.push(success_entry(path, &result));
                all_images.extend(result.images);
            } else {
                failed += 1;
                files.push(failure_entry(path, &result));
            }
        }

        let count = files.len();
        let payload = serde_json::json!({
            "files": files,
            "count": count,
            "succeeded": succeeded,
            "failed": failed,
        });

        let output = match serde_json::to_string_pretty(&payload) {
            Ok(s) => s,
            Err(_e) => {
                return ToolResult::err_with_recovery(
                    ToolErrorType::ExecutionFailed,
                    "read_files failed to serialize batch results.",
                    "Retry with fewer paths or read files individually with read_file.",
                );
            }
        };

        let mut result = if all_images.is_empty() {
            ToolResult::ok(output)
        } else {
            ToolResult::ok_with_images(output, all_images)
        };
        result.metadata = Some(payload);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaolin_core::agent_config::FileAccessMode;

    use crate::filesystem::with_file_access_mode;

    #[tokio::test]
    async fn partial_failure_one_exists_one_missing() {
        let cwd = std::env::current_dir().expect("cwd");
        let existing = cwd.join("Cargo.toml");
        assert!(
            existing.exists(),
            "Cargo.toml should exist in workspace root"
        );
        let missing = cwd.join("_read_files_missing_xyz.txt");
        assert!(!missing.exists());

        let tool = ReadFilesTool;
        let args = serde_json::json!({
            "paths": [
                existing.to_string_lossy(),
                missing.to_string_lossy(),
            ]
        })
        .to_string();

        let out = with_file_access_mode(FileAccessMode::Workspace, tool.execute(&args)).await;
        assert!(
            out.success,
            "partial failure should be soft success: {}",
            out.output
        );

        let parsed: serde_json::Value = serde_json::from_str(&out.output).expect("JSON output");
        assert_eq!(parsed["count"], 2);
        assert_eq!(parsed["succeeded"], 1);
        assert_eq!(parsed["failed"], 1);

        let files = parsed["files"].as_array().expect("files array");
        assert_eq!(files.len(), 2);

        let ok_entry = files
            .iter()
            .find(|f| f["success"] == true)
            .expect("one success");
        assert!(ok_entry["content"]
            .as_str()
            .unwrap_or("")
            .starts_with("=== "));
        assert!(ok_entry["content"]
            .as_str()
            .unwrap_or("")
            .contains("[package]"));

        let fail_entry = files
            .iter()
            .find(|f| f["success"] == false)
            .expect("one failure");
        assert_eq!(fail_entry["error_type"], "file_not_found");
        let recovery_hint = fail_entry["recovery_hint"].as_str().unwrap_or("");
        assert!(recovery_hint.len() > 10);
        assert!(
            recovery_hint.contains("Stop retrying"),
            "not-found paths should use no_retry hint: {recovery_hint}"
        );
    }

    #[tokio::test]
    async fn dedup_reads_each_path_once() {
        let cwd = std::env::current_dir().expect("cwd");
        let path = cwd.join("Cargo.toml");
        let path_str = path.to_string_lossy().to_string();

        let tool = ReadFilesTool;
        let args = serde_json::json!({
            "paths": [path_str.clone(), path_str.clone(), path_str]
        })
        .to_string();

        let out = with_file_access_mode(FileAccessMode::Workspace, tool.execute(&args)).await;
        assert!(out.success, "{}", out.output);

        let parsed: serde_json::Value = serde_json::from_str(&out.output).unwrap();
        assert_eq!(parsed["count"], 1);
        assert_eq!(parsed["succeeded"], 1);
        assert_eq!(parsed["failed"], 0);
    }

    #[tokio::test]
    async fn rejects_batch_over_limit() {
        let paths: Vec<String> = (0..=MAX_READ_FILES_BATCH)
            .map(|i| format!("/tmp/read_files_test_{i}.txt"))
            .collect();

        let tool = ReadFilesTool;
        let args = serde_json::json!({ "paths": paths }).to_string();
        let out = with_file_access_mode(FileAccessMode::Workspace, tool.execute(&args)).await;

        assert!(!out.success, "should reject > MAX paths");
        assert_eq!(out.error_type, Some(ToolErrorType::InvalidToolParams));
        assert!(out.output.contains(&MAX_READ_FILES_BATCH.to_string()));
    }

    #[test]
    fn dedup_paths_preserves_order() {
        let out = dedup_paths(vec![
            "a".into(),
            "b".into(),
            "a".into(),
            "c".into(),
            "b".into(),
        ]);
        assert_eq!(out, vec!["a", "b", "c"]);
    }

    #[tokio::test]
    async fn all_paths_fail_still_soft_success() {
        let cwd = std::env::current_dir().expect("cwd");
        let missing_a = cwd.join("_read_files_missing_a.txt");
        let missing_b = cwd.join("_read_files_missing_b.txt");
        assert!(!missing_a.exists());
        assert!(!missing_b.exists());

        let tool = ReadFilesTool;
        let args = serde_json::json!({
            "paths": [
                missing_a.to_string_lossy(),
                missing_b.to_string_lossy(),
            ]
        })
        .to_string();

        let out = with_file_access_mode(FileAccessMode::Workspace, tool.execute(&args)).await;
        assert!(
            out.success,
            "all failures should still be soft success: {}",
            out.output
        );

        let parsed: serde_json::Value = serde_json::from_str(&out.output).expect("JSON output");
        assert_eq!(parsed["count"], 2);
        assert_eq!(parsed["succeeded"], 0);
        assert_eq!(parsed["failed"], 2);

        let files = parsed["files"].as_array().expect("files array");
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|f| f["success"] == false));
    }

    #[tokio::test]
    async fn invalid_json_returns_recovery() {
        let tool = ReadFilesTool;
        let out = with_file_access_mode(FileAccessMode::Workspace, tool.execute("not-json")).await;

        assert!(!out.success);
        assert_eq!(out.error_type, Some(ToolErrorType::InvalidToolParams));
        assert!(out.output.contains("What to do next:"));
    }

    #[tokio::test]
    async fn empty_paths_returns_recovery() {
        let tool = ReadFilesTool;
        let args = serde_json::json!({ "paths": [] }).to_string();
        let out = with_file_access_mode(FileAccessMode::Workspace, tool.execute(&args)).await;

        assert!(!out.success);
        assert_eq!(out.error_type, Some(ToolErrorType::InvalidToolParams));
        assert!(out.output.contains("What to do next:"));
    }
}
