use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use xiaolin_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolRegistry, ToolResult};

/// Searches deferred tools by keyword or activates them by name.
///
/// Input modes:
/// - `{"query": "keyword"}` — fuzzy search deferred tools
/// - `{"query": "select:tool_name"}` — activate the named tool
pub struct ToolSearchTool {
    registry: Arc<ToolRegistry>,
}

impl ToolSearchTool {
    pub fn new(registry: Arc<ToolRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for ToolSearchTool {
    fn supports_parallel(&self) -> bool {
        true
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Think
    }

    fn name(&self) -> &str {
        "tool_search"
    }

    fn description(&self) -> &str {
        "Search for additional tools by keyword, or activate a specific tool. \
         Input: {\"query\": \"keyword\"} to search, or {\"query\": \"select:tool_name\"} to activate. \
         Returns matching deferred tools or activation confirmation."
    }

    fn prompt(&self) -> String {
        "Search for or activate additional tools from the deferred pool.\n\n\
## Two Modes\n\
1. **Search**: Pass a keyword to find relevant deferred tools\n\
   - Matches against tool name, description, and search hints\n\
   - Returns a list of matching tools with descriptions\n\
2. **Select**: Pass 'select:tool_name' to activate a specific deferred tool\n\
   - The tool becomes available for immediate use\n\
   - Use this after finding the right tool via search\n\n\
## When to Use\n\
- You need a capability not in the current eager tool set\n\
- The task requires a specialized tool (e.g., MCP, WASM plugin)\n\
- You're unsure if a tool exists for a particular task\n\n\
## Search Strategy\n\
- Use broad keywords first: 'database', 'http', 'git'\n\
- Refine if too many results: 'postgres migration'\n\
- Search by capability, not tool name: 'send email' not 'smtp_tool'\n\n\
## After Activation\n\
- The activated tool appears in your available tools immediately\n\
- You can call it in the same turn\n\
- Don't re-search for tools you've already activated"
            .to_string()
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "query".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Search keyword, or 'select:tool_name' to activate a deferred tool."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["query".to_string()],
        }
    }

    fn is_deferred(&self) -> bool {
        false
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::err(format!(
                    "tool_search arguments are not valid JSON: {e}. \
                     Pass {{\"query\": \"keyword\"}}."
                ))
            }
        };

        let query = match args.get("query").and_then(|v| v.as_str()) {
            Some(q) => q,
            None => {
                return ToolResult::err(
                    "tool_search is missing required string field 'query'. \
                     Example: {\"query\": \"file search\"}."
                        .to_string(),
                )
            }
        };

        if let Some(tool_name) = query.strip_prefix("select:") {
            let tool_name = tool_name.trim();
            if self.registry.activate_deferred(tool_name) {
                return ToolResult::ok(format!(
                    "{{\"activated\": true, \"tool\": \"{tool_name}\", \
                     \"message\": \"Tool '{tool_name}' is now available.\"}}"
                ));
            } else {
                return ToolResult::err(format!(
                    "Tool '{tool_name}' not found in deferred tools. \
                     Use a plain query to search available tools first."
                ));
            }
        }

        let matches = self.registry.search_deferred(query);
        let total_deferred = self.registry.deferred_count();

        let results: Vec<serde_json::Value> = matches
            .iter()
            .map(|def| {
                serde_json::json!({
                    "name": def.function.name,
                    "description": def.function.description,
                })
            })
            .collect();

        ToolResult::ok(
            serde_json::json!({
                "matches": results,
                "match_count": matches.len(),
                "total_deferred_tools": total_deferred,
            })
            .to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeTool {
        name_str: &'static str,
        desc: &'static str,
        hint: &'static str,
    }

    #[async_trait]
    impl Tool for FakeTool {
        fn name(&self) -> &str {
            self.name_str
        }
        fn description(&self) -> &str {
            self.desc
        }
        fn parameters_schema(&self) -> ToolParameterSchema {
            ToolParameterSchema {
                schema_type: "object".into(),
                properties: HashMap::new(),
                required: vec![],
            }
        }
        fn search_hint(&self) -> &str {
            self.hint
        }
        async fn execute(&self, _: &str) -> ToolResult {
            ToolResult::ok("ok")
        }
    }

    fn setup() -> (Arc<ToolRegistry>, ToolSearchTool) {
        let reg = Arc::new(ToolRegistry::new());
        reg.register_deferred(Arc::new(FakeTool {
            name_str: "web_fetch",
            desc: "Fetch a URL",
            hint: "http download curl",
        }));
        reg.register_deferred(Arc::new(FakeTool {
            name_str: "grep_tool",
            desc: "Search files with regex",
            hint: "ripgrep rg",
        }));
        let tool = ToolSearchTool::new(reg.clone());
        (reg, tool)
    }

    #[tokio::test]
    async fn search_returns_matching_tools() {
        let (_, tool) = setup();
        let result = tool.execute(r#"{"query": "http"}"#).await;
        assert!(result.success);
        let v: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["match_count"], 1);
        assert_eq!(v["matches"][0]["name"], "web_fetch");
        assert_eq!(v["total_deferred_tools"], 2);
    }

    #[tokio::test]
    async fn search_no_match_returns_empty() {
        let (_, tool) = setup();
        let result = tool.execute(r#"{"query": "nonexistent"}"#).await;
        assert!(result.success);
        let v: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["match_count"], 0);
        assert_eq!(v["total_deferred_tools"], 2);
    }

    #[tokio::test]
    async fn select_activates_deferred_tool() {
        let (reg, tool) = setup();
        assert_eq!(reg.eager_definitions().len(), 0);

        let result = tool.execute(r#"{"query": "select:web_fetch"}"#).await;
        assert!(result.success);
        assert!(result.output.contains("activated"));

        assert_eq!(reg.eager_definitions().len(), 1);
        assert_eq!(reg.deferred_count(), 1);
    }

    #[tokio::test]
    async fn select_nonexistent_returns_error() {
        let (_, tool) = setup();
        let result = tool.execute(r#"{"query": "select:nope"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("not found"));
    }

    #[tokio::test]
    async fn tool_search_is_eager() {
        let (_, tool) = setup();
        assert!(!tool.is_deferred());
    }

    #[tokio::test]
    async fn missing_query_field() {
        let (_, tool) = setup();
        let result = tool.execute(r#"{}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("missing"));
    }

    #[tokio::test]
    async fn invalid_json_returns_error() {
        let (_, tool) = setup();
        let result = tool.execute("not json").await;
        assert!(!result.success);
        assert!(result.output.contains("not valid JSON"));
    }

    #[tokio::test]
    async fn search_matches_by_description() {
        let (_, tool) = setup();
        let result = tool.execute(r#"{"query": "regex"}"#).await;
        assert!(result.success);
        let v: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["match_count"], 1);
        assert_eq!(v["matches"][0]["name"], "grep_tool");
    }

    #[tokio::test]
    async fn activate_reduces_deferred_count() {
        let (reg, tool) = setup();
        assert_eq!(reg.deferred_count(), 2);

        tool.execute(r#"{"query": "select:grep_tool"}"#).await;
        assert_eq!(reg.deferred_count(), 1);

        let result = tool.execute(r#"{"query": "regex"}"#).await;
        let v: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["match_count"], 0);
        assert_eq!(v["total_deferred_tools"], 1);
    }

    #[tokio::test]
    async fn search_matches_multiple_results() {
        let reg = Arc::new(ToolRegistry::new());
        reg.register_deferred(Arc::new(FakeTool {
            name_str: "file_read",
            desc: "Read a file from disk",
            hint: "fs file",
        }));
        reg.register_deferred(Arc::new(FakeTool {
            name_str: "file_write",
            desc: "Write a file to disk",
            hint: "fs file",
        }));
        let tool = ToolSearchTool::new(reg.clone());

        let result = tool.execute(r#"{"query": "file"}"#).await;
        assert!(result.success);
        let v: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["match_count"], 2);
    }
}
