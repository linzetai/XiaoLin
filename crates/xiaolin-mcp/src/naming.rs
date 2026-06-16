//! MCP tool naming utilities — centralized naming logic for MCP tool registration.
//!
//! Format: `mcp__{sanitized_server_id}__{sanitized_tool_name}`

pub const MCP_DELIMITER: &str = "__";
pub const MCP_PREFIX: &str = "mcp";

/// Sanitize name for LLM API compatibility.
/// Replaces any character not in [a-zA-Z0-9_-] with '_'.
pub fn sanitize_for_api(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            result.push(c);
        } else {
            result.push('_');
        }
    }
    if result.is_empty() {
        "_".to_string()
    } else {
        result
    }
}

/// Build the full MCP prefix for a server: `mcp__{sanitized_id}__`
pub fn mcp_server_prefix(server_id: &str) -> String {
    format!(
        "{}{}{}{}{}",
        MCP_PREFIX,
        MCP_DELIMITER,
        sanitize_for_api(server_id),
        MCP_DELIMITER,
        ""
    )
}

/// Build fully qualified MCP tool name: `mcp__{server_id}__{tool_name}`
pub fn mcp_tool_name(server_id: &str, tool_name: &str) -> String {
    format!(
        "{}{}",
        mcp_server_prefix(server_id),
        sanitize_for_api(tool_name)
    )
}

/// Parse a fully qualified MCP tool name back into (server_id, tool_name).
/// Returns None if the name doesn't match the MCP format.
pub fn parse_mcp_tool_name(full_name: &str) -> Option<(&str, &str)> {
    let rest = full_name.strip_prefix("mcp__")?;
    let idx = rest.find("__")?;
    let server_id = &rest[..idx];
    let tool_name = &rest[idx + 2..];
    if server_id.is_empty() || tool_name.is_empty() {
        return None;
    }
    Some((server_id, tool_name))
}

/// Check if a name is an MCP tool name
pub fn is_mcp_tool(name: &str) -> bool {
    name.starts_with("mcp__")
}

/// Extract the server ID from a prefix like `mcp__serverid__`.
pub fn parse_server_id_from_prefix(prefix: &str) -> Option<&str> {
    let rest = prefix.strip_prefix("mcp__")?;
    let id = rest.strip_suffix("__")?;
    if id.is_empty() { None } else { Some(id) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_preserves_valid_chars() {
        assert_eq!(sanitize_for_api("hello_world-123"), "hello_world-123");
    }

    #[test]
    fn sanitize_replaces_invalid_chars() {
        assert_eq!(sanitize_for_api("my.server/v2"), "my_server_v2");
        assert_eq!(sanitize_for_api("name with spaces"), "name_with_spaces");
    }

    #[test]
    fn sanitize_empty_string() {
        assert_eq!(sanitize_for_api(""), "_");
    }

    #[test]
    fn mcp_server_prefix_format() {
        assert_eq!(mcp_server_prefix("chrome-devtools"), "mcp__chrome-devtools__");
        assert_eq!(mcp_server_prefix("my.server"), "mcp__my_server__");
    }

    #[test]
    fn mcp_tool_name_format() {
        assert_eq!(
            mcp_tool_name("chrome-devtools", "read_console"),
            "mcp__chrome-devtools__read_console"
        );
    }

    #[test]
    fn parse_roundtrip() {
        let full = mcp_tool_name("server", "tool");
        let (s, t) = parse_mcp_tool_name(&full).unwrap();
        assert_eq!(s, "server");
        assert_eq!(t, "tool");
    }

    #[test]
    fn parse_with_underscores_in_ids() {
        let full = "mcp__chrome_devtools__read_console";
        let (s, t) = parse_mcp_tool_name(full).unwrap();
        assert_eq!(s, "chrome_devtools");
        assert_eq!(t, "read_console");
    }

    #[test]
    fn parse_rejects_old_format() {
        assert!(parse_mcp_tool_name("mcp_server_tool").is_none());
    }

    #[test]
    fn is_mcp_tool_checks() {
        assert!(is_mcp_tool("mcp__server__tool"));
        assert!(!is_mcp_tool("mcp_server_tool"));
        assert!(!is_mcp_tool("read_file"));
    }
}
