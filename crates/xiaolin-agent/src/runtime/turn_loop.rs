use xiaolin_core::types::{ChatMessage, Role};
use xiaolin_protocol::{TurnQualityDiagnosisCode, TurnQualitySeverity, TurnSummary};

use super::agent_step::AgentStep;
use super::end_turn::{self, EndTurnOutcome};
use super::goal_prompts;
use super::iteration_check::{self, PreCheckOutcome};
use super::llm_call::{self, LlmCallOutcome};
use super::make_turn_summary;
use super::post_tool::{self, PostToolOutcome};
use super::query_state;
use super::runtime_quality;
use super::stream_engine::send_step;
use super::tool_round::{self, ToolRoundOutcome};
use super::turn_state::{TurnMutableState, TurnServices};

/// The core agent loop, expressed as composition of sub-phase functions.
///
/// This replaces the monolithic body of `execute_stream_inner`.
/// Each iteration: pre-check → LLM call → dispatch transition → [tool round → post-tool].
pub(crate) async fn run_turn_loop(
    ms: &mut TurnMutableState,
    svc: &TurnServices,
) -> anyhow::Result<TurnSummary> {
    loop {
        ms.had_tool_calls_this_round = false;
        ms.had_progress_this_round = false;
        ms.had_verification_this_round = false;

        // ═══════════════════════════════════════════════════════════════════
        // Phase 0: Cancellation check
        // ═══════════════════════════════════════════════════════════════════
        if let Some(ref token) = svc.cancel_token {
            if token.is_cancelled() {
                tracing::info!(
                    agent_id = %svc.config.agent_id,
                    "agent loop cancelled via CancellationToken"
                );
                let _ = send_step(
                    &svc.step_tx,
                    AgentStep::Error {
                        turn_id: svc.turn_id.clone(),
                        message: "Execution cancelled".to_string(),
                        error_code: None,
                        recoverable: false,
                    },
                    false,
                )
                .await;
                svc.runtime
                    .finalize_injected_skills(&ms.injected_skill_ids, false)
                    .await;
                persist_runtime_quality_summary(
                    ms,
                    svc,
                    Some((TurnQualityDiagnosisCode::Aborted, TurnQualitySeverity::Warn)),
                )
                .await;
                return Ok(make_turn_summary(
                    &svc.turn_id,
                    &ms.query_loop,
                    svc.stream_start,
                    svc.context_window,
                ));
            }
        }

        // ═══════════════════════════════════════════════════════════════════
        // Phase 1: Per-iteration pre-checks (context compaction, limits)
        // ═══════════════════════════════════════════════════════════════════
        let estimated_tokens = match iteration_check::iteration_pre_check(ms, svc).await {
            PreCheckOutcome::Continue { estimated_tokens } => estimated_tokens,
            PreCheckOutcome::EarlyFinish(summary) => {
                persist_runtime_quality_summary(
                    ms,
                    svc,
                    Some((TurnQualityDiagnosisCode::Error, TurnQualitySeverity::Error)),
                )
                .await;
                return Ok(summary);
            }
            PreCheckOutcome::FatalError(e) => {
                persist_runtime_quality_summary(
                    ms,
                    svc,
                    Some((TurnQualityDiagnosisCode::Error, TurnQualitySeverity::Error)),
                )
                .await;
                return Err(e);
            }
        };

        // ═══════════════════════════════════════════════════════════════════
        // Phase 2: LLM streaming call (includes recovery + model critic)
        // ═══════════════════════════════════════════════════════════════════
        let mut llm_output = match llm_call::perform_llm_call(ms, svc, estimated_tokens).await {
            LlmCallOutcome::Completed(output) => *output,
            LlmCallOutcome::RetryIteration => continue, // mode_turn_counted stays true → skip re-increment
            LlmCallOutcome::FatalError(e) => {
                persist_runtime_quality_summary(
                    ms,
                    svc,
                    Some((TurnQualityDiagnosisCode::Error, TurnQualitySeverity::Error)),
                )
                .await;
                return Err(e);
            }
            LlmCallOutcome::EarlyFinish(summary) => return Ok(summary),
        };
        // Reset after successful LLM call so the next iteration can count.
        // NOT reset on RetryIteration (continue above) to prevent double-counting.
        ms.mode_turn_counted = false;

        // ═══════════════════════════════════════════════════════════════════
        // Phase 3: Dispatch on LLM transition
        // ═══════════════════════════════════════════════════════════════════
        match llm_output.transition {
            query_state::LoopTransition::Terminal(ref reason) => {
                // EndTurn / MaxIterations — finalize and return (or continue if stop hook fires)
                match end_turn::handle_end_turn(ms, svc, &llm_output, reason).await {
                    EndTurnOutcome::Done(summary) => return Ok(summary),
                    EndTurnOutcome::StopHookContinuation => continue,
                }
            }
            query_state::LoopTransition::Continue(_) => {
                // ───────────────────────────────────────────────────────────
                // Pre-tool: check if goal was externally cancelled
                // ───────────────────────────────────────────────────────────
                if check_goal_cancelled(ms, svc).await {
                    continue;
                }

                // ═══════════════════════════════════════════════════════════
                // Phase 4: Tool execution round
                // ═══════════════════════════════════════════════════════════
                match tool_round::execute_tool_round(ms, svc, &mut llm_output).await {
                    ToolRoundOutcome::Continue {
                        pre_snapshot,
                        force_stop_loop,
                        plan_approval_pending,
                    } => {
                        // ═══════════════════════════════════════════════════
                        // Phase 5: Post-tool processing
                        // ═══════════════════════════════════════════════════
                        match post_tool::post_tool_processing(
                            ms,
                            svc,
                            pre_snapshot,
                            force_stop_loop,
                            plan_approval_pending,
                            &llm_output,
                        )
                        .await
                        {
                            PostToolOutcome::Continue => {}
                            PostToolOutcome::ForceContinue => continue,
                            PostToolOutcome::EarlyFinish(summary) => return Ok(summary),
                        }
                    }
                    ToolRoundOutcome::EarlyFinish(summary) => return Ok(summary),
                }
            }
        }
    }
}

async fn persist_runtime_quality_summary(
    ms: &TurnMutableState,
    svc: &TurnServices,
    diagnosis_override: Option<(TurnQualityDiagnosisCode, TurnQualitySeverity)>,
) {
    if let Err(e) = runtime_quality::persist_runtime_quality_summary(
        svc.runtime_quality_store.as_deref(),
        svc.session_id.as_deref(),
        &ms.runtime_quality,
        &svc.turn_id,
        &svc.config,
        &svc.model,
        svc.stream_start,
        svc.context_window,
        &ms.query_loop,
        diagnosis_override,
    )
    .await
    {
        tracing::warn!(
            error = %e,
            session_id = ?svc.session_id,
            turn_id = %svc.turn_id,
            "failed to persist runtime quality summary"
        );
    }
}

/// Check if the current goal was externally deleted (user cancelled).
/// If so, inject a cancellation message and set force_stop_after_next.
/// Returns `true` if the outer loop should `continue`.
async fn check_goal_cancelled(ms: &mut TurnMutableState, svc: &TurnServices) -> bool {
    let (Some(ref gs), Some(ref gid)) = (&svc.goal_store, &ms.last_seen_goal_id) else {
        return false;
    };

    if gs.get_current().await.is_some() || gs.row_exists(gid).await {
        return false;
    }

    tracing::info!(
        agent_id = %svc.config.agent_id,
        goal_id = %gid,
        "goal externally deleted — injecting cancellation notice"
    );
    ms.messages.push(ChatMessage {
        role: Role::User,
        content: Some(serde_json::Value::String(
            goal_prompts::GOAL_CANCELLED_PROMPT.to_string(),
        )),
        ..Default::default()
    });
    ms.query_loop.force_stop_after_next = true;
    ms.last_seen_goal_id = None;
    true
}
