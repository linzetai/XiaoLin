//! Unified tool dispatch layer.
//!
//! `ToolDispatcher` replaces the legacy `execute_tool_batch` and the inline
//! guarded/unguarded split in `execute_stream_inner`. Every tool call — whether
//! invoked from the streaming executor, batch path, or any other entry point —
//! goes through `dispatch_one`, ensuring consistent policy checks, orchestrator
//! routing for guarded tools, and hooks/truncation for all tools.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use xiaolin_core::agent_config::BehaviorConfig;
use xiaolin_core::tool::{PostToolInfo, PreToolAction, ToolHook, ToolHookContext, ToolKind, ToolRegistry, ToolResult};
use xiaolin_core::tool_runtime::{ApprovalStrategy, SandboxPreference};
use xiaolin_core::types::ToolCall;
use xiaolin_protocol::{AgentEvent, TurnId};
use xiaolin_session_actor::InteractionHandle;

use super::approval_cache::ApprovalCache;
use super::orchestrator::{OrchestratorContext, ToolOrchestrator};
use super::permissions::DenialTracker;
use super::runtimes::RuntimeRegistry;
use super::tool_executor::truncate_tool_result_output_with_limit;
use crate::builtin_tools::ExecutionModeState;
use xiaolin_core::agent_config::FileAccessMode;
use xiaolin_tools_fs::filesystem::{with_additional_allowed_paths, with_file_access_mode, with_work_dir};

/// Result tuple returned by dispatch: (tool_name, call_id, arguments, result).
pub type DispatchResult = (String, String, String, ToolResult);

/// Outcome of running pre-tool-use hooks.
enum PreHookOutcome {
    Allow,
    Rewrite(String),
    Block(String),
}

/// Per-dispatch context shared across a batch of tool calls.
pub struct DispatchContext<'a> {
    pub turn_id: &'a TurnId,
    pub behavior: &'a BehaviorConfig,
    pub work_dir: &'a Option<String>,
    pub mode_state: Option<&'a ExecutionModeState>,
    pub plan_file_path: Option<PathBuf>,
    pub event_tx: &'a tokio::sync::mpsc::Sender<AgentEvent>,
    pub approval_strategy: &'a ApprovalStrategy,
    pub interaction_handle: Option<&'a InteractionHandle>,
    pub approval_cache: &'a mut ApprovalCache,
    pub denial_tracker: &'a mut DenialTracker,
    pub agent_id: &'a str,
}

/// Unified tool dispatcher that routes all tool calls through a consistent
/// pipeline: policy checks → pre-hooks → orchestrator (for guarded tools)
/// or direct execution (for safe tools) → post-hooks → truncation.
pub struct ToolDispatcher {
    tool_registry: Arc<ToolRegistry>,
    runtime_registry: Arc<RuntimeRegistry>,
    orchestrator: Arc<ToolOrchestrator>,
    hooks: Vec<Arc<dyn ToolHook>>,
}

impl ToolDispatcher {
    pub fn new(
        tool_registry: Arc<ToolRegistry>,
        runtime_registry: Arc<RuntimeRegistry>,
        orchestrator: Arc<ToolOrchestrator>,
    ) -> Self {
        Self {
            tool_registry,
            runtime_registry,
            orchestrator,
            hooks: Vec::new(),
        }
    }

    pub fn with_hooks(mut self, hooks: Vec<Arc<dyn ToolHook>>) -> Self {
        self.hooks = hooks;
        self
    }

    /// Execute a single tool call through the unified pipeline.
    ///
    /// Pipeline stages:
    /// 1. Pre-execution policy checks (allow/deny list, plan mode)
    /// 2. Pre-tool-use hooks (can block or rewrite arguments)
    /// 3. Execute (guarded via orchestrator, or direct)
    /// 4. Post-tool-use hooks (observe results, metrics)
    /// 5. Truncate output
    pub async fn dispatch_one(
        &self,
        tc: &ToolCall,
        ctx: &mut DispatchContext<'_>,
    ) -> DispatchResult {
        let tool_name = tc.function.name.clone();
        let call_id = tc.id.clone();
        let arguments = tc.function.arguments.clone();

        // 1. Pre-execution policy checks (allow/deny list, confirmation, plan mode)
        if let Some(result) = self.pre_execution_checks(tc, ctx) {
            return result;
        }

        // 2. Pre-tool-use hooks: can block or rewrite arguments
        let effective_tc = if self.hooks.is_empty() {
            tc.clone()
        } else {
            let hook_ctx = ToolHookContext {
                tool_name: tool_name.clone(),
                tool_kind: self.tool_kind(&tool_name),
                call_id: call_id.clone(),
                arguments: arguments.clone(),
                agent_id: ctx.agent_id.to_string(),
            };
            match self.run_pre_hooks(&hook_ctx).await {
                PreHookOutcome::Allow => tc.clone(),
                PreHookOutcome::Rewrite(new_args) => {
                    let mut rewritten = tc.clone();
                    rewritten.function.arguments = new_args;
                    rewritten
                }
                PreHookOutcome::Block(reason) => {
                    return (tool_name, call_id, arguments, ToolResult::err(reason));
                }
            }
        };

        // 3. Route to orchestrator or direct execution
        let start = std::time::Instant::now();
        let result = if self.runtime_registry.has(&tool_name) {
            self.execute_guarded(&effective_tc, ctx).await
        } else {
            self.execute_unguarded(&effective_tc, ctx).await
        };
        let latency_ms = start.elapsed().as_millis() as u64;

        // 4. Post-tool-use hooks (observe)
        if !self.hooks.is_empty() {
            let hook_ctx = ToolHookContext {
                tool_name: tool_name.clone(),
                tool_kind: self.tool_kind(&tool_name),
                call_id: call_id.clone(),
                arguments: effective_tc.function.arguments.clone(),
                agent_id: ctx.agent_id.to_string(),
            };
            let info = PostToolInfo {
                success: result.success,
                output_len: result.output.len(),
                latency_ms,
            };
            self.run_post_hooks(&hook_ctx, &info).await;
        }

        // 5. Truncate output if needed
        let result = self.truncate_result(&tool_name, result);

        (tool_name, call_id, arguments, result)
    }

    /// Execute a batch of tool calls with concurrency control.
    ///
    /// Concurrent-safe tools (Read/Search/Fetch/Think) run in parallel.
    /// Mutating tools (Edit/Execute) and guarded tools run sequentially.
    /// Duplicate `read_file` calls in the same batch are deduplicated.
    pub async fn dispatch_batch(
        &self,
        tool_calls: &[ToolCall],
        ctx: &mut DispatchContext<'_>,
    ) -> Vec<DispatchResult> {
        if tool_calls.is_empty() {
            return Vec::new();
        }

        // Batch-level dedup for read_file
        let dedup_source = self.build_dedup_map(tool_calls);

        // Partition into concurrent vs sequential
        let (concurrent_indices, sequential_indices) =
            self.partition_by_concurrency(tool_calls, &dedup_source);

        let mut results: Vec<Option<DispatchResult>> = vec![None; tool_calls.len()];

        // Execute concurrent tools in parallel
        // NOTE: concurrent tools are always unguarded (safe), so they don't need
        // mutable access to approval_cache/denial_tracker. We can safely execute
        // them without the mutable context by using a temporary context per call.
        if !concurrent_indices.is_empty() {
            let concurrent_futures: Vec<_> = concurrent_indices
                .iter()
                .map(|&i| {
                    let tc = tool_calls[i].clone();
                    let tool_registry = Arc::clone(&self.tool_registry);
                    let behavior = ctx.behavior.clone();
                    let work_dir = ctx.work_dir.clone();
                    let mode_state_current =
                        ctx.mode_state.map(|ms| ms.current_mode());
                    let plan_fp = ctx.plan_file_path.clone();
                    async move {
                        let result =
                            Self::execute_unguarded_standalone(
                                &tc,
                                &tool_registry,
                                &behavior,
                                &work_dir,
                                mode_state_current,
                                plan_fp.as_ref(),
                            )
                            .await;
                        let result = Self::truncate_result_static(&tc.function.name, result);
                        (
                            tc.function.name.clone(),
                            tc.id.clone(),
                            tc.function.arguments.clone(),
                            result,
                        )
                    }
                })
                .collect();
            let concurrent_results = futures::future::join_all(concurrent_futures).await;
            for (slot, result) in concurrent_indices.iter().zip(concurrent_results) {
                results[*slot] = Some(result);
            }
        }

        // Execute sequential tools one by one (includes guarded tools)
        for &i in &sequential_indices {
            let result = self.dispatch_one(&tool_calls[i], ctx).await;
            results[i] = Some(result);
        }

        // Fill dedup results
        for (i, source) in dedup_source.iter().enumerate() {
            if let Some(src_idx) = source {
                if let Some(ref original) = results[*src_idx] {
                    let dedup_output = format!(
                        "[duplicate read_file in same batch — identical to call_id {}]",
                        original.1,
                    );
                    let mut dedup_result = original.3.clone();
                    dedup_result.output = dedup_output;
                    results[i] = Some((
                        tool_calls[i].function.name.clone(),
                        tool_calls[i].id.clone(),
                        tool_calls[i].function.arguments.clone(),
                        dedup_result,
                    ));
                }
            }
        }

        results
            .into_iter()
            .map(|r| r.expect("all slots filled"))
            .collect()
    }

    /// Check if a tool name is guarded (registered in RuntimeRegistry).
    pub fn is_guarded(&self, tool_name: &str) -> bool {
        self.runtime_registry.has(tool_name)
    }

    /// Get the tool kind for concurrency classification.
    pub fn tool_kind(&self, tool_name: &str) -> ToolKind {
        self.tool_registry
            .get(tool_name)
            .map(|t| t.kind())
            .unwrap_or(ToolKind::Other)
    }

    // ─── Hook execution ──────────────────────────────────────────────────

    async fn run_pre_hooks(&self, hook_ctx: &ToolHookContext) -> PreHookOutcome {
        for hook in &self.hooks {
            let action: PreToolAction = hook.pre_tool_use(hook_ctx).await;
            if let Some(reason) = action.block_reason {
                tracing::info!(
                    hook = hook.name(),
                    tool = %hook_ctx.tool_name,
                    "pre-hook blocked tool: {reason}"
                );
                return PreHookOutcome::Block(reason);
            }
            if let Some(new_args) = action.modified_arguments {
                tracing::debug!(
                    hook = hook.name(),
                    tool = %hook_ctx.tool_name,
                    "pre-hook rewrote tool arguments"
                );
                return PreHookOutcome::Rewrite(new_args);
            }
        }
        PreHookOutcome::Allow
    }

    async fn run_post_hooks(&self, hook_ctx: &ToolHookContext, info: &PostToolInfo) {
        for hook in &self.hooks {
            hook.post_tool_use(hook_ctx, info).await;
        }
    }

    // ─── Internal helpers ─────────────────────────────────────────────────

    fn pre_execution_checks(
        &self,
        tc: &ToolCall,
        ctx: &DispatchContext<'_>,
    ) -> Option<DispatchResult> {
        let tool_name = &tc.function.name;
        let call_id = &tc.id;
        let arguments = &tc.function.arguments;

        if !is_tool_allowed(tool_name, ctx.behavior) {
            tracing::warn!(tool = %tool_name, "tool blocked by allow/deny policy");
            return Some((
                tool_name.clone(),
                call_id.clone(),
                arguments.clone(),
                ToolResult::err(format!("Tool '{}' is not in the allowed tool list.", tool_name)),
            ));
        }

        if let Some(ms) = ctx.mode_state {
            let kind = self.tool_kind(tool_name);
            if ms.is_blocked_for_tool(tool_name, kind) {
                if let Some(ref plan_path) = ctx.plan_file_path {
                    if is_plan_file_write(tool_name, arguments, plan_path) {
                        tracing::info!(tool = %tool_name, "plan file write allowed in plan mode");
                    } else {
                        tracing::info!(tool = %tool_name, kind = ?kind, "tool blocked by plan mode");
                        return Some((
                            tool_name.clone(),
                            call_id.clone(),
                            arguments.clone(),
                            ToolResult::typed_err(
                                xiaolin_core::tool::ToolErrorType::ExecutionDenied,
                                ExecutionModeState::blocked_message(tool_name),
                            ),
                        ));
                    }
                } else {
                    tracing::info!(tool = %tool_name, kind = ?kind, "tool blocked by plan mode");
                    return Some((
                        tool_name.clone(),
                        call_id.clone(),
                        arguments.clone(),
                        ToolResult::typed_err(
                            xiaolin_core::tool::ToolErrorType::ExecutionDenied,
                            ExecutionModeState::blocked_message(tool_name),
                        ),
                    ));
                }
            }
            if tool_name == "shell_exec"
                && ms.current_mode() == xiaolin_core::types::ExecutionMode::Plan
            {
                if let Some(cmd) = extract_command_from_args(arguments) {
                    if let Err(reason) = crate::builtin_tools::validate_readonly_command(&cmd) {
                        return Some((
                            tool_name.clone(),
                            call_id.clone(),
                            arguments.clone(),
                            ToolResult::typed_err(
                                xiaolin_core::tool::ToolErrorType::ExecutionDenied,
                                format!(
                                    "Plan mode (read-only) blocks this command: {reason}. \
                                     Only read-only commands (ls, cat, grep, git status, cargo check, etc.) \
                                     are allowed. Use exit_plan_mode to switch back to Agent mode for writes."
                                ),
                            ),
                        ));
                    }
                }
            }
        }

        // Validate tool arguments if the tool is registered
        if let Some(tool) = self.tool_registry.get(tool_name) {
            if let Some(err) = validate_tool_arguments(tool.as_ref(), arguments) {
                tracing::warn!(tool = %tool_name, "tool parameter validation failed: {err}");
                return Some((
                    tool_name.clone(),
                    call_id.clone(),
                    arguments.clone(),
                    ToolResult::err(err),
                ));
            }
        }

        None
    }

    async fn execute_guarded(
        &self,
        tc: &ToolCall,
        ctx: &mut DispatchContext<'_>,
    ) -> ToolResult {
        let tool_name = &tc.function.name;
        let args: serde_json::Value =
            serde_json::from_str(&tc.function.arguments).unwrap_or_default();
        let cwd = ctx
            .work_dir
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));

        let Some(rt) = self.runtime_registry.get(tool_name) else {
            return ToolResult::err(format!("runtime not found: {tool_name}"));
        };

        let skip_sandbox = rt.sandbox_preference() == SandboxPreference::Skip;

        if skip_sandbox {
            // For tools that skip sandbox (edit_file, write_file, etc.), run
            // approval through the orchestrator but execute via the actual Tool
            // implementation so that rich fields (display_output, metadata) are
            // preserved. The simplified ToolRuntime::run() only returns a plain
            // String, losing those fields.
            let mut orch_ctx = OrchestratorContext {
                turn_id: ctx.turn_id,
                cwd: &cwd,
                call_id: &tc.id,
                approval_cache: ctx.approval_cache,
                approval_strategy: ctx.approval_strategy,
                interaction_handle: ctx.interaction_handle,
                event_tx: ctx.event_tx,
                denial_tracker: ctx.denial_tracker,
            };

            if let Err(e) = self.orchestrator.authorize(rt.as_ref(), &args, &mut orch_ctx).await {
                return match e {
                    xiaolin_core::tool_runtime::ToolRuntimeError::Rejected { reason } => {
                        ToolResult::err(format!("Denied: {reason}"))
                    }
                    xiaolin_core::tool_runtime::ToolRuntimeError::Timeout { elapsed_ms } => {
                        ToolResult::err(format!("Timeout after {elapsed_ms}ms"))
                    }
                    other => ToolResult::err(other.to_string()),
                };
            }

            return self.execute_with_full_access(tc, ctx).await;
        }

        // Full orchestrator pipeline (approval + sandbox + execution) for tools
        // that need sandbox (e.g. shell_exec). The runtime's run() handles
        // sandbox-aware execution.
        let mut orch_ctx = OrchestratorContext {
            turn_id: ctx.turn_id,
            cwd: &cwd,
            call_id: &tc.id,
            approval_cache: ctx.approval_cache,
            approval_strategy: ctx.approval_strategy,
            interaction_handle: ctx.interaction_handle,
            event_tx: ctx.event_tx,
            denial_tracker: ctx.denial_tracker,
        };

        match self.orchestrator.run(rt.as_ref(), &args, &mut orch_ctx).await {
            Ok(orch_result) => ToolResult::ok(orch_result.output),
            Err(xiaolin_core::tool_runtime::ToolRuntimeError::Rejected { reason }) => {
                ToolResult::err(format!("Denied: {reason}"))
            }
            Err(xiaolin_core::tool_runtime::ToolRuntimeError::Timeout { elapsed_ms }) => {
                ToolResult::err(format!("Timeout after {elapsed_ms}ms"))
            }
            Err(e) => ToolResult::err(e.to_string()),
        }
    }

    async fn execute_unguarded(
        &self,
        tc: &ToolCall,
        ctx: &DispatchContext<'_>,
    ) -> ToolResult {
        let extra_paths =
            resolve_additional_allowed_paths(&ctx.behavior.additional_allowed_paths);
        let work_dir_path = ctx.work_dir.as_ref().map(PathBuf::from);

        match self.tool_registry.get(&tc.function.name) {
            Some(tool) => {
                with_file_access_mode(
                    ctx.behavior.file_access,
                    with_additional_allowed_paths(
                        extra_paths,
                        with_work_dir(work_dir_path, tool.execute(&tc.function.arguments)),
                    ),
                )
                .await
            }
            None => ToolResult::err(format!("tool not found: {}", tc.function.name)),
        }
    }

    /// Execute a tool with Full file access after explicit user approval.
    /// User approval means the user has reviewed and accepted the specific action,
    /// so path restrictions should not block execution.
    async fn execute_with_full_access(
        &self,
        tc: &ToolCall,
        ctx: &DispatchContext<'_>,
    ) -> ToolResult {
        let extra_paths =
            resolve_additional_allowed_paths(&ctx.behavior.additional_allowed_paths);
        let work_dir_path = ctx.work_dir.as_ref().map(PathBuf::from);

        match self.tool_registry.get(&tc.function.name) {
            Some(tool) => {
                with_file_access_mode(
                    FileAccessMode::Full,
                    with_additional_allowed_paths(
                        extra_paths,
                        with_work_dir(work_dir_path, tool.execute(&tc.function.arguments)),
                    ),
                )
                .await
            }
            None => ToolResult::err(format!("tool not found: {}", tc.function.name)),
        }
    }

    /// Standalone unguarded execution without mutable dispatch context.
    /// Used for concurrent (safe) tools in `dispatch_batch` and by
    /// `StreamingToolExecutor`.
    pub async fn execute_unguarded_standalone(
        tc: &ToolCall,
        tool_registry: &ToolRegistry,
        behavior: &BehaviorConfig,
        work_dir: &Option<String>,
        mode_state_mode: Option<xiaolin_core::types::ExecutionMode>,
        plan_file_path: Option<&PathBuf>,
    ) -> ToolResult {
        let tool_name = &tc.function.name;

        if !is_tool_allowed(tool_name, behavior) {
            return ToolResult::err(format!(
                "Tool '{}' is not in the allowed tool list.",
                tool_name
            ));
        }

        if let Some(mode) = mode_state_mode {
            if mode == xiaolin_core::types::ExecutionMode::Plan {
                let kind = tool_registry
                    .get(tool_name)
                    .map(|t| t.kind())
                    .unwrap_or(ToolKind::Other);

                if tool_name == "shell_exec" {
                    if let Some(cmd) = extract_command_from_args(&tc.function.arguments) {
                        if let Err(reason) = crate::builtin_tools::validate_readonly_command(&cmd) {
                            return ToolResult::typed_err(
                                xiaolin_core::tool::ToolErrorType::ExecutionDenied,
                                format!(
                                    "Plan mode (read-only) blocks this command: {reason}. \
                                     Only read-only commands are allowed."
                                ),
                            );
                        }
                    }
                } else if matches!(kind, ToolKind::Edit | ToolKind::Execute) {
                    let allowed = plan_file_path.is_some_and(|pfp|
                        is_plan_file_write(tool_name, &tc.function.arguments, pfp)
                    );
                    if !allowed {
                        return ToolResult::typed_err(
                            xiaolin_core::tool::ToolErrorType::ExecutionDenied,
                            ExecutionModeState::blocked_message(tool_name),
                        );
                    }
                }
            }
        }

        let extra_paths = resolve_additional_allowed_paths(&behavior.additional_allowed_paths);
        let work_dir_path = work_dir.as_ref().map(PathBuf::from);

        match tool_registry.get(tool_name) {
            Some(tool) => {
                with_file_access_mode(
                    behavior.file_access,
                    with_additional_allowed_paths(
                        extra_paths,
                        with_work_dir(work_dir_path, tool.execute(&tc.function.arguments)),
                    ),
                )
                .await
            }
            None => ToolResult::err(format!("tool not found: {}", tool_name)),
        }
    }

    fn truncate_result(&self, tool_name: &str, result: ToolResult) -> ToolResult {
        let char_limit = self
            .tool_registry
            .get(tool_name)
            .map(|t| t.max_result_size_chars())
            .unwrap_or(100_000);
        Self::truncate_result_with_limit(tool_name, result, char_limit)
    }

    pub fn truncate_result_static(tool_name: &str, result: ToolResult) -> ToolResult {
        Self::truncate_result_with_limit(tool_name, result, 100_000)
    }

    fn truncate_result_with_limit(tool_name: &str, mut result: ToolResult, char_limit: usize) -> ToolResult {
        if char_limit == usize::MAX {
            return result;
        }
        let truncated = truncate_tool_result_output_with_limit(&result.output, tool_name, Some(char_limit));
        if truncated.len() != result.output.len() {
            result.output = truncated;
        }
        result
    }

    fn build_dedup_map(&self, tool_calls: &[ToolCall]) -> Vec<Option<usize>> {
        let mut read_file_seen: HashMap<String, usize> = HashMap::new();
        let mut dedup_source: Vec<Option<usize>> = vec![None; tool_calls.len()];

        for (i, tc) in tool_calls.iter().enumerate() {
            if tc.function.name == "read_file" {
                if let Some(path) = extract_target_key("read_file", &tc.function.arguments) {
                    if let Some(&first_idx) = read_file_seen.get(&path) {
                        dedup_source[i] = Some(first_idx);
                        tracing::info!(
                            tool = "read_file",
                            path = %path,
                            "skipping duplicate read_file in same batch (first at index {first_idx})"
                        );
                    } else {
                        read_file_seen.insert(path, i);
                    }
                }
            }
        }

        dedup_source
    }

    fn partition_by_concurrency(
        &self,
        tool_calls: &[ToolCall],
        dedup_source: &[Option<usize>],
    ) -> (Vec<usize>, Vec<usize>) {
        let mut concurrent_indices = Vec::new();
        let mut sequential_indices = Vec::new();

        for (i, tc) in tool_calls.iter().enumerate() {
            if dedup_source[i].is_some() {
                continue;
            }
            // Guarded tools always run sequentially (need mutable approval context)
            if self.runtime_registry.has(&tc.function.name) {
                sequential_indices.push(i);
                continue;
            }
            if self.supports_parallel(&tc.function.name) {
                concurrent_indices.push(i);
            } else {
                sequential_indices.push(i);
            }
        }

        (concurrent_indices, sequential_indices)
    }

    /// Query whether a tool declares itself safe for parallel execution.
    fn supports_parallel(&self, tool_name: &str) -> bool {
        self.tool_registry
            .get(tool_name)
            .map(|t| t.supports_parallel())
            .unwrap_or(false)
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────

fn is_tool_allowed(tool_name: &str, behavior: &BehaviorConfig) -> bool {
    behavior.is_tool_allowed(tool_name)
}

fn extract_command_from_args(arguments: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(arguments)
        .ok()
        .and_then(|v| v.get("command").and_then(|c| c.as_str()).map(String::from))
}

fn extract_path_from_args(arguments: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(arguments)
        .ok()
        .and_then(|v| {
            v.get("file_path")
                .or_else(|| v.get("path"))
                .and_then(|p| p.as_str())
                .map(String::from)
        })
}

/// Check if a file-editing tool targets the plan file, which is allowed even in Plan mode.
fn is_plan_file_write(tool_name: &str, arguments: &str, plan_file_path: &std::path::Path) -> bool {
    let write_tools = ["write_file", "edit_file"];
    if !write_tools.contains(&tool_name) {
        return false;
    }
    let Some(target_path) = extract_path_from_args(arguments) else {
        return false;
    };
    let target = PathBuf::from(&target_path);
    let canonical_target = target.canonicalize().unwrap_or(target);
    let canonical_plan = plan_file_path.canonicalize().unwrap_or_else(|_| plan_file_path.to_path_buf());
    canonical_target == canonical_plan
}

fn extract_target_key(tool_name: &str, arguments: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(arguments).ok()?;
    match tool_name {
        "read_file" => v.get("path").and_then(|p| p.as_str()).map(String::from),
        _ => None,
    }
}

fn validate_tool_arguments(
    tool: &dyn xiaolin_core::tool::Tool,
    arguments: &str,
) -> Option<String> {
    let schema = tool.parameters_schema();
    if schema.required.is_empty() {
        return None;
    }
    let parsed: serde_json::Value = match serde_json::from_str(arguments) {
        Ok(v) => v,
        Err(e) => {
            return Some(format!(
                "Invalid JSON arguments for tool '{}': {}. Please provide valid JSON.",
                tool.name(),
                e
            ));
        }
    };
    let obj = match parsed.as_object() {
        Some(o) => o,
        None => {
            return Some(format!(
                "Arguments for tool '{}' must be a JSON object, got: {}",
                tool.name(),
                parsed
            ));
        }
    };
    let missing: Vec<&str> = schema
        .required
        .iter()
        .filter(|r| !obj.contains_key(r.as_str()))
        .map(|r| r.as_str())
        .collect();
    if missing.is_empty() {
        None
    } else {
        Some(format!(
            "Missing required parameter(s) for tool '{}': {}",
            tool.name(),
            missing.join(", ")
        ))
    }
}

fn resolve_additional_allowed_paths(raw: &[String]) -> Vec<PathBuf> {
    let home = dirs::home_dir();
    raw.iter()
        .map(|s| {
            if let Some(rest) = s.strip_prefix("~/") {
                home.as_ref()
                    .map(|h| h.join(rest))
                    .unwrap_or_else(|| PathBuf::from(s))
            } else {
                PathBuf::from(s)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_tool_allowed_respects_allow_list() {
        let mut behavior = BehaviorConfig::default();
        behavior.tools_allow = vec!["read_file".into()];
        assert!(is_tool_allowed("read_file", &behavior));
        assert!(!is_tool_allowed("shell_exec", &behavior));
    }

    #[test]
    fn is_tool_allowed_respects_deny_list() {
        let mut behavior = BehaviorConfig::default();
        behavior.tools_deny = vec!["shell_exec".into()];
        assert!(is_tool_allowed("read_file", &behavior));
        assert!(!is_tool_allowed("shell_exec", &behavior));
    }

    #[test]
    fn extract_command_from_args_works() {
        let args = r#"{"command":"ls -la","cwd":"/tmp"}"#;
        assert_eq!(extract_command_from_args(args), Some("ls -la".into()));
        assert_eq!(extract_command_from_args("{}"), None);
    }

    fn make_tool_call(id: &str, name: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            call_type: "function".into(),
            function: xiaolin_core::types::FunctionCall {
                name: name.into(),
                arguments: arguments.into(),
            },
            output: None,
            success: None,
            duration_ms: None,
        }
    }

    #[test]
    fn dedup_map_detects_duplicates() {
        let registry = Arc::new(ToolRegistry::new());
        let rt_registry = Arc::new(RuntimeRegistry::new());
        let orchestrator = Arc::new(ToolOrchestrator::new());
        let dispatcher = ToolDispatcher::new(registry, rt_registry, orchestrator);

        let calls = vec![
            make_tool_call("1", "read_file", r#"{"path":"/tmp/a.txt"}"#),
            make_tool_call("2", "read_file", r#"{"path":"/tmp/a.txt"}"#),
            make_tool_call("3", "read_file", r#"{"path":"/tmp/b.txt"}"#),
        ];

        let dedup = dispatcher.build_dedup_map(&calls);
        assert_eq!(dedup[0], None);
        assert_eq!(dedup[1], Some(0));
        assert_eq!(dedup[2], None);
    }
}
