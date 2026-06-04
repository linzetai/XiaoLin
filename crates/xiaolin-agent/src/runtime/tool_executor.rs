
use xiaolin_core::agent_config::AgentConfig;
use xiaolin_core::tool::ToolDefinition;
use serde::Serialize;


use super::prompt_builder::memory_tool_suffix;
use super::tool_result_storage::TOOL_RESULT_CLEARED_MESSAGE;


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
    let dir = std::env::temp_dir().join("xiaolin_truncated");
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
///
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
    messages: &[xiaolin_core::types::ChatMessage],
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
    messages: &[xiaolin_core::types::ChatMessage],
) -> Vec<(usize, String, String, Option<String>)> {
    use xiaolin_core::types::Role;

    messages
        .iter()
        .enumerate()
        .filter(|(_, m)| matches!(m.role, Role::Tool))
        .filter_map(|(i, m)| {
            let name = m.name.as_deref()?.to_string();
            let content = m.text_content()?.into_owned();
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
    messages: &mut [xiaolin_core::types::ChatMessage],
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
    messages: &mut [xiaolin_core::types::ChatMessage],
    keep_recent: usize,
    protected: &std::collections::HashSet<usize>,
) {
    use xiaolin_core::types::Role;

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
    messages: &[xiaolin_core::types::ChatMessage],
    iteration_boundaries: &[(usize, std::time::Instant)],
    config: &ProtectionWindowConfig,
) -> std::collections::HashSet<usize> {
    use xiaolin_core::types::Role;

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
    messages: &mut [xiaolin_core::types::ChatMessage],
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
    messages: &mut [xiaolin_core::types::ChatMessage],
    iteration_boundaries: &[(usize, std::time::Instant)],
    cache_window: std::time::Duration,
    protected: &std::collections::HashSet<usize>,
) -> usize {
    use xiaolin_core::types::Role;

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
    messages: &[xiaolin_core::types::ChatMessage],
    tool_msg_idx: usize,
    original_text: &str,
    _tier: RetentionTier,
) -> String {
    use xiaolin_core::types::Role;
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

pub(crate) fn dedup_repeated_tool_calls(messages: &mut [xiaolin_core::types::ChatMessage]) {
    use xiaolin_core::types::Role;
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
    messages: &mut [xiaolin_core::types::ChatMessage],
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
    messages: &mut [xiaolin_core::types::ChatMessage],
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
            config.behavior.is_tool_allowed(name)
        })
        .cloned()
        .collect()
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
pub(crate) fn rebuild_recall_registry(messages: &[xiaolin_core::types::ChatMessage]) {
    use xiaolin_core::types::Role;

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

