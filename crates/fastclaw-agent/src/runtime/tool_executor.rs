use std::sync::Arc;

use fastclaw_core::agent_config::AgentConfig;
use fastclaw_core::agent_config::BehaviorConfig;
use fastclaw_core::tool::{ToolDefinition, ToolRegistry};
use fastclaw_core::types::ToolCall;

use crate::builtin_tools::{with_file_access_mode, with_work_dir};

use super::prompt_builder::memory_tool_suffix;

/// Max characters of tool output embedded in chat history (per tool message).
pub const MAX_TOOL_RESULT_CHARS: usize = 8000;

pub(crate) fn truncate_tool_result_output(output: &str) -> String {
    let total = output.chars().count();
    if total <= MAX_TOOL_RESULT_CHARS {
        return output.to_string();
    }
    let head: String = output.chars().take(MAX_TOOL_RESULT_CHARS).collect();
    format!("{head}\n... (truncated, showing first {MAX_TOOL_RESULT_CHARS} of {total} chars)")
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
    use super::{truncate_tool_result_output, MAX_TOOL_RESULT_CHARS};

    #[test]
    fn no_truncation_at_or_below_char_limit() {
        let s = "a".repeat(MAX_TOOL_RESULT_CHARS);
        let out = truncate_tool_result_output(&s);
        assert_eq!(out, s);
        assert!(!out.contains("truncated"));
    }

    #[test]
    fn truncates_long_output_and_suffix_reports_total_chars() {
        let total = MAX_TOOL_RESULT_CHARS + 999;
        let s = "a".repeat(total);
        let out = truncate_tool_result_output(&s);
        let expected_suffix = format!(
            "\n... (truncated, showing first {MAX_TOOL_RESULT_CHARS} of {total} chars)"
        );
        assert!(out.ends_with(&expected_suffix), "got len {}", out.len());
        assert_eq!(
            out.chars().take(MAX_TOOL_RESULT_CHARS).collect::<String>(),
            "a".repeat(MAX_TOOL_RESULT_CHARS)
        );
    }
}
