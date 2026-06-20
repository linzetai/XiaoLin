use xiaolin_core::types::{ChatMessage, Role};
use xiaolin_protocol::TurnSummary;

use super::agent_step::{AgentStep, TurnEndReason};
use super::goal_prompts;
use super::llm_call::LlmStreamOutput;
use super::stream_engine::send_step;
use super::tool_executor;
use super::tool_round::PreToolSnapshot;
use super::turn_state::{TurnMutableState, TurnServices};
use super::make_turn_summary;

/// Outcome of the post-tool processing phase.
pub(crate) enum PostToolOutcome {
    /// Normal: loop should proceed to the next iteration.
    Continue,
    /// The tool repetition hard limit was hit; the outer loop should
    /// `continue` immediately to give the LLM one final turn to explain.
    ForceContinue,
    /// Turn ended early (plan approval pending); return this summary.
    EarlyFinish(TurnSummary),
}

/// Post-tool processing after all tool calls in a round have been executed.
///
/// This phase covers:
/// 1. Plan approval pending check → early return
/// 2. Force-stop (tool repetition hard limit) → signal outer loop to `continue`
/// 3. Microcompact tool results + dedup repeated tool calls
/// 4. Context usage update event emission
/// 5. Mode change detection → `ModeChange` + `PlanFileUpdate` events
/// 6. Goal state change detection:
///    - New goal appeared → `GoalUpdated`
///    - Goal status changed → `GoalUpdated` + budget limit wrap-up injection
///    - Goal externally deleted → `GoalCleared` + cancel message injection
///    - Goal budget warning (80%) → warning message injection
/// 7. LLM truncation (finish_reason=length) with write tools → retry guidance injection
///
/// # Parameters
///
/// - `pre_snapshot`: mode and goal state captured *before* tool execution
/// - `force_stop_loop`: whether the tool repetition hard limit was hit
/// - `plan_approval_pending`: whether a plan tool awaits user approval
/// - `llm_output`: accumulated LLM response (for `last_finish_reason` and tool call names)
pub(crate) async fn post_tool_processing(
    ms: &mut TurnMutableState,
    svc: &TurnServices,
    pre_snapshot: PreToolSnapshot,
    force_stop_loop: bool,
    plan_approval_pending: bool,
    llm_output: &LlmStreamOutput,
) -> PostToolOutcome {
    // 1. Plan approval pending → emit TurnEnd and return early.
    if plan_approval_pending {
        tracing::info!(
            agent_id = %svc.config.agent_id,
            "breaking execution loop — plan approval pending, waiting for user"
        );
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
                reason: TurnEndReason::PlanApprovalPending,
                summary: summary.clone(),
                session_id: svc.session_id.clone(),
            },
            false,
        )
        .await;
        svc.runtime
            .finalize_injected_skills(&ms.injected_skill_ids, true)
            .await;
        return PostToolOutcome::EarlyFinish(summary);
    }

    // 2. Force-stop (repetition hard limit) → skip remaining post-processing.
    if force_stop_loop {
        tracing::warn!(
            agent_id = %svc.config.agent_id,
            "tool repetition hard limit reached — giving LLM one final turn to explain (stream)"
        );
        return PostToolOutcome::ForceContinue;
    }

    // 3. Microcompact + dedup so reported token count reflects what the next LLM
    //    call will actually see, rather than the raw uncompressed accumulation.
    {
        let keep_recent = tool_executor::keep_recent_for_context_window(svc.context_window);
        tool_executor::microcompact_tool_results(&mut ms.messages, keep_recent);
        tool_executor::dedup_repeated_tool_calls(&mut ms.messages);
        let post_tool_tokens = xiaolin_context::estimate_messages_tokens(&ms.messages);
        ms.query_loop.last_estimated_tokens = post_tool_tokens;
        let _ = send_step(
            &svc.step_tx,
            AgentStep::ContextUsage {
                turn_id: svc.turn_id.clone(),
                used_tokens: post_tool_tokens as u32,
                limit_tokens: svc.context_window,
                compressed: false,
                tokens_saved: 0,
            },
            false,
        )
        .await;
    }

    // 4. Mode change detection.
    if let (Some(before), Some(mode_state_ref)) = (pre_snapshot.mode_before, svc.mode_state.as_ref()) {
        let after = mode_state_ref.current_mode();
        if before != after {
            let _ = send_step(
                &svc.step_tx,
                AgentStep::ModeChange {
                    turn_id: svc.turn_id.clone(),
                    from: before,
                    to: after,
                },
                false,
            )
            .await;

            if let Some(pc) = crate::builtin_tools::plan_mode::current_plan_context() {
                let path = pc.store.plan_path(&pc.session_id);
                let exists = pc.store.plan_exists(&pc.session_id);
                let content = if exists {
                    tokio::fs::read_to_string(&path).await.ok()
                } else {
                    None
                };
                let _ = send_step(
                    &svc.step_tx,
                    AgentStep::PlanFileUpdate {
                        turn_id: svc.turn_id.clone(),
                        session_id: pc.session_id.clone(),
                        path: path.to_string_lossy().to_string(),
                        exists,
                        content,
                    },
                    false,
                )
                .await;
            }
        }
    }

    // 5. Goal state change detection.
    if let Some(ref gs) = svc.goal_store {
        let goal_after = gs.get_current().await;
        match (&pre_snapshot.goal_before, &goal_after) {
            (None, Some(g)) => {
                ms.last_seen_goal_id = Some(g.id.clone());
                let _ = send_step(
                    &svc.step_tx,
                    AgentStep::GoalUpdated {
                        turn_id: svc.turn_id.clone(),
                        goal: g.to_goal_data(),
                    },
                    false,
                )
                .await;
            }
            (Some((_, prev_status)), Some(g)) if *prev_status != g.status => {
                let _ = send_step(
                    &svc.step_tx,
                    AgentStep::GoalUpdated {
                        turn_id: svc.turn_id.clone(),
                        goal: g.to_goal_data(),
                    },
                    false,
                )
                .await;
                if g.status == crate::builtin_tools::GoalStatus::BudgetLimited
                    && *prev_status == crate::builtin_tools::GoalStatus::Active
                {
                    tracing::info!(
                        goal_id = %g.id,
                        "budget limit reached mid-turn, injecting wrap-up steering"
                    );
                    let hint = crate::runtime::goal_prompts::render_budget_limit_prompt(g);
                    ms.messages.push(ChatMessage {
                        role: Role::User,
                        content: Some(serde_json::Value::String(hint)),
                        ..Default::default()
                    });
                }
            }
            (Some((prev_id, prev_status)), None) => {
                let row_deleted = !gs.row_exists(prev_id).await;
                if row_deleted {
                    let _ = send_step(
                        &svc.step_tx,
                        AgentStep::GoalCleared {
                            turn_id: svc.turn_id.clone(),
                            goal_id: prev_id.clone(),
                        },
                        false,
                    )
                    .await;
                    if !prev_status.is_terminal() {
                        tracing::info!(
                            agent_id = %svc.config.agent_id,
                            prev_goal_id = %prev_id,
                            "goal externally deleted — stopping autonomous loop"
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
                        return PostToolOutcome::ForceContinue;
                    }
                } else {
                    if let Ok(Some(row)) = gs.session_store().get_goal(prev_id).await {
                        let g = crate::builtin_tools::Goal::from_row(row);
                        let _ = send_step(
                            &svc.step_tx,
                            AgentStep::GoalUpdated {
                                turn_id: svc.turn_id.clone(),
                                goal: g.to_goal_data(),
                            },
                            false,
                        )
                        .await;
                    }
                }
            }
            _ => {}
        }

        if let Some(ref g) = goal_after {
            if g.status == crate::builtin_tools::GoalStatus::Active
                && gs.check_budget_warning(g).await
            {
                tracing::info!(
                    goal_id = %g.id,
                    tokens_used = g.tokens_used,
                    "goal budget 80% warning triggered"
                );
                let warning = crate::runtime::goal_prompts::render_budget_warning_prompt(g);
                ms.messages.push(ChatMessage {
                    role: Role::User,
                    content: Some(serde_json::Value::String(warning)),
                    ..Default::default()
                });
            }
        }
    }

    // 6. LLM truncation guidance: if finish_reason=length and write tools were used,
    //    inject a system message so the model knows to verify and fix truncated content.
    if let Some(ref reason) = llm_output.last_finish_reason {
        if reason == "length" {
            let has_write_tools = llm_output.tool_call_accum.iter().any(|tc| {
                let n = tc.name.as_str();
                n == "write_file" || n == "edit_file" || n == "multi_edit"
            });
            if has_write_tools {
                tracing::warn!(
                    agent_id = %svc.config.agent_id,
                    "LLM output truncated (finish_reason=length) with write/edit tool calls — injecting retry guidance"
                );
                ms.messages.push(ChatMessage {
                    role: Role::System,
                    content: Some(serde_json::Value::String(
                        "[WARNING] Your previous response was truncated (finish_reason=length). \
                         The file content you wrote may be incomplete. Please verify the file \
                         with read_file and fix any truncated content. When writing large files, \
                         break the work into smaller edit_file calls instead of one large write_file."
                            .to_string(),
                    )),
                    ..Default::default()
                });
            }
        }
    }

    // 7. Message queue drain: inject steering messages from external sources
    //    (SendMessageTool, gateway WS, completion hooks) as user-role messages.
    if let Some(ref mq) = svc.message_queue {
        let queued = mq.drain_all();
        if !queued.is_empty() {
            let sources: Vec<String> = queued.iter().map(|m| m.source.clone()).collect();
            let count = queued.len();
            tracing::info!(
                agent_id = %svc.config.agent_id,
                count,
                sources = ?sources,
                "injecting steering messages from message queue"
            );
            for msg in &queued {
                let steering_content = format!(
                    "[Steering from {}]: {}",
                    msg.source, msg.content
                );
                ms.messages.push(ChatMessage {
                    role: Role::User,
                    content: Some(serde_json::Value::String(steering_content)),
                    ..Default::default()
                });
            }
            let _ = send_step(
                &svc.step_tx,
                AgentStep::SteeringInjected { count, sources },
                false,
            )
            .await;
        }
    }

    PostToolOutcome::Continue
}
