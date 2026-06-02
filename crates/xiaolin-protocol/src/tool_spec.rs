use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[cfg(feature = "ts")]
use ts_rs::TS;

/// Categorizes a tool by the nature of its operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum ToolKind {
    Read,
    Search,
    Fetch,
    Edit,
    Execute,
    Think,
    Other,
}

impl ToolKind {
    #[deprecated(note = "use Tool::supports_parallel() instead for per-tool concurrency control")]
    pub fn is_concurrency_safe(&self) -> bool {
        matches!(self, Self::Read | Self::Search | Self::Fetch | Self::Think)
    }
}

/// JSON Schema describing a tool's parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ToolParameterSchema {
    #[serde(rename = "type")]
    pub schema_type: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, Record<string, unknown>>"))]
    pub properties: HashMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<String>,
}

/// OpenAI-compatible tool definition sent to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDefinition,
}

/// Definition of a callable function within a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct FunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: ToolParameterSchema,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(deprecated)]
    fn tool_kind_concurrency() {
        assert!(ToolKind::Read.is_concurrency_safe());
        assert!(ToolKind::Search.is_concurrency_safe());
        assert!(!ToolKind::Edit.is_concurrency_safe());
        assert!(!ToolKind::Execute.is_concurrency_safe());
    }

    #[test]
    fn tool_definition_serde_roundtrip() {
        let def = ToolDefinition {
            tool_type: "function".into(),
            function: FunctionDefinition {
                name: "read_file".into(),
                description: "Read a file".into(),
                parameters: ToolParameterSchema {
                    schema_type: "object".into(),
                    properties: HashMap::new(),
                    required: vec![],
                },
            },
        };
        let json = serde_json::to_string(&def).unwrap();
        let back: ToolDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(back.function.name, "read_file");
    }
}
