//! Structured execution events for DAG runs (observability, logging, tracing).

use serde::{Deserialize, Serialize};

/// One timestamped event emitted during a DAG run (for logging, metrics, or external buses).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionEvent {
    /// DAG or run identifier (from executor / context).
    pub dag_id: String,
    /// Structured payload for this milestone.
    pub event: EventKind,
    /// RFC3339 wall time when the event was emitted.
    pub timestamp: String,
}

/// Kind of execution milestone or failure.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    DagStarted {
        node_count: usize,
    },
    LevelStarted {
        level: usize,
        node_count: usize,
    },
    NodeStarted {
        node_id: String,
        node_kind: String,
    },
    NodeCompleted {
        node_id: String,
        duration_ms: u64,
    },
    NodeFailed {
        node_id: String,
        error: String,
        will_retry: bool,
    },
    NodeSkipped {
        node_id: String,
        reason: String,
    },
    NodeRetrying {
        node_id: String,
        attempt: u32,
        max_attempts: u32,
        backoff_ms: u64,
    },
    LoopIteration {
        node_id: String,
        iteration: u32,
        max: u32,
    },
    DagCompleted {
        total_duration_ms: u64,
        nodes_executed: usize,
    },
    DagFailed {
        error: String,
        nodes_executed: usize,
    },
}

/// Receives [`ExecutionEvent`] values (logging, metrics, bus, etc.).
#[async_trait::async_trait]
pub trait EventSink: Send + Sync {
    async fn emit(&self, event: ExecutionEvent);
}

/// No-op [`EventSink`] when events are not needed.
#[allow(dead_code)]
pub struct NullSink;

#[async_trait::async_trait]
impl EventSink for NullSink {
    async fn emit(&self, _event: ExecutionEvent) {}
}
