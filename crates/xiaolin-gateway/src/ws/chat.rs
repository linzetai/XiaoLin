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
use xiaolin_core::types::{AgentId, ChatMessage, ChatRequest};
use xiaolin_protocol::{AgentEvent, ChatParams};
use xiaolin_session_actor::SessionOp;

use super::send_resp;
use super::types::WsResponse;

pub async fn handle_chat_cancel(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
    active_chat_sessions: Arc<tokio::sync::Mutex<HashMap<String, String>>>,
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

    let session_id_for_cancel = {
        let mut guard = active_chat_sessions.lock().await;
        guard.remove(target_req_id)
    };

    let mut cancelled = false;
    if let Some(sid) = session_id_for_cancel {
        if let Some(handle) = state.svc.session_manager.get(&xiaolin_protocol::SessionId::new(&sid)).await {
            let _ = handle.submit(SessionOp::Interrupt).await;
            cancelled = true;
        }
    }

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

    let session_id = params
        .get("sessionId")
        .or_else(|| params.get("session_id"))
        .and_then(|v| v.as_str());

    let mut ok = false;
    if let Some(sid) = session_id {
        if let Some(handle) = state
            .svc
            .session_manager
            .get(&xiaolin_protocol::SessionId::new(sid))
            .await
        {
            ok = handle
                .submit(xiaolin_session_actor::SessionOp::ResolveAnswer {
                    interaction_id: request_id.to_string(),
                    answer: answer.clone(),
                })
                .await
                .is_ok();
        }
    }

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

/// Triggers manual context compaction for a session via `SessionOp::Compact`.
pub async fn handle_chat_compact(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    session_id: &str,
    bg_tx: &tokio::sync::mpsc::Sender<WsResponse>,
) {
    let sid = xiaolin_protocol::SessionId::new(session_id);
    let Some(handle) = state.svc.session_manager.get(&sid).await else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "session not found"})),
            },
        )
        .await;
        return;
    };

    match handle
        .submit_and_subscribe(SessionOp::Compact, 16)
        .await
    {
        Ok((_sub_id, mut event_rx)) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id.clone(),
                    msg_type: "compact.started".into(),
                    data: Some(json!({"sessionId": session_id})),
                    error: None,
                },
            )
            .await;

            let bg_tx = bg_tx.clone();
            let rid = req_id;
            tokio::spawn(async move {
                while let Some(event) = event_rx.recv().await {
                    let resp = forward_event(&event.msg, &rid);
                    if bg_tx.send(resp).await.is_err() {
                        break;
                    }
                    if matches!(
                        event.msg,
                        xiaolin_protocol::AgentEvent::TurnEnd { .. }
                            | xiaolin_protocol::AgentEvent::Error { .. }
                            | xiaolin_protocol::AgentEvent::CompactBoundary { .. }
                    ) {
                        break;
                    }
                }
            });
        }
        Err(e) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"message": format!("compact failed: {e}")})),
                },
            )
            .await;
        }
    }
}

/// Injects mid-turn steering input into an active session turn.
pub async fn handle_chat_steer(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    session_id: &str,
    messages: Vec<xiaolin_protocol::ChatSteerMessage>,
) {
    let steer_messages: Vec<xiaolin_session_actor::turn::SteerMessage> = messages
        .into_iter()
        .map(|m| xiaolin_session_actor::turn::SteerMessage {
            role: m.role,
            content: m.content,
        })
        .collect();

    let mut ok = false;
    if let Some(handle) = state
        .svc
        .session_manager
        .get(&xiaolin_protocol::SessionId::new(session_id))
        .await
    {
        ok = handle
            .submit(SessionOp::SteerInput {
                messages: steer_messages,
            })
            .await
            .is_ok();
    }

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "steer.ok".into(),
            data: Some(json!({"sessionId": session_id, "ok": ok})),
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

    use xiaolin_core::types::ExecutionMode;
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
    active_chat_sessions: Arc<tokio::sync::Mutex<HashMap<String, String>>>,
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
    let active_chat_sessions_for_task = active_chat_sessions.clone();

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
                    let mut guard = active_chat_sessions_for_task.lock().await;
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
            response_language: params.response_language.clone(),
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
                    let mut guard = active_chat_sessions_for_task.lock().await;
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

        // Persist user messages to session
        for msg in &setup.user_messages {
            let _ = state
                .store
                .session_store
                .append_message(&session_id, msg)
                .await;
            // Dual-write: persist as HistoryItems alongside legacy messages
            {
                let turn_id = xiaolin_protocol::TurnId::generate();
                let history_items =
                    xiaolin_core::history_compat::chat_message_to_history(msg, turn_id);
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

        let after_turn_messages = setup.enriched_request.messages.clone();

        let typed_data = Some(xiaolin_core::typed_turn_data::TypedTurnData::wrap_with_llm_override(
            setup.enriched_request.clone(),
            agent_config.clone(),
            setup.llm_override.clone().map(|p| {
                std::sync::Arc::new(p) as std::sync::Arc<dyn std::any::Any + Send + Sync>
            }),
        ));

        let mut op_extra = serde_json::Map::new();
        op_extra.insert(
            "_stream_context_key".into(),
            serde_json::Value::String(stream_context_key.clone()),
        );

        // Get session handle and submit turn via session actor.
        let session_handle = state
            .svc
            .session_manager
            .get_or_create(
                xiaolin_protocol::SessionId::new(&session_id),
                &agent_id,
            )
            .await;

        // Register requestId → session_id for cancel routing.
        if let Some(ref rid_str) = rid {
            let mut guard = active_chat_sessions_for_task.lock().await;
            guard.insert(rid_str.clone(), session_id.clone());
        }

        let (sub_id, mut event_rx) = match session_handle
            .submit_and_subscribe(
                SessionOp::UserTurn {
                    messages: serde_json::Value::Array(vec![]),
                    agent_id: Some(agent_id.clone()),
                    model: setup.enriched_request.model.clone(),
                    work_dir: setup.enriched_request.work_dir.clone(),
                    extra: op_extra,
                    typed_data,
                },
                128,
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                let _ = bg_tx
                    .send(WsResponse {
                        id: rid,
                        msg_type: "error".into(),
                        data: None,
                        error: Some(json!({"message": format!("session error: {e}")})),
                    })
                    .await;
                if let Some(rid) = &rid_for_cleanup {
                    let mut guard = active_chat_sessions_for_task.lock().await;
                    guard.remove(rid);
                }
                return;
            }
        };
        let _sub_id = sub_id;

        // Forward stream events → WsResponse → bg_tx, collect assistant content.
        // On Done: persist assistant message BEFORE forwarding to client.
        let mut assistant_content = String::new();
        let mut stream_ended = false;
        let mut current_turn_id: Option<xiaolin_protocol::TurnId> = None;

        // Track tool calls during the stream so we can persist enriched data
        // (including display_output, metadata) alongside the assistant message.
        let mut tracked_tools: Vec<TrackedToolCallData> = Vec::new();

        let mode_at_start = state
            .rt
            .session_modes
            .get_or_create(&session_id)
            .current_mode();

        const MAX_CONTENT_BYTES: usize = 2 * 1024 * 1024; // 2MB safety cap
        let turn_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(600); // 10min

        loop {
            let event = match tokio::time::timeout_at(turn_deadline, event_rx.recv()).await {
                Ok(Some(se)) => se.msg,
                Ok(None) => break,
                Err(_elapsed) => {
                    tracing::error!(
                        session_id = %session_id,
                        content_len = assistant_content.len(),
                        "turn exceeded 10-minute deadline, forcing cancellation"
                    );
                    turn_cancel.cancel();
                    break;
                }
            };
            if let AgentEvent::TurnStart { turn_id, .. } = &event {
                current_turn_id = Some(turn_id.clone());
            }
            // Skip TurnAborted events from a previous turn that was replaced by
            // our submission. The subscriber is registered before the actor
            // processes the op, so it may receive abort events for the old turn.
            if let AgentEvent::TurnAborted { ref turn_id, .. } = event {
                if current_turn_id.as_ref().is_some_and(|id| id != turn_id) {
                    continue;
                }
                if current_turn_id.is_none() {
                    // We haven't received our TurnStart yet — this abort
                    // belongs to a prior turn; skip it.
                    continue;
                }
            }
            if turn_cancel.is_cancelled() {
                if reserved > 0.0 {
                    let _ = state.obs.budget_tracker.release_reservation(reserved);
                    reserved = 0.0;
                }
                if !assistant_content.is_empty() {
                    let assistant_msg = ChatMessage {
                        role: xiaolin_core::types::Role::Assistant,
                        content: Some(serde_json::Value::String(assistant_content.clone())),
                        enriched_tool_calls_json: build_enriched_tool_calls_json(&tracked_tools),
                        ..Default::default()
                    };
                    let _ = after_chat(&state, &setup, &assistant_msg, false).await;
                }
                break;
            }
            state.store.event_log.append(&session_id, &event);
            // Capture tool events for enriched persistence
            if let AgentEvent::ToolExecuting { ref call_id, ref tool_name, ref args, .. } = event {
                tracked_tools.push(TrackedToolCallData {
                    id: call_id.clone(),
                    name: tool_name.clone(),
                    args: args.clone(),
                    output: None,
                    display_output: None,
                    success: None,
                    metadata: None,
                    start_ms: chat_start.elapsed().as_millis() as u64,
                    duration_ms: None,
                });
            }
            if let AgentEvent::ToolResult { ref call_id, ref output, ref display_output, success, ref metadata, .. } = event {
                if let Some(tc) = tracked_tools.iter_mut().find(|t| t.id == *call_id) {
                    tc.output = Some(output.clone());
                    tc.display_output = display_output.clone();
                    tc.success = Some(success);
                    tc.metadata = metadata.clone();
                    tc.duration_ms = Some(chat_start.elapsed().as_millis() as u64 - tc.start_ms);

                    if success {
                        let should_refresh = match tc.name.as_str() {
                            "edit_file" | "write_file" | "create_file" | "apply_patch" | "str_replace_editor" => true,
                            "shell_exec" | "execute_command" => {
                                tc.args.as_deref()
                                    .and_then(|a| serde_json::from_str::<serde_json::Value>(a).ok())
                                    .map(|a| {
                                        let cmd = a.get("command").and_then(|v| v.as_str()).unwrap_or("");
                                        cmd.contains("git add") || cmd.contains("git commit")
                                            || cmd.contains("git checkout") || cmd.contains("git reset")
                                            || cmd.contains("git stash") || cmd.contains("git merge")
                                            || cmd.contains("git rebase") || cmd.contains("git rm")
                                    }).unwrap_or(false)
                            }
                            _ => false,
                        };
                        if should_refresh {
                            if let Some(ref wd) = setup.enriched_request.work_dir {
                                let mgr = state.strm.git_watcher_manager.clone();
                                let wd = std::path::PathBuf::from(wd.as_str());
                                let sid = setup.session_id.clone();
                                let store = state.store.session_store.clone();
                                tokio::spawn(async move {
                                    if let Ok(Some(session)) = store.get_session(&sid).await {
                                        if let Some(pid) = session.project_id {
                                            mgr.trigger_refresh(&pid, &wd).await;
                                        }
                                    }
                                });
                            }
                        }
                    }
                }
            }
            if matches!(&event, AgentEvent::Error { .. }) {
                if reserved > 0.0 {
                    let _ = state.obs.budget_tracker.release_reservation(reserved);
                    reserved = 0.0;
                }
                if !assistant_content.is_empty() {
                    let assistant_msg = ChatMessage {
                        role: xiaolin_core::types::Role::Assistant,
                        content: Some(serde_json::Value::String(assistant_content.clone())),
                        enriched_tool_calls_json: build_enriched_tool_calls_json(&tracked_tools),
                        ..Default::default()
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
                    if assistant_content.len() > MAX_CONTENT_BYTES {
                        tracing::error!(
                            session_id = %session_id,
                            content_len = assistant_content.len(),
                            "assistant content exceeded 2MB cap, forcing cancellation"
                        );
                        turn_cancel.cancel();
                        break;
                    }
                }
            }
            let is_done = matches!(&event, AgentEvent::TurnEnd { .. });
            if is_done || matches!(&event, AgentEvent::Error { .. }) {
                stream_ended = true;
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
                    role: xiaolin_core::types::Role::Assistant,
                    content: Some(serde_json::Value::String(assistant_content.clone())),
                    enriched_tool_calls_json: build_enriched_tool_calls_json(&tracked_tools),
                    ..Default::default()
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

                    let mode_now = state
                        .rt
                        .session_modes
                        .get_or_create(&session_id)
                        .current_mode();
                    if mode_now != mode_at_start {
                        tracing::info!(
                            session_id = %session_id,
                            from = ?mode_at_start,
                            to = ?mode_now,
                            "auto mode change detected — embedding in turn_end"
                        );
                        data.insert("modeChange".into(), json!({
                            "from": format!("{mode_at_start}"),
                            "to": format!("{mode_now}"),
                        }));
                    }
                }
            }
            if bg_tx.send(resp).await.is_err() {
                break;
            }
            if is_done {
                break;
            }
            if matches!(&event, AgentEvent::TurnAborted { .. } | AgentEvent::Error { .. }) {
                break;
            }
        }

        // Session actor emits TurnAborted when cancelled, so the event loop
        // above already forwards it. No need to synthesize one.

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

        // Release budget reservation if not consumed.
        if !stream_ended && reserved > 0.0 {
            let _ = state.obs.budget_tracker.release_reservation(reserved);
            if !assistant_content.is_empty() {
                let assistant_msg = ChatMessage {
                    role: xiaolin_core::types::Role::Assistant,
                    content: Some(serde_json::Value::String(assistant_content.clone())),
                    enriched_tool_calls_json: build_enriched_tool_calls_json(&tracked_tools),
                    ..Default::default()
                };
                let _ = after_chat(&state, &setup, &assistant_msg, false).await;
            }
            if !turn_cancel.is_cancelled() {
                let _ = bg_tx
                    .send(WsResponse {
                        id: rid,
                        msg_type: "error".into(),
                        data: None,
                        error: Some(json!({"message": "Chat completed without response. The model provider may be unavailable."})),
                    })
                    .await;
            }
        }

        if let Some(rid) = &rid_for_cleanup {
            let mut guard = active_chat_sessions_for_task.lock().await;
            guard.remove(rid);
        }
    });
}

/// Build enriched `tool_calls_json` string from tracked tool events.
/// Includes `display_output` and `metadata` which are UI-only fields
/// not present on the `ToolCall` struct. The extra fields are silently
/// ignored by `serde_json::from_str::<Vec<ToolCall>>` on the LLM load
/// path, keeping the LLM context clean.
fn build_enriched_tool_calls_json(
    tracked: &[TrackedToolCallData],
) -> Option<String> {
    if tracked.is_empty() {
        return None;
    }
    let arr: Vec<serde_json::Value> = tracked
        .iter()
        .map(|tc| {
            let mut obj = json!({
                "id": tc.id,
                "type": "function",
                "function": {
                    "name": tc.name,
                    "arguments": tc.args.as_deref().unwrap_or("")
                }
            });
            if let Some(ref output) = tc.output {
                obj["output"] = json!(output);
            }
            if let Some(ref display_output) = tc.display_output {
                obj["display_output"] = json!(display_output);
            }
            if let Some(success) = tc.success {
                obj["success"] = json!(success);
            }
            if let Some(duration_ms) = tc.duration_ms {
                obj["duration_ms"] = json!(duration_ms);
            }
            if let Some(ref metadata) = tc.metadata {
                obj["metadata"] = metadata.clone();
            }
            obj
        })
        .collect();
    Some(serde_json::to_string(&arr).unwrap_or_default())
}

/// Captured tool call data during a streaming turn.
struct TrackedToolCallData {
    id: String,
    name: String,
    args: Option<String>,
    output: Option<String>,
    display_output: Option<String>,
    success: Option<bool>,
    metadata: Option<serde_json::Value>,
    start_ms: u64,
    duration_ms: Option<u64>,
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
