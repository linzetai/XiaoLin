use xiaolin_core::types::{FunctionCall, ToolCall};

/// Accumulates streaming tool call deltas into a complete tool call.
pub(crate) struct ToolCallAccumulator {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) arguments: String,
}

impl ToolCallAccumulator {
    pub(crate) fn to_tool_calls(&self) -> Vec<ToolCall> {
        let trimmed = self.arguments.trim();

        if let Some(objects) = try_split_concatenated_json(trimmed) {
            match try_merge_json_objects(&objects) {
                MergeResult::Merged(merged) => {
                    tracing::debug!(
                        tool = %self.name,
                        fragment_count = objects.len(),
                        "merged fragmented tool call arguments into single call"
                    );
                    return vec![ToolCall {
                        id: self.id.clone(),
                        call_type: "function".to_string(),
                        function: FunctionCall {
                            name: self.name.clone(),
                            arguments: merged,
                        },
                        output: None,
                        success: None,
                        duration_ms: None,
                    }];
                }
                MergeResult::Split(split) => {
                    tracing::info!(
                        tool = %self.name,
                        count = split.len(),
                        "split concatenated tool call arguments into separate calls"
                    );
                    return split
                        .into_iter()
                        .enumerate()
                        .map(|(i, args)| ToolCall {
                            id: if i == 0 {
                                self.id.clone()
                            } else {
                                format!("{}_split_{}", self.id, i)
                            },
                            call_type: "function".to_string(),
                            function: FunctionCall {
                                name: self.name.clone(),
                                arguments: args,
                            },
                            output: None,
                            success: None,
                            duration_ms: None,
                        })
                        .collect();
                }
            }
        }

        let arguments = ensure_json_arguments(trimmed);
        vec![ToolCall {
            id: self.id.clone(),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: self.name.clone(),
                arguments,
            },
            output: None,
            success: None,
            duration_ms: None,
        }]
    }
}

enum MergeResult {
    /// All fragments had distinct keys and were merged into a single JSON object.
    Merged(String),
    /// Key conflicts detected — keep as separate tool calls.
    Split(Vec<String>),
}

/// Attempt to merge multiple JSON objects into one.
/// If all keys across objects are distinct, merge into a single object.
/// If any key appears more than once, return them as separate calls.
fn try_merge_json_objects(objects: &[String]) -> MergeResult {
    let mut merged = serde_json::Map::new();

    for obj_str in objects {
        let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(obj_str)
        else {
            return MergeResult::Split(objects.to_vec());
        };

        for (key, value) in map {
            if merged.contains_key(&key) {
                return MergeResult::Split(objects.to_vec());
            }
            merged.insert(key, value);
        }
    }

    match serde_json::to_string(&serde_json::Value::Object(merged)) {
        Ok(s) => MergeResult::Merged(s),
        Err(_) => MergeResult::Split(objects.to_vec()),
    }
}

/// Detect and split concatenated JSON objects like `{"cmd":"a"}{"cmd":"b"}`.
/// Returns None if the input is a single valid JSON or not splittable.
fn try_split_concatenated_json(s: &str) -> Option<Vec<String>> {
    if s.is_empty() || !s.starts_with('{') {
        return None;
    }
    if serde_json::from_str::<serde_json::Value>(s).is_ok() {
        return None;
    }

    let mut objects = Vec::new();
    let mut chars = s.chars().peekable();

    while chars.peek().is_some() {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        if chars.peek() != Some(&'{') {
            break;
        }

        let mut depth = 0i32;
        let mut in_str = false;
        let mut escape = false;
        let mut obj = String::new();

        for ch in chars.by_ref() {
            obj.push(ch);
            if escape {
                escape = false;
                continue;
            }
            if ch == '\\' && in_str {
                escape = true;
                continue;
            }
            if ch == '"' {
                in_str = !in_str;
                continue;
            }
            if in_str {
                continue;
            }
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }

        if depth == 0 && serde_json::from_str::<serde_json::Value>(&obj).is_ok() {
            objects.push(obj);
        } else {
            return None;
        }
    }

    if objects.len() >= 2 {
        Some(objects)
    } else {
        None
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

    #[test]
    fn split_concatenated_json_objects() {
        let input = r#"{"command": "ls", "description": "list"}{"command": "pwd", "description": "show cwd"}"#;
        let result = try_split_concatenated_json(input);
        assert!(result.is_some());
        let parts = result.unwrap();
        assert_eq!(parts.len(), 2);
        assert!(parts[0].contains("\"ls\""));
        assert!(parts[1].contains("\"pwd\""));
    }

    #[test]
    fn split_three_concatenated_objects() {
        let input = r#"{"command": "a"}{"command": "b"}{"command": "c"}"#;
        let result = try_split_concatenated_json(input);
        assert!(result.is_some());
        assert_eq!(result.unwrap().len(), 3);
    }

    #[test]
    fn single_valid_json_not_split() {
        let input = r#"{"command": "ls -la", "description": "list all"}"#;
        let result = try_split_concatenated_json(input);
        assert!(result.is_none());
    }

    #[test]
    fn to_tool_calls_splits_on_key_conflict() {
        let acc = ToolCallAccumulator {
            id: "call_123".to_string(),
            name: "shell_exec".to_string(),
            arguments: r#"{"command": "ls"}{"command": "pwd"}"#.to_string(),
        };
        let calls = acc.to_tool_calls();
        assert_eq!(calls.len(), 2, "key conflict → split into separate calls");
        assert_eq!(calls[0].id, "call_123");
        assert_eq!(calls[1].id, "call_123_split_1");
        assert!(calls[0].function.arguments.contains("\"ls\""));
        assert!(calls[1].function.arguments.contains("\"pwd\""));
    }

    #[test]
    fn to_tool_calls_merges_fragmented_args() {
        let acc = ToolCallAccumulator {
            id: "call_789".to_string(),
            name: "write_file".to_string(),
            arguments: r##"{"file_path": "/tmp/test.md"}{"content": "# Hello"}"##.to_string(),
        };
        let calls = acc.to_tool_calls();
        assert_eq!(calls.len(), 1, "distinct keys → merge into single call");
        assert_eq!(calls[0].id, "call_789");
        let args: serde_json::Value = serde_json::from_str(&calls[0].function.arguments).unwrap();
        assert_eq!(args["file_path"], "/tmp/test.md");
        assert_eq!(args["content"].as_str().unwrap(), "# Hello");
    }

    #[test]
    fn to_tool_calls_merges_three_fragments() {
        let acc = ToolCallAccumulator {
            id: "call_abc".to_string(),
            name: "write_file".to_string(),
            arguments: r#"{"file_path": "/tmp/x"}{"content": "data"}{"description": "test"}"#
                .to_string(),
        };
        let calls = acc.to_tool_calls();
        assert_eq!(calls.len(), 1, "three distinct-key fragments → merge");
        let args: serde_json::Value = serde_json::from_str(&calls[0].function.arguments).unwrap();
        assert_eq!(args["file_path"], "/tmp/x");
        assert_eq!(args["content"], "data");
        assert_eq!(args["description"], "test");
    }

    #[test]
    fn to_tool_calls_single_normal_call() {
        let acc = ToolCallAccumulator {
            id: "call_456".to_string(),
            name: "shell_exec".to_string(),
            arguments: r#"{"command": "echo hello"}"#.to_string(),
        };
        let calls = acc.to_tool_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_456");
    }

    #[test]
    fn split_handles_nested_braces_in_strings() {
        let input = r#"{"command": "echo '{hello}'"} {"command": "cat /tmp/x"}"#;
        let result = try_split_concatenated_json(input);
        assert!(result.is_some());
        let parts = result.unwrap();
        assert_eq!(parts.len(), 2);
    }

    #[test]
    fn merge_result_with_partial_key_overlap() {
        let acc = ToolCallAccumulator {
            id: "call_overlap".to_string(),
            name: "test_tool".to_string(),
            arguments: r#"{"a": 1, "b": 2}{"b": 3, "c": 4}"#.to_string(),
        };
        let calls = acc.to_tool_calls();
        assert_eq!(calls.len(), 2, "partial key overlap → split");
    }
}
