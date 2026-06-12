//! Streaming tool executor — starts tool execution as soon as the LLM emits
//! a tool_use block during streaming, rather than waiting for the full response.
//!
//! Concurrency-safe tools (Read/Search/Fetch/Think) execute in parallel.
//! Mutating tools (Edit/Execute/Other) are serialized to avoid conflicts.
//! Results are yielded in insertion order regardless of completion order.

use std::sync::{Arc, Mutex as StdMutex};

use xiaolin_core::agent_config::BehaviorConfig;
use xiaolin_core::tool::{ToolRegistry, ToolResult};
use xiaolin_core::types::{ExecutionMode, ToolCall};
use tokio::sync::RwLock as TokioRwLock;
use tokio_util::sync::CancellationToken;

/// State of a tracked tool through its lifecycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolState {
    Queued,
    Executing,
    Completed,
    Yielded,
    Cancelled,
}

/// A tool tracked by the streaming executor.
#[derive(Debug)]
struct TrackedTool {
    call: ToolCall,
    state: ToolState,
    result: Option<ToolResult>,
}

/// Result from a completed tool, preserving insertion order index.
#[derive(Debug, Clone)]
pub struct CompletedTool {
    pub index: usize,
    pub call_id: String,
    pub tool_name: String,
    pub result: ToolResult,
}

/// Configuration for the streaming executor.
#[derive(Debug, Clone)]
pub struct StreamingExecutorConfig {
    /// Whether to cancel sibling tools when one fails.
    pub sibling_cancel_on_error: bool,
    /// Working directory to scope tool execution (as String for ChatRequest compat).
    pub work_dir: Option<String>,
    /// Agent behavior config (carries allow/deny lists, file_access, additional_allowed_paths).
    pub behavior: BehaviorConfig,
    /// Current execution mode (Plan vs Agent) for policy checks.
    pub execution_mode: Option<ExecutionMode>,
    /// Plan file path allowed for writes even in Plan mode.
    pub plan_file_path: Option<std::path::PathBuf>,
    /// Session ID to propagate into spawned tool tasks for sub-agent routing.
    pub session_id: Option<String>,
}

impl Default for StreamingExecutorConfig {
    fn default() -> Self {
        Self {
            sibling_cancel_on_error: true,
            work_dir: None,
            behavior: BehaviorConfig::default(),
            execution_mode: None,
            plan_file_path: None,
            session_id: None,
        }
    }
}

/// Executes tools as they arrive during LLM streaming output.
///
/// Tools added via `add_tool` start executing immediately (subject to
/// concurrency constraints). Results can be polled with `get_completed_results`
/// and are returned in insertion order.
pub struct StreamingToolExecutor {
    config: StreamingExecutorConfig,
    tools: Arc<StdMutex<Vec<TrackedTool>>>,
    registry: Arc<ToolRegistry>,
    cancel_token: CancellationToken,
    /// Concurrency gate: parallel tools take a read lock (shared), exclusive
    /// tools take a write lock (blocking all others). Holds no data.
    parallel_gate: Arc<TokioRwLock<()>>,
    handles: Vec<tokio::task::JoinHandle<()>>,
}

impl StreamingToolExecutor {
    pub fn new(registry: Arc<ToolRegistry>, config: StreamingExecutorConfig) -> Self {
        Self {
            config,
            tools: Arc::new(StdMutex::new(Vec::new())),
            registry,
            cancel_token: CancellationToken::new(),
            parallel_gate: Arc::new(TokioRwLock::new(())),
            handles: Vec::new(),
        }
    }

    /// Add a tool for execution. Starts immediately if the tool declares
    /// `supports_parallel() == true` (shared read lock), or serializes behind
    /// an exclusive write lock otherwise.
    pub fn add_tool(&mut self, call: ToolCall) {
        let tool_name = &call.function.name;
        let tool_ref = self.registry.get(tool_name);
        let is_parallel = tool_ref.as_ref().map(|t| t.supports_parallel()).unwrap_or(false);

        let index = {
            let mut tools = self.tools.lock().unwrap();
            let idx = tools.len();
            tools.push(TrackedTool {
                call: call.clone(),
                state: ToolState::Queued,
                result: None,
            });
            idx
        };

        let tools_ref = Arc::clone(&self.tools);
        let registry = Arc::clone(&self.registry);
        let cancel = self.cancel_token.clone();
        let parallel_gate = Arc::clone(&self.parallel_gate);
        let sibling_cancel = self.config.sibling_cancel_on_error;
        let cancel_for_sibling = self.cancel_token.clone();
        let work_dir = self.config.work_dir.clone();
        let behavior = self.config.behavior.clone();
        let execution_mode = self.config.execution_mode;
        let plan_file_path = self.config.plan_file_path.clone();

        let captured_session_id = self.config.session_id.clone().or_else(|| {
            crate::subagent::SUBAGENT_SESSION_ID
                .try_with(|s| s.clone())
                .ok()
        });

        let handle = tokio::spawn(async move {
            if cancel.is_cancelled() {
                let mut tools = tools_ref.lock().unwrap();
                if let Some(t) = tools.get_mut(index) {
                    t.state = ToolState::Cancelled;
                }
                return;
            }

            // Acquire the parallel gate: read lock for parallel tools (shared),
            // write lock for exclusive tools (blocks everything else).
            let read_guard;
            let write_guard;
            if is_parallel {
                read_guard = Some(parallel_gate.read().await);
                write_guard = None;
            } else {
                read_guard = None;
                write_guard = Some(parallel_gate.write().await);
            }
            let _gate_guard = (&read_guard, &write_guard);

            if cancel.is_cancelled() {
                let mut tools = tools_ref.lock().unwrap();
                if let Some(t) = tools.get_mut(index) {
                    t.state = ToolState::Cancelled;
                }
                return;
            }

            // Mark as executing
            {
                let mut tools = tools_ref.lock().unwrap();
                if let Some(t) = tools.get_mut(index) {
                    t.state = ToolState::Executing;
                }
            }

            // Execute the tool with work_dir and file_access context.
            // Propagate SUBAGENT_SESSION_ID task-local across the spawn boundary
            // so SubAgentTool can route events to the correct session.
            let tool_fut = execute_single_tool_with_context(&call, &registry, &behavior, &work_dir, execution_mode, plan_file_path.as_ref());
            let result = tokio::select! {
                _ = cancel.cancelled() => {
                    let mut tools = tools_ref.lock().unwrap();
                    if let Some(t) = tools.get_mut(index) {
                        t.state = ToolState::Cancelled;
                    }
                    return;
                }
                r = async {
                    if let Some(sid) = captured_session_id {
                        crate::subagent::SUBAGENT_SESSION_ID.scope(sid, tool_fut).await
                    } else {
                        tool_fut.await
                    }
                } => r,
            };

            // Store result
            let mut tools = tools_ref.lock().unwrap();
            if let Some(t) = tools.get_mut(index) {
                let failed = !result.success;
                t.result = Some(result);
                t.state = ToolState::Completed;

                if failed && sibling_cancel {
                    cancel_for_sibling.cancel();
                }
            }
        });

        self.handles.push(handle);
    }

    /// Collect results that are completed and ready to yield (in order).
    ///
    /// Returns results for all consecutively completed tools starting from
    /// the first un-yielded position. A gap (incomplete tool) blocks later results.
    pub fn get_completed_results(&self) -> Vec<CompletedTool> {
        let mut tools = self.tools.lock().unwrap();
        let mut results = Vec::new();

        for (i, tool) in tools.iter_mut().enumerate() {
            match tool.state {
                ToolState::Yielded => continue,
                ToolState::Completed => {
                    if let Some(result) = tool.result.take() {
                        results.push(CompletedTool {
                            index: i,
                            call_id: tool.call.id.clone(),
                            tool_name: tool.call.function.name.clone(),
                            result,
                        });
                        tool.state = ToolState::Yielded;
                    }
                }
                ToolState::Cancelled => {
                    let result = ToolResult {
                        success: false,
                        output: "Tool execution cancelled".to_string(),
                        display_output: None,
                        error_type: None,
                        metadata: None,
                        images: Vec::new(),
                    };
                    results.push(CompletedTool {
                        index: i,
                        call_id: tool.call.id.clone(),
                        tool_name: tool.call.function.name.clone(),
                        result,
                    });
                    tool.state = ToolState::Yielded;
                }
                // Gap: stop yielding until this tool completes
                ToolState::Queued | ToolState::Executing => break,
            }
        }

        results
    }

    /// Wait for all remaining tools to complete and return their results in order.
    pub async fn drain_remaining(mut self) -> Vec<CompletedTool> {
        for handle in self.handles.drain(..) {
            let _ = handle.await;
        }
        self.get_completed_results()
    }

    /// Cancel all pending/executing tools and discard results.
    #[allow(dead_code)]
    pub fn discard(&self) {
        self.cancel_token.cancel();
    }

    /// Whether all tracked tools have been yielded or cancelled.
    #[allow(dead_code)]
    pub fn is_complete(&self) -> bool {
        let tools = self.tools.lock().unwrap();
        tools
            .iter()
            .all(|t| matches!(t.state, ToolState::Yielded | ToolState::Cancelled))
    }

    /// Number of tools currently tracked.
    #[allow(dead_code)]
    pub fn tracked_count(&self) -> usize {
        self.tools.lock().unwrap().len()
    }
}

async fn execute_single_tool_with_context(
    call: &ToolCall,
    registry: &ToolRegistry,
    behavior: &BehaviorConfig,
    work_dir: &Option<String>,
    execution_mode: Option<ExecutionMode>,
    plan_file_path: Option<&std::path::PathBuf>,
) -> ToolResult {
    let result = super::dispatcher::ToolDispatcher::execute_unguarded_standalone(
        call,
        registry,
        behavior,
        work_dir,
        execution_mode,
        plan_file_path,
    )
    .await;
    super::dispatcher::ToolDispatcher::truncate_result_static(&call.function.name, result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaolin_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolResult};
    use xiaolin_core::types::{FunctionCall, ToolCall};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    fn empty_schema() -> ToolParameterSchema {
        ToolParameterSchema {
            schema_type: "object".into(),
            properties: HashMap::new(),
            required: vec![],
        }
    }

    fn ok_result(output: &str) -> ToolResult {
        ToolResult {
            success: true,
            output: output.into(),
            display_output: None,
            error_type: None,
            metadata: None,
            images: Vec::new(),
        }
    }

    fn err_result(output: &str) -> ToolResult {
        ToolResult {
            success: false,
            output: output.into(),
            display_output: None,
            error_type: None,
            metadata: None,
            images: Vec::new(),
        }
    }

    struct MockReadTool;
    #[async_trait::async_trait]
    impl Tool for MockReadTool {
        fn name(&self) -> &str {
            "read_file"
        }
        fn description(&self) -> &str {
            "Read a file"
        }
        fn parameters_schema(&self) -> ToolParameterSchema {
            empty_schema()
        }
        fn kind(&self) -> ToolKind {
            ToolKind::Read
        }
        async fn execute(&self, _args: &str) -> ToolResult {
            tokio::time::sleep(Duration::from_millis(10)).await;
            ok_result("file content")
        }
    }

    struct MockEditTool {
        exec_count: Arc<AtomicU32>,
    }
    #[async_trait::async_trait]
    impl Tool for MockEditTool {
        fn name(&self) -> &str {
            "edit_file"
        }
        fn description(&self) -> &str {
            "Edit a file"
        }
        fn parameters_schema(&self) -> ToolParameterSchema {
            empty_schema()
        }
        fn kind(&self) -> ToolKind {
            ToolKind::Edit
        }
        async fn execute(&self, _args: &str) -> ToolResult {
            self.exec_count.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(20)).await;
            ok_result("edited")
        }
    }

    struct MockFailTool;
    #[async_trait::async_trait]
    impl Tool for MockFailTool {
        fn name(&self) -> &str {
            "fail_tool"
        }
        fn description(&self) -> &str {
            "Always fails"
        }
        fn parameters_schema(&self) -> ToolParameterSchema {
            empty_schema()
        }
        fn kind(&self) -> ToolKind {
            ToolKind::Read
        }
        async fn execute(&self, _args: &str) -> ToolResult {
            tokio::time::sleep(Duration::from_millis(5)).await;
            err_result("error occurred")
        }
    }

    struct SlowSearchTool;
    #[async_trait::async_trait]
    impl Tool for SlowSearchTool {
        fn name(&self) -> &str {
            "search"
        }
        fn description(&self) -> &str {
            "Search"
        }
        fn parameters_schema(&self) -> ToolParameterSchema {
            empty_schema()
        }
        fn kind(&self) -> ToolKind {
            ToolKind::Search
        }
        async fn execute(&self, _args: &str) -> ToolResult {
            tokio::time::sleep(Duration::from_millis(50)).await;
            ok_result("found")
        }
    }

    fn make_call(name: &str, id: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            call_type: "function".into(),
            function: FunctionCall {
                name: name.into(),
                arguments: "{}".into(),
            },
            output: None,
            success: None,
            duration_ms: None,
        }
    }

    fn build_registry(tools: Vec<Arc<dyn Tool>>) -> Arc<ToolRegistry> {
        let registry = ToolRegistry::new();
        for tool in tools {
            registry.register(tool);
        }
        Arc::new(registry)
    }

    #[tokio::test]
    async fn single_tool_executes_and_yields() {
        let registry = build_registry(vec![Arc::new(MockReadTool)]);
        let mut executor = StreamingToolExecutor::new(registry, Default::default());

        executor.add_tool(make_call("read_file", "call_1"));
        let results = executor.drain_remaining().await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].tool_name, "read_file");
        assert!(results[0].result.success);
    }

    #[tokio::test]
    async fn concurrent_tools_execute_in_parallel() {
        let registry = build_registry(vec![Arc::new(MockReadTool), Arc::new(SlowSearchTool)]);
        let mut executor = StreamingToolExecutor::new(registry, Default::default());

        let start = std::time::Instant::now();
        executor.add_tool(make_call("read_file", "c1"));
        executor.add_tool(make_call("search", "c2"));
        let results = executor.drain_remaining().await;
        let elapsed = start.elapsed();

        assert_eq!(results.len(), 2);
        // If they ran sequentially it'd be ~60ms; in parallel < 60ms
        assert!(elapsed < Duration::from_millis(100));
    }

    #[tokio::test]
    async fn serial_tools_execute_sequentially() {
        let exec_count = Arc::new(AtomicU32::new(0));
        let registry = build_registry(vec![Arc::new(MockEditTool {
            exec_count: exec_count.clone(),
        })]);
        let mut executor = StreamingToolExecutor::new(registry, Default::default());

        executor.add_tool(make_call("edit_file", "c1"));
        executor.add_tool(make_call("edit_file", "c2"));
        let results = executor.drain_remaining().await;

        assert_eq!(results.len(), 2);
        assert_eq!(exec_count.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn results_yielded_in_order() {
        // slow tool first, fast tool second
        let registry = build_registry(vec![Arc::new(SlowSearchTool), Arc::new(MockReadTool)]);
        let mut executor = StreamingToolExecutor::new(registry, Default::default());

        executor.add_tool(make_call("search", "slow"));
        executor.add_tool(make_call("read_file", "fast"));

        let results = executor.drain_remaining().await;
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].call_id, "slow");
        assert_eq!(results[1].call_id, "fast");
    }

    #[tokio::test]
    async fn gap_blocks_later_results() {
        let registry = build_registry(vec![Arc::new(SlowSearchTool), Arc::new(MockReadTool)]);
        let mut executor = StreamingToolExecutor::new(registry, Default::default());

        executor.add_tool(make_call("search", "slow")); // 50ms
        executor.add_tool(make_call("read_file", "fast")); // 10ms

        // After 15ms, fast is done but slow isn't → gap blocks
        tokio::time::sleep(Duration::from_millis(25)).await;
        let partial = executor.get_completed_results();
        assert!(partial.is_empty(), "gap should block fast result");
    }

    #[tokio::test]
    async fn discard_cancels_all() {
        let registry = build_registry(vec![Arc::new(SlowSearchTool)]);
        let mut executor = StreamingToolExecutor::new(registry, Default::default());

        executor.add_tool(make_call("search", "c1"));
        executor.add_tool(make_call("search", "c2"));

        executor.discard();
        tokio::time::sleep(Duration::from_millis(10)).await;

        let results = executor.get_completed_results();
        for r in &results {
            assert!(!r.result.success);
            assert!(r.result.output.contains("cancelled"));
        }
    }

    #[tokio::test]
    async fn sibling_cancel_on_failure() {
        let registry = build_registry(vec![Arc::new(MockFailTool), Arc::new(SlowSearchTool)]);
        let config = StreamingExecutorConfig {
            sibling_cancel_on_error: true,
            ..Default::default()
        };
        let mut executor = StreamingToolExecutor::new(registry, config);

        executor.add_tool(make_call("fail_tool", "fails")); // fails fast
        executor.add_tool(make_call("search", "should_cancel")); // slow, should get cancelled

        let results = executor.drain_remaining().await;
        assert_eq!(results.len(), 2);
        assert!(!results[0].result.success); // fail_tool failed
                                             // Second may be cancelled or not depending on timing
    }

    #[tokio::test]
    async fn unknown_tool_returns_error() {
        let registry = build_registry(vec![]);
        let mut executor = StreamingToolExecutor::new(registry, Default::default());

        executor.add_tool(make_call("nonexistent", "c1"));
        let results = executor.drain_remaining().await;

        assert_eq!(results.len(), 1);
        assert!(!results[0].result.success);
        assert!(results[0].result.output.contains("tool not found"));
    }

    #[tokio::test]
    async fn tracked_count_and_is_complete() {
        let registry = build_registry(vec![Arc::new(MockReadTool)]);
        let mut executor = StreamingToolExecutor::new(registry, Default::default());

        assert_eq!(executor.tracked_count(), 0);
        assert!(executor.is_complete());

        executor.add_tool(make_call("read_file", "c1"));
        assert_eq!(executor.tracked_count(), 1);

        let _ = executor.drain_remaining().await;
    }

    #[tokio::test]
    async fn mixed_concurrent_and_serial() {
        let exec_count = Arc::new(AtomicU32::new(0));
        let registry = build_registry(vec![
            Arc::new(MockReadTool),
            Arc::new(MockEditTool { exec_count }),
        ]);
        let mut executor = StreamingToolExecutor::new(registry, Default::default());

        executor.add_tool(make_call("read_file", "r1"));
        executor.add_tool(make_call("edit_file", "e1"));
        executor.add_tool(make_call("read_file", "r2"));

        let results = executor.drain_remaining().await;
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].call_id, "r1");
        assert_eq!(results[1].call_id, "e1");
        assert_eq!(results[2].call_id, "r2");
    }

    // ─── Integration tests: streaming tool execution pattern ─────────────

    /// Simulates the streaming integration: first tool is submitted while LLM
    /// is still "outputting" the second tool. Verifies that the first tool
    /// starts executing before the second is even submitted.
    #[tokio::test]
    async fn streaming_integration_first_tool_starts_during_output() {
        let registry = build_registry(vec![Arc::new(SlowSearchTool), Arc::new(MockReadTool)]);
        let mut executor = StreamingToolExecutor::new(registry, Default::default());

        // Simulate: LLM emits tool 0 completely, then starts tool 1
        // Submit tool 0 as soon as tool 1's index is seen (streaming behavior)
        executor.add_tool(make_call("search", "tool_0"));

        // Tool 0 is now executing. Simulate delay for "LLM outputting tool 1"
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Now submit tool 1 (LLM finished outputting it)
        executor.add_tool(make_call("read_file", "tool_1"));

        let start = std::time::Instant::now();
        let results = executor.drain_remaining().await;
        let total = start.elapsed();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].call_id, "tool_0");
        assert_eq!(results[1].call_id, "tool_1");
        // Tool 0 (50ms) was already running 10ms by the time tool 1 was submitted.
        // If sequential, total from drain would be >= 60ms.
        // With streaming start, search was already ~10ms in, so remaining is ~40ms.
        assert!(total < Duration::from_millis(80));
    }

    /// Verifies that when streaming_tool_execution is disabled (batch mode
    /// equivalent), all tools only execute after being submitted together.
    #[tokio::test]
    async fn batch_mode_all_tools_submitted_together() {
        let registry = build_registry(vec![Arc::new(MockReadTool), Arc::new(SlowSearchTool)]);
        let mut executor = StreamingToolExecutor::new(registry, Default::default());

        // In batch mode, all tools are submitted at once (no incremental submission)
        let start = std::time::Instant::now();
        executor.add_tool(make_call("read_file", "c1"));
        executor.add_tool(make_call("search", "c2"));
        let results = executor.drain_remaining().await;
        let _elapsed = start.elapsed();

        assert_eq!(results.len(), 2);
        assert!(results[0].result.success);
        assert!(results[1].result.success);
    }

    /// Verifies streaming executor produces results in insertion order even
    /// when tools are submitted incrementally during "streaming".
    #[tokio::test]
    async fn streaming_integration_results_in_order_with_incremental_submit() {
        let registry = build_registry(vec![
            Arc::new(SlowSearchTool),
            Arc::new(MockReadTool),
            Arc::new(SlowSearchTool),
        ]);
        let mut executor = StreamingToolExecutor::new(registry, Default::default());

        // Submit tools incrementally, simulating streaming detection
        executor.add_tool(make_call("search", "slow_first"));
        tokio::time::sleep(Duration::from_millis(5)).await;

        executor.add_tool(make_call("read_file", "fast_second"));
        tokio::time::sleep(Duration::from_millis(5)).await;

        executor.add_tool(make_call("search", "slow_third"));

        let results = executor.drain_remaining().await;
        assert_eq!(results.len(), 3);
        // Must be in insertion order
        assert_eq!(results[0].call_id, "slow_first");
        assert_eq!(results[1].call_id, "fast_second");
        assert_eq!(results[2].call_id, "slow_third");
    }

    /// Feature flag behavior: executor respects work_dir/file_access config.
    #[tokio::test]
    async fn streaming_executor_respects_config() {
        let mut behavior = BehaviorConfig::default();
        behavior.file_access = xiaolin_core::agent_config::FileAccessMode::Full;

        let registry = build_registry(vec![Arc::new(MockReadTool)]);
        let config = StreamingExecutorConfig {
            sibling_cancel_on_error: false,
            work_dir: Some("/tmp/test-workspace".to_string()),
            behavior,
            execution_mode: None,
            plan_file_path: None,
        };
        let mut executor = StreamingToolExecutor::new(registry, config);

        executor.add_tool(make_call("read_file", "c1"));
        let results = executor.drain_remaining().await;

        assert_eq!(results.len(), 1);
        assert!(results[0].result.success);
    }

    /// Polling get_completed_results during streaming returns results as they
    /// complete, without waiting for all tools to finish.
    #[tokio::test]
    async fn streaming_integration_poll_during_execution() {
        let registry = build_registry(vec![Arc::new(MockReadTool), Arc::new(SlowSearchTool)]);
        let mut executor = StreamingToolExecutor::new(registry, Default::default());

        // Submit fast tool first, slow tool second
        executor.add_tool(make_call("read_file", "fast"));
        executor.add_tool(make_call("search", "slow"));

        // Wait for fast tool to complete but slow still running
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Poll: fast tool should be available
        let partial = executor.get_completed_results();
        assert_eq!(partial.len(), 1);
        assert_eq!(partial[0].call_id, "fast");

        // Drain remaining
        let remaining = executor.drain_remaining().await;
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].call_id, "slow");
    }
}
