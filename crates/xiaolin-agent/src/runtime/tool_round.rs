use xiaolin_core::types::{ChatMessage, ChatRequest, Role, SessionId, ToolCall};
use xiaolin_evolution::TrajectoryStep;
use xiaolin_protocol::{ExecutionMode, TurnSummary};

use crate::builtin_tools::GoalStatus;

use super::agent_step::{AgentStep, TurnEndReason};
use super::dispatcher::DispatchContext;
use super::file_persistence::{FileArtifact, FileOp};
use super::llm_call::LlmStreamOutput;
use super::permissions;
use super::query_state;
use super::stream_engine::send_step;
use super::tool_executor::semantic_header;
use super::trajectory::truncate_for_trajectory;
use super::turn_state::{TurnMutableState, TurnServices};
use super::validation_pipeline::ValidationContext;
use super::{
    extract_file_path_from_args, extract_file_paths_from_args, inject_tool_recovery_guidance,
    make_turn_summary, process_tool_output, tool_result_content, track_restoration_state,
};

const FILE_ARTIFACT_TOOLS: &[&str] = &[
    "edit_file",
    "write_file",
    "create_file",
    "str_replace_editor",
    "apply_patch",
    "multi_edit",
];

fn is_file_artifact_tool(tool_name: &str) -> bool {
    FILE_ARTIFACT_TOOLS.contains(&tool_name)
}

fn resolve_artifact_path(work_dir: Option<&str>, path: &std::path::Path) -> std::path::PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    work_dir
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
        .join(path)
}

fn record_successful_file_artifacts(
    svc: &TurnServices,
    tool_name: &str,
    call_id: &str,
    arguments: &str,
    file_pre_snapshots: &std::collections::HashMap<std::path::PathBuf, (Option<String>, bool)>,
) {
    let Some(session_id) = svc.session_id.as_deref() else {
        return;
    };

    let paths = extract_file_paths_from_args(tool_name, arguments);
    if paths.is_empty() {
        return;
    }

    for path in paths {
        let resolved = resolve_artifact_path(svc.work_dir.as_deref(), &path);
        let op = match tool_name {
            "edit_file" | "multi_edit" | "str_replace_editor" | "apply_patch" => FileOp::Modified,
            "create_file" => FileOp::Created,
            _ => {
                let file_existed = file_pre_snapshots
                    .get(&path)
                    .map(|(_, exists)| *exists)
                    .unwrap_or_else(|| resolved.exists());
                if file_existed { FileOp::Modified } else { FileOp::Created }
            }
        };
        let bytes = std::fs::metadata(&resolved).map(|m| m.len()).unwrap_or(0);
        let artifact = FileArtifact::new(session_id, resolved, op, call_id, bytes);
        svc.services.record_file_artifact(&svc.event_tx, &svc.turn_id, artifact);
    }
}

/// State captured before tool execution for detecting external changes.
pub(crate) struct PreToolSnapshot {
    pub mode_before: Option<ExecutionMode>,
    pub goal_before: Option<(String, GoalStatus)>,
}

/// Outcome of a tool execution round.
pub(crate) enum ToolRoundOutcome {
    /// All tools processed; the outer loop should continue to post-tool phase.
    Continue {
        pre_snapshot: PreToolSnapshot,
        force_stop_loop: bool,
        plan_approval_pending: bool,
    },
    /// Turn ended early because assembled tool calls were empty
    /// (stream produced no valid calls).
    EarlyFinish(TurnSummary),
}

fn minimal_chat_request(svc: &TurnServices) -> ChatRequest {
    ChatRequest {
        model: None,
        messages: Vec::new(),
        agent_id: None,
        session_id: svc
            .session_id
            .as_ref()
            .map(|s| SessionId::from(s.clone())),
        stream: false,
        temperature: None,
        max_tokens: None,
        tools: None,
        slash_intent: None,
        work_dir: svc.work_dir.clone(),
        response_language: None,
        goal_mode: None,
    }
}

/// Execute all tool calls from the LLM response in a single round.
///
/// This phase covers:
/// 1. Assembling tool calls from accumulated deltas
/// 2. Handling empty-call edge case (→ EarlyFinish)
/// 3. Pushing assistant message with tool calls
/// 4. Emitting ToolExecuting events
/// 5. Dispatching tools (streaming path or batch path)
/// 6. Processing each tool result:
///    - Permission check
///    - UndoEngine + FilePersistence
///    - Pre/Post tool hooks
///    - Restoration state tracking
///    - Observer + trajectory recording
///    - Repetition detection
///    - Error/success tracking + error limit check
///    - ValidationPipeline
///    - Auto-fix guidance injection
///    - Self-iter recovery
/// 7. Grace turn injection when error limit is hit
///
/// # Returns
///
/// - `Continue { ... }` when all tools processed successfully or error limit
///   triggered a `break` from the tool loop (outer loop should proceed to post-tool).
/// - `EarlyFinish(summary)` when no valid tool calls were assembled.
pub(crate) async fn execute_tool_round(
    ms: &mut TurnMutableState,
    svc: &TurnServices,
    llm_output: &mut LlmStreamOutput,
) -> ToolRoundOutcome {
    let accumulated_content = llm_output.accumulated_content.clone();
    let accumulated_reasoning = llm_output.accumulated_reasoning.clone();
    let last_submitted_tool_idx = llm_output.last_submitted_tool_idx;
    let streaming_executor = llm_output.streaming_executor.take();
    let max_errors = svc.config.behavior.max_consecutive_errors;
    let minimal_request = minimal_chat_request(svc);

    let assembled_calls: Vec<ToolCall> = llm_output
        .tool_call_accum
        .iter()
        .filter(|a| !a.name.is_empty())
        .flat_map(|a| a.to_tool_calls())
        .collect();

    if assembled_calls.is_empty() {
        tracing::warn!("stream tool call deltas produced no valid tool calls, stopping");
        let summary = make_turn_summary(
            &svc.turn_id,
            &ms.query_loop,
            svc.stream_start,
            svc.context_window,
        );
        let _ = send_step(
            &svc.step_tx,
            AgentStep::TurnEnd {
                turn_id: svc.turn_id.clone(),
                reason: TurnEndReason::Completed,
                summary: summary.clone(),
                session_id: svc.session_id.clone(),
            },
            false,
        )
        .await;
        svc.services
            .fire_stop_hooks(&ms.messages, &[])
            .await;
        svc.runtime
            .finalize_injected_skills(&ms.injected_skill_ids, true)
            .await;
        return ToolRoundOutcome::EarlyFinish(summary);
    }

    ms.messages.push(ChatMessage {
        role: Role::Assistant,
        content: if accumulated_content.is_empty() {
            None
        } else {
            Some(serde_json::Value::String(accumulated_content.clone()))
        },
        reasoning_content: if accumulated_reasoning.is_empty() {
            None
        } else {
            Some(accumulated_reasoning.clone())
        },
        tool_calls: Some(assembled_calls.clone()),
        ..Default::default()
    });

    ms.had_tool_calls_this_round = true;

    // Emit ToolExecuting events for all tool calls first.
    for tc in &assembled_calls {
        let args_str = if tc.function.arguments.is_empty() {
            None
        } else {
            Some(tc.function.arguments.clone())
        };
        let _ = send_step(
            &svc.step_tx,
            AgentStep::ToolExecuting {
                turn_id: svc.turn_id.clone(),
                tool_name: tc.function.name.clone(),
                call_id: tc.id.clone(),
                args: args_str,
            },
            false,
        )
        .await;
    }

    let mode_before = svc.mode_state.as_ref().map(|ms| ms.current_mode());
    let goal_before = if let Some(ref gs) = svc.goal_store {
        gs.get_current().await.map(|g| (g.id.clone(), g.status))
    } else {
        None
    };

    // Execute tool calls through the ToolDispatcher (unified pipeline).
    //
    // When a streaming executor exists, drain it for tools already submitted
    // and route remaining guarded tools through dispatch_one.
    // When no streaming executor, dispatch_batch handles everything.
    let tool_dispatch_t0 = std::time::Instant::now();
    let tool_count = assembled_calls.len();
    let file_pre_snapshots: std::collections::HashMap<
        std::path::PathBuf,
        (Option<String>, bool),
    > = {
        let mut snaps = std::collections::HashMap::new();
        for tc in &assembled_calls {
            if is_file_artifact_tool(&tc.function.name) {
                for fp in extract_file_paths_from_args(&tc.function.name, &tc.function.arguments) {
                    if let std::collections::hash_map::Entry::Vacant(e) = snaps.entry(fp) {
                        let resolved = resolve_artifact_path(svc.work_dir.as_deref(), e.key());
                        let exists = resolved.exists();
                        let content = tokio::fs::read_to_string(&resolved).await.ok();
                        e.insert((content, exists));
                    }
                }
            }
        }
        snaps
    };

    let stream_results = if let Some(mut executor) = streaming_executor {
        // Streaming path: some tools were already submitted during streaming.
        let submit_start = last_submitted_tool_idx.map(|i| i + 1).unwrap_or(0);
        for tc in &assembled_calls[submit_start..] {
            if !svc.dispatcher.is_guarded(&tc.function.name) && !tc.function.name.is_empty() {
                executor.add_tool(tc.clone());
            }
        }
        let completed = executor.drain_remaining().await;
        let mut all_results: Vec<
            Option<(String, String, String, xiaolin_core::tool::ToolResult)>,
        > = vec![None; assembled_calls.len()];

        // Place streaming results by call_id lookup
        let completed_map: std::collections::HashMap<String, (String, xiaolin_core::tool::ToolResult)> =
            completed
                .into_iter()
                .map(|ct| (ct.call_id, (ct.tool_name, ct.result)))
                .collect();
        for (i, tc) in assembled_calls.iter().enumerate() {
            if !svc.dispatcher.is_guarded(&tc.function.name) {
                if let Some((name, result)) = completed_map.get(&tc.id) {
                    all_results[i] = Some((
                        name.clone(),
                        tc.id.clone(),
                        tc.function.arguments.clone(),
                        result.clone(),
                    ));
                } else {
                    tracing::warn!(
                        call_id = %tc.id,
                        tool_name = %tc.function.name,
                        "streaming result not found for call_id, tool execution may have been skipped"
                    );
                }
            }
        }

        // Dispatch guarded tools through ToolDispatcher
        let plan_file_path_for_ctx = crate::builtin_tools::plan_mode::current_plan_context()
            .map(|pc| pc.store.plan_path(&pc.session_id))
            .or_else(|| svc.plan_file_path.clone());
        for (i, tc) in assembled_calls.iter().enumerate() {
            if svc.dispatcher.is_guarded(&tc.function.name) {
                let mut dispatch_ctx = DispatchContext {
                    turn_id: &svc.turn_id,
                    behavior: &svc.config.behavior,
                    work_dir: &svc.work_dir,
                    mode_state: svc.mode_state.as_ref(),
                    plan_file_path: plan_file_path_for_ctx.clone(),
                    event_tx: &svc.event_tx,
                    approval_strategy: &svc.approval_strategy,
                    interaction_handle: svc.interaction_handle.as_ref(),
                    approval_cache: &mut ms.approval_cache,
                    denial_tracker: &mut ms.denial_tracker,
                    agent_id: &svc.config.agent_id,
                    session_id: svc.session_id.as_deref(),
                    behavior_overrides: svc.behavior_overrides.as_ref(),
                };
                let result = svc.dispatcher.dispatch_one(tc, &mut dispatch_ctx).await;
                all_results[i] = Some(result);
            }
        }

        all_results.into_iter().flatten().collect::<Vec<_>>()
    } else {
        // Non-streaming path: dispatch_batch handles everything.
        let plan_file_path_batch = crate::builtin_tools::plan_mode::current_plan_context()
            .map(|pc| pc.store.plan_path(&pc.session_id))
            .or_else(|| svc.plan_file_path.clone());
        let mut dispatch_ctx = DispatchContext {
            turn_id: &svc.turn_id,
            behavior: &svc.config.behavior,
            work_dir: &svc.work_dir,
            mode_state: svc.mode_state.as_ref(),
            plan_file_path: plan_file_path_batch,
            event_tx: &svc.event_tx,
            approval_strategy: &svc.approval_strategy,
            interaction_handle: svc.interaction_handle.as_ref(),
            approval_cache: &mut ms.approval_cache,
            denial_tracker: &mut ms.denial_tracker,
            agent_id: &svc.config.agent_id,
            session_id: svc.session_id.as_deref(),
            behavior_overrides: svc.behavior_overrides.as_ref(),
        };
        svc.dispatcher
            .dispatch_batch(&assembled_calls, &mut dispatch_ctx)
            .await
    };
    tracing::info!(
        elapsed_ms = tool_dispatch_t0.elapsed().as_millis() as u64,
        tool_count,
        "perf: tool_dispatch"
    );

    let mut force_stop_loop = false;
    let mut plan_approval_pending = false;
    for (tool_name, call_id, arguments, mut result) in stream_results {
        let tool_start_time = std::time::Instant::now();
        ms.query_loop.total_tool_calls += 1;
        let rep_action = ms
            .query_loop
            .record_tool_call(&tool_name, &arguments);

        // ── Permission check ──
        if let Some(permissions::PermissionDecision::Denied(reason)) =
            svc.services.check_permission(&tool_name)
        {
            let msg = reason.unwrap_or_else(|| {
                format!("Tool '{}' is denied by permission rules", tool_name)
            });
            tracing::warn!(tool = %tool_name, %msg, "tool blocked by permission engine");
            result = xiaolin_core::tool::ToolResult::err(&msg);
        }

        // ── UndoEngine + FilePersistence: capture file snapshot before edit ──
        if matches!(
            tool_name.as_str(),
            "edit_file" | "write_file" | "create_file" | "str_replace_editor"
        ) {
            if let Some(file_path) = extract_file_path_from_args(&arguments) {
                let (file_exists, content) = if let Some((snap_content, snap_exists)) =
                    file_pre_snapshots.get(&file_path)
                {
                    (*snap_exists, snap_content.clone())
                } else {
                    (file_path.exists(), tokio::fs::read_to_string(&file_path).await.ok())
                };
                if let Some(ref c) = content {
                    ms.undo_engine.capture_before_edit(&file_path, c);
                }
                let op = if file_exists {
                    FileOp::Modified
                } else {
                    FileOp::Created
                };
                ms.file_tracker.record(file_path, op, &tool_name);
            }
        }

        // ── Pre-tool hook ──
        let input_json: serde_json::Value =
            serde_json::from_str(&arguments).unwrap_or_default();
        if let Some(hook_result) = svc
            .services
            .fire_pre_tool_hooks(&tool_name, &call_id, &input_json)
            .await
        {
            if let Some(err) = hook_result.blocking_error {
                tracing::warn!(tool = %tool_name, %err, "tool blocked by pre-hook");
                result = xiaolin_core::tool::ToolResult::err(&err);
            }
        }

        // ── Progress tracking for stagnation detection ──
        // Only count SUCCESSFUL progress-tool calls as real progress.
        // Failed writes/shell commands should not reset the stagnation counter,
        // otherwise the agent can loop indefinitely alternating between failed
        // operations and read-only verification.
        if result.success
            && matches!(
                tool_name.as_str(),
                "edit_file"
                    | "write_file"
                    | "create_file"
                    | "str_replace_editor"
                    | "shell_exec"
                    | "execute_command"
                    | "terminal_input"
                    | "terminal_open"
                    | "spawn_subagent"
                    | "update_goal"
                    | "apply_patch"
            )
        {
            ms.had_progress_this_round = true;
        }

        // ── Track restoration state for post-compact recovery ──
        track_restoration_state(
            &mut ms.query_loop.restoration_state,
            &tool_name,
            &arguments,
            &result.output,
            result.success,
        );

        // ── Post-tool hook (fire-and-forget) ──
        let output_json =
            serde_json::Value::String(result.output.chars().take(2000).collect::<String>());
        svc.services
            .fire_post_tool_hooks(
                &tool_name,
                &call_id,
                &input_json,
                &output_json,
                tool_start_time.elapsed(),
            )
            .await;

        // ── Observer: record tool call observation ──
        let tool_duration = tool_start_time.elapsed();
        svc.runtime_observer
            .record_tool_call(
                &tool_name,
                result.success,
                tool_duration,
                &result.output.chars().take(200).collect::<String>(),
            )
            .await;
        svc.services
            .record_tool_call_stat(&tool_name, result.success, tool_duration.as_millis() as u64)
            .await;

        ms.trajectory_steps.push(TrajectoryStep {
            role: "assistant".into(),
            action_type: "tool_call".into(),
            tool_name: Some(tool_name.clone()),
            summary: truncate_for_trajectory(&arguments),
            success: None,
        });

        match rep_action {
            query_state::ToolRepetitionAction::ForceStop => {
                if let Some(nudge) = ms.query_loop.build_repetition_nudge(true) {
                    inject_tool_recovery_guidance(&mut ms.messages, &nudge);
                }
                force_stop_loop = true;
            }
            query_state::ToolRepetitionAction::Warn => {
                if let Some(nudge) = ms.query_loop.build_repetition_nudge(false) {
                    inject_tool_recovery_guidance(&mut ms.messages, &nudge);
                }
            }
            query_state::ToolRepetitionAction::None => {}
        }

        if !result.success {
            ms.query_loop.record_tool_error(&tool_name, &result.output);
            ms.undo_engine.record_failure(&tool_name);
        } else {
            ms.query_loop.clear_error_streak();
            ms.undo_engine.record_success();

            if is_file_artifact_tool(&tool_name) {
                record_successful_file_artifacts(
                    svc,
                    &tool_name,
                    &call_id,
                    &arguments,
                    &file_pre_snapshots,
                );
            }
        }

        // ── ValidationPipeline: append findings to tool output ───
        let work_dir_for_validation = svc
            .work_dir
            .as_ref()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let val_ctx = ValidationContext {
            tool_name: &tool_name,
            arguments: &arguments,
            output: &result.output,
            success: result.success,
            work_dir: &work_dir_for_validation,
        };
        let val_result = svc.validation_pipeline.validate(&val_ctx);
        let validation_suffix = if !val_result.findings.is_empty() {
            let msgs: Vec<String> = val_result
                .findings
                .iter()
                .map(|f| format!("[{:?}] {}", f.severity, f.message))
                .collect();
            tracing::info!(
                tool = %tool_name,
                findings = msgs.len(),
                "validation_pipeline: findings appended"
            );
            format!("\n\n─── Validation Findings ───\n{}", msgs.join("\n"))
        } else {
            String::new()
        };

        // ── UndoEngine: rollback on excessive failures ───────────
        if ms.undo_engine.should_rollback() {
            if let Some(rb) = ms.undo_engine.execute_rollback() {
                for file_path in &rb.restored_files {
                    if let Some(content) = ms.undo_engine.get_restore_content(file_path) {
                        let _ = std::fs::write(file_path, content);
                    }
                }
                inject_tool_recovery_guidance(&mut ms.messages, &rb.guidance);
                tracing::warn!(
                    restored = rb.restored_files.len(),
                    "undo_engine: auto-rollback triggered"
                );
            }
        }

        let max_chars = svc
            .tool_registry
            .get(&tool_name)
            .map(|t| t.max_result_size_chars())
            .unwrap_or(100_000);
        let tool_output_with_validation = if validation_suffix.is_empty() {
            result.output.clone()
        } else {
            format!("{}{}", result.output, validation_suffix)
        };
        let processed = process_tool_output(
            &svc.tool_storage,
            &tool_name,
            &call_id,
            &tool_output_with_validation,
            max_chars,
        );
        let header = semantic_header(
            &tool_name,
            &arguments,
            &tool_output_with_validation,
            result.success,
        );
        let llm_out = format!("{header}\n{processed}");
        let _ = send_step(
            &svc.step_tx,
            AgentStep::ToolResult {
                turn_id: svc.turn_id.clone(),
                tool_name: tool_name.clone(),
                call_id: call_id.clone(),
                output: result.ui_output().to_string(),
                display_output: result.display_output.clone(),
                success: result.success,
                metadata: result.metadata.clone(),
            },
            false,
        )
        .await;

        ms.trajectory_steps.push(TrajectoryStep {
            role: "tool".into(),
            action_type: "tool_result".into(),
            tool_name: Some(tool_name.clone()),
            summary: truncate_for_trajectory(&result.output),
            success: Some(result.success),
        });

        if result
            .metadata
            .as_ref()
            .and_then(|m| m.get("approval_pending"))
            .and_then(|v| v.as_bool())
            == Some(true)
        {
            tracing::info!(
                agent_id = %svc.config.agent_id,
                tool = %tool_name,
                "plan approval pending — ending turn to wait for user decision"
            );
            plan_approval_pending = true;
        }

        let content = tool_result_content(&llm_out, &result);
        ms.messages.push(ChatMessage {
            role: Role::Tool,
            content: Some(content),
            name: Some(tool_name.clone()),
            tool_call_id: Some(call_id),
            ..Default::default()
        });

        // ── Auto-fix loop (streaming path) ──
        if !result.success {
            if let Some(build_cmd) =
                crate::autofix::extract_build_command(&tool_name, &arguments)
            {
                if let Some(guide) = crate::autofix::detect_and_plan(
                    &build_cmd,
                    &result.output,
                    -1,
                    ms.query_loop.autofix.iteration,
                ) {
                    let error_count_for_state = guide.diagnostics.len();
                    ms.query_loop.autofix.record_build_result(
                        &build_cmd,
                        -1,
                        error_count_for_state,
                    );
                    inject_tool_recovery_guidance(&mut ms.messages, &guide.formatted);
                    tracing::info!(
                        compiler = %crate::autofix::compiler_name(guide.compiler),
                        errors = error_count_for_state,
                        iteration = guide.iteration,
                        "auto-fix guidance injected (stream)"
                    );
                }
            }
        } else if crate::autofix::extract_build_command(&tool_name, &arguments).is_some() {
            ms.query_loop.autofix.reset();
        }

        if svc.runtime.try_self_iter_tool_recovery(
            &mut ms.messages,
            &svc.config,
            &minimal_request,
            ms.query_loop.iteration,
            ms.query_loop.consecutive_errors,
            max_errors,
            &ms.query_loop.failure_streak_traces,
            &mut ms.query_loop.self_iter_recovery_used,
        ) {
            ms.query_loop.clear_error_streak();
        }

        let failure_summary = ms.query_loop.format_failure_summary();
        let error_count = ms.query_loop.consecutive_errors;
        if let Some(transition) = ms.query_loop.check_error_limit(max_errors) {
            match transition {
                query_state::LoopTransition::Terminal(
                    query_state::TerminalReason::ConsecutiveErrors,
                ) => {
                    tracing::warn!(
                        agent_id = %svc.config.agent_id,
                        consecutive_errors = error_count,
                        "consecutive error limit reached after grace turn"
                    );
                    break;
                }
                _ => break,
            }
        } else if ms.query_loop.grace_turn_active {
            tracing::info!(
                agent_id = %svc.config.agent_id,
                consecutive_errors = error_count,
                "consecutive error limit reached — entering grace turn"
            );
            let has_active_goal = if let Some(gs) = svc.goal_store.as_ref() {
                gs.get_active().await.is_some()
            } else {
                false
            };
            let guidance = if has_active_goal {
                format!(
                    "[TOOL ERROR LIMIT] You have hit {error_count} consecutive tool errors. \
                     The failing calls were:\n{failure_summary}\n\n\
                     You are in Goal Mode — recover autonomously. Do NOT ask the user.\n\
                     1. Analyze why the tools failed (wrong paths, missing deps, bad args).\n\
                     2. Try a completely different approach to achieve the same outcome.\n\
                     3. If the failing tool is not essential, skip it and continue with \
                     the next subtask.\n\n\
                     Do NOT retry the same failing tool calls with the same arguments.",
                )
            } else {
                format!(
                    "[TOOL ERROR LIMIT] You have hit {error_count} consecutive tool errors. \
                     The failing calls were:\n{failure_summary}\n\n\
                     STOP calling the tools that keep failing. Instead:\n\
                     1. Explain to the user what you were trying to do and what went wrong.\n\
                     2. Suggest how to fix the issue (e.g. correct file paths, adjust permissions, change approach).\n\
                     3. Ask the user if they want you to try a different approach.\n\n\
                     Do NOT retry the same failing tool calls.",
                )
            };
            ms.messages.push(ChatMessage {
                role: Role::System,
                content: Some(serde_json::Value::String(guidance)),
                ..Default::default()
            });
            break;
        }
    }

    ToolRoundOutcome::Continue {
        pre_snapshot: PreToolSnapshot {
            mode_before,
            goal_before,
        },
        force_stop_loop,
        plan_approval_pending,
    }
}
