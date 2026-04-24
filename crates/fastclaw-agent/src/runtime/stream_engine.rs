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
