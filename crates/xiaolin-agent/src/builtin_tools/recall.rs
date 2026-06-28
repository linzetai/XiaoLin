//! Recall tools for tool output assets — session-scoped tools that let the
//! model recover raw output content from previously-created output assets.
//!
//! # Tools
//!
//! - `output_read`  — read by line range, byte range, or page
//! - `output_search` — search within output for pattern matches
//! - `output_tail`   — read the last N lines
//! - `output_summary` — typed summary using the appropriate projector
//!
//! # Bounded Output
//!
//! All recall tools enforce hard caps on returned data to satisfy the spec
//! requirement that recall results are bounded and non-recursive. The caps
//! prevent the recall tools from reintroducing large outputs that would
//! require further downstream truncation.
//!
//! # Authorization
//!
//! All tools validate that the handle belongs to the current session before
//! reading any content. Errors are non-disclosing: a handle that doesn't exist
//! in the session returns "not found" regardless of whether it exists elsewhere.

use std::sync::Arc;

use async_trait::async_trait;
use xiaolin_core::tool::{Tool, ToolExposure, ToolKind, ToolParameterSchema, ToolResult};
use xiaolin_session::tool_output_store::ToolOutputAssetStore;

// ============================================================================
// Task-local for the output asset store
// ============================================================================

tokio::task_local! {
    /// Per-turn output asset store, set by the runtime before tool execution.
    /// Recall tools read this to access session-scoped output assets.
    static OUTPUT_ASSET_STORE: Arc<ToolOutputAssetStore>;
    /// Current session id for authorization checks.
    static RECALL_SESSION_ID: String;
}

/// Wrap a future so recall tools can access the output asset store and session id.
pub async fn with_output_store<F, T>(
    store: Arc<ToolOutputAssetStore>,
    session_id: String,
    fut: F,
) -> T
where
    F: std::future::Future<Output = T>,
{
    OUTPUT_ASSET_STORE
        .scope(store, RECALL_SESSION_ID.scope(session_id, fut))
        .await
}

/// Get the current output asset store from the task-local.
fn current_store() -> Option<Arc<ToolOutputAssetStore>> {
    OUTPUT_ASSET_STORE.try_with(|s| Arc::clone(s)).ok()
}

/// Get the current session id from the task-local.
fn current_session_id() -> Option<String> {
    RECALL_SESSION_ID.try_with(|s| s.clone()).ok()
}

// ============================================================================
// Common helpers
// ============================================================================

/// Look up an asset by handle, producing a ToolResult error on failure.
async fn authorize_handle(
    handle: &str,
) -> Result<
    (
        Arc<ToolOutputAssetStore>,
        xiaolin_session::tool_output_store::ToolOutputAsset,
        String,
    ),
    ToolResult,
> {
    let store = current_store()
        .ok_or_else(|| ToolResult::err("output asset store not available for this session"))?;
    let session_id = current_session_id()
        .ok_or_else(|| ToolResult::err("session id not available for recall authorization"))?;

    let asset = store
        .get_asset(handle, &session_id)
        .await
        .map_err(|e| ToolResult::err(format!("{e}")))?;

    Ok((store, asset, session_id))
}

/// Format line range output: include the actual content, the range info,
/// and continuation pagination metadata.
fn format_line_read(
    content: &str,
    start_line: usize,
    end_line: usize,
    total_lines: usize,
    has_before: bool,
    has_after: bool,
) -> String {
    let mut out = String::new();

    // Pagination header
    if total_lines > 0 {
        out.push_str(&format!(
            "[lines {start_line}-{end_line} of {total_lines}]\n"
        ));
    }

    out.push_str(content);

    // Ensure trailing newline for readability
    if !content.ends_with('\n') {
        out.push('\n');
    }

    // Pagination metadata
    let mut nav = Vec::new();
    if has_before {
        let prev_start = start_line.saturating_sub(end_line - start_line + 1).max(1);
        nav.push(format!(
            "use output_read with start_line={prev_start} for previous lines"
        ));
    }
    if has_after {
        nav.push(format!(
            "use output_read with start_line={} for next lines",
            end_line + 1
        ));
    }
    if !nav.is_empty() {
        out.push('\n');
        out.push_str(&nav.join("; "));
    }

    out
}

/// Maximum lines for any recall result (prevents re-introducing large output).
#[allow(dead_code)]
const RECALL_MAX_LINES: usize = 500;
/// Maximum bytes for any recall result (10 MB soft cap, 1 MB hard cap for line mode).
#[allow(dead_code)]
const RECALL_MAX_BYTES: usize = 1_000_000;
/// Maximum byte range delta: recall refuses to read > this many bytes.
const MAX_BYTE_RANGE: usize = 100_000;
/// Maximum line range delta: recall refuses to read > this many lines.
const MAX_LINE_RANGE: usize = 500;
/// Maximum total search result lines (context included).
#[allow(dead_code)]
const SEARCH_MAX_LINES: usize = 200;
/// Max context lines per match.
const SEARCH_MAX_CONTEXT: usize = 5;
/// Max matches to return.
const SEARCH_MAX_MATCHES: usize = 20;
/// Max lines for output_tail.
const TAIL_MAX_LINES: usize = 200;

/// Line count to use as default page size for line-range reads.
/// Retained for documentation; MAX_LINE_RANGE serves the same value after
/// saturating-arithmetic simplification.
const _DEFAULT_LINE_PAGE_SIZE: usize = 500;

// ============================================================================
// output_read
// ============================================================================

pub struct OutputReadTool;

#[async_trait]
impl Tool for OutputReadTool {
    fn name(&self) -> &str {
        "output_read"
    }

    fn description(&self) -> &str {
        "Read stored tool output by handle, with line range, byte range, or page-based reads."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = std::collections::HashMap::new();
        properties.insert(
            "handle".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The output handle (e.g. out_a1b2c3d4_<uuid>)."
            }),
        );
        properties.insert(
            "start_line".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "1-indexed start line (inclusive). Default: 1."
            }),
        );
        properties.insert(
            "end_line".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "1-indexed end line (inclusive). Default: start_line + 499."
            }),
        );
        properties.insert(
            "start_byte".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "0-indexed start byte offset. Overrides line range when set."
            }),
        );
        properties.insert(
            "end_byte".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Exclusive end byte offset. Required when start_byte is set."
            }),
        );
        properties.insert(
            "page".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "1-indexed page number (page size ~4KB, aligned to line boundaries). Overrides line and byte ranges."
            }),
        );

        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties,
            required: vec!["handle".to_string()],
        }
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn supports_parallel(&self) -> bool {
        true
    }

    fn prompt(&self) -> String {
        "\
Read stored tool output by exact handle. Use this instead of rerunning \
expensive commands (shell_exec, rg, read_file on large files) when a \
handle is available in the tool output context.\n\
\n\
Three read modes (in priority order):\n\
1. Page-based: set `page` (1-indexed, ~4KB aligned to newlines).\n\
2. Byte range: set `start_byte` (0-indexed, inclusive) and `end_byte` (exclusive).\n\
3. Line range: set `start_line` and `end_line` (both 1-indexed, inclusive).\n\
   Default: lines 1-500 if only `handle` is provided.\n\
\n\
Pagination metadata is included so you can navigate forward/backward."
            .to_string()
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("Invalid JSON arguments: {e}")),
        };

        let handle = match args.get("handle").and_then(|v| v.as_str()) {
            Some(h) => h,
            None => return ToolResult::err("Missing required parameter: handle"),
        };

        let (store, asset, session_id) = match authorize_handle(handle).await {
            Ok(t) => t,
            Err(e) => return e,
        };

        let total_bytes = asset.byte_count;
        let total_lines = asset.line_count;

        // Mode 1: page-based read
        if let Some(page) = args.get("page").and_then(|v| v.as_u64()) {
            let page = page as usize;
            let chunk_idx = match ToolOutputAssetStore::load_chunk_index(&asset).await {
                Ok(idx) => idx,
                Err(e) => return ToolResult::err(format!("{e}")),
            };
            let (start, end) = match chunk_idx.page_range(page, total_bytes) {
                Some(r) => r,
                None => {
                    return ToolResult::err(format!(
                        "Page {page} is out of range. The output has {} pages.",
                        chunk_idx.total_pages
                    ))
                }
            };
            let content = match store.read_blob_range(&asset, &session_id, start, end).await {
                Ok(c) => c,
                Err(e) => return ToolResult::err(format!("{e}")),
            };

            // Calculate approximate line range for this page
            let line_idx = ToolOutputAssetStore::load_line_index(&asset).await.ok();
            let (approx_start, approx_end) = if let Some(ref idx) = line_idx {
                let s = idx.line_offsets.partition_point(|&off| off < start) + 1;
                let e = idx.line_offsets.partition_point(|&off| off < end);
                (s, e)
            } else {
                (1, total_lines)
            };

            let has_before = chunk_idx.has_before(page);
            let has_after = chunk_idx.has_after(page);

            return ToolResult::ok(format_line_read(
                &content,
                approx_start,
                approx_end,
                total_lines,
                has_before,
                has_after,
            ));
        }

        // Mode 2: byte range read
        if let Some(start_byte) = args.get("start_byte").and_then(|v| v.as_u64()) {
            let start = start_byte as usize;
            let end = match args.get("end_byte").and_then(|v| v.as_u64()) {
                Some(e) => e as usize,
                None => {
                    return ToolResult::err("end_byte is required when start_byte is specified")
                }
            };
            // Cap: refuse byte ranges exceeding MAX_BYTE_RANGE to satisfy "bounded" spec
            if end.saturating_sub(start) > MAX_BYTE_RANGE {
                return ToolResult::err(format!(
                    "Byte range too large ({} bytes > {MAX_BYTE_RANGE} max). \
                     Use a smaller range, page-based reads, or output_search.",
                    end.saturating_sub(start)
                ));
            }
            let content = match store.read_blob_range(&asset, &session_id, start, end).await {
                Ok(c) => c,
                Err(e) => return ToolResult::err(format!("{e}")),
            };
            let has_before = start > 0;
            let has_after = end < total_bytes;
            let mut nav = Vec::new();
            if has_before {
                let prev_end = start;
                let prev_start = start.saturating_sub(4096);
                nav.push(format!(
                    "use output_read with start_byte={prev_start}, end_byte={prev_end} for previous bytes"
                ));
            }
            if has_after {
                let next_end = (end + 4096).min(total_bytes);
                nav.push(format!(
                    "use output_read with start_byte={end}, end_byte={next_end} for next bytes"
                ));
            }
            let nav_str = if nav.is_empty() {
                String::new()
            } else {
                format!("\n{}", nav.join("; "))
            };
            return ToolResult::ok(format!(
                "[bytes {start}-{end} of {total_bytes}]\n{content}{}{nav_str}",
                if !content.ends_with('\n') { "\n" } else { "" },
            ));
        }

        // Mode 3: line range read — only accepted when start_line and/or end_line
        // are explicitly provided. Per spec § "Unbounded read rejected", calling
        // output_read with only a handle must be rejected.
        let has_line_range = args.get("start_line").is_some() || args.get("end_line").is_some();

        if !has_line_range {
            return ToolResult::err(
                "output_read requires a bounded selector (page, start_byte/end_byte, or \
                 start_line/end_line). Use output_tail for the last N lines, \
                 output_search to find patterns, or output_summary for an overview.",
            );
        }

        let start_line = args
            .get("start_line")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(1)
            .max(1);
        let default_end_line = start_line.saturating_add(MAX_LINE_RANGE).saturating_sub(1);
        let end_line = args
            .get("end_line")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(default_end_line);

        // Cap: refuse line ranges exceeding MAX_LINE_RANGE
        if end_line.saturating_sub(start_line) >= MAX_LINE_RANGE {
            return ToolResult::err(format!(
                "Line range too large ({} lines >= {MAX_LINE_RANGE} max). \
                 Use a smaller range, page-based reads, or output_search.",
                end_line.saturating_sub(start_line) + 1
            ));
        }

        let line_idx = match ToolOutputAssetStore::load_line_index(&asset).await {
            Ok(idx) => idx,
            Err(e) => return ToolResult::err(format!("{e}")),
        };

        let (byte_start, byte_end) =
            match line_idx.line_range_span(start_line, end_line + 1, total_bytes) {
                Some(r) => r,
                None => {
                    return ToolResult::err(format!(
                        "Line range {start_line}-{end_line} is out of bounds. \
                     The output has {total_lines} lines."
                    ))
                }
            };

        let content = match store
            .read_blob_range(&asset, &session_id, byte_start, byte_end)
            .await
        {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("{e}")),
        };

        // Count actual lines returned
        let actual_lines = if content.is_empty() {
            0
        } else if content.ends_with('\n') {
            content.bytes().filter(|&b| b == b'\n').count()
        } else {
            content.bytes().filter(|&b| b == b'\n').count() + 1
        };
        let actual_end = start_line + actual_lines.saturating_sub(1);

        let has_before = start_line > 1;
        let has_after = actual_end < total_lines;

        ToolResult::ok(format_line_read(
            &content,
            start_line,
            actual_end,
            total_lines,
            has_before,
            has_after,
        ))
    }
}

// ============================================================================
// output_search
// ============================================================================

pub struct OutputSearchTool;

#[async_trait]
impl Tool for OutputSearchTool {
    fn name(&self) -> &str {
        "output_search"
    }

    fn description(&self) -> &str {
        "Search within a stored tool output for pattern matches with context lines."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = std::collections::HashMap::new();
        properties.insert(
            "handle".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The output handle (e.g. out_a1b2c3d4_<uuid>)."
            }),
        );
        properties.insert(
            "pattern".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Substring to search for."
            }),
        );
        properties.insert(
            "context_lines".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Lines of context before and after each match. Max: 5. Default: 2."
            }),
        );
        properties.insert(
            "max_matches".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Maximum number of matches to return. Max: 20. Default: 20."
            }),
        );
        properties.insert(
            "case_sensitive".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "Whether matching is case-sensitive. Default: true."
            }),
        );

        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties,
            required: vec!["handle".to_string(), "pattern".to_string()],
        }
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn supports_parallel(&self) -> bool {
        true
    }

    fn prompt(&self) -> String {
        "\
Search within a stored tool output for pattern matches with surrounding \
context lines. Use this to locate specific error messages, file names, \
or other content without rerunning the original tool.\n\
\n\
Returns:\n\
- Match count and total match count when available\n\
- Surrounding context lines for each match\n\
- Continuation guidance when more matches exist than max_matches\n\
\n\
Best for: error stack traces, specific log entries, grep results within \
already-captured output."
            .to_string()
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("Invalid JSON arguments: {e}")),
        };

        let handle = match args.get("handle").and_then(|v| v.as_str()) {
            Some(h) => h,
            None => return ToolResult::err("Missing required parameter: handle"),
        };

        let pattern = match args.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::err("Missing required parameter: pattern"),
        };

        // Cap to hard limits
        let context_lines = (args
            .get("context_lines")
            .and_then(|v| v.as_u64())
            .unwrap_or(2) as usize)
            .min(SEARCH_MAX_CONTEXT);
        let max_matches = (args
            .get("max_matches")
            .and_then(|v| v.as_u64())
            .unwrap_or(SEARCH_MAX_MATCHES as u64) as usize)
            .min(SEARCH_MAX_MATCHES);
        let case_sensitive = args
            .get("case_sensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let (store, asset, session_id) = match authorize_handle(handle).await {
            Ok(t) => t,
            Err(e) => return e,
        };

        // Cap search at 10 MB to prevent unbounded memory allocation.
        // For larger outputs, direct the user to output_read with line ranges.
        const MAX_SEARCH_SIZE_BYTES: usize = 10 * 1024 * 1024;
        if asset.byte_count > MAX_SEARCH_SIZE_BYTES {
            return ToolResult::err(format!(
                "Output is too large to search in memory ({} bytes > {} MB). \
                 Use output_read with line ranges or output_tail to browse the output.",
                asset.byte_count,
                MAX_SEARCH_SIZE_BYTES / (1024 * 1024)
            ));
        }

        // Read full blob through the authorized store API (defense-in-depth).
        let content = match store
            .read_blob_range(&asset, &session_id, 0, asset.byte_count)
            .await
        {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("Failed to read output blob: {e}")),
        };

        // Collect matching line indices
        let matching_lines: Vec<usize> = if case_sensitive {
            content
                .lines()
                .enumerate()
                .filter(|(_, line)| line.contains(pattern))
                .map(|(i, _)| i)
                .collect()
        } else {
            let pattern_lower = pattern.to_ascii_lowercase();
            content
                .lines()
                .enumerate()
                .filter(|(_, line)| line.to_ascii_lowercase().contains(&pattern_lower))
                .map(|(i, _)| i)
                .collect()
        };

        let total_matches = matching_lines.len();
        let truncated = total_matches > max_matches;
        let shown = matching_lines.iter().take(max_matches);

        let all_lines: Vec<&str> = content.lines().collect();
        let total_lines = all_lines.len();

        let mut out = String::new();
        out.push_str(&format!(
            "[search: \"{pattern}\" — {total_matches} match{}",
            if total_matches == 1 { "" } else { "es" }
        ));
        if truncated {
            out.push_str(&format!(", showing first {max_matches}"));
        }
        out.push_str("]\n\n");

        for &line_idx in shown {
            let ctx_start = line_idx.saturating_sub(context_lines);
            let ctx_end = (line_idx + context_lines + 1).min(total_lines);

            out.push_str(&format!("--- match at line {} ---\n", line_idx + 1));
            for i in ctx_start..ctx_end {
                let marker = if i == line_idx { ">" } else { " " };
                out.push_str(&format!("{marker} {:>5}: {}\n", i + 1, all_lines[i]));
            }
            out.push('\n');
        }

        if truncated {
            out.push_str(&format!(
                "\nSearch truncated. {total_matches} total matches. \
                Narrow your search pattern or use output_read with \
                line ranges to explore remaining matches."
            ));
        } else if total_matches == 0 {
            out.push_str("No matches found. Consider using output_read to browse the output.");
        }

        ToolResult::ok(out)
    }
}

// ============================================================================
// output_tail
// ============================================================================

pub struct OutputTailTool;

#[async_trait]
impl Tool for OutputTailTool {
    fn name(&self) -> &str {
        "output_tail"
    }

    fn description(&self) -> &str {
        "Read the last N lines of a stored tool output, with shell/test status metadata."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = std::collections::HashMap::new();
        properties.insert(
            "handle".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The output handle (e.g. out_a1b2c3d4_<uuid>)."
            }),
        );
        properties.insert(
            "lines".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Number of lines to return from the end. Max: 200. Default: 50."
            }),
        );

        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties,
            required: vec!["handle".to_string()],
        }
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn supports_parallel(&self) -> bool {
        true
    }

    fn prompt(&self) -> String {
        "\
Read the last N lines of a stored tool output. This is especially useful \
for shell command outputs and test logs where the most relevant information \
(exit status, final errors, summary) is at the end.\n\
\n\
Returns:\n\
- The last N lines of output\n\
- Shell/test status metadata when available (tool name, success/failure)\n\
- Total line count and navigation hints for reading earlier content"
            .to_string()
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("Invalid JSON arguments: {e}")),
        };

        let handle = match args.get("handle").and_then(|v| v.as_str()) {
            Some(h) => h,
            None => return ToolResult::err("Missing required parameter: handle"),
        };

        let line_count =
            (args.get("lines").and_then(|v| v.as_u64()).unwrap_or(50) as usize).min(TAIL_MAX_LINES);

        let (store, asset, session_id) = match authorize_handle(handle).await {
            Ok(t) => t,
            Err(e) => return e,
        };

        let total_lines = asset.line_count;
        let total_bytes = asset.byte_count;

        // For shell/test outputs, include metadata header
        let mut header = String::new();
        if matches!(
            asset.projector_kind,
            xiaolin_session::tool_output_store::ProjectorKind::ShellTest
        ) {
            let status = if asset.success { "SUCCESS" } else { "FAILED" };
            header.push_str(&format!(
                "[{}: {} — exit: {} — total: {total_lines} lines, {total_bytes} bytes]\n",
                asset.tool_name,
                status,
                if asset.success { "0" } else { "non-zero" }
            ));
        } else {
            header.push_str(&format!(
                "[last {line_count} of {total_lines} lines, {total_bytes} bytes total]\n"
            ));
        }

        // Handle empty output gracefully
        if total_lines == 0 {
            return ToolResult::ok("[output is empty — no lines to tail]\n");
        }

        let start_line = if total_lines > line_count {
            total_lines - line_count + 1
        } else {
            1
        };

        let line_idx = match ToolOutputAssetStore::load_line_index(&asset).await {
            Ok(idx) => idx,
            Err(e) => return ToolResult::err(format!("{e}")),
        };

        let (byte_start, byte_end) =
            match line_idx.line_range_span(start_line, total_lines + 1, total_bytes) {
                Some(r) => r,
                None => return ToolResult::err("Failed to compute tail byte range"),
            };

        let content = match store
            .read_blob_range(&asset, &session_id, byte_start, byte_end)
            .await
        {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("{e}")),
        };

        let has_before = start_line > 1;
        let mut out = header;
        out.push_str(&content);
        if !content.ends_with('\n') {
            out.push('\n');
        }
        if has_before {
            out.push_str(&format!(
                "\n[use output_read handle={handle} start_line=1 end_line={} for earlier content]",
                start_line.saturating_sub(1)
            ));
        }

        ToolResult::ok(out)
    }
}

// ============================================================================
// output_summary
// ============================================================================

pub struct OutputSummaryTool;

#[async_trait]
impl Tool for OutputSummaryTool {
    fn name(&self) -> &str {
        "output_summary"
    }

    fn description(&self) -> &str {
        "Get a typed summary of stored tool output using the appropriate projector."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = std::collections::HashMap::new();
        properties.insert(
            "handle".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The output handle (e.g. out_a1b2c3d4_<uuid>)."
            }),
        );

        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties,
            required: vec!["handle".to_string()],
        }
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn supports_parallel(&self) -> bool {
        true
    }

    fn prompt(&self) -> String {
        "\
Get a typed summary of a stored tool output. Returns metadata only \
(no raw content loaded). The summary includes tool name, exit status \
(for shell/test), byte/line/token counts, and recall guidance.\n\
\n\
Use this to quickly understand what a handle contains without reading \
the full output. For precise content, use output_read, output_search, \
or output_tail."
            .to_string()
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("Invalid JSON arguments: {e}")),
        };

        let handle = match args.get("handle").and_then(|v| v.as_str()) {
            Some(h) => h,
            None => return ToolResult::err("Missing required parameter: handle"),
        };

        let (_store, asset, _session_id) = match authorize_handle(handle).await {
            Ok(t) => t,
            Err(e) => return e,
        };

        let summary = build_typed_summary(&asset);
        ToolResult::ok(summary)
    }
}

/// Build a typed summary for an asset using the projector registry.
/// Produces a metadata-only projection (no raw content loaded) for the
/// `output_summary` recall tool.
fn build_typed_summary(asset: &xiaolin_session::tool_output_store::ToolOutputAsset) -> String {
    let projection =
        xiaolin_session::tool_output_projector::PROJECTOR_REGISTRY.project_metadata_only(asset);
    projection.format()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_line_read_basic() {
        let result = format_line_read("hello\nworld\n", 1, 2, 10, false, true);
        assert!(result.contains("[lines 1-2 of 10]"));
        assert!(result.contains("hello\nworld\n"));
        assert!(result.contains("start_line=3"));
    }

    #[test]
    fn format_line_read_no_before() {
        let result = format_line_read("first\n", 1, 1, 5, false, true);
        assert!(result.contains("start_line=2")); // has_after
        assert!(!result.contains("previous lines")); // no has_before
    }

    #[test]
    fn format_line_read_no_after() {
        let result = format_line_read("last\n", 5, 5, 5, true, false);
        assert!(!result.contains("next lines")); // no has_after
        assert!(result.contains("previous lines")); // has_before
    }

    #[test]
    fn output_read_parameters_schema_requires_handle() {
        let tool = OutputReadTool;
        let schema = tool.parameters_schema();
        assert!(schema.required.contains(&"handle".to_string()));
        assert!(schema.properties.contains_key("handle"));
        assert!(schema.properties.contains_key("start_line"));
        assert!(schema.properties.contains_key("page"));
    }

    #[test]
    fn output_search_parameters_schema_requires_handle_and_pattern() {
        let tool = OutputSearchTool;
        let schema = tool.parameters_schema();
        assert!(schema.required.contains(&"handle".to_string()));
        assert!(schema.required.contains(&"pattern".to_string()));
    }

    #[test]
    fn output_tail_parameters_schema_requires_handle() {
        let tool = OutputTailTool;
        let schema = tool.parameters_schema();
        assert!(schema.required.contains(&"handle".to_string()));
    }

    #[test]
    fn output_summary_parameters_schema_requires_handle() {
        let tool = OutputSummaryTool;
        let schema = tool.parameters_schema();
        assert!(schema.required.contains(&"handle".to_string()));
    }

    #[test]
    fn all_tools_are_deferred() {
        assert!(matches!(OutputReadTool.exposure(), ToolExposure::Deferred));
        assert!(matches!(
            OutputSearchTool.exposure(),
            ToolExposure::Deferred
        ));
        assert!(matches!(OutputTailTool.exposure(), ToolExposure::Deferred));
        assert!(matches!(
            OutputSummaryTool.exposure(),
            ToolExposure::Deferred
        ));
    }

    #[test]
    fn all_tools_are_read_or_search_kind() {
        assert!(matches!(OutputReadTool.kind(), ToolKind::Read));
        assert!(matches!(OutputSearchTool.kind(), ToolKind::Search));
        assert!(matches!(OutputTailTool.kind(), ToolKind::Read));
        assert!(matches!(OutputSummaryTool.kind(), ToolKind::Read));
    }

    #[test]
    fn all_recall_tools_support_parallel() {
        assert!(OutputReadTool.supports_parallel());
        assert!(OutputSearchTool.supports_parallel());
        assert!(OutputTailTool.supports_parallel());
        assert!(OutputSummaryTool.supports_parallel());
    }

    #[test]
    fn build_summary_each_projector_kind_produces_handle() {
        use xiaolin_session::tool_output_store::{
            OutputSizeClass, ProjectorKind, ToolOutputAsset, ToolOutputHandle,
        };

        let base = ToolOutputAsset {
            handle: ToolOutputHandle::new("sess_test"),
            session_id: "sess_test".into(),
            turn_id: "turn_1".into(),
            tool_call_id: "call_1".into(),
            tool_name: "Bash".into(),
            arguments_digest: "abc".into(),
            success: true,
            lifecycle: xiaolin_session::tool_output_store::AssetLifecycle::Active,
            projector_kind: ProjectorKind::GenericText,
            byte_count: 1000,
            line_count: 50,
            estimated_tokens: 250,
            size_class: OutputSizeClass::Small,
            content_hash: "hash".into(),
            blob_path: "/tmp/blob".into(),
            line_index_path: None,
            chunk_index_path: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            last_accessed_at: "2026-01-01T00:00:00Z".into(),
            expired_at: None,
        };

        for kind in &[
            ProjectorKind::ReadFile,
            ProjectorKind::Search,
            ProjectorKind::ShellTest,
            ProjectorKind::DirectoryTree,
            ProjectorKind::BrowserSnapshot,
            ProjectorKind::JsonDefault,
            ProjectorKind::GenericText,
        ] {
            let mut a = base.clone();
            a.projector_kind = *kind;
            a.tool_name = match kind {
                ProjectorKind::ReadFile => "Read",
                ProjectorKind::Search => "Grep",
                ProjectorKind::ShellTest => "Bash",
                _ => "Tool",
            }
            .to_string();
            let summary = build_typed_summary(&a);
            assert!(
                summary.contains(base.handle.as_str()),
                "summary for {kind:?} must contain handle"
            );
        }
    }

    #[test]
    fn shell_test_summary_shows_failed_status() {
        use xiaolin_session::tool_output_store::{
            AssetLifecycle, OutputSizeClass, ProjectorKind, ToolOutputAsset, ToolOutputHandle,
        };

        let asset = ToolOutputAsset {
            handle: ToolOutputHandle::new("sess_x"),
            session_id: "sess_x".into(),
            turn_id: "t1".into(),
            tool_call_id: "c1".into(),
            tool_name: "Bash".into(),
            arguments_digest: "abc".into(),
            success: false,
            lifecycle: AssetLifecycle::Active,
            projector_kind: ProjectorKind::ShellTest,
            byte_count: 2000,
            line_count: 100,
            estimated_tokens: 500,
            size_class: OutputSizeClass::Medium,
            content_hash: "hash".into(),
            blob_path: "/tmp/b".into(),
            line_index_path: None,
            chunk_index_path: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            last_accessed_at: "2026-01-01T00:00:00Z".into(),
            expired_at: None,
        };

        let summary = build_typed_summary(&asset);
        assert!(summary.contains("FAILED"));
        assert!(summary.contains("shell/test"));
    }

    // ── Integration tests: real store + task-local ──
    // These exercise the full execute path: create asset, set task-locals, call tool.

    mod integration {
        use super::*;
        use tempfile::TempDir;
        use xiaolin_session::tool_output_store::{CreateAssetInput, ProjectionSizeConfig};

        async fn setup_store_with_asset() -> (Arc<ToolOutputAssetStore>, TempDir, String) {
            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .expect("in-memory pool");
            let store = Arc::new(ToolOutputAssetStore::open(pool).await.expect("open store"));
            let tmp = TempDir::new().expect("tempdir");
            let content = (0..100u32)
                .map(|i| format!("line {:03}: record_{i}\n", i))
                .collect::<Vec<_>>()
                .join("");
            let input = CreateAssetInput {
                session_id: "sess_int".to_string(),
                turn_id: "turn_001".to_string(),
                tool_call_id: "call_int".to_string(),
                tool_name: "Bash".to_string(),
                arguments: r#"{"command": "generate"}"#.to_string(),
                success: false,
                output: content,
                storage_root: tmp.path().to_path_buf(),
                size_config: ProjectionSizeConfig::default(),
            };
            let handle = store
                .create_asset(input)
                .await
                .expect("create asset")
                .as_str()
                .to_string();
            (store, tmp, handle)
        }

        async fn execute_with_store(
            store: Arc<ToolOutputAssetStore>,
            session_id: String,
            tool: &(dyn Tool + Sync),
            args: &str,
        ) -> ToolResult {
            with_output_store(store, session_id, tool.execute(args)).await
        }

        #[tokio::test]
        async fn output_read_line_range_with_real_store() {
            let (store, _tmp, handle) = setup_store_with_asset().await;
            let result = execute_with_store(
                store,
                "sess_int".into(),
                &OutputReadTool,
                &format!(r#"{{"handle": "{handle}", "start_line": 10, "end_line": 15}}"#),
            )
            .await;
            assert!(result.success, "{}", result.output);
            assert!(result.output.contains("[lines 10-15 of 100]"));
            assert!(result.output.contains("record_10"));
            assert!(result.output.contains("record_14"));
            // Has pagination
            assert!(result.output.contains("use output_read"));
        }

        #[tokio::test]
        async fn output_read_handle_only_rejected_per_spec() {
            let (store, _tmp, handle) = setup_store_with_asset().await;
            let result = execute_with_store(
                store,
                "sess_int".into(),
                &OutputReadTool,
                &format!(r#"{{"handle": "{handle}"}}"#),
            )
            .await;
            assert!(!result.success, "handle-only output_read must be rejected");
            assert!(result.output.contains("bounded selector"));
            assert!(result.output.contains("output_tail"));
            assert!(result.output.contains("output_search"));
        }

        #[tokio::test]
        async fn output_read_page_mode_reads_correct_page() {
            let (store, _tmp, handle) = setup_store_with_asset().await;
            let result = execute_with_store(
                store,
                "sess_int".into(),
                &OutputReadTool,
                &format!(r#"{{"handle": "{handle}", "page": 1}}"#),
            )
            .await;
            assert!(result.success, "{}", result.output);
            // Page 1 should contain the first lines
            assert!(result.output.contains("record_0"));
            // Page 1 has no "before" page, should NOT say "previous"
            assert!(!result.output.contains("previous"));
        }

        #[tokio::test]
        async fn output_read_range_too_large_rejected() {
            let (store, _tmp, handle) = setup_store_with_asset().await;
            // Request 1000-line range > MAX_LINE_RANGE (500)
            let result = execute_with_store(
                store,
                "sess_int".into(),
                &OutputReadTool,
                &format!(r#"{{"handle": "{handle}", "start_line": 1, "end_line": 600}}"#),
            )
            .await;
            assert!(
                !result.success,
                "range exceeding MAX_LINE_RANGE must be rejected"
            );
            assert!(result.output.contains("too large"));
        }

        #[tokio::test]
        async fn output_search_with_context() {
            let (store, _tmp, handle) = setup_store_with_asset().await;
            let result = execute_with_store(
                store,
                "sess_int".into(),
                &OutputSearchTool,
                &format!(r#"{{"handle": "{handle}", "pattern": "record_42", "context_lines": 1}}"#),
            )
            .await;
            assert!(result.success, "{}", result.output);
            assert!(result.output.contains("match"));
            assert!(result.output.contains("record_42"));
        }

        #[tokio::test]
        async fn output_search_no_matches() {
            let (store, _tmp, handle) = setup_store_with_asset().await;
            let result = execute_with_store(
                store,
                "sess_int".into(),
                &OutputSearchTool,
                &format!(r#"{{"handle": "{handle}", "pattern": "nonexistent_xyz"}}"#),
            )
            .await;
            assert!(result.success);
            assert!(result.output.contains("No matches found"));
        }

        #[tokio::test]
        async fn output_search_context_capped() {
            let (store, _tmp, handle) = setup_store_with_asset().await;
            // Request context_lines=100 but it should be capped to SEARCH_MAX_CONTEXT=5
            let result = execute_with_store(
                store,
                "sess_int".into(),
                &OutputSearchTool,
                &format!(
                    r#"{{"handle": "{handle}", "pattern": "record_10", "context_lines": 100}}"#
                ),
            )
            .await;
            assert!(result.success, "{}", result.output);
            // Should still work fine — context is just capped to 5
            assert!(result.output.contains("record_10"));
        }

        #[tokio::test]
        async fn output_tail_with_shell_status() {
            let (store, _tmp, handle) = setup_store_with_asset().await;
            let result = execute_with_store(
                store,
                "sess_int".into(),
                &OutputTailTool,
                &format!(r#"{{"handle": "{handle}", "lines": 10}}"#),
            )
            .await;
            assert!(result.success, "{}", result.output);
            // Shell test output should show FAILED status
            assert!(result.output.contains("FAILED"));
            // Should contain the last lines
            assert!(result.output.contains("record_99"));
        }

        #[tokio::test]
        async fn output_summary_returns_typed_info() {
            let (store, _tmp, handle) = setup_store_with_asset().await;
            let result = execute_with_store(
                store,
                "sess_int".into(),
                &OutputSummaryTool,
                &format!(r#"{{"handle": "{handle}"}}"#),
            )
            .await;
            assert!(result.success, "{}", result.output);
            assert!(result.output.contains("shell/test output"));
            assert!(result.output.contains("FAILED"));
            assert!(result.output.contains("output_read"));
            assert!(result.output.contains("output_tail"));
        }

        #[tokio::test]
        async fn cross_session_access_denied() {
            let (store, _tmp, handle) = setup_store_with_asset().await;
            let result = execute_with_store(
                store,
                "other_sess".into(), // Different session
                &OutputReadTool,
                &format!(r#"{{"handle": "{handle}", "start_line": 1, "end_line": 5}}"#),
            )
            .await;
            assert!(!result.success, "cross-session access must be denied");
            // Non-disclosing: says "not found", NOT "unauthorized"
            assert!(
                result.output.contains("No output asset found")
                    || result.output.contains("not available"),
                "cross-session error must be non-disclosing"
            );
        }

        #[tokio::test]
        async fn nonexistent_handle_returns_not_found() {
            let (store, _tmp, _handle) = setup_store_with_asset().await;
            let result = execute_with_store(
                store,
                "sess_int".into(),
                &OutputReadTool,
                r#"{"handle": "out_nonexist_00000001deadbeef00000000", "start_line": 1, "end_line": 5}"#,
            )
            .await;
            assert!(!result.success);
            assert!(result.output.contains("No output asset found"));
        }
    }
}
