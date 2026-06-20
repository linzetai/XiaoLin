use std::sync::Arc;

use futures::StreamExt;
use xiaolin_core::tool::ToolProfile;
use xiaolin_core::types::{ChatMessage, Role};
use xiaolin_protocol::{AgentEvent, ExecutionMode, TurnSummary, WarningCategory};

use crate::llm::CompletionParams;

use super::agent_step::AgentStep;
use super::accumulator::{accumulate_tool_call, ToolCallAccumulator};
use super::query_deps::QueryDeps;
use super::cache_break_detection;
use super::cost_tracker;
use super::mode_attachments;
use super::model_critic;
use super::query_state::{self, ESCALATED_MAX_TOKENS};
use super::stream_engine::{send_step, send_stream_event};
use super::streaming_tool_executor::{StreamingExecutorConfig, StreamingToolExecutor};
use super::task_decomposer;
use super::tool_executor::{filter_tool_definitions, demote_tools_for_plan_mode};
use super::turn_state::{TurnMutableState, TurnServices};
use super::{
    apply_message_budget, classify_stream_error_code, inject_tool_recovery_guidance,
    make_turn_summary, AgentRuntime,
};

/// Accumulated output from the LLM streaming phase.
pub(crate) struct LlmStreamOutput {
    pub accumulated_content: String,
    pub accumulated_reasoning: String,
    pub tool_call_accum: Vec<ToolCallAccumulator>,
    pub last_finish_reason: Option<String>,
    pub streaming_executor: Option<StreamingToolExecutor>,
    pub last_submitted_tool_idx: Option<usize>,
    pub transition: query_state::LoopTransition,
}

/// Outcome of the full LLM call phase (prep + streaming + recovery + transition).
pub(crate) enum LlmCallOutcome {
    /// Stream completed successfully; includes accumulated output and
    /// the determined loop transition (EndTurn or Continue with tool calls).
    Completed(Box<LlmStreamOutput>),
    /// Turn ended early due to an unrecoverable error (stream failure,
    /// prompt_too_long without recovery, etc.).
    FatalError(anyhow::Error),
    /// Turn ended early but gracefully (e.g. budget exceeded).
    EarlyFinish(TurnSummary),
    /// The outer loop should `continue` (reactive compact recovery,
    /// max_output_tokens escalation, model critic rejection).
    RetryIteration,
}

/// Performs the full LLM call phase for one iteration of the agent loop.
///
/// This is the most complex phase, encompassing:
/// 1. Pre-LLM preparation (message budget, mode attachments, sanitization)
/// 2. Streaming LLM call with resume/retry logic
/// 3. Token usage tracking, cost accounting, cache detection
/// 4. Post-stream recovery (prompt_too_long reactive compact, max_output_tokens escalation)
/// 5. Post-LLM transition determination (EndTurn vs Continue)
/// 6. Model critic (optional review of final output before accepting EndTurn)
///
/// # Control Flow
///
/// - Returns `Completed(output)` when the stream finishes and a valid transition is determined.
///   The caller should then dispatch on `output.transition`.
/// - Returns `RetryIteration` when recovery requires restarting the outer loop
///   (e.g. after reactive compact or model critic rejection).
/// - Returns `FatalError` or `EarlyFinish` when the turn must stop immediately.
///
/// # Side Effects
///
/// - Modifies `ms.messages` (message budget replacement, mode attachments, partial resume messages)
/// - Modifies `ms.max_tokens` (max_output_tokens escalation)
/// - Modifies `ms.query_loop` state (token accounting, recovery counts)
/// - Sends steps via `svc.step_tx` (Delta, Error) and warnings via `svc.event_tx`
/// - Calls `svc.runtime.finalize_injected_skills()` on fatal paths
pub(crate) async fn perform_llm_call(
    ms: &mut TurnMutableState,
    svc: &TurnServices,
    estimated_tokens: usize,
) -> LlmCallOutcome {
    // Refresh tool_defs if the registry changed (e.g. tool_search activated a deferred tool)
    let current_reg_version = svc.tool_registry.version();
    if current_reg_version != ms.registry_version_at_setup {
        let mode_profile = svc
            .mode_state
            .as_ref()
            .map(|mode_s| match mode_s.current_mode() {
                ExecutionMode::Plan => ToolProfile::plan_mode(),
                _ => ToolProfile::default(),
            })
            .unwrap_or_default();
        let mut all_defs = svc.tool_registry.definitions_with_profile(&mode_profile);
        all_defs.extend(ms.extra_tool_defs.iter().cloned());
        ms.tool_defs = filter_tool_definitions(&all_defs, &svc.config);
        if svc.mode_state.as_ref().is_some_and(|ms| ms.current_mode() == ExecutionMode::Plan) {
            demote_tools_for_plan_mode(&mut ms.tool_defs);
        }
        ms.tool_defs_est_tokens = svc.tool_registry.estimated_json_chars(&ms.tool_defs) / 4;
        ms.registry_version_at_setup = current_reg_version;
        tracing::info!(
            count = ms.tool_defs.len(),
            est_tokens = ms.tool_defs_est_tokens,
            "tool_defs refreshed after registry change"
        );
    }

    let total_est_with_tools = estimated_tokens + ms.tool_defs_est_tokens;
    tracing::info!(
        agent_id = %svc.config.agent_id,
        model = %svc.model,
        iteration = ms.query_loop.iteration,
        msg_count = ms.messages.len(),
        msg_tokens = estimated_tokens,
        tool_def_tokens = ms.tool_defs_est_tokens,
        total_est = total_est_with_tools,
        context_window = svc.context_window,
        "streaming LLM call"
    );

    const MAX_STREAM_RESUME_ATTEMPTS: u32 = 5;
    const STREAM_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);
    const STREAM_TOTAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);
    let mut stream_resume_attempts: u32 = 0;

    let newly_replaced = apply_message_budget(
        &svc.tool_storage,
        &mut ms.messages,
        &mut ms.replacement_state,
        &svc.skip_tool_names,
    );
    AgentRuntime::persist_replacement_records(
        &svc.session_store,
        svc.session_id.as_deref(),
        &newly_replaced,
    )
    .await;

    // ── Mode Attachment: inject plan mode instructions per-turn ──────
    if let Some(ref mode_state) = svc.mode_state {
        if mode_state.current_mode() == ExecutionMode::Plan {
            let turn_count = mode_state.plan_turn_count();
            let plan_path_str = svc.plan_file_path.as_ref()
                .map(|p| p.display().to_string());
            let plan_exists = svc.plan_file_path.as_ref()
                .is_some_and(|p| p.exists());
            let lang: Option<&str> = None;
            let attachment = mode_attachments::plan_mode_attachment(
                plan_path_str.as_deref(),
                plan_exists,
                lang,
            );
            if let Some(text) = attachment.text_for_turn(turn_count) {
                let mut inject_text = String::new();
                if turn_count == 0 && mode_state.has_exited_plan() {
                    inject_text.push_str(&mode_attachments::plan_reentry_notice(lang));
                    inject_text.push('\n');
                }
                inject_text.push_str(text);
                ms.messages.push(ChatMessage {
                    role: Role::User,
                    content: Some(serde_json::Value::String(inject_text)),
                    ..Default::default()
                });
                tracing::debug!(
                    turn_count,
                    is_reentry = mode_state.has_exited_plan() && turn_count == 0,
                    "mode_attachment: injected plan mode instructions"
                );
            }
            mode_state.increment_plan_turn();
        }
    }

    xiaolin_context::compressor::sanitize_tool_call_pairing(&mut ms.messages);
    xiaolin_context::compressor::ensure_valid_assistant_messages(&mut ms.messages);

    if !xiaolin_context::model_supports_vision_with_caps(
        &svc.model,
        svc.config.model.capabilities.as_ref(),
    ) {
        xiaolin_context::compressor::strip_image_content(&mut ms.messages);
    }

    let mut accumulated_content = String::new();
    let mut accumulated_reasoning = String::new();
    let mut tool_call_accum: Vec<ToolCallAccumulator> = Vec::new();
    let mut stream_errored = false;
    let mut force_stop = false;
    let mut last_finish_reason: Option<String> = None;
    let mut withheld_prompt_too_long: Option<String> = None;

    // PlanArgInterceptor: extracts plan content deltas from write_file arguments
    let mut plan_interceptor = svc.plan_file_path.as_ref().and_then(|pfp| {
        if svc.mode_state.as_ref().is_some_and(|ms| ms.current_mode() == ExecutionMode::Plan) {
            Some(super::plan_arg_interceptor::PlanArgInterceptor::new(pfp.clone()))
        } else {
            None
        }
    });

    // Streaming tool execution: create executor and track submission state.
    // A new executor is created per iteration since it's consumed via drain_remaining().
    let streaming_exec_enabled = svc.config.behavior.streaming_tool_execution;
    let mut streaming_executor = if streaming_exec_enabled {
        let streaming_plan_fp = crate::builtin_tools::plan_mode::current_plan_context()
            .map(|pc| pc.store.plan_path(&pc.session_id));
        let exec_config = StreamingExecutorConfig {
            sibling_cancel_on_error: true,
            work_dir: svc.work_dir.clone(),
            behavior: svc.config.behavior.clone(),
            execution_mode: svc.mode_state.as_ref().map(|ms| ms.current_mode()),
            plan_file_path: streaming_plan_fp,
            session_id: svc.session_id.clone(),
        };
        Some(StreamingToolExecutor::new(
            Arc::clone(&svc.tool_registry),
            exec_config,
        ))
    } else {
        None
    };
    let mut last_submitted_tool_idx: Option<usize> = None;

    let stream_consume_t0 = std::time::Instant::now();
    let acc_tokens_before_stream =
        ms.query_loop.acc_prompt_tokens + ms.query_loop.acc_completion_tokens;

    let tools_for_llm = if ms.tool_defs.is_empty() {
        None
    } else {
        Some(ms.tool_defs.as_slice())
    };

    'stream_try: loop {
        let params = CompletionParams {
            model: &svc.model,
            messages: &ms.messages,
            temperature: svc.temperature,
            max_tokens: ms.max_tokens,
            tools: tools_for_llm,
        };

        let llm_call_t0 = std::time::Instant::now();
        tracing::info!(
            model = %svc.model,
            msg_count = ms.messages.len(),
            provider = %svc.deps.provider_name(),
            "LLM call starting"
        );
        let stream_result = svc.deps.call_model_stream(&params).await;
        let mut stream = match stream_result {
            Ok(s) => {
                tracing::info!(
                    elapsed_ms = llm_call_t0.elapsed().as_millis() as u64,
                    "perf: stream_connect_success"
                );
                s
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    model = %svc.model,
                    provider = %svc.deps.provider_name(),
                    "LLM stream call failed"
                );
                if query_state::is_prompt_too_long_error(&e.to_string()) {
                    if !ms.query_loop.has_attempted_reactive_compact {
                        ms.query_loop.has_attempted_reactive_compact = true;
                        tracing::warn!(
                            error = %e,
                            "prompt_too_long detected — attempting reactive compaction"
                        );
                        let reactive_result = svc.deps.reactive_compact(&ms.messages);
                        if reactive_result.recovered {
                            tracing::info!(
                                level = ?reactive_result.level_used,
                                tokens_after = reactive_result.tokens_after,
                                "reactive compaction recovered — retrying LLM call"
                            );
                            ms.messages = reactive_result.messages;
                            continue 'stream_try;
                        }
                    } else {
                        tracing::warn!(
                            "prompt_too_long on connect but reactive compact already attempted — not retrying"
                        );
                    }
                }
                return LlmCallOutcome::FatalError(e);
            }
        };
        tracing::info!(
            elapsed_ms = llm_call_t0.elapsed().as_millis() as u64,
            "perf: stream_connect"
        );

        let mut first_chunk = true;
        let mut should_resume = false;
        let mut delta_count: u64 = 0;
        let stream_deadline = tokio::time::Instant::now() + STREAM_TOTAL_TIMEOUT;
        loop {
            let remaining = stream_deadline
                .checked_duration_since(tokio::time::Instant::now())
                .unwrap_or_default();
            if remaining.is_zero() {
                tracing::error!(
                    delta_count,
                    total_secs = STREAM_TOTAL_TIMEOUT.as_secs(),
                    "stream total timeout — generation exceeded maximum wall-clock time"
                );
                return LlmCallOutcome::FatalError(anyhow::anyhow!(
                    "stream total timeout: generation exceeded {}s after {} chunks",
                    STREAM_TOTAL_TIMEOUT.as_secs(),
                    delta_count
                ));
            }
            let idle_dur = if first_chunk {
                std::time::Duration::from_secs(180)
            } else {
                STREAM_IDLE_TIMEOUT
            };
            let effective_timeout = idle_dur.min(remaining);

            let maybe_result = tokio::time::timeout(
                effective_timeout,
                stream.next(),
            )
            .await;

            let result = match maybe_result {
                Ok(Some(r)) => r,
                Ok(None) => break,
                Err(_elapsed) => {
                    if remaining <= idle_dur {
                        tracing::error!(
                            delta_count,
                            total_secs = STREAM_TOTAL_TIMEOUT.as_secs(),
                            "stream total timeout — generation exceeded maximum wall-clock time"
                        );
                        return LlmCallOutcome::FatalError(anyhow::anyhow!(
                            "stream total timeout: generation exceeded {}s after {} chunks",
                            STREAM_TOTAL_TIMEOUT.as_secs(),
                            delta_count
                        ));
                    }
                    tracing::error!(
                        delta_count,
                        idle_secs = if first_chunk { 180 } else { 90 },
                        "stream idle timeout — no chunk received, treating as stall"
                    );
                    Err(anyhow::anyhow!(
                        "stream idle timeout: no data for {}s after {} chunks",
                        if first_chunk { 180 } else { 90 },
                        delta_count
                    ))
                }
            };
            delta_count += 1;
            if first_chunk {
                tracing::info!(
                    elapsed_ms = llm_call_t0.elapsed().as_millis() as u64,
                    "perf: time_to_first_chunk"
                );
                first_chunk = false;
            }
            let delta = match result {
                Ok(d) => d,
                Err(e) => {
                    if query_state::is_prompt_too_long_error(&e.to_string()) {
                        tracing::warn!(
                            error = %e,
                            "prompt_too_long during stream — withholding error for recovery attempt"
                        );
                        withheld_prompt_too_long = Some(e.to_string());
                        break;
                    }

                    if stream_resume_attempts < MAX_STREAM_RESUME_ATTEMPTS {
                        if accumulated_content.is_empty()
                            && accumulated_reasoning.is_empty()
                            && tool_call_accum.is_empty()
                        {
                            tracing::warn!(
                                error = %e,
                                attempt = stream_resume_attempts + 1,
                                "stream interrupted before any content; direct retry"
                            );
                            stream_resume_attempts += 1;
                            should_resume = true;
                            break;
                        }

                        if tool_call_accum.is_empty() {
                            tracing::warn!(
                                error = %e,
                                attempt = stream_resume_attempts + 1,
                                partial_len = accumulated_content.len(),
                                "streaming LLM interrupted; best-effort resume with partial assistant context"
                            );
                            let rc = std::mem::take(&mut accumulated_reasoning);
                            let partial = std::mem::take(&mut accumulated_content);
                            if !partial.is_empty() || !rc.is_empty() {
                                if let Some(last) = ms.messages.last() {
                                    if last.role == Role::Assistant
                                        && last.tool_calls.is_none()
                                    {
                                        ms.messages.pop();
                                    }
                                }
                                ms.messages.push(ChatMessage {
                                    role: Role::Assistant,
                                    content: if partial.is_empty() {
                                        None
                                    } else {
                                        Some(serde_json::Value::String(partial))
                                    },
                                    reasoning_content: if rc.is_empty() {
                                        None
                                    } else {
                                        Some(rc)
                                    },
                                    ..Default::default()
                                });
                            }
                            stream_resume_attempts += 1;
                            should_resume = true;
                            break;
                        }

                        tracing::warn!(
                            error = %e,
                            attempt = stream_resume_attempts + 1,
                            tool_calls_partial = tool_call_accum.len(),
                            "stream interrupted during tool call accumulation; discarding partial tool calls and retrying"
                        );
                        tool_call_accum.clear();
                        let rc = std::mem::take(&mut accumulated_reasoning);
                        let partial = std::mem::take(&mut accumulated_content);
                        if !partial.is_empty() || !rc.is_empty() {
                            if let Some(last) = ms.messages.last() {
                                if last.role == Role::Assistant
                                    && last.tool_calls.is_none()
                                {
                                    ms.messages.pop();
                                }
                            }
                            ms.messages.push(ChatMessage {
                                role: Role::Assistant,
                                content: if partial.is_empty() {
                                    None
                                } else {
                                    Some(serde_json::Value::String(partial))
                                },
                                reasoning_content: if rc.is_empty() {
                                    None
                                } else {
                                    Some(rc)
                                },
                                ..Default::default()
                            });
                        }
                        stream_resume_attempts += 1;
                        should_resume = true;
                        break;
                    }

                    let err_msg = e.to_string();
                    tracing::error!(
                        error = %err_msg,
                        attempts = stream_resume_attempts,
                        "stream recovery exhausted — giving up"
                    );
                    let _ = send_step(
                        &svc.step_tx,
                        AgentStep::Error {
                            turn_id: svc.turn_id.clone(),
                            message: format!(
                                "流式响应超时（已重试{}次）: {}",
                                stream_resume_attempts, err_msg
                            ),
                            error_code: classify_stream_error_code(&err_msg),
                            recoverable: true,
                        },
                        false,
                    )
                    .await;
                    stream_errored = true;
                    break;
                }
            };

            if delta_count <= 3 || delta.choices.is_empty() {
                let preview_content = delta
                    .choices
                    .first()
                    .and_then(|c| c.delta.content.as_deref());
                let preview_rc = delta
                    .choices
                    .first()
                    .and_then(|c| c.delta.reasoning_content.as_deref());
                let has_tc = delta
                    .choices
                    .first()
                    .map(|c| c.delta.tool_calls.is_some())
                    .unwrap_or(false);
                let fr = delta
                    .choices
                    .first()
                    .and_then(|c| c.finish_reason.as_deref());
                tracing::info!(
                    delta_count,
                    choices_len = delta.choices.len(),
                    content_preview = ?preview_content.map(|s| &s[..s.floor_char_boundary(60)]),
                    reasoning_preview = ?preview_rc.map(|s| &s[..s.floor_char_boundary(60)]),
                    has_tool_calls = has_tc,
                    finish_reason = ?fr,
                    has_usage = delta.usage.is_some(),
                    "stream delta inspect"
                );
            }

            if let Some(choice) = delta.choices.first() {
                if let Some(ref content) = choice.delta.content {
                    accumulated_content.push_str(content);
                }
                if let Some(ref rc) = choice.delta.reasoning_content {
                    accumulated_reasoning.push_str(rc);
                    let _ = send_step(
                        &svc.step_tx,
                        AgentStep::ReasoningDelta {
                            turn_id: svc.turn_id.clone(),
                            content: rc.clone(),
                        },
                        true,
                    )
                    .await;
                }

                if let Some(ref tc_deltas) = choice.delta.tool_calls {
                    for tc_delta in tc_deltas {
                        // In streaming mode: when a new tool index appears, all
                        // prior tools are fully accumulated and can start executing.
                        // Guarded tools (in RuntimeRegistry) are NOT submitted here
                        // — they'll go through orchestrator after stream completes.
                        if let Some(ref mut executor) = streaming_executor {
                            let new_idx = tc_delta.index as usize;
                            let submit_start =
                                last_submitted_tool_idx.map(|i| i + 1).unwrap_or(0);
                            if new_idx > 0 && submit_start < new_idx {
                                for si in submit_start..new_idx {
                                    if let Some(acc) = tool_call_accum.get(si) {
                                        if !acc.name.is_empty() {
                                            if !svc.runtime_registry.has(&acc.name) {
                                                for tc in acc.to_tool_calls() {
                                                    executor.add_tool(tc);
                                                }
                                            }
                                            last_submitted_tool_idx = Some(si);
                                        }
                                    }
                                }
                            }
                        }
                        accumulate_tool_call(&mut tool_call_accum, tc_delta);

                        // Feed argument deltas to the PlanArgInterceptor
                        if let Some(ref mut interceptor) = plan_interceptor {
                            let idx = tc_delta.index as usize;
                            // Notify interceptor when tool name becomes known
                            if let Some(ref func) = tc_delta.function {
                                if let Some(ref name) = func.name {
                                    if !name.is_empty() {
                                        interceptor.on_tool_start(name);
                                    }
                                }
                                // Feed argument chunk
                                if let Some(ref args) = func.arguments {
                                    if !args.is_empty() {
                                        let deltas = interceptor.feed(args);
                                        for d in deltas {
                                            let _ = send_step(
                                                &svc.step_tx,
                                                AgentStep::PlanDelta {
                                                    turn_id: svc.turn_id.clone(),
                                                    delta: d,
                                                },
                                                true,
                                            )
                                            .await;
                                        }
                                    }
                                }
                            }
                            // Reset interceptor when a new tool index starts
                            if idx > 0 && tool_call_accum.len() == idx + 1 {
                                interceptor.reset();
                                if let Some(acc) = tool_call_accum.get(idx) {
                                    if !acc.name.is_empty() {
                                        interceptor.on_tool_start(&acc.name);
                                    }
                                }
                            }
                        }
                    }
                }

                if let Some(ref reason) = choice.finish_reason {
                    last_finish_reason = Some(reason.clone());
                }
            }

            if let Some(ref u) = delta.usage {
                ms.query_loop.acc_prompt_tokens += u.prompt_tokens;
                ms.query_loop.acc_completion_tokens += u.completion_tokens;
                if u.prompt_tokens > 0 {
                    ms.query_loop.last_estimated_tokens = u.prompt_tokens as usize;
                }

                // ── Cost tracker: record LLM usage ──
                if u.prompt_tokens > 0 || u.completion_tokens > 0 {
                    let call_usage = cost_tracker::CallUsage {
                        model: svc.model.clone(),
                        prompt_tokens: u.prompt_tokens,
                        completion_tokens: u.completion_tokens,
                        cache_read_tokens: u.effective_cache_read_tokens(),
                        cache_creation_tokens: u.effective_cache_creation_tokens(),
                    };
                    if let Some(alert) = svc.services.record_llm_usage(call_usage).await {
                        match alert {
                            cost_tracker::BudgetAlert::Warning => {
                                let cost = svc.services.accumulated_cost_usd().await;
                                let _ = send_stream_event(
                                    &svc.event_tx,
                                    AgentEvent::Warning {
                                        turn_id: svc.turn_id.clone(),
                                        message: format!(
                                            "Budget warning: accumulated cost ${:.4} is approaching the limit.",
                                            cost,
                                        ),
                                        category: WarningCategory::Budget,
                                    },
                                    false,
                                )
                                .await;
                            }
                            cost_tracker::BudgetAlert::Exceeded => {
                                let cost = svc.services.accumulated_cost_usd().await;
                                let _ = send_step(
                                    &svc.step_tx,
                                    AgentStep::Error {
                                        turn_id: svc.turn_id.clone(),
                                        message: format!(
                                            "Budget exceeded: accumulated cost ${:.4}. Stopping execution.",
                                            cost,
                                        ),
                                        error_code: Some(
                                            xiaolin_protocol::ErrorCode::UsageLimitExceeded,
                                        ),
                                        recoverable: false,
                                    },
                                    false,
                                )
                                .await;
                                force_stop = true;
                            }
                        }
                    }

                    // ── Observer: record LLM call ──
                    svc.runtime_observer
                        .record_llm_call(
                            &svc.model,
                            u.prompt_tokens,
                            u.completion_tokens,
                            llm_call_t0.elapsed(),
                        )
                        .await;

                    // ── CacheBreakDetector: check for cache invalidation ──
                    let cache_usage = cache_break_detection::CacheAwareUsage {
                        prompt_tokens: u.prompt_tokens,
                        completion_tokens: u.completion_tokens,
                        cache_read_tokens: u.effective_cache_read_tokens(),
                        cache_creation_tokens: u.effective_cache_creation_tokens(),
                    };
                    let cache_snapshot = ms.cache_detector.pre_call_snapshot(
                        "",
                        "",
                        &svc.model,
                        false,
                        false,
                    );
                    if let Some(report) =
                        ms.cache_detector.post_call_analyze(&cache_snapshot, &cache_usage)
                    {
                        tracing::warn!(
                            cause = %report.summary(),
                            "cache_break_detection: prompt cache break detected"
                        );
                    }
                }
            }

            if tool_call_accum.is_empty() {
                let _ = send_step(
                    &svc.step_tx,
                    AgentStep::Delta {
                        turn_id: svc.turn_id.clone(),
                        delta: serde_json::to_value(&delta).unwrap_or_default(),
                        raw_bytes: delta.raw_sse_json.clone(),
                    },
                    true,
                )
                .await;
            }
        }

        if stream_errored {
            break 'stream_try;
        }
        if should_resume {
            continue 'stream_try;
        }
        break 'stream_try;
    }

    tracing::info!(
        elapsed_ms = stream_consume_t0.elapsed().as_millis() as u64,
        agent_id = %svc.config.agent_id,
        iteration = ms.query_loop.iteration,
        accumulated_content_len = accumulated_content.len(),
        accumulated_reasoning_len = accumulated_reasoning.len(),
        tool_calls_count = tool_call_accum.len(),
        last_finish_reason = ?last_finish_reason,
        stream_errored,
        "perf: stream_consumed"
    );

    // ── Fallback token estimation when provider omits usage ──────
    // Some providers (e.g. deepseek) don't include `usage` in streaming
    // responses even with `stream_options.include_usage = true`.
    // Without this fallback, goal token accounting stays at 0.
    let acc_tokens_after_stream =
        ms.query_loop.acc_prompt_tokens + ms.query_loop.acc_completion_tokens;
    if acc_tokens_after_stream == acc_tokens_before_stream && !stream_errored {
        let est_prompt = ms.query_loop.last_estimated_tokens as u32;
        let output_bytes = accumulated_content.len() + accumulated_reasoning.len();
        let est_completion = (output_bytes / 4).max(1) as u32;
        ms.query_loop.acc_prompt_tokens += est_prompt;
        ms.query_loop.acc_completion_tokens += est_completion;
        tracing::info!(
            est_prompt,
            est_completion,
            output_bytes,
            "token_fallback: provider omitted usage, using estimation"
        );
    }

    // Withheld prompt_too_long recovery: attempt reactive compact
    // before surfacing the error to the client.
    if let Some(ref withheld_err) = withheld_prompt_too_long {
        if !ms.query_loop.has_attempted_reactive_compact {
            ms.query_loop.has_attempted_reactive_compact = true;
            let reactive_result = svc.deps.reactive_compact(&ms.messages);
            if reactive_result.recovered {
                tracing::info!(
                    level = ?reactive_result.level_used,
                    tokens_after = reactive_result.tokens_after,
                    "withheld prompt_too_long recovered via reactive compact — retrying"
                );
                ms.messages = reactive_result.messages;
                return LlmCallOutcome::RetryIteration;
            }
        }
        tracing::error!(
            error = %withheld_err,
            "withheld prompt_too_long: reactive compact failed — yielding error to client"
        );
        let _ = send_step(
            &svc.step_tx,
            AgentStep::Error {
                turn_id: svc.turn_id.clone(),
                message: withheld_err.clone(),
                error_code: None,
                recoverable: false,
            },
            false,
        )
        .await;
        svc.runtime
            .finalize_injected_skills(&ms.injected_skill_ids, false)
            .await;
        return LlmCallOutcome::FatalError(anyhow::anyhow!(
            "prompt_too_long: recovery failed"
        ));
    }

    if stream_errored {
        svc.runtime
            .finalize_injected_skills(&ms.injected_skill_ids, false)
            .await;
        return LlmCallOutcome::FatalError(anyhow::anyhow!(
            "provider stream error (already sent to client)"
        ));
    }

    if force_stop {
        tracing::warn!("budget exceeded — stopping execution");
        svc.runtime
            .finalize_injected_skills(&ms.injected_skill_ids, false)
            .await;
        return LlmCallOutcome::EarlyFinish(make_turn_summary(
            &svc.turn_id,
            &ms.query_loop,
            svc.stream_start,
            svc.context_window,
        ));
    }

    // max_output_tokens recovery: when finish_reason=length and no
    // tool calls, the model's output was truncated by the token limit.
    // Escalate max_tokens and retry up to MAX_OUTPUT_TOKENS_RECOVERY_LIMIT times.
    let has_valid_tool_calls = tool_call_accum.iter().any(|a| !a.name.is_empty());
    if last_finish_reason.as_deref() == Some("length") && !has_valid_tool_calls {
        if let Some(query_state::LoopTransition::Continue(
            query_state::ContinueReason::MaxOutputTokensRecovery,
        )) = ms.query_loop.try_max_output_tokens_recovery()
        {
            let escalated = ESCALATED_MAX_TOKENS;
            tracing::warn!(
                agent_id = %svc.config.agent_id,
                attempt = ms.query_loop.max_output_tokens_recovery_count,
                escalated_max_tokens = escalated,
                "max_output_tokens recovery — retrying with escalated limit"
            );
            ms.max_tokens = Some(escalated);
            let rc = std::mem::take(&mut accumulated_reasoning);
            let partial = std::mem::take(&mut accumulated_content);
            if !partial.is_empty() || !rc.is_empty() {
                ms.messages.push(ChatMessage {
                    role: Role::Assistant,
                    content: if partial.is_empty() {
                        None
                    } else {
                        Some(serde_json::Value::String(partial))
                    },
                    reasoning_content: if rc.is_empty() {
                        None
                    } else {
                        Some(rc)
                    },
                    ..Default::default()
                });
            }
            return LlmCallOutcome::RetryIteration;
        }
    }

    let transition = ms
        .query_loop
        .determine_post_llm_transition(has_valid_tool_calls);

    let transition = if ms.query_loop.force_stop_after_next
        && matches!(transition, query_state::LoopTransition::Continue(_))
    {
        tracing::info!(
            agent_id = %svc.config.agent_id,
            "force_stop_after_next — overriding Continue to EndTurn"
        );
        query_state::LoopTransition::Terminal(query_state::TerminalReason::EndTurn)
    } else {
        transition
    };

    if let query_state::LoopTransition::Terminal(ref reason) = transition {
        if matches!(reason, query_state::TerminalReason::EndTurn) {
            // ── Model Critic: review final output before accepting ──
            // Skipped when force-stopping (goal cancelled) to avoid
            // an extra loop iteration that delays the termination.
            let critic_config = model_critic::CriticConfig {
                model: svc.config.model.model.clone(),
                ..Default::default()
            };
            if !ms.query_loop.force_stop_after_next
                && critic_config.enabled
                && !accumulated_content.is_empty()
            {
                let critic_provider = svc.runtime.provider();
                let task_type =
                    task_decomposer::TaskType::from_str_loose_pub(&svc.last_user_msg);
                if let Some(review) = model_critic::run_critic(
                    &critic_provider,
                    task_type,
                    &accumulated_content,
                    &critic_config,
                )
                .await
                {
                    if !review.approved {
                        if let Some(feedback) = review.format_for_injection() {
                            tracing::info!(
                                issues = review.issues.len(),
                                "model_critic: output rejected, injecting feedback"
                            );
                            if !accumulated_content.is_empty() || !accumulated_reasoning.is_empty()
                            {
                                ms.messages.push(ChatMessage {
                                    role: Role::Assistant,
                                    content: if accumulated_content.is_empty() {
                                        None
                                    } else {
                                        Some(serde_json::Value::String(std::mem::take(
                                            &mut accumulated_content,
                                        )))
                                    },
                                    reasoning_content: if accumulated_reasoning.is_empty() {
                                        None
                                    } else {
                                        Some(std::mem::take(&mut accumulated_reasoning))
                                    },
                                    ..Default::default()
                                });
                            }
                            inject_tool_recovery_guidance(&mut ms.messages, &feedback);
                            return LlmCallOutcome::RetryIteration;
                        }
                    }
                }
            }
        }
    }

    LlmCallOutcome::Completed(Box::new(LlmStreamOutput {
        accumulated_content,
        accumulated_reasoning,
        tool_call_accum,
        last_finish_reason,
        streaming_executor,
        last_submitted_tool_idx,
        transition,
    }))
}
