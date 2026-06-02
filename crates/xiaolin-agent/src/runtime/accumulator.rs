use xiaolin_core::types::{FunctionCall, ToolCall};

/// Accumulates streaming tool call deltas into a complete tool call.
pub(crate) struct ToolCallAccumulator {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) arguments: String,
}

impl ToolCallAccumulator {
    pub(crate) fn to_tool_call(&self) -> ToolCall {
        let arguments = ensure_json_arguments(&self.arguments);
        ToolCall {
            id: self.id.clone(),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: self.name.clone(),
                arguments,
            },
            output: None,
            success: None,
            duration_ms: None,
        }
    }
}

/// Ensure `arguments` is valid JSON. Some LLM APIs (e.g. Qwen code models)
/// reject messages whose `function.arguments` is not valid JSON. This can
/// happen when the stream is truncated or the model outputs malformed JSON.
fn ensure_json_arguments(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "{}".to_string();
    }
    if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
        return trimmed.to_string();
    }
    tracing::debug!(
        raw_len = raw.len(),
        "repairing malformed tool call arguments"
    );
    repair_json(trimmed)
}

/// Best-effort repair of truncated JSON by closing open braces/brackets/strings.
fn repair_json(s: &str) -> String {
    let mut result = s.to_string();
    let mut in_string = false;
    let mut escape_next = false;
    let mut stack: Vec<char> = Vec::new();

    for ch in s.chars() {
        if escape_next {
            escape_next = false;
            continue;
        }
        if ch == '\\' && in_string {
            escape_next = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        match ch {
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '}' | ']' => {
                stack.pop();
            }
            _ => {}
        }
    }

    if in_string {
        result.push('"');
    }
    while let Some(closer) = stack.pop() {
        result.push(closer);
    }

    if serde_json::from_str::<serde_json::Value>(&result).is_ok() {
        result
    } else {
        format!(
            "{{\"_raw\":{}}}",
            serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
        )
    }
}

pub(crate) fn accumulate_tool_call(
    accum: &mut Vec<ToolCallAccumulator>,
    delta: &xiaolin_core::types::StreamToolCallDelta,
) {
    let idx = delta.index as usize;

    while accum.len() <= idx {
        accum.push(ToolCallAccumulator {
            id: String::new(),
            name: String::new(),
            arguments: String::new(),
        });
    }

    let entry = &mut accum[idx];

    if let Some(ref id) = delta.id {
        if !id.is_empty() {
            entry.id = id.clone();
        }
    }

    if let Some(ref func) = delta.function {
        if let Some(ref name) = func.name {
            if !name.is_empty() {
                entry.name = name.clone();
            }
        }
        if let Some(ref args) = func.arguments {
            entry.arguments.push_str(args);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_json_passes_through() {
        let input = r#"{"path": "/tmp/test.rs"}"#;
        assert_eq!(ensure_json_arguments(input), input);
    }

    #[test]
    fn empty_becomes_empty_object() {
        assert_eq!(ensure_json_arguments(""), "{}");
        assert_eq!(ensure_json_arguments("   "), "{}");
    }

    #[test]
    fn truncated_json_gets_repaired() {
        let input = r#"{"path": "/tmp/test.rs", "content": "hello"#;
        let repaired = ensure_json_arguments(input);
        assert!(
            serde_json::from_str::<serde_json::Value>(&repaired).is_ok(),
            "repaired JSON should be valid: {repaired}"
        );
    }

    #[test]
    fn unclosed_brace_gets_closed() {
        let input = r#"{"key": "value""#;
        let repaired = ensure_json_arguments(input);
        assert!(
            serde_json::from_str::<serde_json::Value>(&repaired).is_ok(),
            "repaired JSON should be valid: {repaired}"
        );
    }

    #[test]
    fn totally_invalid_falls_back() {
        let input = "not json at all";
        let repaired = ensure_json_arguments(input);
        assert!(
            serde_json::from_str::<serde_json::Value>(&repaired).is_ok(),
            "fallback JSON should be valid: {repaired}"
        );
    }
}
