use axum::extract::ws::{Message, WebSocket};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::chat_pipeline::{
    after_chat, maybe_spawn_smart_title_background, setup_chat, SetupChatOptions,
};
use crate::state::AppState;
use fastclaw_core::types::{AgentId, ChatMessage, ChatRequest};
use fastclaw_protocol::{AgentEvent, ChatParams};

use super::send_resp;
use super::types::WsResponse;

pub async fn handle_chat_cancel(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    req_id: Option<String>,
    params: serde_json::Value,
    active_chat_cancels: Arc<tokio::sync::Mutex<HashMap<String, CancellationToken>>>,
) {
    let Some(target_req_id) = params.get("requestId").and_then(|v| v.as_str()) else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "requestId required"})),
            },
        )
        .await;
        return;
    };

    let token = {
        let mut guard = active_chat_cancels.lock().await;
        guard.remove(target_req_id)
    };

    let cancelled = if let Some(token) = token {
        token.cancel();
        true
    } else {
        false
    };

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "cancel".into(),
            data: Some(json!({"requestId": target_req_id, "cancelled": cancelled})),
            error: None,
        },
    )
    .await;
}

/// Delivers a user answer to a pending `ask_question` / `confirm` request.
pub async fn handle_chat_answer(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let Some(request_id) = params.get("requestId").and_then(|v| v.as_str()) else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "requestId required"})),
            },
        )
        .await;
        return;
    };

    let answer = params
        .get("answer")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let tx = state
        .strm
        .ask_question_pending
        .remove(request_id)
        .map(|(_k, v)| v);

    let ok = if let Some(tx) = tx {
        let _ = tx.send(answer);
        true
    } else {
        false
    };

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "answer".into(),
            data: Some(json!({"requestId": request_id, "ok": ok})),
            error: None,
        },
    )
    .await;
}

/// Switches execution mode between agent and plan for a given session.
pub async fn handle_chat_set_mode(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
    bg_tx: &Option<tokio::sync::mpsc::Sender<WsResponse>>,
) {
    let Some(mode_str) = params.get("mode").and_then(|v| v.as_str()) else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(
                    json!({"code": -32602, "message": "mode required ('agent' or 'plan')"}),
                ),
            },
        )
        .await;
        return;
    };

    let session_id = params
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    use fastclaw_core::types::ExecutionMode;
    let target = match mode_str {
        "plan" => ExecutionMode::Plan,
        "agent" => ExecutionMode::Agent,
        _ => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": -32602, "message": "Invalid mode. Expected 'agent' or 'plan'."})),
                },
            )
            .await;
            return;
        }
    };

    let (from, to) = state.rt.session_modes.transition(session_id, target);

    // RPC response
    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "set_mode".into(),
            data: Some(json!({"ok": true, "from": format!("{from}"), "to": format!("{to}")})),
            error: None,
        },
    )
    .await;

    // Broadcast mode_change event so other listeners (multi-window) stay in sync
    if from != to {
        if let Some(tx) = bg_tx {
            let _ = tx
                .send(WsResponse {
                    id: None,
                    msg_type: "mode_change".into(),
                    data: Some(
                        json!({"from": format!("{from}"), "to": format!("{to}"), "session_id": session_id}),
                    ),
                    error: None,
                })
                .await;
        }
    }
}

/// Spawns streaming chat on a background task that sends WsResponse messages
/// through `bg_tx`. Uses the same session/memory/compaction logic as the HTTP path.
/// Cancelled when the client disconnects (cancel token fires).
pub async fn spawn_chat(
    state: &AppState,
    owned_sessions: &mut HashSet<String>,
    bg_tx: tokio::sync::mpsc::Sender<WsResponse>,
    cancel: CancellationToken,
    active_chat_cancels: Arc<tokio::sync::Mutex<HashMap<String, CancellationToken>>>,
    req_id: Option<String>,
    params: ChatParams,
) {
    let chat_start = Instant::now();
    // Auto-claim session if provided and not yet owned.
    // This allows the frontend to chat directly without an explicit claim step.
    if let Some(sid) = params.session_id.as_deref() {
        if !owned_sessions.contains(sid) {
            // Verify session exists before claiming
            match state.store.session_store.get_session(sid).await {
                Ok(Some(_)) => {
                    owned_sessions.insert(sid.to_string());
                    tracing::debug!(session_id = %sid, "auto-claimed session on first chat");
                }
                Ok(None) => {
                    // Session doesn't exist - will be created by setup_chat
                    tracing::debug!(session_id = %sid, "session not found, will be created");
                }
                Err(e) => {
                    let _ = bg_tx
                        .send(WsResponse {
                            id: req_id,
                            msg_type: "error".into(),
                            data: None,
                            error: Some(json!({"code": 500, "message": format!("failed to verify session: {e}")})),
                        })
                        .await;
                    return;
                }
            }
        }
    }
    let state = state.clone();
    let rid = req_id.clone();
    let req_cancel = CancellationToken::new();
    let stream_context_key = uuid::Uuid::new_v4().to_string();
    if let Some(ref rid_str) = rid {
        let mut guard = active_chat_cancels.lock().await;
        guard.insert(rid_str.clone(), req_cancel.clone());
    }
    let active_chat_cancels_for_task = active_chat_cancels.clone();

    tokio::spawn(async move {
        let rid_for_cleanup = rid.clone();

        let messages: Vec<ChatMessage> = match serde_json::from_value::<Vec<ChatMessage>>(
            if params.messages.is_null() {
                serde_json::Value::Array(vec![])
            } else {
                params.messages.clone()
            },
        ) {
            Ok(m) => {
                for msg in m.iter() {
                    if let Some(serde_json::Value::Array(parts)) = &msg.content {
                        let image_count = parts.iter().filter(|p| p.get("type").and_then(|t| t.as_str()) == Some("image_url")).count();
                        if image_count > 0 {
                            tracing::info!(image_count, role = ?msg.role, "received multimodal message with images");
                        }
                    }
                }
                m
            }
            Err(e) => {
                let _ = bg_tx
                    .send(WsResponse {
                        id: rid,
                        msg_type: "error".into(),
                        data: None,
                        error: Some(json!({"message": format!("invalid messages: {e}")})),
                    })
                    .await;
                if let Some(rid) = &rid_for_cleanup {
                    let mut guard = active_chat_cancels_for_task.lock().await;
                    guard.remove(rid);
                }
                return;
            }
        };

        let request = ChatRequest {
            messages,
            stream: true,
            model: params.model.clone(),
            temperature: params.temperature.map(|f| f as f32),
            max_tokens: params.max_tokens,
            agent_id: params.agent_id.as_deref().map(AgentId::from),
            session_id: params.session_id.as_deref().map(Into::into),
            tools: None,
            slash_intent: params
                .extra
                .get("slashIntent")
                .and_then(|v| serde_json::from_value(v.clone()).ok()),
            work_dir: params.work_dir.clone(),
        };

        let setup = match setup_chat(
            &state,
            &request,
            SetupChatOptions {
                chat_stream: true,
                propagate_context_ingest_errors: false,
                set_resolved_session_on_request: true,
                record_chat_observe: false,
            },
        )
        .await
        {
            Ok(s) => s,
            Err(e) => {
                let _ = bg_tx
                    .send(WsResponse {
                        id: rid,
                        msg_type: "error".into(),
                        data: None,
                        error: Some(e.to_ws_error_value()),
                    })
                    .await;
                if let Some(rid) = &rid_for_cleanup {
                    let mut guard = active_chat_cancels_for_task.lock().await;
                    guard.remove(rid);
                }
                return;
            }
        };

        let turn_cancel = CancellationToken::new();
        {
            let turn_cancel2 = turn_cancel.clone();
            let conn_cancel = cancel.clone();
            let req_cancel2 = req_cancel.clone();
            tokio::spawn(async move {
                tokio::select! {
                    _ = conn_cancel.cancelled() => {},
                    _ = req_cancel2.cancelled() => {},
                }
                turn_cancel2.cancel();
            });
        }

        let agent_config = setup.agent_config.clone();
        let agent_id = setup.agent_id.clone();
        let session_id = setup.session_id.clone();
        let needs_title = setup.needs_title;
        let resolve_reason = setup.resolve_reason;
        let input_estimate = setup.input_estimate;
        let model_for_budget = setup.model_for_budget.clone();
        let state_budget = state.clone();

        let coordinator = state
            .strm
            .coordinator_registry
            .get_or_create(&fastclaw_protocol::SessionId::new(&session_id));
        if coordinator.is_active().await {
            coordinator.cancel_active_turn().await;
        }

        // Persist user messages to session
        for msg in &setup.user_messages {
            let _ = state
                .store
                .session_store
                .append_message(&session_id, msg)
                .await;
            // Dual-write: persist as HistoryItems alongside legacy messages
            {
                let turn_id = fastclaw_protocol::TurnId::generate();
                let history_items =
                    fastclaw_core::history_compat::chat_message_to_history(msg, turn_id);
                if let Err(e) = state
                    .store
                    .session_store
                    .append_history_items(&session_id, &history_items)
                    .await
                {
                    tracing::warn!(session_id = %session_id, error = %e, "failed to dual-write history items");
                }
            }
        }

        let (mut reserved, budget_degraded) = (setup.reserved_cost, setup.budget_degraded);

        let start_model = setup
            .enriched_request
            .model
            .clone()
            .unwrap_or_else(|| agent_config.model.model.clone());

        // chat.start
        let mut start_payload = json!({
            "model": start_model,
            "sessionId": &session_id,
            "resolvedAgent": &agent_id,
            "resolveReason": resolve_reason,
        });
        if budget_degraded {
            start_payload["budgetDegraded"] = json!(true);
        }
        let _ = bg_tx
            .send(WsResponse {
                id: rid.clone(),
                msg_type: "turn_start".into(),
                data: Some(start_payload),
                error: None,
            })
            .await;

        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);

        let after_turn_messages = setup.enriched_request.messages.clone();

        let runtime = state.rt.runtime.clone();
        let tool_reg = state.rt.tool_registry.clone();
        let cfg = agent_config;
        let enriched = setup.enriched_request.clone();
        let cancel2 = turn_cancel.clone();
        let llm_for_task = setup.llm_override.clone();
        state
            .strm
            .stream_event_tx
            .insert(stream_context_key.clone(), tx.clone());
        let stream_context_key_for_task = stream_context_key.clone();
        let stream_event_map_for_task = state.strm.stream_event_tx.clone();
        let confirm_pending_for_task = state.strm.ask_question_pending.clone();
        let subagent_prompt = {
            let policy = &cfg.behavior.subagent;
            let available = state.strm.subagent_manager.agent_descriptions();
            let ctx = fastclaw_agent::SubAgentPromptContext {
                policy,
                available_agents: &available,
                current_depth: 0,
            };
            fastclaw_agent::build_subagent_prompt_block(&ctx)
        };

        let mode_state_for_task = state.rt.session_modes.get_or_create(&session_id);
        let session_store_for_task = Some(state.store.session_store.clone());
        let todo_store_for_task = Some(state.rt.todo_store.clone());
        let orchestrator_for_task = Some(state.strm.tool_orchestrator.clone());
        let plan_ctx_for_task = Some(fastclaw_agent::builtin_tools::PlanContext {
            session_id: session_id.clone(),
            store: state.rt.plan_file_store.clone(),
        });
        let task = tokio::spawn(async move {
            let ms_clone = mode_state_for_task.clone();
            tokio::select! {
                result = fastclaw_agent::builtin_tools::with_stream_context(
                    stream_context_key_for_task.clone(),
                    fastclaw_agent::builtin_tools::with_session_mode(
                        ms_clone,
                        plan_ctx_for_task,
                        runtime.execute_stream_with_confirm(&cfg, &enriched, &tool_reg, tx, llm_for_task, confirm_pending_for_task, subagent_prompt, Some(mode_state_for_task), session_store_for_task, todo_store_for_task, orchestrator_for_task),
                    ),
                ) => result,
                _ = cancel2.cancelled() => Err(anyhow::anyhow!("cancelled")),
            }
        });

        // Forward stream events → WsResponse → bg_tx, collect assistant content.
        // On Done: persist assistant message BEFORE forwarding to client.
        let mut assistant_content = String::new();
        let mut pending_question_ids: Vec<String> = Vec::new();
        let mut stream_ended = false; // true if Done or Error received
        let mut current_turn_id: Option<fastclaw_protocol::TurnId> = None;

        // Use tokio::select! to race rx.recv() against task completion.
        // This prevents a deadlock when the task panics/errors but the extra
        // tx clone in stream_event_tx keeps the channel alive.
        let mut task = task;
        let mut task_completed = false;
        let mut task_result: Option<Result<anyhow::Result<fastclaw_protocol::TurnSummary>, tokio::task::JoinError>> = None;

        loop {
            let event = tokio::select! {
                biased;
                ev = rx.recv() => {
                    match ev {
                        Some(e) => e,
                        None => break,
                    }
                }
                result = &mut task, if !task_completed => {
                    task_completed = true;
                    task_result = Some(result);
                    // Drop the extra tx clone so rx.recv() can return None
                    // once any buffered events are drained.
                    stream_event_map_for_task.remove(&stream_context_key);
                    continue;
                }
            };
            if let AgentEvent::TurnStart { turn_id, .. } = &event {
                current_turn_id = Some(turn_id.clone());
                let _ = coordinator
                    .register_turn(turn_id.clone(), turn_cancel.clone())
                    .await;
            }
            if turn_cancel.is_cancelled() {
                if reserved > 0.0 {
                    let _ = state.obs.budget_tracker.release_reservation(reserved);
                    reserved = 0.0;
                }
                if !assistant_content.is_empty() {
                    let assistant_msg = ChatMessage {
                        role: fastclaw_core::types::Role::Assistant,
                        content: Some(serde_json::Value::String(assistant_content.clone())),
                        reasoning_content: None,
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
            compact_metadata: None,
                    };
                    let _ = after_chat(&state, &setup, &assistant_msg, false).await;
                }
                break;
            }
            if let Err(e) = state.store.event_log.append(&session_id, &event).await {
                tracing::warn!(
                    session_id = %session_id,
                    error = %e,
                    "failed to append stream event to event log"
                );
            }
            let _ = coordinator.event_sender().send(event.clone());
            if matches!(&event, AgentEvent::Error { .. }) {
                if reserved > 0.0 {
                    let _ = state.obs.budget_tracker.release_reservation(reserved);
                    reserved = 0.0;
                }
                if !assistant_content.is_empty() {
                    let assistant_msg = ChatMessage {
                        role: fastclaw_core::types::Role::Assistant,
                        content: Some(serde_json::Value::String(assistant_content.clone())),
                        reasoning_content: None,
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
            compact_metadata: None,
                    };
                    let _ = after_chat(&state, &setup, &assistant_msg, false).await;
                }
            }
            if let AgentEvent::ContentDelta { ref delta, .. } = event {
                if let Some(text) = delta
                    .get("choices")
                    .and_then(|c| c.get(0))
                    .and_then(|c| c.get("delta"))
                    .and_then(|d| d.get("content"))
                    .and_then(|c| c.as_str())
                {
                    assistant_content.push_str(text);
                }
            }
            let is_done = matches!(&event, AgentEvent::TurnEnd { .. });
            if is_done || matches!(&event, AgentEvent::Error { .. }) {
                stream_ended = true;
            }
            if let AgentEvent::AskQuestion { request_id, .. } = &event {
                pending_question_ids.push(request_id.clone());
            }
            if is_done {
                crate::routes::record_chat_budget_stream_estimate(
                    &state_budget,
                    model_for_budget.as_str(),
                    input_estimate,
                    assistant_content.len(),
                );
            }
            if is_done && !assistant_content.is_empty() {
                let assistant_msg = ChatMessage {
                    role: fastclaw_core::types::Role::Assistant,
                    content: Some(serde_json::Value::String(assistant_content.clone())),
                    reasoning_content: None,
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
            compact_metadata: None,
                };
                let _ = after_chat(&state, &setup, &assistant_msg, false).await;
            }
            // Persist per-message and session-level usage on Done
            if let AgentEvent::TurnEnd {
                ref summary,
                ..
            } = event
            {
                let wall_ms = chat_start.elapsed().as_millis() as u64;
                let pt = summary.usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0);
                let ct = summary.usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0);
                let tt = summary.usage.as_ref().map(|u| u.total_tokens).unwrap_or(0);
                let ems = if wall_ms > 0 {
                    wall_ms
                } else {
                    summary.elapsed_ms
                };
                let _ = state
                    .store
                    .session_store
                    .accumulate_usage(&session_id, pt, ct, ems)
                    .await;
                let _ = state
                    .store
                    .session_store
                    .stamp_last_assistant_usage(&session_id, pt, ct, tt, ems)
                    .await;
            }
            if let AgentEvent::TurnEnd {
                session_id: Some(ref sid),
                ..
            } = &event
            {
                let _ = state.strm.ws_broadcast.send(
                    json!({"type":"event","event":"sessions.changed","data":{"sessionId": sid}})
                        .to_string(),
                );
            }
            let mut resp = forward_event(&event, &rid);
            if is_done {
                if let Some(data) = resp.data.as_mut().and_then(|d| d.as_object_mut()) {
                    let elapsed_ms = chat_start.elapsed().as_millis() as u64;
                    data.insert("elapsedMs".into(), json!(elapsed_ms));
                    data.insert("inputTokensEstimate".into(), json!(input_estimate));
                    let output_estimate = assistant_content.len() / 4;
                    data.insert("outputTokensEstimate".into(), json!(output_estimate));
                }
            }
            if bg_tx.send(resp).await.is_err() {
                break;
            }
        }

        if turn_cancel.is_cancelled() {
            let turn_id = current_turn_id.unwrap_or_else(fastclaw_protocol::TurnId::generate);
            let aborted = AgentEvent::TurnAborted {
                turn_id: turn_id.clone(),
                reason: fastclaw_protocol::AbortReason::Interrupted,
                completed_at: Some(chrono::Utc::now().to_rfc3339()),
                duration_ms: Some(chat_start.elapsed().as_millis() as u64),
            };
            if let Err(e) = state.store.event_log.append(&session_id, &aborted).await {
                tracing::warn!(
                    session_id = %session_id,
                    error = %e,
                    "failed to append turn_aborted event to event log"
                );
            }
            let _ = bg_tx.send(forward_event(&aborted, &rid)).await;
        }

        coordinator.complete_turn().await;

        // Run after_turn hooks (memory updates, compaction, etc.)
        if !turn_cancel.is_cancelled() {
            let _ = state
                .store
                .context_engine
                .after_turn(&after_turn_messages, &agent_id, &session_id)
                .await;
        }

        // Generate a smart session title via LLM (background, non-blocking)
        if needs_title && !assistant_content.is_empty() {
            maybe_spawn_smart_title_background(&state, &setup, &assistant_content);
        }

        // If select! already captured the task result, use it; otherwise await.
        let task_result = match task_result {
            Some(r) => r,
            None => task.await,
        };
        match task_result {
            Ok(Err(e)) if !turn_cancel.is_cancelled() => {
                if reserved > 0.0 {
                    let _ = state.obs.budget_tracker.release_reservation(reserved);
                }
                if !assistant_content.is_empty() {
                    let assistant_msg = ChatMessage {
                        role: fastclaw_core::types::Role::Assistant,
                        content: Some(serde_json::Value::String(assistant_content.clone())),
                        reasoning_content: None,
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
            compact_metadata: None,
                    };
                    let _ = after_chat(&state, &setup, &assistant_msg, false).await;
                }
                let _ = bg_tx
                    .send(WsResponse {
                        id: rid,
                        msg_type: "error".into(),
                        data: None,
                        error: Some(json!({"message": format!("{e}")})),
                    })
                    .await;
            }
            Err(e) => {
                if reserved > 0.0 {
                    let _ = state.obs.budget_tracker.release_reservation(reserved);
                }
                if !assistant_content.is_empty() {
                    let assistant_msg = ChatMessage {
                        role: fastclaw_core::types::Role::Assistant,
                        content: Some(serde_json::Value::String(std::mem::take(
                            &mut assistant_content,
                        ))),
                        reasoning_content: None,
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
            compact_metadata: None,
                    };
                    let _ = after_chat(&state, &setup, &assistant_msg, false).await;
                }
                let _ = bg_tx
                    .send(WsResponse {
                        id: rid,
                        msg_type: "error".into(),
                        data: None,
                        error: Some(json!({"message": format!("task panic: {e}")})),
                    })
                    .await;
            }
            Ok(Ok(_)) if !turn_cancel.is_cancelled() && !stream_ended => {
                if reserved > 0.0 {
                    let _ = state.obs.budget_tracker.release_reservation(reserved);
                }
                let _ = bg_tx
                    .send(WsResponse {
                        id: rid,
                        msg_type: "error".into(),
                        data: None,
                        error: Some(json!({"message": "Chat completed without response. The model provider may be unavailable."})),
                    })
                    .await;
            }
            _ => {}
        }
        if !pending_question_ids.is_empty() {
            let pending = &state.strm.ask_question_pending;
            for request_id in pending_question_ids {
                pending.remove(&request_id);
            }
        }
        stream_event_map_for_task.remove(&stream_context_key);
        if let Some(rid) = &rid_for_cleanup {
            let mut guard = active_chat_cancels_for_task.lock().await;
            guard.remove(rid);
        }
    });
}

pub fn forward_event(event: &AgentEvent, req_id: &Option<String>) -> WsResponse {
    let data = serde_json::to_value(event).unwrap_or_default();
    let msg_type = data
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let error = if matches!(event, AgentEvent::Error { .. }) {
        data.get("message").map(|msg| json!({"message": msg}))
    } else {
        None
    };

    WsResponse {
        id: req_id.clone(),
        msg_type,
        data: Some(data),
        error,
    }
}
