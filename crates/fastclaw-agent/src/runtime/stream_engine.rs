use std::time::Duration;

use fastclaw_protocol::AgentEvent;

/// Trace of a single tool call (used by query_state.rs and self-iter).
/// When the `self-iter` feature is enabled, this re-exports from fastclaw_self_iter.
#[cfg(feature = "self-iter")]
pub(crate) use fastclaw_self_iter::ToolCallTrace;

#[cfg(not(feature = "self-iter"))]
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct ToolCallTrace {
    pub tool_name: String,
    pub success: bool,
    pub latency_ms: u64,
    pub error: Option<String>,
}

// LoopState has been replaced by QueryLoopState in query_state.rs.
// ToolCallTrace is still defined here for shared use.

pub(crate) async fn send_stream_event(
    tx: &tokio::sync::mpsc::Sender<AgentEvent>,
    ev: AgentEvent,
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
