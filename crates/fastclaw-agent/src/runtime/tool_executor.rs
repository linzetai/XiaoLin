use std::sync::Arc;

use fastclaw_core::agent_config::AgentConfig;
use fastclaw_core::agent_config::BehaviorConfig;
use fastclaw_core::tool::{ToolDefinition, ToolRegistry};
use fastclaw_core::types::ToolCall;

use crate::builtin_tools::{with_file_access_mode, with_work_dir};

use super::prompt_builder::memory_tool_suffix;

/// Max characters of tool output embedded in chat history (per tool message).
pub const MAX_TOOL_RESULT_CHARS: usize = 4000;

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
    let total_chars = output.chars().count();
    let lines: Vec<&str> = output.lines().collect();
    let total_lines = lines.len();

    if total_chars <= MAX_TOOL_RESULT_CHARS && total_lines <= MAX_TOOL_RESULT_LINES {
        return output.to_string();
    }

    let effective_lines = total_lines.min(MAX_TOOL_RESULT_LINES);
    let head_line_count = (effective_lines / 5).max(1);
    let tail_line_count = effective_lines - head_line_count;

    let head_budget = MAX_TOOL_RESULT_CHARS / 5;
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

    let tail_budget = MAX_TOOL_RESULT_CHARS.saturating_sub(head_used).saturating_sub(TRUNCATION_SEPARATOR.len());
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

const MICROCOMPACT_CLEARED: &str = "[Old tool result cleared to save context]";

/// Compactable tool names whose old results can be safely cleared.
const COMPACTABLE_TOOLS: &[&str] = &[
    "read_file", "shell_exec", "shell", "grep", "glob", "web_search",
    "web_fetch", "write_file", "edit_file", "list_dir",
];

/// Heuristic: does this tool result look like an error?
/// Preserving error results prevents the agent from repeating the same mistakes.
fn is_error_tool_result(content: &str) -> bool {
    let lower = content.to_lowercase();
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

/// Clear old tool result content from non-recent tool messages.
/// Keeps the last `keep_recent` tool results intact; older ones get replaced
/// with a short marker to save context tokens. Error results are never cleared
/// so the agent can learn from past mistakes.
pub(crate) fn microcompact_tool_results(
    messages: &mut [fastclaw_core::types::ChatMessage],
    keep_recent: usize,
) {
    use fastclaw_core::types::Role;

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

    if tool_indices.len() <= keep_recent {
        return;
    }

    let clear_count = tool_indices.len() - keep_recent;
    for &idx in tool_indices.iter().take(clear_count) {
        let msg = &mut messages[idx];
        if let Some(text) = msg.text_content() {
            if text == MICROCOMPACT_CLEARED {
                continue;
            }
            if is_error_tool_result(&text) {
                continue;
            }
            msg.content = Some(serde_json::Value::String(MICROCOMPACT_CLEARED.to_string()));
        }
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

type ToolExecResult = (String, String, String, fastclaw_core::tool::ToolResult);

/// Execute a batch of tool calls in parallel (fork-join).
pub(crate) async fn execute_tool_batch(
    tool_calls: &[ToolCall],
    tool_registry: &Arc<ToolRegistry>,
    behavior: &BehaviorConfig,
    work_dir: &Option<String>,
    log_suffix: &str,
) -> Vec<ToolExecResult> {
    let shared_registry = Arc::clone(tool_registry);
    let shared_behavior = Arc::new(behavior.clone());
    let futures: Vec<_> = tool_calls
        .iter()
        .map(|tc| {
            let tool_name = tc.function.name.clone();
            let call_id = tc.id.clone();
            let arguments = tc.function.arguments.clone();
            let registry = Arc::clone(&shared_registry);
            let behavior = Arc::clone(&shared_behavior);
            let work_dir = work_dir.clone();
            async move {
                if !is_tool_allowed(&tool_name, &behavior) {
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
                let result = match registry.get(&tool_name) {
                    Some(tool) => {
                        let work_dir_path = work_dir.as_ref().map(std::path::PathBuf::from);
                        with_file_access_mode(
                            behavior.file_access,
                            with_work_dir(work_dir_path, tool.execute(&arguments)),
                        )
                        .await
                    }
                    None => {
                        let msg = format!("tool not found: {}", tool_name);
                        fastclaw_core::tool::ToolResult::err(msg)
                    }
                };
                tracing::info!(
                    tool = %tool_name, success = result.success,
                    output_len = result.output.len(), "tool result{log_suffix}"
                );
                (tool_name, call_id, arguments, result)
            }
        })
        .collect();
    futures::future::join_all(futures).await
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
    fn microcompact_clears_old_tool_results() {
        use super::microcompact_tool_results;
        use fastclaw_core::types::{ChatMessage, Role};

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

        for msg in &msgs[..7] {
            assert_eq!(
                msg.text_content().as_deref(),
                Some("[Old tool result cleared to save context]"),
            );
        }
        for msg in &msgs[7..] {
            assert!(msg.text_content().unwrap().starts_with("output"));
        }
    }

    #[test]
    fn microcompact_preserves_error_results() {
        use super::microcompact_tool_results;
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

        // Error results should be preserved even though they're old
        assert!(msgs[0].text_content().unwrap().contains("Error: file not found"));
        // Successful old results should be cleared
        assert_eq!(msgs[1].text_content().as_deref(), Some("[Old tool result cleared to save context]"));
        // Error results should be preserved
        assert!(msgs[2].text_content().unwrap().contains("Failed to connect"));
        // Successful old results should be cleared
        assert_eq!(msgs[3].text_content().as_deref(), Some("[Old tool result cleared to save context]"));
        // Recent results should be preserved
        assert!(msgs[4].text_content().unwrap().contains("recent output"));
    }
}
