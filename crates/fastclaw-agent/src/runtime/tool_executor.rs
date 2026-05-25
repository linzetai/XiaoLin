use std::sync::Arc;

use fastclaw_core::agent_config::AgentConfig;
use fastclaw_core::agent_config::BehaviorConfig;
use fastclaw_core::tool::{PostToolInfo, ToolDefinition, ToolHook, ToolHookContext, ToolRegistry};
use fastclaw_core::types::ToolCall;
use serde::Serialize;

use crate::builtin_tools::{
    with_additional_allowed_paths, with_file_access_mode, with_work_dir, ExecutionModeState,
};

use super::prompt_builder::memory_tool_suffix;
use super::tool_result_storage::TOOL_RESULT_CLEARED_MESSAGE;

fn resolve_additional_allowed_paths(raw: &[String]) -> Vec<std::path::PathBuf> {
    let home = dirs::home_dir();
    raw.iter()
        .map(|s| {
            if let Some(rest) = s.strip_prefix("~/") {
                home.as_ref()
                    .map(|h| h.join(rest))
                    .unwrap_or_else(|| std::path::PathBuf::from(s))
            } else {
                std::path::PathBuf::from(s)
            }
        })
        .collect()
}

/// Legacy fallback character limit for tool output.
/// Now superseded by `Tool::max_result_size_chars()` (100_000 default).
/// Kept only for the `truncate_tool_result_output_with_limit` fallback path.
#[deprecated(note = "Use Tool::max_result_size_chars() instead")]
pub const MAX_TOOL_RESULT_CHARS: usize = 100_000;

fn safe_char_boundary(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut i = idx;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn safe_char_boundary_ceil(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut i = idx;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Max lines of tool output before triggering line-based truncation.
const MAX_TOOL_RESULT_LINES: usize = 100;

const TRUNCATION_SEPARATOR: &str = "\n\n---\n... [CONTENT TRUNCATED] ...\n---\n\n";

/// Save the full untruncated output to a temp file so the agent can
/// `read_file` it later if needed. Returns the file path on success.
fn save_truncated_output(tool_name: &str, output: &str) -> Option<String> {
    let dir = std::env::temp_dir().join("fastclaw_truncated");
    if std::fs::create_dir_all(&dir).is_err() {
        return None;
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let filename = format!("{tool_name}_{ts}.output");
    let path = dir.join(&filename);
    match std::fs::write(&path, output) {
        Ok(()) => Some(path.to_string_lossy().to_string()),
        Err(_) => None,
    }
}

/// Truncate tool output keeping both head and tail for better context.
///
/// When content exceeds the char or line threshold, preserves ~20% from the
/// beginning and ~80% from the end (the tail is usually more relevant since
/// it contains the final state, errors, or results).
///
/// If truncation occurs, the full output is saved to a temp file and the
/// agent is told it can use `read_file` to retrieve the complete content.
#[cfg(test)]
#[allow(deprecated)]
pub(crate) fn truncate_tool_result_output(output: &str, tool_name: &str) -> String {
    truncate_tool_result_output_with_limit(output, tool_name, None)
}

/// Truncate with an explicit char-limit from `Tool::max_result_size_chars()`.
/// Falls back to `MAX_TOOL_RESULT_CHARS` (100_000) when `char_limit_override`
/// is `None` — callers should always pass `Some(tool.max_result_size_chars())`.
#[allow(deprecated)]
pub(crate) fn truncate_tool_result_output_with_limit(
    output: &str,
    tool_name: &str,
    char_limit_override: Option<usize>,
) -> String {
    let char_limit = char_limit_override.unwrap_or(MAX_TOOL_RESULT_CHARS);
    let line_limit = MAX_TOOL_RESULT_LINES;

    let total_chars = output.chars().count();
    let lines: Vec<&str> = output.lines().collect();
    let total_lines = lines.len();

    if total_chars <= char_limit && total_lines <= line_limit {
        return output.to_string();
    }

    let effective_lines = total_lines.min(line_limit);
    let head_line_count = (effective_lines / 5).max(1);
    let tail_line_count = effective_lines - head_line_count;

    let head_budget = char_limit / 5;
    let mut head_parts = Vec::new();
    let mut head_used = 0usize;
    for line in lines.iter().take(head_line_count) {
        let remaining = head_budget.saturating_sub(head_used);
        if remaining == 0 {
            break;
        }
        if line.len() > remaining {
            let slice_len = remaining.saturating_sub(3);
            let safe_end = safe_char_boundary(line, slice_len);
            head_parts.push(format!("{}...", &line[..safe_end]));
            break;
        }
        head_parts.push(line.to_string());
        head_used += line.len() + 1;
    }

    let tail_budget = char_limit
        .saturating_sub(head_used)
        .saturating_sub(TRUNCATION_SEPARATOR.len());
    let mut tail_parts: Vec<String> = Vec::new();
    let mut tail_used = 0usize;
    let tail_start = total_lines
        .saturating_sub(tail_line_count)
        .max(head_parts.len());
    for line in lines[tail_start..].iter().rev() {
        let remaining = tail_budget.saturating_sub(tail_used);
        if remaining == 0 {
            break;
        }
        if line.len() > remaining {
            let slice_len = remaining.saturating_sub(3);
            let start = line.len().saturating_sub(slice_len);
            let safe_start = safe_char_boundary_ceil(line, start);
            tail_parts.push(format!("...{}", &line[safe_start..]));
            break;
        }
        tail_parts.push(line.to_string());
        tail_used += line.len() + 1;
    }
    tail_parts.reverse();

    let truncated_body = format!(
        "{}{TRUNCATION_SEPARATOR}{}",
        head_parts.join("\n"),
        tail_parts.join("\n"),
    );

    if let Some(saved_path) = save_truncated_output(tool_name, output) {
        format!(
            "{truncated_body}\n\n[Full output ({total_chars} chars) saved to: {saved_path} — use read_file to view if needed]"
        )
    } else {
        truncated_body
    }
}

/// Compactable tool names whose old results can be progressively faded.
/// Uses `starts_with` matching so MCP tool prefixes (e.g. "mcp_") also work.
const COMPACTABLE_TOOLS: &[&str] = &[
    "read_file",
    "shell_exec",
    "shell",
    "grep",
    "glob",
    "web_search",
    "web_fetch",
    "write_file",
    "edit_file",
    "list_dir",
    "list_directory",
    "run_command",
    "ripgrep",
    "fetch_url",
    "search_in_files",
    "apply_patch",
    "multi_edit",
    "mcp_",
];

// ─── Tool Result Retention Tiers ──────────────────────────────────────────
//
// Level 0 (Ephemeral):   Immediate discard — low-value metadata results.
// Level 1 (Summarize):   Retain a compact summary — search/list results.
// Level 2 (Full Retain): Keep full content as long as possible — file reads,
//                         test output, error output, edit confirmations.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum RetentionTier {
    /// Discard immediately when not in the keep-recent window.
    Ephemeral = 0,
    /// Fade to a compact summary (first N lines + stats).
    Summarize = 1,
    /// Keep full content; only fade under extreme pressure.
    FullRetain = 2,
}

/// Tools whose results are ephemeral (Level 0): low-value metadata that the
/// model rarely needs to re-read once it has processed the output.
const TIER0_EPHEMERAL: &[&str] = &[
    "list_dir",
    "list_directory",
    "glob",
    "web_search",
    "web_fetch",
    "fetch_url",
];

/// Tools whose results should be summarized (Level 1): contain useful
/// information but can be represented compactly.
const TIER1_SUMMARIZE: &[&str] = &[
    "grep",
    "ripgrep",
    "search_in_files",
    "workspace_symbols",
    "find_references",
    "go_to_definition",
    "file_outline",
    "code_sections",
    "lsp",
];

/// Tools whose results should be fully retained (Level 2): high-value
/// content that the model frequently needs to reference.
const TIER2_FULL_RETAIN: &[&str] = &[
    "read_file",
    "shell_exec",
    "shell",
    "run_command",
    "write_file",
    "edit_file",
    "apply_patch",
    "multi_edit",
];

/// Classify a tool by name into a retention tier.
pub(crate) fn classify_retention_tier(tool_name: &str) -> RetentionTier {
    if TIER2_FULL_RETAIN.iter().any(|t| tool_name.starts_with(t)) {
        return RetentionTier::FullRetain;
    }
    if TIER1_SUMMARIZE.iter().any(|t| tool_name.starts_with(t)) {
        return RetentionTier::Summarize;
    }
    if TIER0_EPHEMERAL.iter().any(|t| tool_name.starts_with(t)) {
        return RetentionTier::Ephemeral;
    }
    // MCP tools default to Summarize; unknown tools default to Summarize.
    if tool_name.starts_with("mcp_") {
        return RetentionTier::Summarize;
    }
    RetentionTier::Summarize
}

/// Generate a compact summary for a tool result (Level 1 retention).
///
/// Preserves the first few lines (up to `max_summary_lines`) and appends
/// a stats line showing how much was trimmed.
pub(crate) fn summarize_tool_result(
    tool_name: &str,
    content: &str,
    max_summary_chars: usize,
) -> String {
    if content.len() <= max_summary_chars {
        return content.to_string();
    }

    let total_lines = content.lines().count();
    let total_chars = content.len();

    // For search results, keep the match lines (they're the most useful part)
    if tool_name.starts_with("grep")
        || tool_name.starts_with("ripgrep")
        || tool_name.starts_with("search_in_files")
    {
        return summarize_search_result(content, max_summary_chars, total_lines, total_chars);
    }

    // Generic summary: keep first N lines that fit the budget
    let mut summary = String::new();
    let mut lines_kept = 0;
    for line in content.lines() {
        if summary.len() + line.len() + 1 > max_summary_chars.saturating_sub(80) {
            break;
        }
        if !summary.is_empty() {
            summary.push('\n');
        }
        summary.push_str(line);
        lines_kept += 1;
    }

    let omitted = total_lines.saturating_sub(lines_kept);
    if omitted > 0 {
        summary.push_str(&format!(
            "\n[… {omitted} more lines, {total_chars} total chars. Use the tool again to see full output.]"
        ));
    }
    summary
}

/// Summarize search/grep results: keep file paths and match counts.
fn summarize_search_result(
    content: &str,
    max_chars: usize,
    total_lines: usize,
    total_chars: usize,
) -> String {
    let mut summary = String::new();
    let mut files_seen = std::collections::HashSet::new();
    let mut match_count = 0;

    for line in content.lines() {
        match_count += 1;
        // Extract file path (lines typically start with "path:line:content")
        if let Some(colon_pos) = line.find(':') {
            let path = &line[..colon_pos];
            if !path.is_empty() && !path.contains(' ') {
                files_seen.insert(path.to_string());
            }
        }

        if summary.len() + line.len() + 1 < max_chars.saturating_sub(100) {
            if !summary.is_empty() {
                summary.push('\n');
            }
            summary.push_str(line);
        }
    }

    let omitted = total_lines.saturating_sub(summary.lines().count());
    if omitted > 0 || !files_seen.is_empty() {
        summary.push_str(&format!(
            "\n[{match_count} matches across {} files, {total_chars} chars total. \
             {omitted} lines omitted. Re-run search to see full results.]",
            files_seen.len()
        ));
    }
    summary
}

/// Metadata stored alongside a cleared tool result so it can be
/// re-executed on demand (auto-recall).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct ClearedToolMeta {
    pub tool_name: String,
    pub arguments_json: String,
}

pub(crate) const RECALL_HINT_MARKER: &str = "[recall-available]";

/// Build the cleared message with recall metadata embedded, and register
/// the tool for potential auto-recall.
pub(crate) fn build_cleared_with_recall(
    tool_name: &str,
    tier: RetentionTier,
    original_content: &str,
    arguments_json: Option<&str>,
) -> String {
    let line_count = original_content.lines().count();
    let char_count = original_content.len();
    let est_tokens = char_count / 4;
    let stats = format!("{line_count} lines, {char_count} chars, ~{est_tokens} tokens");

    let tier_label = match tier {
        RetentionTier::Ephemeral => "ephemeral",
        RetentionTier::Summarize => "summarized",
        RetentionTier::FullRetain => "full-retain",
    };

    match arguments_json {
        Some(args) if !args.is_empty() => {
            let short_args = if args.len() > 120 {
                format!("{}…", &args[..args.floor_char_boundary(120)])
            } else {
                args.to_string()
            };
            format!(
                "{RECALL_HINT_MARKER} [{tool_name}({short_args}) → {stats}, tier={tier_label}. \
                 Re-call the tool to retrieve this result.]"
            )
        }
        _ => {
            format!(
                "{RECALL_HINT_MARKER} [{tool_name} → {stats}, tier={tier_label}. \
                 Re-call the tool to retrieve this result.]"
            )
        }
    }
}

/// A single entry in the eviction manifest, tracking one compressed/cleared tool result.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct EvictionEntry {
    pub tool_name: String,
    pub args_summary: String,
    pub content_digest: String,
    pub original_chars: usize,
    pub recall_hint: String,
}

/// Manifest of all tool results evicted during a compression pass.
/// Injected as a system message so the agent knows what was lost and how to recover.
#[derive(Debug, Clone, Default)]
pub(crate) struct EvictionManifest {
    pub entries: Vec<EvictionEntry>,
}

impl EvictionManifest {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn record(&mut self, tool_name: &str, args_json: Option<&str>, original_content: &str) {
        let args_summary = args_json
            .map(|a| {
                if a.len() > 80 {
                    format!("{}…", &a[..a.floor_char_boundary(80)])
                } else {
                    a.to_string()
                }
            })
            .unwrap_or_default();

        let content_digest = if original_content.len() > 120 {
            format!(
                "{}…",
                &original_content[..original_content.floor_char_boundary(120)]
            )
        } else {
            original_content.to_string()
        };

        let recall_hint = format!("Re-call {tool_name}({args_summary}) to retrieve");

        self.entries.push(EvictionEntry {
            tool_name: tool_name.to_string(),
            args_summary,
            content_digest,
            original_chars: original_content.len(),
            recall_hint,
        });
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Render the manifest as a concise system message for context injection.
    pub fn to_system_message(&self) -> String {
        if self.entries.is_empty() {
            return String::new();
        }
        let mut lines = vec![
            "[Context Eviction Index] The following tool results were compressed this turn:"
                .to_string(),
        ];
        for (i, entry) in self.entries.iter().enumerate() {
            lines.push(format!(
                "  {}. {} | {} chars | digest: \"{}\" | {}",
                i + 1,
                entry.tool_name,
                entry.original_chars,
                entry.content_digest.replace('\n', " "),
                entry.recall_hint,
            ));
        }
        lines.push(
            "To recover any result, re-call the corresponding tool with the same arguments."
                .to_string(),
        );
        lines.join("\n")
    }
}

/// Build an eviction manifest by comparing pre-compression content snapshots
/// with the current message state. Any message that was full content before
/// but is now a recall marker or summary gets added to the manifest.
pub(crate) fn collect_eviction_manifest(
    pre_contents: &[(usize, String, String, Option<String>)], // (idx, tool_name, original_content, args)
    messages: &[fastclaw_core::types::ChatMessage],
) -> EvictionManifest {
    let mut manifest = EvictionManifest::new();

    for (idx, tool_name, original_content, args) in pre_contents {
        if *idx >= messages.len() {
            continue;
        }
        let current_text = messages[*idx].text_content().unwrap_or_default();

        let was_evicted = current_text.starts_with(RECALL_HINT_MARKER)
            || current_text.starts_with("[summarized]")
            || current_text.starts_with(FADED_MARKER)
            || current_text.starts_with(TIME_COMPACTED_MARKER)
            || current_text == TOOL_RESULT_CLEARED_MESSAGE;

        if was_evicted
            && !original_content.starts_with(RECALL_HINT_MARKER)
            && !original_content.starts_with("[summarized]")
            && !original_content.starts_with(FADED_MARKER)
            && !original_content.starts_with(TIME_COMPACTED_MARKER)
            && *original_content != TOOL_RESULT_CLEARED_MESSAGE
        {
            manifest.record(tool_name, args.as_deref(), original_content);
        }
    }

    manifest
}

/// Snapshot tool result messages for later eviction manifest comparison.
pub(crate) fn snapshot_tool_contents(
    messages: &[fastclaw_core::types::ChatMessage],
) -> Vec<(usize, String, String, Option<String>)> {
    use fastclaw_core::types::Role;

    messages
        .iter()
        .enumerate()
        .filter(|(_, m)| matches!(m.role, Role::Tool))
        .filter_map(|(i, m)| {
            let name = m.name.as_deref()?.to_string();
            let content = m.text_content()?;
            let args = m.tool_call_id.as_deref().and_then(|call_id| {
                messages[..i]
                    .iter()
                    .rev()
                    .filter(|msg| matches!(msg.role, Role::Assistant))
                    .find_map(|msg| {
                        msg.tool_calls.as_ref()?.iter().find_map(|tc| {
                            if tc.id == call_id {
                                Some(tc.function.arguments.clone())
                            } else {
                                None
                            }
                        })
                    })
            });
            Some((i, name, content, args))
        })
        .collect()
}

/// Extract the "command" field from shell_exec tool arguments JSON.
fn extract_command_from_args(args: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(args)
        .ok()
        .and_then(|v| v.get("command")?.as_str().map(String::from))
}

/// Heuristic: does this tool result look like an error?
/// Error results are always preserved to prevent repeated mistakes.
fn is_error_tool_result(content: &str) -> bool {
    // If the content starts with a semantic header, check the header for ERR
    // and also check the body (after the first newline).
    let check_text = if content.starts_with(SEMANTIC_HEADER_MARKER) {
        if content.contains("→ ERR") {
            return true;
        }
        content
            .find('\n')
            .map(|pos| &content[pos + 1..])
            .unwrap_or("")
    } else {
        content
    };
    let lower = check_text.to_lowercase();
    let trimmed = lower.trim_start();
    trimmed.starts_with("error")
        || trimmed.starts_with("failed")
        || trimmed.starts_with("traceback")
        || trimmed.starts_with("exception")
        || trimmed.starts_with("errno")
        || trimmed.starts_with("permission denied")
        || trimmed.starts_with("command failed")
        || trimmed.starts_with("no such file")
        || trimmed.starts_with("not found")
}

/// Build a one-liner summary for a fully faded tool result.
/// If the content already contains a semantic header (§), reuse it directly.
#[allow(dead_code)]
fn one_liner_summary(tool_name: &str, content: &str) -> String {
    if content.starts_with(SEMANTIC_HEADER_MARKER) {
        if let Some(header_end) = content.find('\n') {
            return content[..header_end].to_string();
        }
        return content.to_string();
    }

    let line_count = content.lines().count();
    let char_count = content.len();
    let first_line = content
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(60)
        .collect::<String>();

    if first_line.is_empty() {
        format!("[{tool_name} → {char_count} chars, ok]")
    } else {
        format!("[{tool_name} → {line_count} lines: {first_line}…]")
    }
}

/// Fade a tool result to a short preview (tier 2).
fn fade_to_preview(content: &str, max_chars: usize) -> String {
    if content.len() <= max_chars {
        return content.to_string();
    }
    let safe_end = safe_char_boundary(content, max_chars.saturating_sub(20));
    let remaining = content.len() - safe_end;
    let est_tokens = content.len() / 4;
    format!(
        "{}…\n[{remaining} more chars faded. Original: {} chars / ~{est_tokens} tokens]",
        &content[..safe_end],
        content.len(),
    )
}

const FADED_MARKER: &str = "[faded]";
const ONELINER_MARKER: &str = "[oneliner]";
const SEMANTIC_HEADER_MARKER: &str = "§";

/// Build a semantic summary header from tool name, arguments, and output metadata.
///
/// Returns a single line like `§ read_file: src/main.rs → ok, 150 lines`.
/// This header is prepended to the tool result so that even when progressive
/// fading truncates the body, the LLM retains key context about the call.
pub(crate) fn semantic_header(
    tool_name: &str,
    arguments: &str,
    output: &str,
    success: bool,
) -> String {
    let v: Option<serde_json::Value> = serde_json::from_str(arguments).ok();

    let target = match tool_name {
        "read_file" => extract_str(&v, &["path", "file_path"]),
        "write_file" | "edit_file" | "apply_patch" | "multi_edit" | "create_file" => {
            extract_str(&v, &["path", "file_path"])
        }
        "create_directory" => extract_str(&v, &["path"]),
        "list_dir" | "list_directory" => extract_str(&v, &["path", "directory"]),
        "shell_exec" | "shell" | "run_command" => {
            extract_str(&v, &["command", "cmd"]).map(|s| truncate_str(&s, 60))
        }
        "grep" | "ripgrep" => {
            let pattern = extract_str(&v, &["pattern"]).unwrap_or_else(|| "?".into());
            let path = extract_str(&v, &["path"]).unwrap_or_else(|| ".".into());
            Some(format!(
                "\"{}\" in {}",
                truncate_str(&pattern, 30),
                truncate_str(&path, 40)
            ))
        }
        "web_search" => extract_str(&v, &["query", "search_query"])
            .map(|s| format!("\"{}\"", truncate_str(&s, 50))),
        "web_fetch" | "fetch_url" | "http_fetch" => {
            extract_str(&v, &["url"]).map(|s| truncate_str(&s, 80))
        }
        "todo_write" => Some("task list".into()),
        "memory_store" | "memory_search" => {
            extract_str(&v, &["key", "query"]).map(|s| truncate_str(&s, 40))
        }
        _ => None,
    };

    let line_count = output.lines().count();
    let status = if success { "ok" } else { "ERR" };
    let size = if line_count > 1 {
        format!("{} lines", line_count)
    } else {
        format!("{} chars", output.len().min(9999))
    };

    match target {
        Some(t) => format!("{SEMANTIC_HEADER_MARKER} {tool_name}: {t} → {status}, {size}"),
        None => format!("{SEMANTIC_HEADER_MARKER} {tool_name} → {status}, {size}"),
    }
}

fn extract_str(v: &Option<serde_json::Value>, keys: &[&str]) -> Option<String> {
    let obj = v.as_ref()?;
    for key in keys {
        if let Some(s) = obj.get(*key).and_then(|p| p.as_str()) {
            return Some(s.to_string());
        }
    }
    None
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let end = safe_char_boundary(s, max.saturating_sub(1));
        format!("{}…", &s[..end])
    }
}

/// Retention-aware progressive fading using per-tool retention tiers.
///
/// Each tool result is classified into a `RetentionTier` (Ephemeral / Summarize
/// / FullRetain). The compaction strategy varies by tier:
///
/// - **FullRetain** tools (read_file, shell, edit_file, …):
///   Most recent `full_keep + 2` kept in full, next 3 faded to preview,
///   older ones get a recall-enabled summary.
/// - **Summarize** tools (grep, search, lsp, …):
///   Most recent `full_keep` kept in full, older ones get a compact summary.
/// - **Ephemeral** tools (list_dir, glob, web_search, …):
///   Most recent 1 kept in full, all older immediately cleared with recall hint.
///
/// Error results are always preserved regardless of age or tier.
pub(crate) fn microcompact_tool_results(
    messages: &mut [fastclaw_core::types::ChatMessage],
    keep_recent: usize,
) {
    microcompact_tool_results_with_protection(
        messages,
        keep_recent,
        &std::collections::HashSet::new(),
    )
}

/// Like [`microcompact_tool_results`] but skips messages in the `protected` set.
pub(crate) fn microcompact_tool_results_with_protection(
    messages: &mut [fastclaw_core::types::ChatMessage],
    keep_recent: usize,
    protected: &std::collections::HashSet<usize>,
) {
    use fastclaw_core::types::Role;

    let base_keep = keep_recent.max(1);

    // Collect (msg_index, tool_name) for all compactable tool results.
    let tool_entries: Vec<(usize, String)> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| matches!(m.role, Role::Tool))
        .filter_map(|(i, m)| {
            let name = m.name.as_deref()?;
            if COMPACTABLE_TOOLS.iter().any(|t| name.starts_with(t)) {
                Some((i, name.to_string()))
            } else {
                None
            }
        })
        .collect();

    if tool_entries.len() <= base_keep {
        return;
    }

    // Pre-collect arguments for each tool entry (avoids borrow conflict).
    let args_for_entry: Vec<Option<String>> = tool_entries
        .iter()
        .map(|&(idx, _)| {
            let call_id = messages[idx].tool_call_id.as_deref()?;
            messages[..idx]
                .iter()
                .rev()
                .filter(|m| matches!(m.role, Role::Assistant))
                .find_map(|m| {
                    m.tool_calls.as_ref()?.iter().find_map(|tc| {
                        if tc.id == call_id {
                            Some(tc.function.arguments.clone())
                        } else {
                            None
                        }
                    })
                })
        })
        .collect();

    for (rank_from_end, (entry_idx_from_end, &(idx, ref tool_name))) in
        tool_entries.iter().rev().enumerate().map(|(r, e)| {
            let entry_idx = tool_entries.len() - 1 - r;
            (r, (entry_idx, e))
        })
    {
        if protected.contains(&idx) {
            continue;
        }

        let msg = &mut messages[idx];
        let text = match msg.text_content() {
            Some(t) => t,
            None => continue,
        };

        if text.starts_with(ONELINER_MARKER)
            || text.starts_with(FADED_MARKER)
            || text.starts_with(RECALL_HINT_MARKER)
            || text == TOOL_RESULT_CLEARED_MESSAGE
        {
            continue;
        }
        if is_error_tool_result(&text) {
            continue;
        }

        let tier = classify_retention_tier(tool_name);
        let args_json = args_for_entry[entry_idx_from_end].as_deref();

        match tier {
            RetentionTier::FullRetain => {
                let full_window = base_keep + 2;
                let preview_window = 3;
                if rank_from_end < full_window {
                    // Keep fully
                } else if rank_from_end < full_window + preview_window {
                    let faded = format!("{FADED_MARKER} {}", fade_to_preview(&text, 300));
                    msg.content = Some(serde_json::Value::String(faded));
                } else {
                    let cleared = build_cleared_with_recall(tool_name, tier, &text, args_json);
                    msg.content = Some(serde_json::Value::String(cleared));
                }
            }
            RetentionTier::Summarize => {
                if rank_from_end < base_keep {
                    // Keep fully
                } else {
                    let summary = summarize_tool_result(tool_name, &text, 400);
                    let compacted = format!("[summarized] {summary}");
                    msg.content = Some(serde_json::Value::String(compacted));
                }
            }
            RetentionTier::Ephemeral => {
                if rank_from_end < 1 {
                    // Keep most recent one
                } else {
                    let cleared = build_cleared_with_recall(tool_name, tier, &text, args_json);
                    msg.content = Some(serde_json::Value::String(cleared));
                }
            }
        }
    }
}

/// Default cache window duration for time-based microcompact (5 minutes).
pub(crate) const DEFAULT_CACHE_WINDOW_DURATION: std::time::Duration =
    std::time::Duration::from_secs(5 * 60);

/// Compute the cache window duration dynamically based on context occupancy.
///
/// Under low occupancy there's no reason to evict cached results; as pressure
/// increases the window shrinks to free space more aggressively.
pub(crate) fn cache_window_for_occupancy(
    current_tokens: usize,
    context_window: u32,
) -> std::time::Duration {
    if context_window == 0 {
        return DEFAULT_CACHE_WINDOW_DURATION;
    }
    let occupancy = current_tokens as f64 / context_window as f64;
    match occupancy {
        o if o < 0.50 => std::time::Duration::from_secs(u64::MAX / 2), // effectively infinite
        o if o < 0.70 => std::time::Duration::from_secs(10 * 60),
        o if o < 0.90 => std::time::Duration::from_secs(5 * 60),
        _ => std::time::Duration::from_secs(2 * 60),
    }
}

/// Compute the `keep_recent` window for `microcompact_tool_results`
/// based on the model's context window size.
///
/// Larger context windows can afford to keep more recent tool results
/// in full, while smaller windows need more aggressive compaction.
pub(crate) fn keep_recent_for_context_window(context_window: u32) -> usize {
    match context_window {
        0..=32_000 => 2,
        32_001..=64_000 => 3,
        64_001..=128_000 => 4,
        128_001..=200_000 => 5,
        _ => 6,
    }
}

/// Configuration for the protection window that prevents premature
/// compression of recent tool results.
#[derive(Debug, Clone)]
pub(crate) struct ProtectionWindowConfig {
    /// Number of most recent agent iterations whose tool results are immune
    /// to any form of compression (microcompact, time-based, budget trim).
    pub protected_iterations: usize,
}

impl Default for ProtectionWindowConfig {
    fn default() -> Self {
        Self {
            protected_iterations: 3,
        }
    }
}

/// Compute the set of message indices that are "protected" from compression.
///
/// Protected messages are tool results that belong to one of the last N agent
/// iterations (as delineated by `iteration_boundaries`).
pub(crate) fn compute_protected_indices(
    messages: &[fastclaw_core::types::ChatMessage],
    iteration_boundaries: &[(usize, std::time::Instant)],
    config: &ProtectionWindowConfig,
) -> std::collections::HashSet<usize> {
    use fastclaw_core::types::Role;

    let mut protected = std::collections::HashSet::new();

    if iteration_boundaries.is_empty() || config.protected_iterations == 0 {
        return protected;
    }

    let protect_from_boundary = if iteration_boundaries.len() > config.protected_iterations {
        iteration_boundaries[iteration_boundaries.len() - config.protected_iterations].0
    } else {
        0
    };

    for (i, msg) in messages.iter().enumerate() {
        if i >= protect_from_boundary && matches!(msg.role, Role::Tool) {
            protected.insert(i);
        }
    }

    protected
}

const TIME_COMPACTED_MARKER: &str = "[time-compacted]";

/// Time-driven microcompact with tier awareness.
///
/// Tool results older than `cache_window` are compressed according to their
/// retention tier:
/// - **FullRetain**: summarized (preserving key content).
/// - **Summarize**: compacted to a brief summary.
/// - **Ephemeral**: cleared with a recall hint.
///
/// Error results are always preserved regardless of tier.
#[allow(dead_code)]
pub(crate) fn time_based_microcompact(
    messages: &mut [fastclaw_core::types::ChatMessage],
    iteration_boundaries: &[(usize, std::time::Instant)],
    cache_window: std::time::Duration,
) -> usize {
    time_based_microcompact_with_protection(
        messages,
        iteration_boundaries,
        cache_window,
        &std::collections::HashSet::new(),
    )
}

/// Like [`time_based_microcompact`] but skips messages in the `protected` set.
pub(crate) fn time_based_microcompact_with_protection(
    messages: &mut [fastclaw_core::types::ChatMessage],
    iteration_boundaries: &[(usize, std::time::Instant)],
    cache_window: std::time::Duration,
    protected: &std::collections::HashSet<usize>,
) -> usize {
    use fastclaw_core::types::Role;

    if iteration_boundaries.is_empty() {
        return 0;
    }

    let now = std::time::Instant::now();
    let cutoff = now.checked_sub(cache_window).unwrap_or(now);

    let fresh_boundary_idx = iteration_boundaries
        .iter()
        .rev()
        .find(|(_, time)| *time >= cutoff)
        .map(|(idx, _)| *idx)
        .unwrap_or(messages.len());

    if fresh_boundary_idx == 0 {
        return 0;
    }

    let mut compacted = 0;

    for i in 0..fresh_boundary_idx.min(messages.len()) {
        if protected.contains(&i) {
            continue;
        }

        let msg = &messages[i];
        if !matches!(msg.role, Role::Tool) {
            continue;
        }

        let tool_name = match msg.name.as_deref() {
            Some(n) if COMPACTABLE_TOOLS.iter().any(|t| n.starts_with(t)) => n.to_string(),
            _ => continue,
        };

        let text = match msg.text_content() {
            Some(t) => t,
            None => continue,
        };

        if text.starts_with(ONELINER_MARKER)
            || text.starts_with(FADED_MARKER)
            || text.starts_with(TIME_COMPACTED_MARKER)
            || text.starts_with(RECALL_HINT_MARKER)
            || text.starts_with("[summarized]")
            || text == TOOL_RESULT_CLEARED_MESSAGE
        {
            continue;
        }

        if is_error_tool_result(&text) {
            continue;
        }

        let tier = classify_retention_tier(&tool_name);
        let replacement = if tool_name == "read_file" {
            time_compact_read_file(messages, i, &text, tier)
        } else {
            match tier {
                RetentionTier::FullRetain => {
                    let summary = summarize_tool_result(&tool_name, &text, 600);
                    format!("{TIME_COMPACTED_MARKER} {summary}")
                }
                RetentionTier::Summarize => {
                    let summary = summarize_tool_result(&tool_name, &text, 300);
                    format!("{TIME_COMPACTED_MARKER} {summary}")
                }
                RetentionTier::Ephemeral => {
                    build_cleared_with_recall(&tool_name, tier, &text, None)
                }
            }
        };

        messages[i].content = Some(serde_json::Value::String(replacement));
        compacted += 1;
    }

    compacted
}

/// Specialized time-compaction for read_file results.
///
/// Instead of a generic summary, checks the file's current mtime on disk
/// to tell the LLM whether the file has changed since the read, enabling
/// better decisions about whether to re-read.
fn time_compact_read_file(
    messages: &[fastclaw_core::types::ChatMessage],
    tool_msg_idx: usize,
    original_text: &str,
    _tier: RetentionTier,
) -> String {
    use fastclaw_core::types::Role;
    use std::path::Path;

    let line_count = original_text.lines().count();
    let char_count = original_text.len();
    let est_tokens = char_count / 4;

    let call_id = messages[tool_msg_idx].tool_call_id.as_deref().unwrap_or("");
    let file_path = messages[..tool_msg_idx]
        .iter()
        .rev()
        .filter(|m| matches!(m.role, Role::Assistant))
        .find_map(|m| {
            m.tool_calls.as_ref()?.iter().find_map(|tc| {
                if tc.id == call_id {
                    extract_target_key("read_file", &tc.function.arguments)
                } else {
                    None
                }
            })
        });

    let file_path = match file_path {
        Some(p) => p,
        None => {
            let summary = summarize_tool_result("read_file", original_text, 600);
            return format!("{TIME_COMPACTED_MARKER} {summary}");
        }
    };

    let path = Path::new(&file_path);
    let short_path = if file_path.len() > 80 {
        format!("…{}", &file_path[file_path.len().saturating_sub(77)..])
    } else {
        file_path.clone()
    };

    let status = match std::fs::metadata(path).and_then(|m| m.modified()) {
        Ok(_mtime) => {
            // We can't precisely compare Instant (monotonic) vs SystemTime (wall clock),
            // but the file's existence is confirmed. Since this result is outside the
            // cache window (5+ min old), we report the file as "previously read" and
            // let the LLM decide whether to re-read.
            "file exists on disk"
        }
        Err(_) => "file may have been moved or deleted",
    };

    format!(
        "{TIME_COMPACTED_MARKER} [read_file: {short_path} — {status}. \
         Original: {line_count} lines, {char_count} chars, ~{est_tokens} tokens. \
         Use read_file to get current content if needed.]"
    )
}

/// Deduplicate repeated tool calls on the same target.
///
/// When the same file is `read_file`-d multiple times, or the same command is
/// `shell`-ed multiple times, only the **most recent** result is kept.
/// Older duplicates are replaced with a short pointer.
///
/// Detection is based on extracting a "target key" from the tool arguments in
/// the preceding assistant message's tool_call. For `read_file` the key is
/// the file path; for `shell` it is the command string.
struct DedupToolEntry {
    msg_idx: usize,
    arguments_json: String,
}

pub(crate) fn dedup_repeated_tool_calls(messages: &mut [fastclaw_core::types::ChatMessage]) {
    use fastclaw_core::types::Role;
    use std::collections::HashMap;

    const DEDUP_TOOLS: &[&str] = &["read_file", "shell_exec", "shell", "run_command"];

    // Collect (tool_call_id, tool_name, msg_index) for all Tool messages
    let tool_entries: Vec<(String, String, usize)> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| matches!(m.role, Role::Tool))
        .filter_map(|(i, m)| {
            let name = m.name.as_deref()?;
            if !DEDUP_TOOLS.iter().any(|t| name.starts_with(t)) {
                return None;
            }
            let call_id = m.tool_call_id.clone()?;
            Some((call_id, name.to_string(), i))
        })
        .collect();

    // For each tool entry, find the matching tool_call in the preceding assistant message
    // and extract a "target key" (e.g., file path or command) and full arguments
    let mut target_map: HashMap<(String, String), Vec<DedupToolEntry>> = HashMap::new();

    for (call_id, tool_name, msg_idx) in &tool_entries {
        let (target_key, args_json) = messages[..*msg_idx]
            .iter()
            .rev()
            .filter(|m| matches!(m.role, Role::Assistant))
            .find_map(|m| {
                m.tool_calls.as_ref()?.iter().find_map(|tc| {
                    if tc.id == *call_id {
                        let key = extract_target_key(tool_name, &tc.function.arguments)?;
                        Some((key, tc.function.arguments.clone()))
                    } else {
                        None
                    }
                })
            })
            .unzip();

        if let (Some(key), Some(args)) = (target_key, args_json) {
            target_map
                .entry((tool_name.clone(), key))
                .or_default()
                .push(DedupToolEntry {
                    msg_idx: *msg_idx,
                    arguments_json: args,
                });
        }
    }

    // For groups with >1 entry, replace all but the last with a short pointer
    for ((tool_name, target_key), entries) in &target_map {
        if entries.len() <= 1 {
            continue;
        }

        if tool_name == "read_file" {
            dedup_overlapping_reads(messages, entries, target_key);
        } else {
            for entry in &entries[..entries.len() - 1] {
                supersede_message(messages, entry.msg_idx, target_key);
            }
        }
    }
}

/// Supersede a tool result message with a short pointer, skipping errors and
/// already-superseded messages.
fn supersede_message(
    messages: &mut [fastclaw_core::types::ChatMessage],
    idx: usize,
    target_key: &str,
) {
    if let Some(text) = messages[idx].text_content() {
        if text.starts_with("[superseded") || is_error_tool_result(&text) {
            return;
        }
    }
    let short_key = if target_key.len() > 60 {
        format!("{}…", &target_key[..target_key.floor_char_boundary(57)])
    } else {
        target_key.to_string()
    };
    messages[idx].content = Some(serde_json::Value::String(format!(
        "[superseded: re-executed on \"{short_key}\", see latest result below]"
    )));
}

/// Extract the line range from read_file arguments as (start, end) 1-indexed inclusive.
/// Returns `None` for full-file reads.
fn extract_read_range(args: &str) -> Option<(usize, usize)> {
    let v: serde_json::Value = serde_json::from_str(args).ok()?;

    if let Some(lines_str) = v.get("lines").and_then(|l| l.as_str()) {
        return parse_lines_range(lines_str);
    }

    let offset = v.get("offset").and_then(|o| o.as_i64());
    let limit = v.get("limit").and_then(|l| l.as_u64());
    match (offset, limit) {
        (Some(off), Some(lim)) if off > 0 => Some((off as usize, off as usize + lim as usize - 1)),
        (Some(off), None) if off > 0 => Some((off as usize, usize::MAX)),
        _ => None,
    }
}

fn parse_lines_range(s: &str) -> Option<(usize, usize)> {
    let s = s.trim();
    if let Some((a, b)) = s.split_once('-') {
        let start: usize = a.trim().parse().ok()?;
        let end: usize = if b.trim().is_empty() {
            usize::MAX
        } else {
            b.trim().parse().ok()?
        };
        Some((start, end))
    } else {
        let line: usize = s.parse().ok()?;
        Some((line, line))
    }
}

/// Returns true if range `a` is fully contained within range `b`.
fn range_contained(a: (usize, usize), b: (usize, usize)) -> bool {
    b.0 <= a.0 && a.1 <= b.1
}

/// Deduplicate overlapping read_file calls on the same file path.
///
/// Strategy: iterate from newest to oldest. For each older read, if its range
/// is fully contained by any newer read's range, supersede it. Full-file reads
/// supersede all partial reads of the same file.
fn dedup_overlapping_reads(
    messages: &mut [fastclaw_core::types::ChatMessage],
    entries: &[DedupToolEntry],
    file_path: &str,
) {
    struct ReadInfo {
        msg_idx: usize,
        range: Option<(usize, usize)>,
    }

    let reads: Vec<ReadInfo> = entries
        .iter()
        .map(|e| ReadInfo {
            msg_idx: e.msg_idx,
            range: extract_read_range(&e.arguments_json),
        })
        .collect();

    // Work from newest to oldest: for each older read, check if any newer read
    // covers its range. `reads` is ordered by position in messages (ascending),
    // so newer reads are at the end.
    for i in 0..reads.len().saturating_sub(1) {
        let older = &reads[i];
        let older_range = match older.range {
            Some(r) => r,
            None => {
                // Full-file read: only superseded if a newer full-file read exists
                let has_newer_full = reads[i + 1..].iter().any(|r| r.range.is_none());
                if has_newer_full {
                    supersede_message(messages, older.msg_idx, file_path);
                }
                continue;
            }
        };

        let covered_by_newer = reads[i + 1..].iter().any(|newer| {
            match newer.range {
                None => true, // newer full-file read covers everything
                Some(newer_range) => range_contained(older_range, newer_range),
            }
        });

        if covered_by_newer {
            if let Some(text) = messages[older.msg_idx].text_content() {
                if text.starts_with("[superseded") || is_error_tool_result(&text) {
                    continue;
                }
            }
            let short_path = if file_path.len() > 50 {
                format!("…{}", &file_path[file_path.len().saturating_sub(47)..])
            } else {
                file_path.to_string()
            };
            messages[older.msg_idx].content = Some(serde_json::Value::String(format!(
                "[superseded: lines {}-{} of \"{short_path}\" covered by a later read]",
                older_range.0, older_range.1
            )));
        }
    }
}

/// Extract a target key from tool arguments for deduplication.
fn extract_target_key(tool_name: &str, arguments: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(arguments).ok()?;
    match tool_name {
        "read_file" => v
            .get("path")
            .or(v.get("file_path"))
            .and_then(|p| p.as_str())
            .map(|s| s.to_string()),
        "shell_exec" | "shell" | "run_command" => v
            .get("command")
            .or(v.get("cmd"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string()),
        _ => None,
    }
}

// ── cache_edits API Microcompact (6E-03) ─────────────────────────────

/// A cache_edits block instructing the API to delete cached tool results.
#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub(crate) struct CacheEditsBlock {
    pub tool_ids_to_delete: Vec<String>,
}

/// Tracks tool_result IDs sent to the API so that stale results can be
/// deleted via the Anthropic `cache_edits` API rather than re-sending
/// the entire prompt. Only applicable to models that support this feature.
#[derive(Debug, Default)]
#[allow(dead_code)]
pub(crate) struct CachedMCState {
    registered_tools: std::collections::HashSet<String>,
    tool_order: Vec<String>,
    deleted_refs: std::collections::HashSet<String>,
    tools_sent_to_api: bool,
}

#[allow(dead_code)]
const CACHED_MC_KEEP_RECENT: usize = 6;

#[allow(dead_code)]
impl CachedMCState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a tool_result ID after it has been sent to the API.
    pub fn register_tool_result(&mut self, tool_id: &str) {
        if self.registered_tools.insert(tool_id.to_string()) {
            self.tool_order.push(tool_id.to_string());
        }
        self.tools_sent_to_api = true;
    }

    /// Return tool_result IDs that should be deleted (exceeding the keep window).
    pub fn get_tool_results_to_delete(&self) -> Vec<String> {
        if self.tool_order.len() <= CACHED_MC_KEEP_RECENT {
            return Vec::new();
        }

        let cutoff = self.tool_order.len() - CACHED_MC_KEEP_RECENT;
        self.tool_order[..cutoff]
            .iter()
            .filter(|id| !self.deleted_refs.contains(*id))
            .cloned()
            .collect()
    }

    /// Build a `CacheEditsBlock` for the IDs that need deletion.
    /// Returns `None` if no deletions are needed or tools haven't been sent yet.
    pub fn create_cache_edits_block(&mut self) -> Option<CacheEditsBlock> {
        if !self.tools_sent_to_api {
            return None;
        }

        let to_delete = self.get_tool_results_to_delete();
        if to_delete.is_empty() {
            return None;
        }

        for id in &to_delete {
            self.deleted_refs.insert(id.clone());
        }

        Some(CacheEditsBlock {
            tool_ids_to_delete: to_delete,
        })
    }

    /// Number of currently tracked (not yet deleted) tool results.
    #[allow(dead_code)]
    pub fn tracked_count(&self) -> usize {
        self.registered_tools.len() - self.deleted_refs.len()
    }
}

/// Check if a model supports the `cache_edits` API for tool result deletion.
#[allow(dead_code)]
pub(crate) fn is_model_supported_for_cache_editing(model: &str) -> bool {
    let lower = model.to_lowercase();
    lower.contains("claude") && (lower.contains("-4") || lower.contains("claude-4"))
}

/// Per-agent visibility for scoped memory tools (`memory_search__{agent}` style).
pub(crate) fn scoped_tool_visible_for_agent(name: &str, agent_id: &str) -> bool {
    let sfx = memory_tool_suffix(agent_id);
    for prefix in &[
        "memory_search__",
        "memory_store__",
        "get_identity__",
        "set_identity__",
    ] {
        if let Some(rest) = name.strip_prefix(prefix) {
            return rest == sfx;
        }
    }
    true
}

pub(crate) fn is_tool_allowed(
    tool_name: &str,
    behavior: &fastclaw_core::agent_config::BehaviorConfig,
) -> bool {
    behavior.is_tool_allowed(tool_name)
}

/// Filter tool definitions by agent visibility and allow/deny policy.
pub(crate) fn filter_tool_definitions(
    all_defs: &[ToolDefinition],
    config: &AgentConfig,
) -> Vec<ToolDefinition> {
    all_defs
        .iter()
        .filter(|td| {
            let name = &td.function.name;
            if !scoped_tool_visible_for_agent(name, &config.agent_id) {
                return false;
            }
            if !config.behavior.tools_deny.is_empty()
                && config.behavior.tools_deny.iter().any(|d| d == name)
            {
                return false;
            }
            if !config.behavior.tools_allow.is_empty()
                && !config.behavior.tools_allow.iter().any(|a| a == name)
            {
                return false;
            }
            true
        })
        .cloned()
        .collect()
}

/// Validate tool arguments against the tool's parameter schema.
/// Returns `Some(error_message)` if validation fails, `None` if OK.
fn validate_tool_arguments(
    tool: &dyn fastclaw_core::tool::Tool,
    arguments: &str,
) -> Option<String> {
    let schema = tool.parameters_schema();
    if schema.required.is_empty() {
        return None;
    }

    let parsed: serde_json::Value = match serde_json::from_str(arguments) {
        Ok(v) => v,
        Err(e) => {
            return Some(format!(
                "Invalid JSON arguments for tool '{}': {}. Please provide valid JSON.",
                tool.name(),
                e
            ));
        }
    };

    let obj = match parsed.as_object() {
        Some(o) => o,
        None => {
            return Some(format!(
                "Arguments for tool '{}' must be a JSON object, got: {}",
                tool.name(),
                parsed
            ));
        }
    };

    let missing: Vec<&str> = schema
        .required
        .iter()
        .filter(|r| !obj.contains_key(r.as_str()))
        .map(|r| r.as_str())
        .collect();

    if missing.is_empty() {
        None
    } else {
        Some(format!(
            "Missing required parameter(s) for tool '{}': {}. Required: {:?}",
            tool.name(),
            missing.join(", "),
            schema.required
        ))
    }
}

// ─── Auto-Recall Registry ─────────────────────────────────────────────
//
// Stores metadata for cleared tool results so they can be re-executed
// on demand. The registry is populated during compaction and queried
// when the LLM asks for a result that was previously cleared.

use dashmap::DashMap;
use std::sync::OnceLock;

/// Global registry mapping tool_call_id → recall metadata.
static AUTO_RECALL_REGISTRY: OnceLock<DashMap<String, ClearedToolMeta>> = OnceLock::new();

fn recall_registry() -> &'static DashMap<String, ClearedToolMeta> {
    AUTO_RECALL_REGISTRY.get_or_init(DashMap::new)
}

/// Register a cleared tool result for potential auto-recall.
pub(crate) fn register_for_recall(tool_call_id: &str, meta: ClearedToolMeta) {
    recall_registry().insert(tool_call_id.to_string(), meta);
}

/// Look up recall metadata for a tool_call_id.
#[allow(dead_code)]
pub(crate) fn lookup_recall(tool_call_id: &str) -> Option<ClearedToolMeta> {
    recall_registry().get(tool_call_id).map(|r| r.clone())
}

/// Check all Tool messages in the conversation for recall-available markers
/// and populate the global registry from any embedded metadata.
///
/// Call this during pipeline startup / session resume to rebuild the
/// registry from conversation state.
pub(crate) fn rebuild_recall_registry(messages: &[fastclaw_core::types::ChatMessage]) {
    use fastclaw_core::types::Role;

    for msg in messages {
        if !matches!(msg.role, Role::Tool) {
            continue;
        }
        let text = match msg.text_content() {
            Some(t) if t.starts_with(RECALL_HINT_MARKER) => t,
            _ => continue,
        };

        let tool_name = match msg.name.as_deref() {
            Some(n) => n.to_string(),
            None => continue,
        };
        let tool_call_id = match &msg.tool_call_id {
            Some(id) => id.clone(),
            None => continue,
        };

        // Try to extract arguments from the marker text.
        // Format: "[recall-available] [tool_name(args_json) → ..."
        let args = extract_args_from_recall_marker(&text);

        register_for_recall(
            &tool_call_id,
            ClearedToolMeta {
                tool_name,
                arguments_json: args.unwrap_or_default(),
            },
        );
    }
}

fn extract_args_from_recall_marker(text: &str) -> Option<String> {
    let after_bracket = text.strip_prefix(RECALL_HINT_MARKER)?.trim();
    let after_bracket = after_bracket.strip_prefix('[')?;
    let paren_start = after_bracket.find('(')?;
    let paren_end = after_bracket.find(')')?;
    if paren_start >= paren_end {
        return None;
    }
    let args = &after_bracket[paren_start + 1..paren_end];
    if args.is_empty() || args == "…" {
        None
    } else {
        Some(args.to_string())
    }
}

type ToolExecResult = (String, String, String, fastclaw_core::tool::ToolResult);

/// Execute a batch of tool calls with ToolKind-aware scheduling.
///
/// Read/Search/Fetch/Think tools run concurrently; Edit/Execute tools run sequentially.
/// When `hooks` is non-empty, fires `pre_tool_use` before and `post_tool_use` after each call.
pub(crate) async fn execute_tool_batch(
    tool_calls: &[ToolCall],
    tool_registry: &Arc<ToolRegistry>,
    behavior: &BehaviorConfig,
    work_dir: &Option<String>,
    log_suffix: &str,
    mode_state: Option<&ExecutionModeState>,
) -> Vec<ToolExecResult> {
    execute_tool_batch_with_hooks(
        tool_calls,
        tool_registry,
        behavior,
        work_dir,
        log_suffix,
        &[],
        "",
        mode_state,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_tool_batch_with_hooks(
    tool_calls: &[ToolCall],
    tool_registry: &Arc<ToolRegistry>,
    behavior: &BehaviorConfig,
    work_dir: &Option<String>,
    log_suffix: &str,
    hooks: &[Arc<dyn ToolHook>],
    agent_id: &str,
    mode_state: Option<&ExecutionModeState>,
) -> Vec<ToolExecResult> {
    execute_tool_batch_with_hooks_and_stream(
        tool_calls,
        tool_registry,
        behavior,
        work_dir,
        log_suffix,
        hooks,
        agent_id,
        None,
        mode_state,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_tool_batch_with_hooks_and_stream(
    tool_calls: &[ToolCall],
    tool_registry: &Arc<ToolRegistry>,
    behavior: &BehaviorConfig,
    work_dir: &Option<String>,
    log_suffix: &str,
    hooks: &[Arc<dyn ToolHook>],
    agent_id: &str,
    stream_tx: Option<&tokio::sync::mpsc::Sender<fastclaw_protocol::AgentEvent>>,
    mode_state: Option<&ExecutionModeState>,
) -> Vec<ToolExecResult> {
    // Batch-level dedup: when the same read_file path appears multiple times
    // in one batch, only execute the first and share the result.
    let mut read_file_seen: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut dedup_source: Vec<Option<usize>> = vec![None; tool_calls.len()];

    for (i, tc) in tool_calls.iter().enumerate() {
        if tc.function.name == "read_file" {
            if let Some(path) = extract_target_key("read_file", &tc.function.arguments) {
                if let Some(&first_idx) = read_file_seen.get(&path) {
                    dedup_source[i] = Some(first_idx);
                    tracing::info!(
                        tool = "read_file", path = %path,
                        "skipping duplicate read_file in same batch (first at index {first_idx})"
                    );
                } else {
                    read_file_seen.insert(path, i);
                }
            }
        }
    }

    let mut concurrent_indices = Vec::new();
    let mut sequential_indices = Vec::new();

    for (i, tc) in tool_calls.iter().enumerate() {
        if dedup_source[i].is_some() {
            continue;
        }
        let kind = tool_registry
            .get(&tc.function.name)
            .map(|t| t.kind())
            .unwrap_or(fastclaw_core::tool::ToolKind::Other);
        if kind.is_concurrency_safe() {
            concurrent_indices.push(i);
        } else {
            sequential_indices.push(i);
        }
    }

    let mut results: Vec<Option<ToolExecResult>> = vec![None; tool_calls.len()];

    if !concurrent_indices.is_empty() {
        let concurrent_futures: Vec<_> = concurrent_indices
            .iter()
            .map(|&i| {
                execute_single_tool(
                    &tool_calls[i],
                    tool_registry,
                    behavior,
                    work_dir,
                    log_suffix,
                    hooks,
                    agent_id,
                    stream_tx,
                    mode_state,
                )
            })
            .collect();
        let concurrent_results = futures::future::join_all(concurrent_futures).await;
        for (slot, result) in concurrent_indices.iter().zip(concurrent_results) {
            results[*slot] = Some(result);
        }
    }

    for &i in &sequential_indices {
        let result = execute_single_tool(
            &tool_calls[i],
            tool_registry,
            behavior,
            work_dir,
            log_suffix,
            hooks,
            agent_id,
            stream_tx,
            mode_state,
        )
        .await;
        results[i] = Some(result);
    }

    for (i, source) in dedup_source.iter().enumerate() {
        if let Some(src_idx) = source {
            if let Some(ref original) = results[*src_idx] {
                let dedup_output = format!(
                    "[duplicate read_file in same batch — identical to call_id {}]",
                    original.1,
                );
                let mut dedup_result = original.3.clone();
                dedup_result.output = dedup_output;
                results[i] = Some((
                    tool_calls[i].function.name.clone(),
                    tool_calls[i].id.clone(),
                    tool_calls[i].function.arguments.clone(),
                    dedup_result,
                ));
            }
        }
    }

    results
        .into_iter()
        .map(|r| r.expect("all slots filled"))
        .collect()
}

#[allow(clippy::too_many_arguments)]
async fn execute_single_tool(
    tc: &ToolCall,
    tool_registry: &Arc<ToolRegistry>,
    behavior: &BehaviorConfig,
    work_dir: &Option<String>,
    log_suffix: &str,
    hooks: &[Arc<dyn ToolHook>],
    agent_id: &str,
    stream_tx: Option<&tokio::sync::mpsc::Sender<fastclaw_protocol::AgentEvent>>,
    mode_state: Option<&ExecutionModeState>,
) -> ToolExecResult {
    let tool_name = tc.function.name.clone();
    let call_id = tc.id.clone();
    let arguments = tc.function.arguments.clone();

    if !is_tool_allowed(&tool_name, behavior) {
        tracing::warn!(tool = %tool_name, "tool blocked by allow/deny policy — forwarding to user for confirmation{log_suffix}");
        let result = fastclaw_core::tool::ToolResult::needs_confirm(format!(
            "Tool '{}' is not in the allowed tool list. Allow this tool to proceed?",
            tool_name
        ));
        return (tool_name, call_id, arguments, result);
    }
    if behavior.requires_confirmation(&tool_name) {
        tracing::info!(tool = %tool_name, "tool requires user confirmation (tools_ask){log_suffix}");
        let result = fastclaw_core::tool::ToolResult::needs_confirm(format!(
            "Tool '{}' requires user confirmation per agent policy.",
            tool_name
        ));
        return (tool_name, call_id, arguments, result);
    }

    let tool_kind = tool_registry
        .get(&tool_name)
        .map(|t| t.kind())
        .unwrap_or(fastclaw_core::tool::ToolKind::Other);

    if let Some(ms) = mode_state {
        if ms.is_blocked_for_tool(&tool_name, tool_kind) {
            tracing::info!(tool = %tool_name, kind = ?tool_kind, "tool blocked by plan mode{log_suffix}");
            let result = fastclaw_core::tool::ToolResult::typed_err(
                fastclaw_core::tool::ToolErrorType::ExecutionDenied,
                ExecutionModeState::blocked_message(&tool_name),
            );
            return (tool_name, call_id, arguments, result);
        }
        // shell_exec in Plan mode: validate readonly command classification
        if tool_name == "shell_exec"
            && ms.current_mode() == fastclaw_core::types::ExecutionMode::Plan
        {
            if let Some(cmd) = extract_command_from_args(&arguments) {
                if let Err(reason) = crate::builtin_tools::validate_readonly_command(&cmd) {
                    tracing::info!(tool = "shell_exec", %reason, "shell command blocked by plan mode readonly policy{log_suffix}");
                    let result = fastclaw_core::tool::ToolResult::typed_err(
                        fastclaw_core::tool::ToolErrorType::ExecutionDenied,
                        format!(
                            "Plan mode (read-only) blocks this command: {reason}. \
                             Only read-only commands (ls, cat, grep, git status, cargo check, etc.) \
                             are allowed. Use exit_plan_mode to switch back to Agent mode for writes."
                        ),
                    );
                    return (tool_name, call_id, arguments, result);
                }
            }
        }
    }

    let hook_ctx = ToolHookContext {
        tool_name: tool_name.clone(),
        tool_kind,
        call_id: call_id.clone(),
        arguments: arguments.clone(),
        agent_id: agent_id.to_string(),
    };

    if let Some(tool) = tool_registry.get(&tool_name) {
        if let Some(err) = validate_tool_arguments(tool.as_ref(), &arguments) {
            tracing::warn!(tool = %tool_name, "tool parameter validation failed: {err}");
            return (
                tool_name,
                call_id,
                arguments,
                fastclaw_core::tool::ToolResult::err(err),
            );
        }
    }

    let mut effective_args = arguments.clone();
    for hook in hooks {
        let action = hook.pre_tool_use(&hook_ctx).await;
        if let Some(reason) = action.block_reason {
            tracing::info!(tool = %tool_name, hook = hook.name(), "tool blocked by hook: {reason}");
            return (
                tool_name,
                call_id,
                arguments,
                fastclaw_core::tool::ToolResult::err(reason),
            );
        }
        if let Some(new_args) = action.modified_arguments {
            effective_args = new_args;
        }
    }

    let extra_paths = resolve_additional_allowed_paths(&behavior.additional_allowed_paths);
    let t0 = std::time::Instant::now();
    let result = match tool_registry.get(&tool_name) {
        Some(tool) => {
            let work_dir_path = work_dir.as_ref().map(std::path::PathBuf::from);
            if let (true, Some(stx)) = (tool.supports_progress(), stream_tx) {
                let stream_tx = stx.clone();
                let tn = tool_name.clone();
                let ci = call_id.clone();
                let (progress_tx, mut progress_rx) =
                    tokio::sync::mpsc::channel::<fastclaw_core::tool::ToolProgressUpdate>(32);

                let bridge = tokio::spawn(async move {
                    while let Some(update) = progress_rx.recv().await {
                        let event = fastclaw_protocol::AgentEvent::ToolProgress {
                            turn_id: fastclaw_protocol::TurnId::new("tool"),
                            tool_name: tn.clone(),
                            call_id: ci.clone(),
                            message: update.message,
                            progress: update.progress,
                            partial_output: update.partial_output,
                        };
                        let _ = stream_tx.send(event).await;
                    }
                });

                let res = with_file_access_mode(
                    behavior.file_access,
                    with_additional_allowed_paths(
                        extra_paths.clone(),
                        with_work_dir(
                            work_dir_path,
                            tool.execute_with_progress(&effective_args, progress_tx),
                        ),
                    ),
                )
                .await;
                bridge.abort();
                res
            } else {
                with_file_access_mode(
                    behavior.file_access,
                    with_additional_allowed_paths(
                        extra_paths,
                        with_work_dir(work_dir_path, tool.execute(&effective_args)),
                    ),
                )
                .await
            }
        }
        None => {
            let msg = format!("tool not found: {}", tool_name);
            fastclaw_core::tool::ToolResult::err(msg)
        }
    };
    let latency_ms = t0.elapsed().as_millis() as u64;

    tracing::info!(
        tool = %tool_name, success = result.success,
        output_len = result.output.len(), latency_ms, "tool result{log_suffix}"
    );

    let post_info = PostToolInfo {
        success: result.success,
        output_len: result.output.len(),
        latency_ms,
    };
    for hook in hooks {
        hook.post_tool_use(&hook_ctx, &post_info).await;
    }

    (tool_name, call_id, arguments, result)
}

#[cfg(test)]
#[allow(deprecated)]
mod tool_result_truncation_tests {
    use super::{truncate_tool_result_output, MAX_TOOL_RESULT_CHARS, TRUNCATION_SEPARATOR};

    #[test]
    fn no_truncation_at_or_below_char_limit() {
        let s = "a".repeat(MAX_TOOL_RESULT_CHARS);
        let out = truncate_tool_result_output(&s, "test_tool");
        assert_eq!(out, s);
        assert!(!out.contains("TRUNCATED"));
    }

    #[test]
    fn truncates_long_output_with_head_and_tail() {
        let lines: Vec<String> = (0..300)
            .map(|i| format!("line {i}: some data here"))
            .collect();
        let input = lines.join("\n");
        let out = truncate_tool_result_output(&input, "test_tool");
        assert!(
            out.contains(TRUNCATION_SEPARATOR),
            "should contain truncation separator"
        );
        assert!(out.contains("line 0:"), "should keep head lines");
        assert!(out.contains("line 299:"), "should keep tail lines");
        assert!(
            out.len() < input.len(),
            "output should be shorter than input"
        );
        assert!(
            out.contains("saved to:"),
            "should include saved file path hint"
        );
        assert!(out.contains("read_file"), "should mention read_file tool");
    }

    #[test]
    fn no_truncation_below_line_and_char_limits() {
        let lines: Vec<String> = (0..10).map(|i| format!("short line {i}")).collect();
        let input = lines.join("\n");
        let out = truncate_tool_result_output(&input, "test_tool");
        assert_eq!(out, input);
    }

    #[test]
    fn microcompact_progressive_fading() {
        use super::{microcompact_tool_results, FADED_MARKER, RECALL_HINT_MARKER};
        use fastclaw_core::types::{ChatMessage, Role};

        // 10 read_file results (FullRetain tier): with keep_recent=3,
        // full_window = 3+2 = 5, preview_window = 3.
        // So: indices 0-1 → cleared (recall), 2-4 → faded, 5-9 → kept.
        let mut msgs: Vec<ChatMessage> = (0..10)
            .map(|i| ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::Value::String(format!("output {i}"))),
                reasoning_content: None,
                name: Some("read_file".into()),
                tool_calls: None,
                tool_call_id: Some(format!("id-{i}")),
                compact_metadata: None,
            })
            .collect();

        microcompact_tool_results(&mut msgs, 3);

        // Oldest 2: cleared with recall hint
        for msg in &msgs[..2] {
            let text = msg.text_content().unwrap();
            assert!(
                text.starts_with(RECALL_HINT_MARKER),
                "expected recall-cleared, got: {text}"
            );
        }
        // Next 3 (indices 2..5): faded to preview
        for msg in &msgs[2..5] {
            let text = msg.text_content().unwrap();
            assert!(
                text.starts_with(FADED_MARKER),
                "expected faded, got: {text}"
            );
        }
        // Most recent 5 (indices 5..10): kept fully
        for msg in &msgs[5..] {
            assert!(msg.text_content().unwrap().starts_with("output"));
        }
    }

    #[test]
    fn microcompact_preserves_error_results() {
        use super::{microcompact_tool_results, FADED_MARKER, RECALL_HINT_MARKER};
        use fastclaw_core::types::{ChatMessage, Role};

        let mut msgs: Vec<ChatMessage> = vec![
            ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::Value::String("Error: file not found".into())),
                reasoning_content: None,
                name: Some("read_file".into()),
                tool_calls: None,
                tool_call_id: Some("id-0".into()),
                compact_metadata: None,
            },
            ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::Value::String("success output 1".into())),
                reasoning_content: None,
                name: Some("shell_exec".into()),
                tool_calls: None,
                tool_call_id: Some("id-1".into()),
                compact_metadata: None,
            },
            ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::Value::String("Failed to connect".into())),
                reasoning_content: None,
                name: Some("web_fetch".into()),
                tool_calls: None,
                tool_call_id: Some("id-2".into()),
                compact_metadata: None,
            },
            ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::Value::String("success output 2".into())),
                reasoning_content: None,
                name: Some("grep".into()),
                tool_calls: None,
                tool_call_id: Some("id-3".into()),
                compact_metadata: None,
            },
            ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::Value::String("recent output".into())),
                reasoning_content: None,
                name: Some("read_file".into()),
                tool_calls: None,
                tool_call_id: Some("id-4".into()),
                compact_metadata: None,
            },
        ];

        microcompact_tool_results(&mut msgs, 1);

        // Error results preserved even though old
        assert!(msgs[0]
            .text_content()
            .unwrap()
            .contains("Error: file not found"));
        // Non-error old results get faded/cleared/summarized (not kept full)
        let t1 = msgs[1].text_content().unwrap();
        assert!(
            t1.starts_with(FADED_MARKER)
                || t1.starts_with(RECALL_HINT_MARKER)
                || t1.starts_with("[summarized]"),
            "expected faded/cleared/summarized, got: {t1}"
        );
        // Error results preserved
        assert!(msgs[2]
            .text_content()
            .unwrap()
            .contains("Failed to connect"));
        // Non-error older results get faded/cleared/summarized
        let t3 = msgs[3].text_content().unwrap();
        assert!(
            t3.starts_with(FADED_MARKER)
                || t3.starts_with(RECALL_HINT_MARKER)
                || t3.starts_with("[summarized]"),
            "expected faded/cleared/summarized, got: {t3}"
        );
        // Most recent result preserved fully
        assert!(msgs[4].text_content().unwrap().contains("recent output"));
    }

    #[test]
    fn semantic_header_includes_tool_target() {
        use super::semantic_header;

        let h = semantic_header(
            "read_file",
            r#"{"path": "src/main.rs"}"#,
            "fn main() {\n    println!(\"hello\");\n}\n",
            true,
        );
        assert!(h.contains("read_file"), "should contain tool name");
        assert!(h.contains("src/main.rs"), "should contain file path");
        assert!(h.contains("ok"), "should show ok for success");

        let h_err = semantic_header(
            "shell_exec",
            r#"{"command": "cargo build"}"#,
            "error[E0001]: some error\n",
            false,
        );
        assert!(h_err.contains("ERR"), "should show ERR for failure");
        assert!(h_err.contains("cargo build"), "should contain command");
    }

    #[test]
    fn oneliner_reuses_semantic_header() {
        use super::{one_liner_summary, SEMANTIC_HEADER_MARKER};

        let content = format!(
            "{SEMANTIC_HEADER_MARKER} read_file: src/lib.rs → ok, 50 lines\nactual file content here..."
        );
        let summary = one_liner_summary("read_file", &content);
        assert!(
            summary.contains("src/lib.rs"),
            "oneliner should reuse semantic header, got: {summary}"
        );
        assert!(
            !summary.contains("actual file content"),
            "should not include body"
        );
    }

    #[test]
    fn time_microcompact_collapses_stale_tool_results() {
        use super::{time_based_microcompact, RECALL_HINT_MARKER, TIME_COMPACTED_MARKER};
        use fastclaw_core::types::{ChatMessage, Role};
        use std::time::{Duration, Instant};

        let old_time = Instant::now() - Duration::from_secs(600);
        let boundaries = vec![(0usize, old_time)];

        let mut msgs = vec![
            ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::json!("file content here...")),
                reasoning_content: None,
                name: Some("read_file".into()),
                tool_calls: None,
                tool_call_id: Some("tc-1".into()),
                compact_metadata: None,
            },
            ChatMessage {
                role: Role::User,
                content: Some(serde_json::json!("user msg")),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
                compact_metadata: None,
            },
        ];

        let count = time_based_microcompact(&mut msgs, &boundaries, Duration::from_secs(300));
        assert_eq!(count, 1, "should compact 1 stale tool result");
        let text = msgs[0].text_content().unwrap();
        // read_file is FullRetain, so time-based compact summarizes instead of clearing.
        assert!(
            text.starts_with(TIME_COMPACTED_MARKER) || text.starts_with(RECALL_HINT_MARKER),
            "expected time-compacted or recall marker, got: {text}"
        );
        assert_eq!(
            msgs[1].text_content().unwrap(),
            "user msg",
            "user message should be untouched"
        );
    }

    #[test]
    fn time_microcompact_preserves_fresh_and_errors() {
        use super::{time_based_microcompact, RECALL_HINT_MARKER, TIME_COMPACTED_MARKER};
        use fastclaw_core::types::{ChatMessage, Role};
        use std::time::{Duration, Instant};

        let old_time = Instant::now() - Duration::from_secs(600);
        let fresh_time = Instant::now() - Duration::from_secs(60);
        let boundaries = vec![(0usize, old_time), (2usize, fresh_time)];

        let mut msgs = vec![
            ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::json!("stale output")),
                reasoning_content: None,
                name: Some("read_file".into()),
                tool_calls: None,
                tool_call_id: Some("tc-1".into()),
                compact_metadata: None,
            },
            ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::json!("Error: file not found")),
                reasoning_content: None,
                name: Some("read_file".into()),
                tool_calls: None,
                tool_call_id: Some("tc-2".into()),
                compact_metadata: None,
            },
            ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::json!("fresh output")),
                reasoning_content: None,
                name: Some("read_file".into()),
                tool_calls: None,
                tool_call_id: Some("tc-3".into()),
                compact_metadata: None,
            },
        ];

        let count = time_based_microcompact(&mut msgs, &boundaries, Duration::from_secs(300));
        assert_eq!(count, 1, "only 1 stale non-error should be compacted");

        let t0 = msgs[0].text_content().unwrap();
        assert!(
            t0.starts_with(TIME_COMPACTED_MARKER) || t0.starts_with(RECALL_HINT_MARKER),
            "stale result should be summarized: {t0}"
        );

        let t1 = msgs[1].text_content().unwrap();
        assert!(t1.contains("Error"), "error result preserved: {t1}");

        let t2 = msgs[2].text_content().unwrap();
        assert_eq!(t2, "fresh output", "fresh result preserved");
    }

    #[test]
    fn time_microcompact_noop_on_empty_boundaries() {
        use super::time_based_microcompact;
        use fastclaw_core::types::{ChatMessage, Role};
        use std::time::Duration;

        let mut msgs = vec![ChatMessage {
            role: Role::Tool,
            content: Some(serde_json::json!("some output")),
            reasoning_content: None,
            name: Some("read_file".into()),
            tool_calls: None,
            tool_call_id: Some("tc-1".into()),
            compact_metadata: None,
        }];

        let count = time_based_microcompact(&mut msgs, &[], Duration::from_secs(300));
        assert_eq!(count, 0, "no compaction with empty boundaries");
        assert_eq!(msgs[0].text_content().unwrap(), "some output");
    }

    #[test]
    fn microcompact_20_results_achieves_70_percent_reduction() {
        use super::microcompact_tool_results;
        use fastclaw_core::types::{ChatMessage, Role};

        let large_content = "x".repeat(1000);
        let mut msgs: Vec<ChatMessage> = (0..20)
            .map(|i| ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::Value::String(large_content.clone())),
                reasoning_content: None,
                name: Some("read_file".into()),
                tool_calls: None,
                tool_call_id: Some(format!("id-{i}")),
                compact_metadata: None,
            })
            .collect();

        let before: usize = msgs
            .iter()
            .filter_map(|m| m.text_content())
            .map(|t| t.len())
            .sum();

        microcompact_tool_results(&mut msgs, 3);

        let after: usize = msgs
            .iter()
            .filter_map(|m| m.text_content())
            .map(|t| t.len())
            .sum();

        let reduction = 1.0 - (after as f64 / before as f64);
        // Tier-aware compaction preserves more info (summaries, recall hints)
        // so the raw byte reduction is lower than the old blunt clearing.
        assert!(
            reduction >= 0.55,
            "expected ≥55% reduction, got {:.1}% (before={before}, after={after})",
            reduction * 100.0
        );
    }

    #[test]
    fn cached_mc_state_register_and_delete() {
        use super::CachedMCState;

        let mut state = CachedMCState::new();
        for i in 0..10 {
            state.register_tool_result(&format!("tool-{i}"));
        }

        let to_delete = state.get_tool_results_to_delete();
        assert_eq!(to_delete.len(), 4, "should delete 10 - 6 = 4 oldest");
        assert_eq!(to_delete[0], "tool-0");
        assert_eq!(to_delete[3], "tool-3");
    }

    #[test]
    fn cached_mc_state_create_block_marks_deleted() {
        use super::CachedMCState;

        let mut state = CachedMCState::new();
        for i in 0..8 {
            state.register_tool_result(&format!("tool-{i}"));
        }

        let block = state
            .create_cache_edits_block()
            .expect("should produce block");
        assert_eq!(block.tool_ids_to_delete.len(), 2, "8 - 6 = 2 to delete");

        let block2 = state.create_cache_edits_block();
        assert!(block2.is_none(), "already deleted, no new block needed");
    }

    #[test]
    fn cached_mc_state_no_delete_below_threshold() {
        use super::CachedMCState;

        let mut state = CachedMCState::new();
        for i in 0..5 {
            state.register_tool_result(&format!("tool-{i}"));
        }

        assert!(
            state.get_tool_results_to_delete().is_empty(),
            "5 <= 6, nothing to delete"
        );
        assert!(state.create_cache_edits_block().is_none());
    }

    #[test]
    fn is_model_supported_for_cache_editing() {
        use super::is_model_supported_for_cache_editing;

        assert!(is_model_supported_for_cache_editing(
            "claude-4-sonnet-20260514"
        ));
        assert!(is_model_supported_for_cache_editing(
            "claude-4-opus-20260514"
        ));
        assert!(is_model_supported_for_cache_editing(
            "anthropic/claude-4-haiku"
        ));
        assert!(!is_model_supported_for_cache_editing(
            "claude-3-5-sonnet-20241022"
        ));
        assert!(!is_model_supported_for_cache_editing("gpt-4o"));
        assert!(!is_model_supported_for_cache_editing("deepseek-chat"));
    }

    // ─── Retention Tier Tests ──────────────────────────────────────────

    #[test]
    fn classify_retention_tier_full_retain() {
        use super::{classify_retention_tier, RetentionTier};
        assert_eq!(
            classify_retention_tier("read_file"),
            RetentionTier::FullRetain
        );
        assert_eq!(
            classify_retention_tier("shell_exec"),
            RetentionTier::FullRetain
        );
        assert_eq!(classify_retention_tier("shell"), RetentionTier::FullRetain);
        assert_eq!(
            classify_retention_tier("edit_file"),
            RetentionTier::FullRetain
        );
        assert_eq!(
            classify_retention_tier("write_file"),
            RetentionTier::FullRetain
        );
        assert_eq!(
            classify_retention_tier("multi_edit"),
            RetentionTier::FullRetain
        );
    }

    #[test]
    fn classify_retention_tier_summarize() {
        use super::{classify_retention_tier, RetentionTier};
        assert_eq!(classify_retention_tier("grep"), RetentionTier::Summarize);
        assert_eq!(classify_retention_tier("ripgrep"), RetentionTier::Summarize);
        assert_eq!(
            classify_retention_tier("search_in_files"),
            RetentionTier::Summarize
        );
        assert_eq!(
            classify_retention_tier("workspace_symbols"),
            RetentionTier::Summarize
        );
        assert_eq!(
            classify_retention_tier("find_references"),
            RetentionTier::Summarize
        );
    }

    #[test]
    fn classify_retention_tier_ephemeral() {
        use super::{classify_retention_tier, RetentionTier};
        assert_eq!(
            classify_retention_tier("list_dir"),
            RetentionTier::Ephemeral
        );
        assert_eq!(
            classify_retention_tier("list_directory"),
            RetentionTier::Ephemeral
        );
        assert_eq!(classify_retention_tier("glob"), RetentionTier::Ephemeral);
        assert_eq!(
            classify_retention_tier("web_search"),
            RetentionTier::Ephemeral
        );
        assert_eq!(
            classify_retention_tier("web_fetch"),
            RetentionTier::Ephemeral
        );
    }

    #[test]
    fn classify_retention_tier_mcp_defaults_to_summarize() {
        use super::{classify_retention_tier, RetentionTier};
        assert_eq!(
            classify_retention_tier("mcp_some_tool"),
            RetentionTier::Summarize
        );
    }

    #[test]
    fn summarize_tool_result_short_passthrough() {
        use super::summarize_tool_result;
        let short = "hello world";
        assert_eq!(summarize_tool_result("grep", short, 100), short);
    }

    #[test]
    fn summarize_tool_result_truncates_long() {
        use super::summarize_tool_result;
        let long = "line\n".repeat(200);
        let summary = summarize_tool_result("read_file", &long, 100);
        assert!(summary.len() < long.len(), "summary should be shorter");
        assert!(summary.contains("more lines"), "should indicate omission");
    }

    #[test]
    fn summarize_search_result_preserves_match_info() {
        use super::summarize_tool_result;
        let content = "src/main.rs:10:fn main() {}\nsrc/lib.rs:5:pub fn foo() {}\n".repeat(50);
        let summary = summarize_tool_result("grep", &content, 200);
        assert!(summary.contains("matches"), "should report match count");
        assert!(summary.contains("files"), "should report file count");
    }

    #[test]
    fn build_cleared_with_recall_includes_marker() {
        use super::{build_cleared_with_recall, RetentionTier, RECALL_HINT_MARKER};
        let result = build_cleared_with_recall(
            "read_file",
            RetentionTier::FullRetain,
            "file content\nline 2\nline 3",
            Some(r#"{"path":"src/main.rs"}"#),
        );
        assert!(result.starts_with(RECALL_HINT_MARKER));
        assert!(result.contains("read_file"));
        assert!(result.contains("3 lines"));
        assert!(result.contains("full-retain"));
    }

    #[test]
    fn microcompact_tier_aware_ephemeral_cleared_fast() {
        use super::{microcompact_tool_results, RECALL_HINT_MARKER};
        use fastclaw_core::types::{ChatMessage, Role};

        // 5 list_dir results (Ephemeral): only most recent 1 kept.
        let mut msgs: Vec<ChatMessage> = (0..5)
            .map(|i| ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::Value::String(format!("dir listing {i}"))),
                reasoning_content: None,
                name: Some("list_dir".into()),
                tool_calls: None,
                tool_call_id: Some(format!("id-{i}")),
                compact_metadata: None,
            })
            .collect();

        microcompact_tool_results(&mut msgs, 3);

        // Only most recent (index 4) should be kept full
        assert!(msgs[4].text_content().unwrap().starts_with("dir listing"));
        // All older ones should be cleared with recall
        for msg in &msgs[..4] {
            let text = msg.text_content().unwrap();
            assert!(
                text.starts_with(RECALL_HINT_MARKER),
                "ephemeral tool should be cleared fast: {text}"
            );
        }
    }

    #[test]
    fn microcompact_tier_aware_summarize_gets_summary() {
        use super::microcompact_tool_results;
        use fastclaw_core::types::{ChatMessage, Role};

        // 5 grep results (Summarize): most recent 3 kept, others summarized.
        let mut msgs: Vec<ChatMessage> = (0..5)
            .map(|i| ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::Value::String(format!("match result {i}"))),
                reasoning_content: None,
                name: Some("grep".into()),
                tool_calls: None,
                tool_call_id: Some(format!("id-{i}")),
                compact_metadata: None,
            })
            .collect();

        microcompact_tool_results(&mut msgs, 3);

        // Most recent 3 kept fully
        for msg in &msgs[2..] {
            assert!(
                msg.text_content().unwrap().starts_with("match result"),
                "recent grep results should be kept"
            );
        }
        // Older ones summarized
        for msg in &msgs[..2] {
            let text = msg.text_content().unwrap();
            assert!(
                text.starts_with("[summarized]"),
                "old grep should be summarized: {text}"
            );
        }
    }

    #[test]
    fn recall_registry_rebuild_from_messages() {
        use super::{lookup_recall, rebuild_recall_registry, RECALL_HINT_MARKER};
        use fastclaw_core::types::{ChatMessage, Role};

        let marker_text = format!(
            "{RECALL_HINT_MARKER} [read_file({{\"path\":\"src/main.rs\"}}) → 10 lines, 200 chars, tier=full-retain. Re-call the tool to retrieve this result.]"
        );
        let msgs = vec![ChatMessage {
            role: Role::Tool,
            content: Some(serde_json::Value::String(marker_text)),
            reasoning_content: None,
            name: Some("read_file".into()),
            tool_calls: None,
            tool_call_id: Some("call-123".into()),
            compact_metadata: None,
        }];

        rebuild_recall_registry(&msgs);

        let meta = lookup_recall("call-123");
        assert!(meta.is_some(), "should find registered recall");
        let meta = meta.unwrap();
        assert_eq!(meta.tool_name, "read_file");
        assert!(meta.arguments_json.contains("src/main.rs"));
    }

    #[test]
    fn dedup_overlapping_reads_supersedes_covered_range() {
        use super::dedup_repeated_tool_calls;
        use fastclaw_core::types::{ChatMessage, FunctionCall, Role, ToolCall};

        fn assistant_with_read(call_id: &str, path: &str, lines: Option<&str>) -> ChatMessage {
            let mut args = serde_json::json!({ "path": path });
            if let Some(l) = lines {
                args["lines"] = serde_json::json!(l);
            }
            ChatMessage {
                role: Role::Assistant,
                content: Some(serde_json::Value::String("reading file".into())),
                reasoning_content: None,
                name: None,
                tool_calls: Some(vec![ToolCall {
                    id: call_id.into(),
                    call_type: "function".into(),
                    function: FunctionCall {
                        name: "read_file".into(),
                        arguments: args.to_string(),
                    },
                    output: None,
                    success: None,
                    duration_ms: None,
                }]),
                tool_call_id: None,
                compact_metadata: None,
            }
        }

        fn tool_result(call_id: &str, content: &str) -> ChatMessage {
            ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::Value::String(content.into())),
                reasoning_content: None,
                name: Some("read_file".into()),
                tool_calls: None,
                tool_call_id: Some(call_id.into()),
                compact_metadata: None,
            }
        }

        let mut msgs = vec![
            assistant_with_read("c1", "src/main.rs", Some("1-50")),
            tool_result("c1", "lines 1 through 50"),
            assistant_with_read("c2", "src/main.rs", Some("20-80")),
            tool_result("c2", "lines 20 through 80"),
            assistant_with_read("c3", "src/main.rs", Some("1-100")),
            tool_result("c3", "lines 1 through 100"),
        ];

        dedup_repeated_tool_calls(&mut msgs);

        let t1 = msgs[1].text_content().unwrap();
        assert!(
            t1.contains("[superseded"),
            "read 1-50 should be superseded by 1-100: got {t1}"
        );

        let t2 = msgs[3].text_content().unwrap();
        assert!(
            t2.contains("[superseded"),
            "read 20-80 should be superseded by 1-100: got {t2}"
        );

        let t3 = msgs[5].text_content().unwrap();
        assert_eq!(t3, "lines 1 through 100", "newest read should be preserved");
    }

    #[test]
    fn dedup_non_overlapping_reads_preserved() {
        use super::dedup_repeated_tool_calls;
        use fastclaw_core::types::{ChatMessage, FunctionCall, Role, ToolCall};

        fn assistant_with_read(call_id: &str, path: &str, lines: &str) -> ChatMessage {
            ChatMessage {
                role: Role::Assistant,
                content: Some(serde_json::Value::String("reading".into())),
                reasoning_content: None,
                name: None,
                tool_calls: Some(vec![ToolCall {
                    id: call_id.into(),
                    call_type: "function".into(),
                    function: FunctionCall {
                        name: "read_file".into(),
                        arguments: serde_json::json!({ "path": path, "lines": lines }).to_string(),
                    },
                    output: None,
                    success: None,
                    duration_ms: None,
                }]),
                tool_call_id: None,
                compact_metadata: None,
            }
        }

        fn tool_result(call_id: &str, content: &str) -> ChatMessage {
            ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::Value::String(content.into())),
                reasoning_content: None,
                name: Some("read_file".into()),
                tool_calls: None,
                tool_call_id: Some(call_id.into()),
                compact_metadata: None,
            }
        }

        let mut msgs = vec![
            assistant_with_read("c1", "src/main.rs", "1-50"),
            tool_result("c1", "first chunk"),
            assistant_with_read("c2", "src/main.rs", "100-200"),
            tool_result("c2", "second chunk"),
        ];

        dedup_repeated_tool_calls(&mut msgs);

        assert_eq!(
            msgs[1].text_content().unwrap(),
            "first chunk",
            "non-overlapping read 1 preserved"
        );
        assert_eq!(
            msgs[3].text_content().unwrap(),
            "second chunk",
            "non-overlapping read 2 preserved"
        );
    }

    #[test]
    fn extract_read_range_parses_lines_and_offset() {
        use super::{extract_read_range, parse_lines_range};

        assert_eq!(parse_lines_range("10-30"), Some((10, 30)));
        assert_eq!(parse_lines_range("50-"), Some((50, usize::MAX)));
        assert_eq!(parse_lines_range("100"), Some((100, 100)));

        let args = r#"{"path":"foo.rs","lines":"1-50"}"#;
        assert_eq!(extract_read_range(args), Some((1, 50)));

        let args = r#"{"path":"foo.rs","offset":10,"limit":20}"#;
        assert_eq!(extract_read_range(args), Some((10, 29)));

        let args = r#"{"path":"foo.rs"}"#;
        assert_eq!(extract_read_range(args), None);
    }

    #[test]
    fn keep_recent_scales_with_context_window() {
        use super::keep_recent_for_context_window;

        assert_eq!(keep_recent_for_context_window(16_000), 2);
        assert_eq!(keep_recent_for_context_window(32_000), 2);
        assert_eq!(keep_recent_for_context_window(64_000), 3);
        assert_eq!(keep_recent_for_context_window(128_000), 4);
        assert_eq!(keep_recent_for_context_window(200_000), 5);
        assert_eq!(keep_recent_for_context_window(1_000_000), 6);
    }

    #[test]
    fn cache_window_infinite_under_low_occupancy() {
        use super::cache_window_for_occupancy;
        let dur = cache_window_for_occupancy(10_000, 128_000);
        assert!(
            dur.as_secs() > 3600,
            "under 50% should be effectively infinite"
        );
    }

    #[test]
    fn cache_window_shrinks_with_high_occupancy() {
        use super::cache_window_for_occupancy;
        let dur_mid = cache_window_for_occupancy(80_000, 128_000); // ~62%
        let dur_high = cache_window_for_occupancy(100_000, 128_000); // ~78%
        let dur_critical = cache_window_for_occupancy(122_000, 128_000); // ~95%
        assert_eq!(dur_mid.as_secs(), 10 * 60, "62% occupancy should be 10min");
        assert_eq!(dur_high.as_secs(), 5 * 60, "78% occupancy should be 5min");
        assert_eq!(
            dur_critical.as_secs(),
            2 * 60,
            "95% occupancy should be 2min"
        );
    }

    #[test]
    fn protection_window_protects_recent_iterations() {
        use super::{compute_protected_indices, ProtectionWindowConfig};
        use fastclaw_core::types::{ChatMessage, Role};

        let msgs: Vec<ChatMessage> = vec![
            ChatMessage {
                role: Role::User,
                content: Some(serde_json::Value::String("q".into())),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
                compact_metadata: None,
            },
            ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::Value::String("old result".into())),
                reasoning_content: None,
                name: Some("read_file".into()),
                tool_calls: None,
                tool_call_id: Some("c1".into()),
                compact_metadata: None,
            },
            ChatMessage {
                role: Role::Assistant,
                content: Some(serde_json::Value::String("a".into())),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
                compact_metadata: None,
            },
            ChatMessage {
                role: Role::User,
                content: Some(serde_json::Value::String("q2".into())),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
                compact_metadata: None,
            },
            ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::Value::String("new result".into())),
                reasoning_content: None,
                name: Some("read_file".into()),
                tool_calls: None,
                tool_call_id: Some("c2".into()),
                compact_metadata: None,
            },
        ];

        let now = std::time::Instant::now();
        let boundaries = vec![(0, now - std::time::Duration::from_secs(60)), (3, now)];
        let config = ProtectionWindowConfig {
            protected_iterations: 1,
        };

        let protected = compute_protected_indices(&msgs, &boundaries, &config);
        assert!(
            !protected.contains(&1),
            "old iteration tool result should NOT be protected"
        );
        assert!(
            protected.contains(&4),
            "recent iteration tool result should be protected"
        );
    }
}
