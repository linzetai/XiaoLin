use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A complete DAG workflow definition, loaded from JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagDefinition {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    pub nodes: Vec<NodeDef>,
    pub edges: Vec<EdgeDef>,
}

fn default_backoff_ms() -> u64 {
    1000
}

fn default_backoff_multiplier() -> f64 {
    2.0
}

/// Configuration for a [`NodeKind::Loop`] node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoopConfig {
    /// Maximum number of body executions (each iteration runs all body nodes in order).
    pub max_iterations: u32,
    /// Optional boolean expression evaluated after each iteration; the loop continues while this is true.
    #[serde(default)]
    pub condition_expr: Option<String>,
}

/// Per-node retry configuration (used when `retry_policy` on [`NodeDef`] is present).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetryPolicy {
    #[serde(default)]
    pub max_retries: u32,
    #[serde(default = "default_backoff_ms")]
    pub backoff_ms: u64,
    #[serde(default = "default_backoff_multiplier")]
    pub backoff_multiplier: f64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 0,
            backoff_ms: default_backoff_ms(),
            backoff_multiplier: default_backoff_multiplier(),
        }
    }
}

/// What happens when a node fails after retries/timeouts are exhausted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FailurePolicy {
    /// Fail the entire graph (default).
    #[default]
    Abort,
    /// Mark the node as skipped and continue.
    Skip,
    /// Mark the node as failed but keep executing downstream nodes.
    Continue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeDef {
    pub id: String,
    pub kind: NodeKind,
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,
    /// Node-level timeout in milliseconds (`None` = no limit).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Optional retry policy; absent means no retries.
    #[serde(default)]
    pub retry_policy: Option<RetryPolicy>,
    /// How to proceed when this node ultimately fails.
    #[serde(default)]
    pub failure_policy: FailurePolicy,
    /// When [`NodeKind::Loop`], carries iteration limits and optional per-iteration condition.
    #[serde(default)]
    pub loop_config: Option<LoopConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// Call an LLM with a prompt template. Config keys: `prompt`, `model`.
    LlmCall,
    /// Execute a registered tool. Config keys: `tool_name`, `arguments`.
    ToolCall,
    /// Conditional branch. Config keys: `condition` (JSONPath or expression).
    /// Edges from this node use `label` to match branch outcomes.
    Condition,
    /// Fan-out: all downstream nodes execute in parallel.
    Parallel,
    /// Fan-in: wait for all upstream nodes to complete.
    Join,
    /// Repeat a subgraph: outgoing edges labeled `body` define the loop body (see [`LoopConfig`]).
    /// Body nodes are still part of the global DAG for ordering; the executor runs them inside the loop
    /// and marks them executed so they are not run again at their scheduled level.
    Loop,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeDef {
    pub from: String,
    pub to: String,
    /// Optional label for conditional branches (e.g. "true", "false").
    #[serde(default)]
    pub label: Option<String>,
}

impl DagDefinition {
    pub fn from_json(json: &str) -> anyhow::Result<Self> {
        let def: Self = serde_json::from_str(json)?;
        def.validate()?;
        Ok(def)
    }

    fn validate(&self) -> anyhow::Result<()> {
        if self.nodes.is_empty() {
            anyhow::bail!("DAG must have at least one node");
        }

        let node_ids: std::collections::HashSet<&str> =
            self.nodes.iter().map(|n| n.id.as_str()).collect();

        for edge in &self.edges {
            if !node_ids.contains(edge.from.as_str()) {
                anyhow::bail!("edge references unknown source node: {}", edge.from);
            }
            if !node_ids.contains(edge.to.as_str()) {
                anyhow::bail!("edge references unknown target node: {}", edge.to);
            }
        }

        // Check for duplicate node IDs
        if node_ids.len() != self.nodes.len() {
            anyhow::bail!("duplicate node IDs detected");
        }

        Ok(())
    }
}
