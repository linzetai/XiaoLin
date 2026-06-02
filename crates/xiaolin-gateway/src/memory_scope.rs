/// Stable suffix for per-agent scoped memory tool names (`memory_search__{suffix}`).
pub(crate) fn memory_tool_agent_suffix(agent_id: &str) -> String {
    agent_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
