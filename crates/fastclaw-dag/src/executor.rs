use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use futures::future::join_all;
use tokio::sync::RwLock;
use tokio::time::{sleep, timeout};

use crate::checkpoint::{CheckpointStore, DagCheckpoint, NodeState};
use crate::definition::{FailurePolicy, NodeDef, NodeKind, RetryPolicy};
use crate::events::{EventKind, EventSink, ExecutionEvent};
use crate::expression::{evaluate_bool, evaluate_condition};
use crate::graph::DagGraph;

/// Shared execution context carrying data between nodes.
/// Each node can read outputs from upstream nodes and write its own output.
#[derive(Clone)]
pub struct ExecutionContext {
    data: Arc<RwLock<std::collections::HashMap<String, serde_json::Value>>>,
    /// Node IDs that have finished (completed, failed, or skipped).
    executed: Arc<RwLock<HashSet<String>>>,
    /// Increments whenever [`ExecutionContext::mark_executed`] runs (for event metrics).
    nodes_seen: Arc<AtomicUsize>,
    /// Optional checkpoint backend (also mirrored on [`DagExecutor`] for convenience).
    checkpoint_store: Option<Arc<dyn CheckpointStore>>,
    dag_id: String,
}

impl std::fmt::Debug for ExecutionContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionContext")
            .field("dag_id", &self.dag_id)
            .field("has_checkpoint_store", &self.checkpoint_store.is_some())
            .field("nodes_seen", &self.nodes_seen.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl ExecutionContext {
    pub fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(std::collections::HashMap::new())),
            executed: Arc::new(RwLock::new(HashSet::new())),
            nodes_seen: Arc::new(AtomicUsize::new(0)),
            checkpoint_store: None,
            dag_id: String::new(),
        }
    }

    pub fn with_input(input: serde_json::Value) -> Self {
        let mut data = std::collections::HashMap::new();
        data.insert("input".to_string(), input);
        Self {
            data: Arc::new(RwLock::new(data)),
            executed: Arc::new(RwLock::new(HashSet::new())),
            nodes_seen: Arc::new(AtomicUsize::new(0)),
            checkpoint_store: None,
            dag_id: String::new(),
        }
    }

    /// Build a context with optional checkpoint I/O metadata.
    pub fn with_checkpoint(
        input: Option<serde_json::Value>,
        checkpoint_store: Arc<dyn CheckpointStore>,
        dag_id: impl Into<String>,
    ) -> Self {
        let data = if let Some(inp) = input {
            let mut m = std::collections::HashMap::new();
            m.insert("input".to_string(), inp);
            m
        } else {
            std::collections::HashMap::new()
        };
        Self {
            data: Arc::new(RwLock::new(data)),
            executed: Arc::new(RwLock::new(HashSet::new())),
            nodes_seen: Arc::new(AtomicUsize::new(0)),
            checkpoint_store: Some(checkpoint_store),
            dag_id: dag_id.into(),
        }
    }

    pub(crate) fn attach_checkpoint(
        &mut self,
        store: Arc<dyn CheckpointStore>,
        dag_id: impl Into<String>,
    ) {
        self.checkpoint_store = Some(store);
        let id = dag_id.into();
        if !id.is_empty() {
            self.dag_id = id;
        }
    }

    pub fn dag_id(&self) -> &str {
        &self.dag_id
    }

    pub async fn get(&self, key: &str) -> Option<serde_json::Value> {
        self.data.read().await.get(key).cloned()
    }

    pub async fn set(&self, key: &str, value: serde_json::Value) {
        self.data.write().await.insert(key.to_string(), value);
    }

    pub async fn snapshot(&self) -> std::collections::HashMap<String, serde_json::Value> {
        self.data.read().await.clone()
    }

    pub(crate) async fn is_executed(&self, node_id: &str) -> bool {
        self.executed.read().await.contains(node_id)
    }

    pub(crate) async fn mark_executed(&self, node_id: &str) {
        self.executed.write().await.insert(node_id.to_string());
        self.nodes_seen.fetch_add(1, Ordering::SeqCst);
    }

    /// Count of nodes that have been marked executed in this context (including from checkpoint restore).
    pub fn nodes_seen_count(&self) -> usize {
        self.nodes_seen.load(Ordering::SeqCst)
    }

    /// Share the same completion counter (for emitting failure metrics after a consuming run).
    pub(crate) fn nodes_seen_handle(&self) -> Arc<AtomicUsize> {
        self.nodes_seen.clone()
    }

    fn checkpoint_ref(&self) -> Option<Arc<dyn CheckpointStore>> {
        self.checkpoint_store.clone()
    }
}

impl Default for ExecutionContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Handler that implementations provide to execute individual node logic.
#[async_trait]
pub trait NodeHandler: Send + Sync {
    async fn execute_node(
        &self,
        node: &NodeDef,
        ctx: &ExecutionContext,
    ) -> anyhow::Result<serde_json::Value>;
}

/// Executes a DAG workflow according to its topology.
pub struct DagExecutor {
    graph: DagGraph,
    handler: Arc<dyn NodeHandler>,
    checkpoint_store: Option<Arc<dyn CheckpointStore>>,
    dag_id: String,
    /// Wall-clock limit for the entire [`Self::run`].
    pub graph_timeout_ms: Option<u64>,
    /// Optional structured event sink (DAG / level / node lifecycle).
    event_sink: Option<Arc<dyn EventSink>>,
}

impl DagExecutor {
    pub fn new(graph: DagGraph, handler: Arc<dyn NodeHandler>) -> Self {
        Self {
            graph,
            handler,
            checkpoint_store: None,
            dag_id: String::new(),
            graph_timeout_ms: None,
            event_sink: None,
        }
    }

    /// Same as [`Self::new`] but with optional checkpoint persistence.
    pub fn with_checkpoint_store(
        graph: DagGraph,
        handler: Arc<dyn NodeHandler>,
        checkpoint_store: Arc<dyn CheckpointStore>,
        dag_id: impl Into<String>,
    ) -> Self {
        Self {
            graph,
            handler,
            checkpoint_store: Some(checkpoint_store),
            dag_id: dag_id.into(),
            graph_timeout_ms: None,
            event_sink: None,
        }
    }

    /// Set a graph-level timeout (milliseconds). Chain before [`Self::run`].
    pub fn with_graph_timeout_ms(mut self, ms: Option<u64>) -> Self {
        self.graph_timeout_ms = ms;
        self
    }

    /// Attach an [`EventSink`] for structured execution events.
    pub fn with_event_sink(mut self, sink: Option<Arc<dyn EventSink>>) -> Self {
        self.event_sink = sink;
        self
    }

    async fn emit_event(&self, dag_id: &str, kind: EventKind) {
        if let Some(ref sink) = self.event_sink {
            sink.emit(ExecutionEvent {
                dag_id: dag_id.to_string(),
                event: kind,
                timestamp: Utc::now().to_rfc3339(),
            })
            .await;
        }
    }

    fn effective_dag_id<'a>(&'a self, ctx: &'a ExecutionContext) -> std::borrow::Cow<'a, str> {
        if !self.dag_id.is_empty() {
            std::borrow::Cow::Borrowed(self.dag_id.as_str())
        } else if !ctx.dag_id().is_empty() {
            std::borrow::Cow::Borrowed(ctx.dag_id())
        } else {
            std::borrow::Cow::Borrowed("__anonymous_dag__")
        }
    }

    fn store_for_run(&self, ctx: &ExecutionContext) -> Option<Arc<dyn CheckpointStore>> {
        self.checkpoint_store
            .clone()
            .or_else(|| ctx.checkpoint_ref())
    }

    /// Execute the entire DAG, returning the final execution context.
    pub async fn run(&self, mut ctx: ExecutionContext) -> anyhow::Result<ExecutionContext> {
        if let Some(store) = self.checkpoint_store.clone() {
            let id = if !self.dag_id.is_empty() {
                self.dag_id.clone()
            } else if !ctx.dag_id().is_empty() {
                ctx.dag_id().to_string()
            } else {
                String::new()
            };
            if !id.is_empty() {
                ctx.attach_checkpoint(store, id);
            } else {
                ctx.attach_checkpoint(store, "__anonymous_dag__");
            }
        }

        let dag_id_emit = self.effective_dag_id(&ctx).into_owned();
        let steps = ctx.nodes_seen_handle();
        let body = self.run_body(ctx);
        let res = if let Some(ms) = self.graph_timeout_ms {
            match timeout(Duration::from_millis(ms), body).await {
                Ok(r) => r,
                Err(_) => Err(anyhow::anyhow!("DAG graph timeout")),
            }
        } else {
            body.await
        };

        if let Err(ref e) = res {
            self.emit_event(
                &dag_id_emit,
                EventKind::DagFailed {
                    error: e.to_string(),
                    nodes_executed: steps.load(Ordering::SeqCst),
                },
            )
            .await;
        }
        res
    }

    async fn run_body(&self, ctx: ExecutionContext) -> anyhow::Result<ExecutionContext> {
        let levels = self.graph.execution_levels()?;
        let store = self.store_for_run(&ctx);
        let dag_id = self.effective_dag_id(&ctx).into_owned();

        if let Some(ref s) = store {
            if let Some(cp) = s.load_checkpoint(&dag_id).await? {
                self.apply_checkpoint(&ctx, &cp).await?;
            }
        }

        tracing::info!(
            levels = levels.len(),
            nodes = self.graph.node_count(),
            dag_id = %dag_id,
            "starting DAG execution"
        );

        self.emit_event(
            &dag_id,
            EventKind::DagStarted {
                node_count: self.graph.node_count(),
            },
        )
        .await;

        let t0 = Instant::now();

        for (level_idx, level) in levels.iter().enumerate() {
            let mut to_run: Vec<String> = Vec::new();
            for node_id in level {
                if ctx.is_executed(node_id).await {
                    continue;
                }
                if self
                    .should_skip_unselected_condition_branch(node_id, &ctx)
                    .await
                {
                    self.emit_event(
                        &dag_id,
                        EventKind::NodeSkipped {
                            node_id: node_id.to_string(),
                            reason: "unselected condition branch".to_string(),
                        },
                    )
                    .await;
                    self.persist_node(&ctx, store.as_ref(), &dag_id, node_id, NodeState::Skipped)
                        .await?;
                    ctx.set(node_id, serde_json::Value::Null).await;
                    ctx.mark_executed(node_id).await;
                    continue;
                }
                to_run.push(node_id.clone());
            }

            if to_run.is_empty() {
                tracing::debug!(level = level_idx, "level empty after skips");
                continue;
            }

            self.emit_event(
                &dag_id,
                EventKind::LevelStarted {
                    level: level_idx,
                    node_count: to_run.len(),
                },
            )
            .await;

            if to_run.len() == 1 {
                self.run_one(&to_run[0], &ctx, store.as_ref(), &dag_id)
                    .await?;
            } else {
                let futs: Vec<_> = to_run
                    .iter()
                    .map(|id| self.run_one(id, &ctx, store.as_ref(), &dag_id))
                    .collect();
                let results = join_all(futs).await;
                let errors: Vec<_> = results
                    .into_iter()
                    .zip(to_run.iter())
                    .filter_map(|(r, id)| r.err().map(|e| format!("{id}: {e}")))
                    .collect();
                if !errors.is_empty() {
                    anyhow::bail!(
                        "parallel node failures ({}/{}): {}",
                        errors.len(),
                        to_run.len(),
                        errors.join("; ")
                    );
                }
            }
            tracing::debug!(level = level_idx, nodes = ?to_run, "level complete");
        }

        tracing::info!("DAG execution complete");

        self.emit_event(
            &dag_id,
            EventKind::DagCompleted {
                total_duration_ms: t0.elapsed().as_millis() as u64,
                nodes_executed: ctx.nodes_seen_count(),
            },
        )
        .await;

        if let Some(s) = store {
            s.clear_checkpoint(&dag_id).await?;
        }

        Ok(ctx)
    }

    async fn apply_checkpoint(
        &self,
        ctx: &ExecutionContext,
        cp: &DagCheckpoint,
    ) -> anyhow::Result<()> {
        for (node_id, state) in &cp.node_states {
            match state {
                NodeState::Completed | NodeState::Skipped => {
                    ctx.mark_executed(node_id).await;
                }
                NodeState::Failed(_) | NodeState::Pending | NodeState::Running => {}
            }
        }
        for (node_id, out) in &cp.node_outputs {
            if cp
                .node_states
                .get(node_id)
                .is_some_and(|s| matches!(s, NodeState::Completed))
            {
                ctx.set(node_id, out.clone()).await;
            }
        }
        for (node_id, state) in &cp.node_states {
            if matches!(state, NodeState::Skipped) {
                ctx.set(node_id, serde_json::Value::Null).await;
            }
        }
        Ok(())
    }

    async fn persist_node(
        &self,
        ctx: &ExecutionContext,
        store: Option<&Arc<dyn CheckpointStore>>,
        dag_id: &str,
        node_id: &str,
        state: NodeState,
    ) -> anyhow::Result<()> {
        if let Some(s) = store {
            s.save_node_state(dag_id, node_id, &state).await?;
            match &state {
                NodeState::Completed => {
                    if let Some(v) = ctx.get(node_id).await {
                        s.save_node_output(dag_id, node_id, &v).await?;
                    }
                }
                NodeState::Skipped => {
                    s.save_node_output(dag_id, node_id, &serde_json::Value::Null)
                        .await?;
                }
                NodeState::Failed(msg) => {
                    s.save_node_output(dag_id, node_id, &serde_json::json!({ "error": msg }))
                        .await?;
                }
                NodeState::Pending | NodeState::Running => {}
            }
        }
        Ok(())
    }

    /// Mirrors legacy `execute_condition` branch resolution: labeled edges matching
    /// the evaluated branch run; if none match, all outgoing edges are considered active.
    async fn should_skip_unselected_condition_branch(
        &self,
        node_id: &str,
        ctx: &ExecutionContext,
    ) -> bool {
        for (pred, _edge_label) in self.graph.incoming_edges(node_id) {
            let Some(pred_node) = self.graph.get_node(&pred) else {
                continue;
            };
            if pred_node.kind != NodeKind::Condition {
                continue;
            }
            let Some(cond_val) = ctx.get(&pred).await else {
                continue;
            };
            let branch = condition_branch_str(&cond_val);
            let targets = self.graph.successors(&pred, Some(&branch));
            if targets.is_empty() {
                continue;
            }
            if !targets.iter().any(|t| t == node_id) {
                return true;
            }
        }
        false
    }

    fn effective_retry_policy(node: &NodeDef) -> RetryPolicy {
        node.retry_policy.clone().unwrap_or_default()
    }

    fn backoff_sleep_ms(policy: &RetryPolicy, retry_index: u32) -> u64 {
        let mult = policy.backoff_multiplier.powi(retry_index as i32);
        ((policy.backoff_ms as f64) * mult).round() as u64
    }

    async fn handle_final_node_failure(
        &self,
        node: &NodeDef,
        ctx: &ExecutionContext,
        store: Option<&Arc<dyn CheckpointStore>>,
        dag_id: &str,
        message: String,
    ) -> anyhow::Result<()> {
        match node.failure_policy {
            FailurePolicy::Abort => {
                self.persist_node(
                    ctx,
                    store,
                    dag_id,
                    &node.id,
                    NodeState::Failed(message.clone()),
                )
                .await?;
                Err(anyhow::anyhow!("{message}"))
            }
            FailurePolicy::Skip => {
                self.emit_event(
                    dag_id,
                    EventKind::NodeSkipped {
                        node_id: node.id.clone(),
                        reason: message.clone(),
                    },
                )
                .await;
                self.persist_node(ctx, store, dag_id, &node.id, NodeState::Skipped)
                    .await?;
                ctx.set(&node.id, serde_json::Value::Null).await;
                ctx.mark_executed(&node.id).await;
                Ok(())
            }
            FailurePolicy::Continue => {
                self.persist_node(
                    ctx,
                    store,
                    dag_id,
                    &node.id,
                    NodeState::Failed(message.clone()),
                )
                .await?;
                ctx.set(
                    &node.id,
                    serde_json::json!({ "error": message }),
                )
                .await;
                ctx.mark_executed(&node.id).await;
                Ok(())
            }
        }
    }

    async fn run_one(
        &self,
        node_id: &str,
        ctx: &ExecutionContext,
        store: Option<&Arc<dyn CheckpointStore>>,
        dag_id: &str,
    ) -> anyhow::Result<()> {
        if ctx.is_executed(node_id).await {
            return Ok(());
        }

        let node = self
            .graph
            .get_node(node_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("node not found: {node_id}"))?;

        if self
            .should_skip_unselected_condition_branch(node_id, ctx)
            .await
        {
            self.emit_event(
                dag_id,
                EventKind::NodeSkipped {
                    node_id: node_id.to_string(),
                    reason: "unselected condition branch".to_string(),
                },
            )
            .await;
            self.persist_node(ctx, store, dag_id, node_id, NodeState::Skipped)
                .await?;
            ctx.set(node_id, serde_json::Value::Null).await;
            ctx.mark_executed(node_id).await;
            return Ok(());
        }

        if node.kind == NodeKind::Condition {
            return self
                .execute_condition(&node, ctx, store, dag_id, false)
                .await;
        }

        if node.kind == NodeKind::Loop {
            return self.execute_loop(&node, ctx, store, dag_id).await;
        }

        self.run_handler_retries(&node, node_id, ctx, store, dag_id, true)
            .await
    }

    async fn execute_loop(
        &self,
        node: &NodeDef,
        ctx: &ExecutionContext,
        store: Option<&Arc<dyn CheckpointStore>>,
        dag_id: &str,
    ) -> anyhow::Result<()> {
        if ctx.is_executed(&node.id).await {
            return Ok(());
        }

        let cfg = node.loop_config.as_ref().ok_or_else(|| {
            anyhow::anyhow!("loop node '{}' requires loop_config", node.id)
        })?;

        let body_order = self.graph.loop_body_topological_order(&node.id)?;
        for bid in &body_order {
            if self.graph.get_node(bid).is_some_and(|n| n.kind == NodeKind::Loop) {
                anyhow::bail!(
                    "nested Loop inside loop body is not supported (node '{}')",
                    bid
                );
            }
        }

        let loop_t0 = Instant::now();
        self.emit_event(
            dag_id,
            EventKind::NodeStarted {
                node_id: node.id.clone(),
                node_kind: "loop".into(),
            },
        )
        .await;
        if let Some(s) = store {
            s.save_node_state(dag_id, &node.id, &NodeState::Running)
                .await?;
        }

        let mut iter_outputs: Vec<serde_json::Value> = Vec::new();
        let mut iteration: u32 = 0;

        while iteration < cfg.max_iterations {
            iteration += 1;
            self.emit_event(
                dag_id,
                EventKind::LoopIteration {
                    node_id: node.id.clone(),
                    iteration,
                    max: cfg.max_iterations,
                },
            )
            .await;

            let mut round = serde_json::Map::new();
            for bid in &body_order {
                self.run_loop_body_node(bid, ctx, store, dag_id).await?;
                let v = ctx.get(bid).await.unwrap_or(serde_json::Value::Null);
                round.insert(bid.clone(), v);
            }
            iter_outputs.push(serde_json::Value::Object(round));

            if let Some(s) = store {
                let progress = serde_json::json!({
                    "iteration": iteration,
                    "max_iterations": cfg.max_iterations,
                    "partial_outputs": iter_outputs,
                });
                if let Err(e) = s
                    .save_node_output(dag_id, &node.id, &progress)
                    .await
                {
                    tracing::warn!(
                        dag_id,
                        node_id = %node.id,
                        iteration,
                        error = %e,
                        "failed to save per-iteration loop checkpoint"
                    );
                }
            }

            if let Some(expr) = cfg.condition_expr.as_deref() {
                let snapshot = ctx.snapshot().await;
                let context_value = serde_json::to_value(&snapshot)?;
                if !evaluate_bool(expr, &context_value)? {
                    break;
                }
            }
        }

        ctx.set(&node.id, serde_json::Value::Array(iter_outputs))
            .await;
        self.persist_node(ctx, store, dag_id, &node.id, NodeState::Completed)
            .await?;
        self.emit_event(
            dag_id,
            EventKind::NodeCompleted {
                node_id: node.id.clone(),
                duration_ms: loop_t0.elapsed().as_millis() as u64,
            },
        )
        .await;

        for bid in &body_order {
            self.persist_node(ctx, store, dag_id, bid, NodeState::Completed)
                .await?;
            ctx.mark_executed(bid).await;
        }

        ctx.mark_executed(&node.id).await;
        Ok(())
    }

    async fn run_loop_body_node(
        &self,
        bid: &str,
        ctx: &ExecutionContext,
        store: Option<&Arc<dyn CheckpointStore>>,
        dag_id: &str,
    ) -> anyhow::Result<()> {
        let node = self
            .graph
            .get_node(bid)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("loop body node not found: {bid}"))?;

        if self
            .should_skip_unselected_condition_branch(bid, ctx)
            .await
        {
            anyhow::bail!(
                "node '{}' would be skipped by an upstream branch inside a loop body; unsupported configuration",
                bid
            );
        }

        match node.kind {
            NodeKind::Condition => {
                self.execute_condition(&node, ctx, store, dag_id, true).await
            }
            NodeKind::Loop => anyhow::bail!("nested Loop in loop body"),
            _ => {
                self.run_handler_retries(&node, bid, ctx, store, dag_id, false)
                    .await
            }
        }
    }

    async fn run_handler_retries(
        &self,
        node: &NodeDef,
        node_id: &str,
        ctx: &ExecutionContext,
        store: Option<&Arc<dyn CheckpointStore>>,
        dag_id: &str,
        mark_and_persist: bool,
    ) -> anyhow::Result<()> {
        let t_wall = Instant::now();
        self.emit_event(
            dag_id,
            EventKind::NodeStarted {
                node_id: node_id.to_string(),
                node_kind: node_kind_str(&node.kind),
            },
        )
        .await;

        if mark_and_persist {
            tracing::info!(node_id, kind = ?node.kind, "executing node");
            if let Some(s) = store {
                s.save_node_state(dag_id, node_id, &NodeState::Running)
                    .await?;
            }
        }

        let policy = Self::effective_retry_policy(node);
        let mut attempt: u32 = 0;
        let max_attempts = policy.max_retries.saturating_add(1);

        loop {
            let result = if let Some(ms) = node.timeout_ms {
                match timeout(Duration::from_millis(ms), self.handler.execute_node(node, ctx)).await
                {
                    Ok(Ok(v)) => Ok(v),
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(_) => Err("node execution timed out".to_string()),
                }
            } else {
                self.handler
                    .execute_node(node, ctx)
                    .await
                    .map_err(|e| e.to_string())
            };

            match result {
                Ok(output) => {
                    ctx.set(node_id, output).await;
                    if mark_and_persist {
                        self.persist_node(ctx, store, dag_id, node_id, NodeState::Completed)
                            .await?;
                        ctx.mark_executed(node_id).await;
                    }
                    self.emit_event(
                        dag_id,
                        EventKind::NodeCompleted {
                            node_id: node_id.to_string(),
                            duration_ms: t_wall.elapsed().as_millis() as u64,
                        },
                    )
                    .await;
                    return Ok(());
                }
                Err(msg) => {
                    if attempt < policy.max_retries {
                        let delay = Self::backoff_sleep_ms(&policy, attempt);
                        self.emit_event(
                            dag_id,
                            EventKind::NodeFailed {
                                node_id: node_id.to_string(),
                                error: msg.clone(),
                                will_retry: true,
                            },
                        )
                        .await;
                        self.emit_event(
                            dag_id,
                            EventKind::NodeRetrying {
                                node_id: node_id.to_string(),
                                attempt: attempt.saturating_add(1),
                                max_attempts,
                                backoff_ms: delay,
                            },
                        )
                        .await;
                        tracing::warn!(
                            node_id,
                            attempt,
                            delay_ms = delay,
                            error = %msg,
                            "node failed, retrying"
                        );
                        sleep(Duration::from_millis(delay)).await;
                        attempt += 1;
                        continue;
                    }
                    self.emit_event(
                        dag_id,
                        EventKind::NodeFailed {
                            node_id: node_id.to_string(),
                            error: msg.clone(),
                            will_retry: false,
                        },
                    )
                    .await;
                    return self
                        .handle_final_node_failure(node, ctx, store, dag_id, msg)
                        .await;
                }
            }
        }
    }

    async fn execute_condition(
        &self,
        node: &NodeDef,
        ctx: &ExecutionContext,
        store: Option<&Arc<dyn CheckpointStore>>,
        dag_id: &str,
        defer_completion: bool,
    ) -> anyhow::Result<()> {
        if ctx.is_executed(&node.id).await {
            return Ok(());
        }

        tracing::info!(node_id = %node.id, "evaluating condition");
        let t0 = Instant::now();
        if !defer_completion {
            if let Some(s) = store {
                s.save_node_state(dag_id, &node.id, &NodeState::Running)
                    .await?;
            }
            self.emit_event(
                dag_id,
                EventKind::NodeStarted {
                    node_id: node.id.clone(),
                    node_kind: "condition".into(),
                },
            )
            .await;
        }

        let snapshot = ctx.snapshot().await;
        let context_value = serde_json::to_value(&snapshot)?;
        let expr = node
            .config
            .get("condition")
            .and_then(|v| v.as_str())
            .unwrap_or("true");

        match evaluate_condition(expr, &context_value) {
            Ok(branch) => {
                let output = serde_json::Value::String(branch);
                ctx.set(&node.id, output).await;
                if defer_completion {
                    return Ok(());
                }
                self.emit_event(
                    dag_id,
                    EventKind::NodeCompleted {
                        node_id: node.id.clone(),
                        duration_ms: t0.elapsed().as_millis() as u64,
                    },
                )
                .await;
                self.persist_node(ctx, store, dag_id, &node.id, NodeState::Completed)
                    .await?;
                ctx.mark_executed(&node.id).await;
            }
            Err(e) => {
                return self
                    .handle_final_node_failure(node, ctx, store, dag_id, e.to_string())
                    .await;
            }
        }
        Ok(())
    }
}

fn node_kind_str(kind: &NodeKind) -> String {
    match kind {
        NodeKind::LlmCall => "llm_call".into(),
        NodeKind::ToolCall => "tool_call".into(),
        NodeKind::Condition => "condition".into(),
        NodeKind::Parallel => "parallel".into(),
        NodeKind::Join => "join".into(),
        NodeKind::Loop => "loop".into(),
    }
}

fn condition_branch_str(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "false".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definition::DagDefinition;
    use crate::InMemoryCheckpointStore;
    use crate::SqliteCheckpointStore;
    use serde_json::json;
    use sqlx::sqlite::SqliteConnectOptions;
    use sqlx::SqlitePool;
    use std::str::FromStr;
    use std::sync::atomic::{AtomicUsize, Ordering};

    async fn sqlite_test_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:?cache=shared")
            .unwrap()
            .create_if_missing(true);
        SqlitePool::connect_with(opts).await.unwrap()
    }

    struct EchoHandler;

    #[async_trait]
    impl NodeHandler for EchoHandler {
        async fn execute_node(
            &self,
            node: &NodeDef,
            ctx: &ExecutionContext,
        ) -> anyhow::Result<serde_json::Value> {
            let input = ctx.get("input").await.unwrap_or(serde_json::Value::Null);
            Ok(serde_json::json!({
                "node_id": node.id,
                "kind": format!("{:?}", node.kind),
                "input": input,
            }))
        }
    }

    struct ConditionBranchCounter {
        pub hits: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl NodeHandler for ConditionBranchCounter {
        async fn execute_node(
            &self,
            node: &NodeDef,
            ctx: &ExecutionContext,
        ) -> anyhow::Result<serde_json::Value> {
            if node.kind == NodeKind::Condition {
                return Ok(serde_json::Value::String("true".into()));
            }
            if node.id == "on_true" || node.id == "on_false" {
                self.hits.fetch_add(1, Ordering::SeqCst);
            }
            let _ = ctx;
            Ok(serde_json::json!({ "id": node.id }))
        }
    }

    #[tokio::test]
    async fn test_linear_dag() {
        let json = r#"{
            "nodes": [
                { "id": "a", "kind": "llm_call" },
                { "id": "b", "kind": "tool_call" },
                { "id": "c", "kind": "llm_call" }
            ],
            "edges": [
                { "from": "a", "to": "b" },
                { "from": "b", "to": "c" }
            ]
        }"#;

        let def = DagDefinition::from_json(json).unwrap();
        let graph = DagGraph::build(&def).unwrap();
        assert_eq!(graph.node_count(), 3);
        assert_eq!(graph.edge_count(), 2);

        let levels = graph.execution_levels().unwrap();
        assert_eq!(levels.len(), 3);

        let executor = DagExecutor::new(graph, Arc::new(EchoHandler));
        let ctx = ExecutionContext::with_input(serde_json::json!("test input"));
        let result = executor.run(ctx).await.unwrap();

        assert!(result.get("a").await.is_some());
        assert!(result.get("b").await.is_some());
        assert!(result.get("c").await.is_some());
    }

    #[tokio::test]
    async fn test_parallel_dag() {
        let json = r#"{
            "nodes": [
                { "id": "start", "kind": "llm_call" },
                { "id": "branch_a", "kind": "tool_call" },
                { "id": "branch_b", "kind": "tool_call" },
                { "id": "join", "kind": "join" }
            ],
            "edges": [
                { "from": "start", "to": "branch_a" },
                { "from": "start", "to": "branch_b" },
                { "from": "branch_a", "to": "join" },
                { "from": "branch_b", "to": "join" }
            ]
        }"#;

        let def = DagDefinition::from_json(json).unwrap();
        let graph = DagGraph::build(&def).unwrap();

        let levels = graph.execution_levels().unwrap();
        assert_eq!(levels.len(), 3);
        // Middle level should have 2 parallel nodes
        assert!(levels.iter().any(|l| l.len() == 2));

        let executor = DagExecutor::new(graph, Arc::new(EchoHandler));
        let ctx = ExecutionContext::new();
        let result = executor.run(ctx).await.unwrap();

        assert!(result.get("start").await.is_some());
        assert!(result.get("branch_a").await.is_some());
        assert!(result.get("branch_b").await.is_some());
        assert!(result.get("join").await.is_some());
    }

    #[tokio::test]
    async fn test_condition_branch_not_double_executed() {
        let json = r#"{
            "nodes": [
                { "id": "start", "kind": "llm_call" },
                { "id": "cond", "kind": "condition" },
                { "id": "on_true", "kind": "tool_call" },
                { "id": "on_false", "kind": "tool_call" }
            ],
            "edges": [
                { "from": "start", "to": "cond" },
                { "from": "cond", "to": "on_true", "label": "true" },
                { "from": "cond", "to": "on_false", "label": "false" }
            ]
        }"#;

        let def = DagDefinition::from_json(json).unwrap();
        let graph = DagGraph::build(&def).unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let executor = DagExecutor::new(
            graph,
            Arc::new(ConditionBranchCounter { hits: hits.clone() }),
        );
        let ctx = ExecutionContext::new();
        executor.run(ctx).await.unwrap();

        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "only the taken branch runs once"
        );
    }

    #[tokio::test]
    async fn test_cycle_detection() {
        let json = r#"{
            "nodes": [
                { "id": "a", "kind": "llm_call" },
                { "id": "b", "kind": "llm_call" }
            ],
            "edges": [
                { "from": "a", "to": "b" },
                { "from": "b", "to": "a" }
            ]
        }"#;

        let def = DagDefinition::from_json(json).unwrap();
        let result = DagGraph::build(&def);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cycle"));
    }

    #[tokio::test]
    async fn successful_run_clears_checkpoint_next_run_reexecutes_all_nodes() {
        let json = r#"{
            "nodes": [
                { "id": "a", "kind": "llm_call" },
                { "id": "b", "kind": "tool_call" }
            ],
            "edges": [
                { "from": "a", "to": "b" }
            ]
        }"#;

        let def = DagDefinition::from_json(json).unwrap();
        let store = Arc::new(InMemoryCheckpointStore::new());
        let dag_id = "resume-test";

        {
            let graph = DagGraph::build(&def).unwrap();
            let ex = DagExecutor::with_checkpoint_store(
                graph,
                Arc::new(EchoHandler),
                store.clone(),
                dag_id,
            );
            let ctx = ExecutionContext::new();
            ex.run(ctx).await.unwrap();
        }

        let hits = Arc::new(AtomicUsize::new(0));

        struct CountingEcho(Arc<AtomicUsize>);

        #[async_trait]
        impl NodeHandler for CountingEcho {
            async fn execute_node(
                &self,
                node: &NodeDef,
                ctx: &ExecutionContext,
            ) -> anyhow::Result<serde_json::Value> {
                self.0.fetch_add(1, Ordering::SeqCst);
                let input = ctx.get("input").await.unwrap_or(serde_json::Value::Null);
                Ok(serde_json::json!({
                    "node_id": node.id,
                    "input": input,
                }))
            }
        }

        let graph2 = DagGraph::build(&def).unwrap();
        let ex2 = DagExecutor::with_checkpoint_store(
            graph2,
            Arc::new(CountingEcho(hits.clone())),
            store,
            dag_id,
        );
        let ctx = ExecutionContext::new();
        ex2.run(ctx).await.unwrap();

        assert_eq!(
            hits.load(Ordering::SeqCst),
            2,
            "after a successful run checkpoints are cleared, so all nodes run again"
        );
    }

    #[tokio::test]
    async fn sqlite_successful_run_clears_checkpoint_next_run_reexecutes_all_nodes() {
        let json = r#"{
            "nodes": [
                { "id": "a", "kind": "llm_call" },
                { "id": "b", "kind": "tool_call" }
            ],
            "edges": [
                { "from": "a", "to": "b" }
            ]
        }"#;

        let def = DagDefinition::from_json(json).unwrap();
        let pool = sqlite_test_pool().await;
        let store = Arc::new(SqliteCheckpointStore::open(pool).await.unwrap());
        let dag_id = "sqlite-resume-skip";

        {
            let graph = DagGraph::build(&def).unwrap();
            let ex = DagExecutor::with_checkpoint_store(
                graph,
                Arc::new(EchoHandler),
                store.clone(),
                dag_id,
            );
            let ctx = ExecutionContext::new();
            ex.run(ctx).await.unwrap();
        }

        let hits = Arc::new(AtomicUsize::new(0));

        struct CountingEcho(Arc<AtomicUsize>);

        #[async_trait]
        impl NodeHandler for CountingEcho {
            async fn execute_node(
                &self,
                node: &NodeDef,
                ctx: &ExecutionContext,
            ) -> anyhow::Result<serde_json::Value> {
                self.0.fetch_add(1, Ordering::SeqCst);
                let input = ctx.get("input").await.unwrap_or(serde_json::Value::Null);
                Ok(serde_json::json!({
                    "node_id": node.id,
                    "input": input,
                }))
            }
        }

        let graph2 = DagGraph::build(&def).unwrap();
        let ex2 = DagExecutor::with_checkpoint_store(
            graph2,
            Arc::new(CountingEcho(hits.clone())),
            store,
            dag_id,
        );
        let ctx = ExecutionContext::new();
        ex2.run(ctx).await.unwrap();

        assert_eq!(
            hits.load(Ordering::SeqCst),
            2,
            "after a successful run checkpoints are cleared, so all nodes run again"
        );
    }

    #[tokio::test]
    async fn checkpoint_cleared_after_successful_run() {
        let json = r#"{
            "nodes": [
                { "id": "a", "kind": "llm_call" },
                { "id": "b", "kind": "tool_call" }
            ],
            "edges": [
                { "from": "a", "to": "b" }
            ]
        }"#;

        let def = DagDefinition::from_json(json).unwrap();
        let store = Arc::new(InMemoryCheckpointStore::new());
        let dag_id = "clear-after-success";

        let graph = DagGraph::build(&def).unwrap();
        let ex = DagExecutor::with_checkpoint_store(
            graph,
            Arc::new(EchoHandler),
            store.clone(),
            dag_id,
        );
        ex.run(ExecutionContext::new()).await.unwrap();

        assert!(
            store.load_checkpoint(dag_id).await.unwrap().is_none(),
            "checkpoint should be cleared after successful DAG completion"
        );
    }

    #[tokio::test]
    async fn sqlite_checkpoint_partial_run_resume_completes_remaining() {
        let json = r#"{
            "nodes": [
                { "id": "a", "kind": "llm_call" },
                { "id": "b", "kind": "tool_call" }
            ],
            "edges": [
                { "from": "a", "to": "b" }
            ]
        }"#;

        let def = DagDefinition::from_json(json).unwrap();
        let pool = sqlite_test_pool().await;
        let store = Arc::new(SqliteCheckpointStore::open(pool).await.unwrap());
        let dag_id = "sqlite-partial-resume";

        struct FailBHandler;

        #[async_trait]
        impl NodeHandler for FailBHandler {
            async fn execute_node(
                &self,
                node: &NodeDef,
                ctx: &ExecutionContext,
            ) -> anyhow::Result<serde_json::Value> {
                if node.id == "b" {
                    anyhow::bail!("b fails first run");
                }
                let input = ctx.get("input").await.unwrap_or(serde_json::Value::Null);
                Ok(serde_json::json!({
                    "node_id": node.id,
                    "input": input,
                }))
            }
        }

        let graph1 = DagGraph::build(&def).unwrap();
        let ex1 = DagExecutor::with_checkpoint_store(
            graph1,
            Arc::new(FailBHandler),
            store.clone(),
            dag_id,
        );
        let err = ex1.run(ExecutionContext::new()).await.unwrap_err();
        assert!(
            err.to_string().contains("b fails first run"),
            "unexpected error: {err}"
        );

        let b_hits = Arc::new(AtomicUsize::new(0));

        struct CountBOnRun(Arc<AtomicUsize>);

        #[async_trait]
        impl NodeHandler for CountBOnRun {
            async fn execute_node(
                &self,
                node: &NodeDef,
                ctx: &ExecutionContext,
            ) -> anyhow::Result<serde_json::Value> {
                if node.id == "b" {
                    self.0.fetch_add(1, Ordering::SeqCst);
                }
                let input = ctx.get("input").await.unwrap_or(serde_json::Value::Null);
                Ok(serde_json::json!({
                    "node_id": node.id,
                    "input": input,
                }))
            }
        }

        let graph2 = DagGraph::build(&def).unwrap();
        let ex2 = DagExecutor::with_checkpoint_store(
            graph2,
            Arc::new(CountBOnRun(b_hits.clone())),
            store,
            dag_id,
        );
        ex2.run(ExecutionContext::new()).await.unwrap();

        assert_eq!(
            b_hits.load(Ordering::SeqCst),
            1,
            "node B should run exactly once on the second executor run"
        );
    }

    #[tokio::test]
    async fn dag_node_timeout_aborts_hung_node() {
        struct HangHandler;

        #[async_trait]
        impl NodeHandler for HangHandler {
            async fn execute_node(
                &self,
                node: &NodeDef,
                _ctx: &ExecutionContext,
            ) -> anyhow::Result<serde_json::Value> {
                if node.id == "hang" {
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                }
                Ok(json!({ "ok": true }))
            }
        }

        let json = r#"{
            "nodes": [
                { "id": "hang", "kind": "tool_call", "timeout_ms": 40 }
            ],
            "edges": []
        }"#;

        let def = DagDefinition::from_json(json).unwrap();
        let graph = DagGraph::build(&def).unwrap();
        let ex = DagExecutor::new(graph, Arc::new(HangHandler));
        let err = ex.run(ExecutionContext::new()).await.unwrap_err();
        assert!(
            err.to_string().contains("timed out"),
            "unexpected: {err}"
        );
    }

    #[tokio::test]
    async fn dag_retry_on_failure_succeeds_after_retries() {
        struct Flaky(Arc<AtomicUsize>);

        #[async_trait]
        impl NodeHandler for Flaky {
            async fn execute_node(
                &self,
                node: &NodeDef,
                _ctx: &ExecutionContext,
            ) -> anyhow::Result<serde_json::Value> {
                if node.id != "x" {
                    return Ok(json!({}));
                }
                let n = self.0.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    anyhow::bail!("fail {n}");
                }
                Ok(json!({ "ok": true }))
            }
        }

        let json = r#"{
            "nodes": [
                { "id": "x", "kind": "tool_call", "retry_policy": { "max_retries": 3, "backoff_ms": 1, "backoff_multiplier": 1.0 } }
            ],
            "edges": []
        }"#;

        let def = DagDefinition::from_json(json).unwrap();
        let graph = DagGraph::build(&def).unwrap();
        let tries = Arc::new(AtomicUsize::new(0));
        let ex = DagExecutor::new(graph, Arc::new(Flaky(tries.clone())));
        ex.run(ExecutionContext::new()).await.unwrap();
        assert_eq!(tries.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn dag_failure_policy_skip_continues_graph() {
        struct SkipAware;

        #[async_trait]
        impl NodeHandler for SkipAware {
            async fn execute_node(
                &self,
                node: &NodeDef,
                _ctx: &ExecutionContext,
            ) -> anyhow::Result<serde_json::Value> {
                if node.id == "b" {
                    anyhow::bail!("b always fails");
                }
                Ok(json!({ "id": node.id }))
            }
        }

        let json = r#"{
            "nodes": [
                { "id": "a", "kind": "tool_call" },
                { "id": "b", "kind": "tool_call", "failure_policy": "skip" },
                { "id": "c", "kind": "tool_call" }
            ],
            "edges": [
                { "from": "a", "to": "b" },
                { "from": "b", "to": "c" }
            ]
        }"#;

        let def = DagDefinition::from_json(json).unwrap();
        let graph = DagGraph::build(&def).unwrap();
        let ex = DagExecutor::new(graph, Arc::new(SkipAware));
        let ctx = ex.run(ExecutionContext::new()).await.unwrap();
        assert!(ctx.get("c").await.is_some(), "downstream should run");
    }

    #[tokio::test]
    async fn dag_condition_expression_routes_correctly() {
        struct PathHits {
            pos: Arc<AtomicUsize>,
            neg: Arc<AtomicUsize>,
        }

        #[async_trait]
        impl NodeHandler for PathHits {
            async fn execute_node(
                &self,
                node: &NodeDef,
                _ctx: &ExecutionContext,
            ) -> anyhow::Result<serde_json::Value> {
                if node.id == "pos_path" {
                    self.pos.fetch_add(1, Ordering::SeqCst);
                } else if node.id == "neg_path" {
                    self.neg.fetch_add(1, Ordering::SeqCst);
                }
                Ok(json!({ "id": node.id }))
            }
        }

        let json = r#"{
            "nodes": [
                { "id": "cond", "kind": "condition", "config": { "condition": "$.input.n > 0 ? \"pos\" : \"neg\"" } },
                { "id": "pos_path", "kind": "tool_call" },
                { "id": "neg_path", "kind": "tool_call" }
            ],
            "edges": [
                { "from": "cond", "to": "pos_path", "label": "pos" },
                { "from": "cond", "to": "neg_path", "label": "neg" }
            ]
        }"#;

        let def = DagDefinition::from_json(json).unwrap();
        let graph = DagGraph::build(&def).unwrap();
        let pos = Arc::new(AtomicUsize::new(0));
        let neg = Arc::new(AtomicUsize::new(0));
        let ex = DagExecutor::new(
            graph,
            Arc::new(PathHits {
                pos: pos.clone(),
                neg: neg.clone(),
            }),
        );
        let ctx = ExecutionContext::with_input(json!({ "n": 3 }));
        ex.run(ctx).await.unwrap();
        assert_eq!(pos.load(Ordering::SeqCst), 1);
        assert_eq!(neg.load(Ordering::SeqCst), 0);
    }

    struct CountingWork {
        hits: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl NodeHandler for CountingWork {
        async fn execute_node(
            &self,
            node: &NodeDef,
            _ctx: &ExecutionContext,
        ) -> anyhow::Result<serde_json::Value> {
            if node.id == "work" {
                let n = self.hits.fetch_add(1, Ordering::SeqCst) + 1;
                Ok(json!({ "n": n }))
            } else {
                Ok(json!({ "id": node.id }))
            }
        }
    }

    #[tokio::test]
    async fn loop_runs_fixed_max_iterations() {
        let json = r#"{
            "nodes": [
                { "id": "start", "kind": "tool_call" },
                { "id": "lp", "kind": "loop", "loop_config": { "max_iterations": 3 } },
                { "id": "work", "kind": "tool_call" },
                { "id": "end", "kind": "tool_call" }
            ],
            "edges": [
                { "from": "start", "to": "lp" },
                { "from": "lp", "to": "work", "label": "body" },
                { "from": "work", "to": "end" }
            ]
        }"#;

        let def = DagDefinition::from_json(json).unwrap();
        let graph = DagGraph::build(&def).unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let ex = DagExecutor::new(
            graph,
            Arc::new(CountingWork {
                hits: hits.clone(),
            }),
        );
        let ctx = ex.run(ExecutionContext::new()).await.unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 3);
        let arr = ctx
            .get("lp")
            .await
            .expect("loop output")
            .as_array()
            .cloned()
            .expect("array");
        assert_eq!(arr.len(), 3);
    }

    #[tokio::test]
    async fn loop_stops_when_condition_expr_false() {
        let json = r#"{
            "nodes": [
                { "id": "start", "kind": "tool_call" },
                { "id": "lp", "kind": "loop", "loop_config": { "max_iterations": 10, "condition_expr": "$.work.n < 3" } },
                { "id": "work", "kind": "tool_call" },
                { "id": "end", "kind": "tool_call" }
            ],
            "edges": [
                { "from": "start", "to": "lp" },
                { "from": "lp", "to": "work", "label": "body" },
                { "from": "work", "to": "end" }
            ]
        }"#;

        let def = DagDefinition::from_json(json).unwrap();
        let graph = DagGraph::build(&def).unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let ex = DagExecutor::new(
            graph,
            Arc::new(CountingWork {
                hits: hits.clone(),
            }),
        );
        let ctx = ex.run(ExecutionContext::new()).await.unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 3);
        let arr = ctx
            .get("lp")
            .await
            .unwrap()
            .as_array()
            .cloned()
            .unwrap();
        assert_eq!(arr.len(), 3);
    }

    #[tokio::test]
    async fn loop_runs_until_max_when_condition_always_true() {
        let json = r#"{
            "nodes": [
                { "id": "start", "kind": "tool_call" },
                { "id": "lp", "kind": "loop", "loop_config": { "max_iterations": 4, "condition_expr": "true" } },
                { "id": "work", "kind": "tool_call" },
                { "id": "end", "kind": "tool_call" }
            ],
            "edges": [
                { "from": "start", "to": "lp" },
                { "from": "lp", "to": "work", "label": "body" },
                { "from": "work", "to": "end" }
            ]
        }"#;

        let def = DagDefinition::from_json(json).unwrap();
        let graph = DagGraph::build(&def).unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let ex = DagExecutor::new(
            graph,
            Arc::new(CountingWork {
                hits: hits.clone(),
            }),
        );
        let ctx = ex.run(ExecutionContext::new()).await.unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 4);
        let arr = ctx.get("lp").await.unwrap().as_array().cloned().unwrap();
        assert_eq!(arr.len(), 4);
    }
}
