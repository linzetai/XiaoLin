use std::sync::Arc;

use fastclaw_core::agent_config::AgentConfig;
use fastclaw_core::agent_config::BehaviorConfig;
use fastclaw_core::tool::{PostToolInfo, ToolDefinition, ToolHook, ToolHookContext, ToolRegistry};
use fastclaw_core::types::ToolCall;

use crate::builtin_tools::{with_file_access_mode, with_work_dir};

use super::prompt_builder::memory_tool_suffix;

/// Default max characters of tool output embedded in chat history.
/// Individual tools override this via `tool_output_char_limit`.
pub const MAX_TOOL_RESULT_CHARS: usize = 2000;

/// Per-tool character limits.
/// Tools that produce large but low-density output get tighter limits;
/// tools whose output is highly actionable get more room.
fn tool_output_char_limit(tool_name: &str) -> usize {
    match tool_name {
        // Write/edit tools: the LLM already knows what it wrote; keep only the status.
        "write_file" | "edit_file" | "apply_patch" | "create_directory" => 300,

        // Search tools: only the first few matches matter; rest can be re-searched.
        "grep" | "ripgrep" | "glob" | "web_search" => 1200,

        // Directory listing: compact structure.
        "list_dir" | "list_directory" => 800,

        // File reading: key content, but agent can re-read specific sections.
        "read_file" => 1500,

        // Shell execution: tail-heavy (errors/results at end).
        "shell_exec" | "shell" | "run_command" => 1500,

        // Web fetch: often huge HTML; extract only key text.
        "web_fetch" | "fetch_url" => 2000,

        // Memory/todo tools are already compact.
        "memory_store" | "memory_search" | "todo_write" => 1500,

        // Default: use the global limit.
        _ => MAX_TOOL_RESULT_CHARS,
    }
}

/// Per-tool line limits (same principle).
fn tool_output_line_limit(tool_name: &str) -> usize {
    match tool_name {
        "write_file" | "edit_file" | "apply_patch" | "create_directory" => 15,
        "grep" | "ripgrep" | "glob" | "web_search" => 60,
        "list_dir" | "list_directory" => 40,
        _ => MAX_TOOL_RESULT_LINES,
    }
}

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
const MAX_TOOL_RESULT_LINES: usize = 200;

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
pub(crate) fn truncate_tool_result_output(output: &str, tool_name: &str) -> String {
    let char_limit = tool_output_char_limit(tool_name);
    let line_limit = tool_output_line_limit(tool_name);

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

    let tail_budget = char_limit.saturating_sub(head_used).saturating_sub(TRUNCATION_SEPARATOR.len());
    let mut tail_parts: Vec<String> = Vec::new();
    let mut tail_used = 0usize;
    let tail_start = total_lines.saturating_sub(tail_line_count).max(head_parts.len());
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
const COMPACTABLE_TOOLS: &[&str] = &[
    "read_file", "shell_exec", "shell", "grep", "glob", "web_search",
    "web_fetch", "write_file", "edit_file", "list_dir", "list_directory",
    "run_command", "ripgrep", "fetch_url",
];

/// Heuristic: does this tool result look like an error?
/// Error results are always preserved to prevent repeated mistakes.
fn is_error_tool_result(content: &str) -> bool {
    // If the content starts with a semantic header, check the header for ERR
    // and also check the body (after the first newline).
    let check_text = if content.starts_with(SEMANTIC_HEADER_MARKER) {
        if content.contains("→ ERR") {
            return true;
        }
        content.find('\n').map(|pos| &content[pos + 1..]).unwrap_or("")
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
fn one_liner_summary(tool_name: &str, content: &str) -> String {
    if content.starts_with(SEMANTIC_HEADER_MARKER) {
        if let Some(header_end) = content.find('\n') {
            return content[..header_end].to_string();
        }
        return content.to_string();
    }

    let line_count = content.lines().count();
    let char_count = content.len();
    let first_line = content.lines().next().unwrap_or("").chars().take(60).collect::<String>();

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
    format!("{}…\n[{} more chars faded]", &content[..safe_end], content.len() - safe_end)
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
        "write_file" | "edit_file" | "apply_patch" | "create_file" => {
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
            Some(format!("\"{}\" in {}", truncate_str(&pattern, 30), truncate_str(&path, 40)))
        }
        "web_search" => {
            extract_str(&v, &["query", "search_query"]).map(|s| format!("\"{}\"", truncate_str(&s, 50)))
        }
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

/// Progressive fading: tool results are kept at three tiers based on recency.
///
/// - **Tier 1** (most recent `full_keep` results): kept in full.
/// - **Tier 2** (next `preview_keep` results): faded to a short preview (~150 chars).
/// - **Tier 3** (all older): collapsed to a single-line summary.
///
/// Error results are always preserved regardless of age.
/// Results already faded/collapsed are not re-processed.
pub(crate) fn microcompact_tool_results(
    messages: &mut [fastclaw_core::types::ChatMessage],
    keep_recent: usize,
) {
    use fastclaw_core::types::Role;

    let full_keep = keep_recent.min(3);
    let preview_keep: usize = 3;

    let tool_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| matches!(m.role, Role::Tool))
        .filter(|(_, m)| {
            m.name
                .as_deref()
                .map(|n| COMPACTABLE_TOOLS.iter().any(|t| n.starts_with(t)))
                .unwrap_or(false)
        })
        .map(|(i, _)| i)
        .collect();

    let total = tool_indices.len();
    if total <= full_keep {
        return;
    }

    for (rank_from_end, &idx) in tool_indices.iter().rev().enumerate() {
        let msg = &mut messages[idx];
        let text = match msg.text_content() {
            Some(t) => t,
            None => continue,
        };

        if text.starts_with(ONELINER_MARKER) || text.starts_with(FADED_MARKER) {
            continue;
        }
        if is_error_tool_result(&text) {
            continue;
        }

        let tool_name = msg.name.clone().unwrap_or_default();

        if rank_from_end < full_keep {
            // Tier 1: keep fully
        } else if rank_from_end < full_keep + preview_keep {
            // Tier 2: fade to preview
            let faded = format!("{FADED_MARKER} {}", fade_to_preview(&text, 150));
            msg.content = Some(serde_json::Value::String(faded));
        } else {
            // Tier 3: collapse to one-liner
            let summary = format!("{ONELINER_MARKER} {}", one_liner_summary(&tool_name, &text));
            msg.content = Some(serde_json::Value::String(summary));
        }
    }
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
pub(crate) fn dedup_repeated_tool_calls(
    messages: &mut [fastclaw_core::types::ChatMessage],
) {
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
    // and extract a "target key" (e.g., file path or command)
    let mut target_map: HashMap<(String, String), Vec<usize>> = HashMap::new(); // (tool_name, target_key) -> [indices]

    for (call_id, tool_name, msg_idx) in &tool_entries {
        // Search backwards for the assistant message containing this tool_call
        let target_key = messages[..*msg_idx]
            .iter()
            .rev()
            .filter(|m| matches!(m.role, Role::Assistant))
            .find_map(|m| {
                m.tool_calls.as_ref()?.iter().find_map(|tc| {
                    if tc.id == *call_id {
                        extract_target_key(&tool_name, &tc.function.arguments)
                    } else {
                        None
                    }
                })
            });

        if let Some(key) = target_key {
            target_map.entry((tool_name.clone(), key)).or_default().push(*msg_idx);
        }
    }

    // For groups with >1 entry, replace all but the last with a short pointer
    for ((_tool_name, target_key), indices) in &target_map {
        if indices.len() <= 1 {
            continue;
        }
        // Keep the last (most recent) result, supersede the rest
        for &idx in &indices[..indices.len() - 1] {
            let msg = &mut messages[idx];
            if let Some(text) = msg.text_content() {
                if text.starts_with("[superseded") {
                    continue;
                }
                if is_error_tool_result(&text) {
                    continue;
                }
            }
            msg.content = Some(serde_json::Value::String(
                format!("[superseded: re-executed on \"{}\", see latest result below]",
                    if target_key.len() > 60 {
                        format!("{}…", &target_key[..57])
                    } else {
                        target_key.clone()
                    }
                ),
            ));
        }
    }
}

/// Extract a target key from tool arguments for deduplication.
fn extract_target_key(tool_name: &str, arguments: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(arguments).ok()?;
    match tool_name {
        "read_file" => v.get("path").or(v.get("file_path")).and_then(|p| p.as_str()).map(|s| s.to_string()),
        "shell_exec" | "shell" | "run_command" => {
            v.get("command").or(v.get("cmd")).and_then(|c| c.as_str()).map(|s| s.to_string())
        }
        _ => None,
    }
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

pub(crate) fn is_tool_allowed(tool_name: &str, behavior: &fastclaw_core::agent_config::BehaviorConfig) -> bool {
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
fn validate_tool_arguments(tool: &dyn fastclaw_core::tool::Tool, arguments: &str) -> Option<String> {
    let schema = tool.parameters_schema();
    if schema.required.is_empty() {
        return None;
    }

    let parsed: serde_json::Value = match serde_json::from_str(arguments) {
        Ok(v) => v,
        Err(e) => {
            return Some(format!(
                "Invalid JSON arguments for tool '{}': {}. Please provide valid JSON.",
                tool.name(), e
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
) -> Vec<ToolExecResult> {
    execute_tool_batch_with_hooks(tool_calls, tool_registry, behavior, work_dir, log_suffix, &[], "").await
}

pub(crate) async fn execute_tool_batch_with_hooks(
    tool_calls: &[ToolCall],
    tool_registry: &Arc<ToolRegistry>,
    behavior: &BehaviorConfig,
    work_dir: &Option<String>,
    log_suffix: &str,
    hooks: &[Arc<dyn ToolHook>],
    agent_id: &str,
) -> Vec<ToolExecResult> {
    execute_tool_batch_with_hooks_and_stream(
        tool_calls, tool_registry, behavior, work_dir, log_suffix, hooks, agent_id, None,
    ).await
}

pub(crate) async fn execute_tool_batch_with_hooks_and_stream(
    tool_calls: &[ToolCall],
    tool_registry: &Arc<ToolRegistry>,
    behavior: &BehaviorConfig,
    work_dir: &Option<String>,
    log_suffix: &str,
    hooks: &[Arc<dyn ToolHook>],
    agent_id: &str,
    stream_tx: Option<&tokio::sync::mpsc::Sender<fastclaw_core::types::StreamEvent>>,
) -> Vec<ToolExecResult> {
    // Batch-level dedup: when the same read_file path appears multiple times
    // in one batch, only execute the first and share the result.
    let mut read_file_seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
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
        let kind = tool_registry.get(&tc.function.name)
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
        let concurrent_futures: Vec<_> = concurrent_indices.iter().map(|&i| {
            execute_single_tool(
                &tool_calls[i], tool_registry, behavior, work_dir, log_suffix, hooks, agent_id, stream_tx,
            )
        }).collect();
        let concurrent_results = futures::future::join_all(concurrent_futures).await;
        for (slot, result) in concurrent_indices.iter().zip(concurrent_results) {
            results[*slot] = Some(result);
        }
    }

    for &i in &sequential_indices {
        let result = execute_single_tool(
            &tool_calls[i], tool_registry, behavior, work_dir, log_suffix, hooks, agent_id, stream_tx,
        ).await;
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

    results.into_iter().map(|r| r.expect("all slots filled")).collect()
}

async fn execute_single_tool(
    tc: &ToolCall,
    tool_registry: &Arc<ToolRegistry>,
    behavior: &BehaviorConfig,
    work_dir: &Option<String>,
    log_suffix: &str,
    hooks: &[Arc<dyn ToolHook>],
    agent_id: &str,
    stream_tx: Option<&tokio::sync::mpsc::Sender<fastclaw_core::types::StreamEvent>>,
) -> ToolExecResult {
    let tool_name = tc.function.name.clone();
    let call_id = tc.id.clone();
    let arguments = tc.function.arguments.clone();

    if !is_tool_allowed(&tool_name, behavior) {
        tracing::warn!(tool = %tool_name, "tool blocked by allow/deny policy{log_suffix}");
        let msg = format!("tool '{}' is not allowed by agent policy", tool_name);
        return (tool_name, call_id, arguments, fastclaw_core::tool::ToolResult::err(msg));
    }
    if behavior.requires_confirmation(&tool_name) {
        tracing::info!(tool = %tool_name, "tool requires user confirmation (tools_ask){log_suffix}");
        let result = fastclaw_core::tool::ToolResult::needs_confirm(
            format!("Tool '{}' requires user confirmation per agent policy.", tool_name),
        );
        return (tool_name, call_id, arguments, result);
    }

    let tool_kind = tool_registry.get(&tool_name)
        .map(|t| t.kind())
        .unwrap_or(fastclaw_core::tool::ToolKind::Other);

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
            return (tool_name, call_id, arguments, fastclaw_core::tool::ToolResult::err(err));
        }
    }

    let mut effective_args = arguments.clone();
    for hook in hooks {
        let action = hook.pre_tool_use(&hook_ctx).await;
        if let Some(reason) = action.block_reason {
            tracing::info!(tool = %tool_name, hook = hook.name(), "tool blocked by hook: {reason}");
            return (tool_name, call_id, arguments, fastclaw_core::tool::ToolResult::err(reason));
        }
        if let Some(new_args) = action.modified_arguments {
            effective_args = new_args;
        }
    }

    let t0 = std::time::Instant::now();
    let result = match tool_registry.get(&tool_name) {
        Some(tool) => {
            let work_dir_path = work_dir.as_ref().map(std::path::PathBuf::from);
            if tool.supports_progress() && stream_tx.is_some() {
                let stream_tx = stream_tx.unwrap().clone();
                let tn = tool_name.clone();
                let ci = call_id.clone();
                let (progress_tx, mut progress_rx) =
                    tokio::sync::mpsc::channel::<fastclaw_core::tool::ToolProgressUpdate>(32);

                let bridge = tokio::spawn(async move {
                    while let Some(update) = progress_rx.recv().await {
                        let event = fastclaw_core::types::StreamEvent::ToolProgress {
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
                    with_work_dir(work_dir_path, tool.execute_with_progress(&effective_args, progress_tx)),
                )
                .await;
                bridge.abort();
                res
            } else {
                with_file_access_mode(
                    behavior.file_access,
                    with_work_dir(work_dir_path, tool.execute(&effective_args)),
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
        let lines: Vec<String> = (0..300).map(|i| format!("line {i}: some data here")).collect();
        let input = lines.join("\n");
        let out = truncate_tool_result_output(&input, "test_tool");
        assert!(out.contains(TRUNCATION_SEPARATOR), "should contain truncation separator");
        assert!(out.contains("line 0:"), "should keep head lines");
        assert!(out.contains("line 299:"), "should keep tail lines");
        assert!(out.len() < input.len(), "output should be shorter than input");
        assert!(out.contains("saved to:"), "should include saved file path hint");
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
        use super::{microcompact_tool_results, ONELINER_MARKER, FADED_MARKER};
        use fastclaw_core::types::{ChatMessage, Role};

        // 10 tool results: indices 0..9, most recent = index 9
        let mut msgs: Vec<ChatMessage> = (0..10)
            .map(|i| ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::Value::String(format!("output {i}"))),
                name: Some("read_file".into()),
                tool_calls: None,
                tool_call_id: Some(format!("id-{i}")),
            })
            .collect();

        microcompact_tool_results(&mut msgs, 3);

        // Tier 3 (oldest, indices 0..4): collapsed to one-liner
        for msg in &msgs[..4] {
            let text = msg.text_content().unwrap();
            assert!(text.starts_with(ONELINER_MARKER), "expected oneliner, got: {text}");
        }
        // Tier 2 (indices 4..7): faded to preview
        for msg in &msgs[4..7] {
            let text = msg.text_content().unwrap();
            assert!(text.starts_with(FADED_MARKER), "expected faded, got: {text}");
        }
        // Tier 1 (most recent 3, indices 7..10): kept fully
        for msg in &msgs[7..] {
            assert!(msg.text_content().unwrap().starts_with("output"));
        }
    }

    #[test]
    fn microcompact_preserves_error_results() {
        use super::{microcompact_tool_results, FADED_MARKER, ONELINER_MARKER};
        use fastclaw_core::types::{ChatMessage, Role};

        let mut msgs: Vec<ChatMessage> = vec![
            ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::Value::String("Error: file not found".into())),
                name: Some("read_file".into()),
                tool_calls: None,
                tool_call_id: Some("id-0".into()),
            },
            ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::Value::String("success output 1".into())),
                name: Some("shell_exec".into()),
                tool_calls: None,
                tool_call_id: Some("id-1".into()),
            },
            ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::Value::String("Failed to connect".into())),
                name: Some("web_fetch".into()),
                tool_calls: None,
                tool_call_id: Some("id-2".into()),
            },
            ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::Value::String("success output 2".into())),
                name: Some("grep".into()),
                tool_calls: None,
                tool_call_id: Some("id-3".into()),
            },
            ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::Value::String("recent output".into())),
                name: Some("read_file".into()),
                tool_calls: None,
                tool_call_id: Some("id-4".into()),
            },
        ];

        microcompact_tool_results(&mut msgs, 1);

        // Error results preserved even though old
        assert!(msgs[0].text_content().unwrap().contains("Error: file not found"));
        // Non-error old results get faded or onelined (not full)
        let t1 = msgs[1].text_content().unwrap();
        assert!(
            t1.starts_with(FADED_MARKER) || t1.starts_with(ONELINER_MARKER),
            "expected faded/oneliner, got: {t1}"
        );
        // Error results preserved
        assert!(msgs[2].text_content().unwrap().contains("Failed to connect"));
        // Non-error older results get faded
        let t3 = msgs[3].text_content().unwrap();
        assert!(
            t3.starts_with(FADED_MARKER) || t3.starts_with(ONELINER_MARKER),
            "expected faded/oneliner, got: {t3}"
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
        assert!(!summary.contains("actual file content"), "should not include body");
    }
}
