use fastclaw_core::types::{FunctionCall, ToolCall};

/// Accumulates streaming tool call deltas into a complete tool call.
pub(crate) struct ToolCallAccumulator {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) arguments: String,
}

impl ToolCallAccumulator {
    pub(crate) fn to_tool_call(&self) -> ToolCall {
        ToolCall {
            id: self.id.clone(),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: self.name.clone(),
                arguments: self.arguments.clone(),
            },
            output: None,
            success: None,
            duration_ms: None,
        }
    }
}

pub(crate) fn accumulate_tool_call(
    accum: &mut Vec<ToolCallAccumulator>,
    delta: &fastclaw_core::types::StreamToolCallDelta,
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
