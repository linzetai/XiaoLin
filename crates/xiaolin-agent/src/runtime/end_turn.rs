use xiaolin_core::types::{
    ChatMessage, DeltaContent, Role, StreamChoice, StreamDelta,
};
use xiaolin_evolution::TrajectoryOutcome;
use xiaolin_protocol::{
    ExecutionMode, TokenUsage, TurnSummary,
};

use crate::builtin_tools::GoalStatus;
use crate::llm::CompletionParams;

use super::agent_step::{AgentStep, TurnEndReason};
use super::llm_call::LlmStreamOutput;
use super::make_turn_summary;
use super::query_state::TerminalReason;
use super::stop_hooks;
use super::stream_engine::send_step;
use super::token_budget;
use super::turn_state::{TurnMutableState, TurnServices};

/// Outcome of the EndTurn finalization phase.
pub(crate) enum EndTurnOutcome {
    /// Turn is truly done — return this summary to the caller.
    Done(TurnSummary),
    /// A stop hook triggered continuation — the outer loop should `continue`
    /// to give the LLM another iteration.
    StopHookContinuation,
}

/// Handle the Terminal transition (EndTurn or MaxIterations) after the LLM
/// call phase has determined the turn should end.
///
/// This phase covers (for EndTurn reason):
/// 1. Goal token/time incremental accounting
/// 2. GoalUpdated event emission after accounting
/// 3. Stop hooks evaluation:
///    - If should_continue → push assistant message + continuation prompt → StopHookContinuation
///    - If force_stop_after_next → skip hooks, proceed to finalization
///
/// For MaxIterations reason:
/// 4. Inject "[SYSTEM] Tool call limit reached" prompt
/// 5. Fire a non-tool summary LLM call and emit summary delta
///
/// Common finalization (both reasons):
/// 6. Auto-exit Plan mode if no plan file was produced
/// 7. Emit TurnEnd event
/// 8. Fire stop hooks on RuntimeServices
/// 9. Finalize injected skills
/// 10. Record trajectory (TODO)
/// 11. Finalize RuntimeObserver
///
/// # Returns
///
/// - `Done(summary)` when the turn truly ends and should return to the caller.
/// - `StopHookContinuation` when stop hooks require another LLM iteration.
pub(crate) async fn handle_end_turn(
    ms: &mut TurnMutableState,
    svc: &TurnServices,
    llm_output: &LlmStreamOutput,
    terminal_reason: &TerminalReason,
) -> EndTurnOutcome {
    if matches!(terminal_reason, TerminalReason::EndTurn) {
        // Goal token/time incremental accounting before stop hook evaluation.
        // Uses account_tokens/account_time to avoid double-counting across
        // continuation rounds within the same query loop.
        //
        // When a per-turn token_budget is active, skip goal budget enforcement
        // to avoid conflicting control signals (token_budget takes priority).
        if let Some(ref gs) = svc.goal_store {
            if let Some(current_goal) = gs.get_current().await {
                let goal_id = current_goal.id.clone();
                if current_goal.status == GoalStatus::Active {
                    let usage = ms.query_loop.build_usage();
                    if let Some(ref u) = usage {
                        let cumulative = TokenUsage {
                            prompt_tokens: u.prompt_tokens,
                            completion_tokens: u.completion_tokens,
                            total_tokens: u.total_tokens,
                            cached_input_tokens: 0,
                        }
                        .goal_token_delta();
                        if let Some((_delta, over_budget)) =
                            gs.account_tokens(&goal_id, cumulative).await
                        {
                            if over_budget && ms.budget_tracker.is_none() {
                                tracing::info!(
                                    goal_id = %goal_id,
                                    "goal budget exceeded, setting budget_limited"
                                );
                                let _ = gs
                                    .update_status(
                                        &goal_id,
                                        GoalStatus::BudgetLimited,
                                        Some("budget_exhausted"),
                                    )
                                    .await;
                            } else if over_budget {
                                tracing::info!(
                                    goal_id = %goal_id,
                                    "goal budget exceeded but token_budget active — deferring to per-turn budget"
                                );
                            }
                        }
                    }
                    let elapsed_secs = svc.stream_start.elapsed().as_secs();
                    gs.account_time(&goal_id, elapsed_secs).await;
                }
                if let Some(updated) = gs.get_current().await {
                    let _ = send_step(
                        &svc.step_tx,
                        AgentStep::GoalUpdated {
                            turn_id: svc.turn_id.clone(),
                            goal: updated.to_goal_data(),
                        },
                        false,
                    )
                    .await;
                }
            }
        }

        if !ms.query_loop.force_stop_after_next {
            let hook_result = stop_hooks::evaluate_stop_hooks(
                &llm_output.accumulated_content,
                llm_output.last_finish_reason.as_deref(),
                svc.todo_store.as_ref(),
                &[],
                svc.goal_store.as_ref().map(|g| g.as_ref()),
                svc.mode_state.as_ref().map(|ms| ms.current_mode()),
                ms.had_tool_calls_this_round,
                ms.had_progress_this_round,
                stop_hooks::RecoveryState {
                    max_output_recovery_exhausted: ms.query_loop.max_output_recovery_exhausted,
                },
            )
            .await;

            if hook_result.should_continue {
                tracing::info!(
                    agent_id = %svc.config.agent_id,
                    reason = hook_result.reason,
                    "stop hook triggered continuation"
                );
                if !llm_output.accumulated_content.is_empty()
                    || !llm_output.accumulated_reasoning.is_empty()
                {
                    ms.messages.push(ChatMessage {
                        role: Role::Assistant,
                        content: if llm_output.accumulated_content.is_empty() {
                            None
                        } else {
                            Some(serde_json::Value::String(
                                llm_output.accumulated_content.clone(),
                            ))
                        },
                        reasoning_content: if llm_output.accumulated_reasoning.is_empty() {
                            None
                        } else {
                            Some(llm_output.accumulated_reasoning.clone())
                        },
                        ..Default::default()
                    });
                }
                if let Some(msg) = hook_result.continuation_message {
                    ms.messages.push(ChatMessage {
                        role: Role::User,
                        content: Some(serde_json::Value::String(msg)),
                        ..Default::default()
                    });
                }
                ms.had_tool_calls_this_round = false;
                ms.had_progress_this_round = false;
                return EndTurnOutcome::StopHookContinuation;
            }
        } else {
            tracing::info!(
                agent_id = %svc.config.agent_id,
                "force_stop_after_next — skipping stop hooks, ending turn"
            );
        }
    }

    if matches!(terminal_reason, TerminalReason::MaxIterations) {
        let max_iterations = ms.query_loop.max_iterations;
        tracing::warn!(
            agent_id = %svc.config.agent_id,
            max_iterations,
            "streaming tool call limit reached — requesting progress summary"
        );

        ms.messages.push(ChatMessage {
            role: Role::User,
            content: Some(serde_json::Value::String(
                "[SYSTEM] Tool call limit reached. You MUST now:\n\
                 1. Summarize your progress so far\n\
                 2. List any unfinished tasks\n\
                 3. Explain what remains to be done\n\
                 Do NOT call any tools — just output text."
                    .to_string(),
            )),
            ..Default::default()
        });

        let summary_params = CompletionParams {
            model: &svc.model,
            messages: &ms.messages,
            temperature: 0.0,
            max_tokens: Some(2048),
            tools: None,
        };
        if let Ok(resp) = svc.runtime.provider().chat_completion(&summary_params).await {
            if let Some(text) = resp
                .choices
                .first()
                .and_then(|c| c.message.text_content())
            {
                let summary_delta = StreamDelta {
                    id: String::new(),
                    object: "chat.completion.chunk".to_string(),
                    created: 0,
                    model: svc.model.clone(),
                    choices: vec![StreamChoice {
                        index: 0,
                        delta: DeltaContent {
                            role: Some(Role::Assistant),
                            content: Some(text.into_owned()),
                            reasoning_content: None,
                            tool_calls: None,
                        },
                        finish_reason: Some("stop".to_string()),
                    }],
                    usage: None,
                    raw_sse_json: None,
                };
                let _ = send_step(
                    &svc.step_tx,
                    AgentStep::Delta {
                        turn_id: svc.turn_id.clone(),
                        delta: serde_json::to_value(&summary_delta).unwrap_or_default(),
                        raw_bytes: None,
                    },
                    false,
                )
                .await;
            }
        }
    }

    // Auto-exit Plan mode when turn ends without a plan file,
    // or emit PlanFileUpdate as fallback when a plan file exists
    // but exit_plan_mode was never called (so frontend can show approval card).
    if let Some(ref mode_state) = svc.mode_state {
        if mode_state.current_mode() == ExecutionMode::Plan {
            let ps = crate::builtin_tools::PlanFileStore::new(None);
            let has_plan = svc.session_id.as_ref().is_some_and(|sid| ps.plan_exists(sid));
            if !has_plan {
                mode_state.transition(ExecutionMode::Agent);
                tracing::info!(
                    agent_id = %svc.config.agent_id,
                    "auto-exited Plan mode — no plan file produced, returning to Agent mode"
                );
            } else if let Some(sid) = svc.session_id.as_ref() {
                let path = ps.plan_path(sid);
                tracing::info!(
                    agent_id = %svc.config.agent_id,
                    path = %path.display(),
                    "plan file exists but exit_plan_mode not called — emitting fallback PlanFileUpdate"
                );
                let _ = send_step(
                    &svc.step_tx,
                    AgentStep::PlanFileUpdate {
                        turn_id: svc.turn_id.clone(),
                        session_id: sid.clone(),
                        path: path.to_string_lossy().to_string(),
                        exists: true,
                    },
                    false,
                )
                .await;
            }
        }
    }

    // Check budget ceiling on all exit paths (not just force_stop).
    // This handles the case where a single LLM iteration exceeds the budget
    // (e.g., a long text response with no tool calls).
    if !ms.token_budget_reached {
        if let Some(ref tracker) = ms.budget_tracker {
            let output_tokens = ms.query_loop.acc_completion_tokens as u64;
            if output_tokens >= tracker.budget.target_tokens {
                ms.token_budget_reached = true;
                if let Some(sid) = svc.session_id.as_deref() {
                    token_budget::clear_session_budget(sid);
                }
            }
        }
    }

    tracing::info!(
        agent_id = %svc.config.agent_id,
        reason = %terminal_reason,
        iterations = ms.query_loop.iteration,
        total_tool_calls = ms.query_loop.total_tool_calls,
        content_len = llm_output.accumulated_content.len(),
        "streaming execution complete — sending Done"
    );

    let summary = make_turn_summary(
        &svc.turn_id,
        &ms.query_loop,
        svc.stream_start,
        svc.context_window,
    );
    let turn_end_reason = if ms.token_budget_reached {
        TurnEndReason::TokenBudgetReached
    } else {
        TurnEndReason::from(terminal_reason.clone())
    };
    let _ = send_step(
        &svc.step_tx,
        AgentStep::TurnEnd {
            turn_id: svc.turn_id.clone(),
            reason: turn_end_reason,
            summary: summary.clone(),
            session_id: svc.session_id.clone(),
        },
        false,
    )
    .await;

    svc.services.fire_stop_hooks(&ms.messages, &[]).await;
    svc.runtime
        .finalize_injected_skills(&ms.injected_skill_ids, true)
        .await;

    // TODO: record_completed_trajectory — needs a session_id-based variant on AgentRuntime.
    // Original: self.record_completed_trajectory(request, config, &trajectory_steps, true)

    let _obs = svc.runtime_observer.summary().await;
    svc.runtime_observer
        .clone()
        .finalize(TrajectoryOutcome::Success {
            user_rating: None,
        })
        .await;

    EndTurnOutcome::Done(summary)
}

