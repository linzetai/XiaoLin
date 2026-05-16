//! Worker isolation — each worker gets its own tool registry and context,
//! preventing side effects between parallel workers.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use fastclaw_core::tool::ToolRegistry;
use fastclaw_core::types::ChatMessage;

/// Isolated context for a single worker in coordinator mode.
#[derive(Debug, Clone)]
pub struct WorkerContext {
    /// Worker identifier (matches WorkerTask.id).
    pub worker_id: String,
    /// Isolated message history for this worker.
    pub messages: Vec<ChatMessage>,
    /// Worker-specific tool restrictions (allow-list).
    pub allowed_tools: Option<Vec<String>>,
    /// Worker-specific tool restrictions (deny-list).
    pub denied_tools: Vec<String>,
    /// Working directory override (for filesystem isolation).
    pub work_dir: Option<String>,
    /// Maximum iterations for this worker.
    pub max_iterations: u32,
    /// Whether the worker can spawn sub-workers.
    pub allow_sub_workers: bool,
}

impl WorkerContext {
    pub fn new(worker_id: &str) -> Self {
        Self {
            worker_id: worker_id.to_string(),
            messages: Vec::new(),
            allowed_tools: None,
            denied_tools: Vec::new(),
            work_dir: None,
            max_iterations: 20,
            allow_sub_workers: false,
        }
    }

    pub fn with_work_dir(mut self, dir: &str) -> Self {
        self.work_dir = Some(dir.to_string());
        self
    }

    pub fn with_max_iterations(mut self, n: u32) -> Self {
        self.max_iterations = n;
        self
    }

    pub fn with_allowed_tools(mut self, tools: Vec<String>) -> Self {
        self.allowed_tools = Some(tools);
        self
    }

    pub fn with_denied_tools(mut self, tools: Vec<String>) -> Self {
        self.denied_tools = tools;
        self
    }
}

/// Manages isolated tool registries for workers.
pub struct WorkerIsolation {
    workers: HashMap<String, WorkerContext>,
}

impl WorkerIsolation {
    pub fn new() -> Self {
        Self {
            workers: HashMap::new(),
        }
    }

    /// Register a new worker with its isolated context.
    pub fn register(&mut self, ctx: WorkerContext) {
        self.workers.insert(ctx.worker_id.clone(), ctx);
    }

    /// Get a worker's context.
    pub fn get(&self, worker_id: &str) -> Option<&WorkerContext> {
        self.workers.get(worker_id)
    }

    /// Remove a worker's context (cleanup after completion).
    pub fn remove(&mut self, worker_id: &str) -> Option<WorkerContext> {
        self.workers.remove(worker_id)
    }

    /// Number of active workers.
    pub fn active_count(&self) -> usize {
        self.workers.len()
    }

    /// Apply tool filtering for a worker's isolated registry.
    ///
    /// Returns the list of tool names that should be available to this worker.
    /// If no allowed_tools are specified, all tools are available except denied ones.
    pub fn filter_tools_for_worker(
        &self,
        worker_id: &str,
        all_tools: &[String],
    ) -> Vec<String> {
        let ctx = match self.workers.get(worker_id) {
            Some(ctx) => ctx,
            None => return all_tools.to_vec(),
        };

        let mut available: Vec<String> = if let Some(ref allowed) = ctx.allowed_tools {
            all_tools
                .iter()
                .filter(|t| allowed.iter().any(|a| a == *t))
                .cloned()
                .collect()
        } else {
            all_tools.to_vec()
        };

        available.retain(|t| !ctx.denied_tools.iter().any(|d| d == t));

        // Workers should never have access to coordinator-only tools
        let coordinator_only = ["create_team", "send_message", "list_team"];
        available.retain(|t| !coordinator_only.contains(&t.as_str()));

        available
    }
}

impl Default for WorkerIsolation {
    fn default() -> Self {
        Self::new()
    }
}

/// Serializable worker state for persistence across sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerState {
    pub worker_id: String,
    pub status: WorkerStatus,
    pub iterations_used: u32,
    pub tool_calls_made: u32,
    pub output: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_context_creation() {
        let ctx = WorkerContext::new("w1")
            .with_max_iterations(10)
            .with_work_dir("/tmp/worker1");

        assert_eq!(ctx.worker_id, "w1");
        assert_eq!(ctx.max_iterations, 10);
        assert_eq!(ctx.work_dir.as_deref(), Some("/tmp/worker1"));
    }

    #[test]
    fn isolation_filters_tools() {
        let mut iso = WorkerIsolation::new();
        let ctx = WorkerContext::new("w1")
            .with_allowed_tools(vec!["read_file".into(), "grep".into()]);
        iso.register(ctx);

        let all = vec![
            "read_file".into(),
            "write_file".into(),
            "grep".into(),
            "shell".into(),
            "create_team".into(),
        ];

        let filtered = iso.filter_tools_for_worker("w1", &all);
        assert_eq!(filtered, vec!["read_file", "grep"]);
    }

    #[test]
    fn isolation_denies_coordinator_tools() {
        let mut iso = WorkerIsolation::new();
        let ctx = WorkerContext::new("w1");
        iso.register(ctx);

        let all = vec![
            "read_file".into(),
            "create_team".into(),
            "send_message".into(),
            "list_team".into(),
        ];

        let filtered = iso.filter_tools_for_worker("w1", &all);
        assert_eq!(filtered, vec!["read_file"]);
    }

    #[test]
    fn isolation_denied_tools() {
        let mut iso = WorkerIsolation::new();
        let ctx = WorkerContext::new("w1")
            .with_denied_tools(vec!["shell".into(), "write_file".into()]);
        iso.register(ctx);

        let all = vec![
            "read_file".into(),
            "write_file".into(),
            "shell".into(),
            "grep".into(),
        ];

        let filtered = iso.filter_tools_for_worker("w1", &all);
        assert_eq!(filtered, vec!["read_file", "grep"]);
    }

    #[test]
    fn unknown_worker_gets_all_tools() {
        let iso = WorkerIsolation::new();
        let all = vec!["read_file".into(), "write_file".into()];
        let filtered = iso.filter_tools_for_worker("unknown", &all);
        assert_eq!(filtered, all);
    }
}
