use std::time::Duration;

use fastclaw_core::types::StreamEvent;
use fastclaw_self_iter::ToolCallTrace;

/// Mutable state tracked across iterations of the tool-calling loop.
pub(crate) struct LoopState {
    pub(crate) total_tool_calls: u32,
    pub(crate) consecutive_errors: u32,
    pub(crate) iteration: u32,
    pub(crate) failure_streak_traces: Vec<ToolCallTrace>,
    pub(crate) self_iter_recovery_used: u32,
    pub(crate) error_limit_reached: bool,
    /// When true, the agent is in a "grace" turn: one final LLM call to explain
    /// failures to the user before hard-stopping.
    pub(crate) grace_turn_active: bool,
    pub(crate) grace_turn_used: bool,
}

impl LoopState {
    pub(crate) fn new() -> Self {
        Self {
            total_tool_calls: 0,
            consecutive_errors: 0,
            iteration: 0,
            failure_streak_traces: Vec::new(),
            self_iter_recovery_used: 0,
            error_limit_reached: false,
            grace_turn_active: false,
            grace_turn_used: false,
        }
    }

    pub(crate) fn record_tool_error(&mut self, tool_name: &str, error_output: &str) {
        self.consecutive_errors += 1;
        self.failure_streak_traces.push(ToolCallTrace {
            tool_name: tool_name.to_string(),
            success: false,
            latency_ms: 0,
            error: Some(error_output.to_string()),
        });
    }

    pub(crate) fn clear_error_streak(&mut self) {
        self.consecutive_errors = 0;
        self.failure_streak_traces.clear();
    }

    /// Build a summary of the failing tools for the grace-turn guidance message.
    pub(crate) fn format_failure_summary(&self) -> String {
        if self.failure_streak_traces.is_empty() {
            return String::new();
        }
        let mut lines = Vec::new();
        for (i, trace) in self.failure_streak_traces.iter().enumerate() {
            let err_msg = trace
                .error
                .as_deref()
                .unwrap_or("unknown error");
            let truncated = if err_msg.len() > 200 {
                let end = err_msg.floor_char_boundary(200);
                format!("{}...", &err_msg[..end])
            } else {
                err_msg.to_string()
            };
            lines.push(format!("  {}. `{}`: {}", i + 1, trace.tool_name, truncated));
        }
        lines.join("\n")
    }
}

pub(crate) async fn send_stream_event(
    tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ev: StreamEvent,
    lossy: bool,
) -> bool {
    let dur = if lossy {
        Duration::from_millis(200)
    } else {
        Duration::from_secs(30)
    };
    match tokio::time::timeout(dur, tx.send(ev)).await {
        Ok(Ok(())) => true,
        Ok(Err(_)) => false,
        Err(_) => {
            if lossy {
                tracing::warn!("stream sink slow: dropped a token delta (backpressure)");
            } else {
                tracing::warn!("stream sink slow: timed out sending control event");
            }
            false
        }
    }
}
