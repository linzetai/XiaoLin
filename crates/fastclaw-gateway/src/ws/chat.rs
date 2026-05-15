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
use fastclaw_agent::QueryEngine;
use fastclaw_core::types::{AgentId, ChatMessage, ChatRequest, StreamEvent};

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
            msg_type: "chat.cancel".into(),
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
            msg_type: "chat.answer".into(),
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
            msg_type: "chat.set_mode".into(),
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
                    msg_type: "chat.mode_change".into(),
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
    params: serde_json::Value,
) {
    let chat_start = Instant::now();
    if let Some(sid) = params.get("sessionId").and_then(|v| v.as_str()) {
        if !owned_sessions.contains(sid) {
            let _ = bg_tx
                .send(WsResponse {
                    id: req_id,
                    msg_type: "chat.error".into(),
                    data: None,
                    error: Some(
                        json!({"code": 403, "message": "session not owned by this connection"}),
                    ),
                })
                .await;
            return;
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

        let messages: Vec<ChatMessage> = match serde_json::from_value(
            params
                .get("messages")
                .cloned()
                .unwrap_or(serde_json::Value::Array(vec![])),
        ) {
            Ok(m) => m,
            Err(e) => {
                let _ = bg_tx
                    .send(WsResponse {
                        id: rid,
                        msg_type: "chat.error".into(),
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
            model: params
                .get("model")
                .and_then(|v| v.as_str())
                .map(String::from),
            temperature: params
                .get("temperature")
                .and_then(|v| v.as_f64())
                .map(|f| f as f32),
            max_tokens: params
                .get("maxTokens")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32),
            agent_id: params
                .get("agentId")
                .and_then(|v| v.as_str())
                .map(AgentId::from),
            session_id: params
                .get("sessionId")
                .and_then(|v| v.as_str())
                .map(String::from),
            tools: None,
            slash_intent: params
                .get("slashIntent")
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok()),
            work_dir: params
                .get("workDir")
                .and_then(|v| v.as_str())
                .map(String::from),
        };

        // #region agent log
        {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/home/linzetai/workspace/my_tools/FastClaw/.cursor/debug-a57040.log") {
                let _ = writeln!(f, r#"{{"sessionId":"a57040","hypothesisId":"E","location":"ws/chat.rs:spawn_chat:before_setup","message":"About to call setup_chat","data":{{"req_id":"{}","agent_id":"{}","model":"{}"}},"timestamp":{}}}"#,
                    rid.as_deref().unwrap_or("none"),
                    request.agent_id.as_ref().map(|a| a.as_str()).unwrap_or("none"),
                    request.model.as_deref().unwrap_or("none"),
                    chrono::Utc::now().timestamp_millis());
            }
        }
        // #endregion
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
            Ok(s) => {
                // #region agent log
                {
                    use std::io::Write;
                    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/home/linzetai/workspace/my_tools/FastClaw/.cursor/debug-a57040.log") {
                        let _ = writeln!(f, r#"{{"sessionId":"a57040","hypothesisId":"E","location":"ws/chat.rs:spawn_chat:setup_ok","message":"setup_chat succeeded","data":{{"agent_id":"{}","session_id":"{}","model_for_budget":"{}"}},"timestamp":{}}}"#,
                            s.agent_id, s.session_id, s.model_for_budget, chrono::Utc::now().timestamp_millis());
                    }
                }
                // #endregion
                s
            }
            Err(e) => {
                // #region agent log
                {
                    use std::io::Write;
                    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/home/linzetai/workspace/my_tools/FastClaw/.cursor/debug-a57040.log") {
                        let _ = writeln!(f, r#"{{"sessionId":"a57040","hypothesisId":"B","location":"ws/chat.rs:spawn_chat:setup_err","message":"setup_chat FAILED","data":{{"error":"{}"}},"timestamp":{}}}"#,
                            e, chrono::Utc::now().timestamp_millis());
                    }
                }
                // #endregion
                let _ = bg_tx
                    .send(WsResponse {
                        id: rid,
                        msg_type: "chat.error".into(),
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

        // Persist user messages to session
        for msg in &setup.user_messages {
            let _ = state
                .store
                .session_store
                .append_message(&session_id, msg)
                .await;
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
                msg_type: "chat.start".into(),
                data: Some(start_payload),
                error: None,
            })
            .await;

        let (tx, mut rx) = tokio::sync::mpsc::channel::<StreamEvent>(64);

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
        let plan_ctx_for_task = Some(fastclaw_agent::builtin_tools::PlanContext {
            session_id: session_id.clone(),
            store: state.rt.plan_file_store.clone(),
        });
        // #region agent log
        {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/home/linzetai/workspace/my_tools/FastClaw/.cursor/debug-a57040.log") {
                let _ = writeln!(f, r#"{{"sessionId":"a57040","hypothesisId":"C","location":"ws/chat.rs:spawn_chat:before_llm_spawn","message":"About to spawn LLM task","data":{{"model":"{}","msg_count":{}}},"timestamp":{}}}"#,
                    start_model, enriched.messages.len(), chrono::Utc::now().timestamp_millis());
            }
        }
        // #endregion
        let task = tokio::spawn(async move {
            let ms_clone = mode_state_for_task.clone();
            tokio::select! {
                result = fastclaw_agent::builtin_tools::with_stream_context(
                    stream_context_key_for_task.clone(),
                    fastclaw_agent::builtin_tools::with_session_mode(
                        ms_clone,
                        plan_ctx_for_task,
                        runtime.execute_stream_with_confirm(&cfg, &enriched, &tool_reg, tx, llm_for_task, confirm_pending_for_task, subagent_prompt, Some(mode_state_for_task), session_store_for_task, todo_store_for_task),
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
        let mut first_event_logged = false;
        while let Some(event) = rx.recv().await {
            // #region agent log
            if !first_event_logged {
                first_event_logged = true;
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/home/linzetai/workspace/my_tools/FastClaw/.cursor/debug-a57040.log") {
                    let event_type = match &event {
                        StreamEvent::Delta(_) => "Delta",
                        StreamEvent::Done { .. } => "Done",
                        StreamEvent::Error(_) => "Error",
                        StreamEvent::ToolExecuting { .. } => "ToolExecuting",
                        StreamEvent::ToolResult { .. } => "ToolResult",
                        StreamEvent::AskQuestion { .. } => "AskQuestion",
                        _ => "Other",
                    };
                    let _ = writeln!(f, r#"{{"sessionId":"a57040","hypothesisId":"C","location":"ws/chat.rs:spawn_chat:first_stream_event","message":"First stream event received from LLM","data":{{"event_type":"{}","elapsed_ms":{}}},"timestamp":{}}}"#,
                        event_type, chat_start.elapsed().as_millis(), chrono::Utc::now().timestamp_millis());
                }
            }
            // #endregion
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
            if matches!(&event, StreamEvent::Error(_)) {
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
            if let StreamEvent::Delta(ref delta) = event {
                if let Some(text) = delta
                    .choices
                    .first()
                    .and_then(|c| c.delta.content.as_deref())
                {
                    assistant_content.push_str(text);
                }
            }
            let is_done = matches!(&event, StreamEvent::Done { .. });
            if is_done || matches!(&event, StreamEvent::Error(_)) {
                stream_ended = true;
            }
            if let StreamEvent::AskQuestion { request_id, .. } = &event {
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
            if let StreamEvent::Done {
                ref usage,
                ref elapsed_ms,
                ..
            } = event
            {
                let wall_ms = chat_start.elapsed().as_millis() as u64;
                let pt = usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0);
                let ct = usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0);
                let tt = usage.as_ref().map(|u| u.total_tokens).unwrap_or(0);
                let ems = if wall_ms > 0 { wall_ms } else { *elapsed_ms };
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
            let mut resp = event_to_response(&event, &rid, &state, setup.context_tokens_estimate);
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

        match task.await {
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
                        msg_type: "chat.error".into(),
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
                        msg_type: "chat.error".into(),
                        data: None,
                        error: Some(json!({"message": format!("task panic: {e}")})),
                    })
                    .await;
            }
            Ok(Ok(_)) if !turn_cancel.is_cancelled() && !stream_ended => {
                // Task completed Ok but stream channel closed without sending
                // Done or Error. Send chat.error so TUI doesn't hang forever.
                if reserved > 0.0 {
                    let _ = state.obs.budget_tracker.release_reservation(reserved);
                }
                let _ = bg_tx
                    .send(WsResponse {
                        id: rid,
                        msg_type: "chat.error".into(),
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

/// Stateful chat submission via `QueryEngine`. Each session gets its own
/// `QueryEngine` instance that accumulates messages across turns. The engine
/// is automatically dropped when the WebSocket connection closes.
#[allow(clippy::too_many_arguments)]
pub async fn handle_chat_submit(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    query_engines: &mut HashMap<String, QueryEngine>,
    owned_sessions: &mut HashSet<String>,
    bg_tx: tokio::sync::mpsc::Sender<WsResponse>,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let Some(message_text) = params.get("message").and_then(|v| v.as_str()) else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "message (string) required"})),
            },
        )
        .await;
        return;
    };

    let session_key = params
        .get("sessionId")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();

    let agent_id_str = params
        .get("agentId")
        .and_then(|v| v.as_str())
        .unwrap_or("main");

    if !query_engines.contains_key(&session_key) {
        let lookup_req = ChatRequest {
            model: None,
            messages: vec![],
            agent_id: Some(AgentId::from(agent_id_str.to_string())),
            session_id: None,
            stream: false,
            temperature: None,
            max_tokens: None,
            tools: None,
            slash_intent: None,
            work_dir: None,
        };
        let agent_config = {
            let router = state.rt.router.read().await;
            match router.resolve(&lookup_req).cloned() {
                Ok(cfg) => cfg,
                Err(_) => {
                    send_resp(
                        sender,
                        &WsResponse {
                            id: req_id,
                            msg_type: "error".into(),
                            data: None,
                            error: Some(
                                json!({"code": 404, "message": format!("agent not found: {}", agent_id_str)}),
                            ),
                        },
                    )
                    .await;
                    return;
                }
            }
        };

        let engine = QueryEngine::new(
            state.rt.runtime.clone(),
            agent_config,
            state.rt.tool_registry.clone(),
        );
        query_engines.insert(session_key.clone(), engine);
    }

    let engine = query_engines.get_mut(&session_key).unwrap();
    let message_text = message_text.to_string();
    let rid = req_id.clone();

    let mut rx = engine.submit_message(&message_text).await;

    let rid_clone = rid.clone();
    let state_clone = state.clone();
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let is_done = matches!(&event, StreamEvent::Done { .. });
            let resp = event_to_response(&event, &rid_clone, &state_clone, None);
            if bg_tx.send(resp).await.is_err() {
                break;
            }
            if is_done {
                break;
            }
        }
    });

    owned_sessions.insert(session_key);
}

pub fn event_to_response(
    event: &StreamEvent,
    req_id: &Option<String>,
    state: &AppState,
    context_estimate: Option<(u32, u32)>,
) -> WsResponse {
    match event {
        StreamEvent::Delta(delta) => {
            let text = delta
                .choices
                .first()
                .and_then(|c| c.delta.content.as_deref());
            WsResponse {
                id: req_id.clone(),
                msg_type: "chat.delta".into(),
                data: Some(json!({"content": text, "model": delta.model})),
                error: None,
            }
        }
        StreamEvent::ToolExecuting {
            tool_name,
            call_id,
            args,
        } => WsResponse {
            id: req_id.clone(),
            msg_type: "chat.tool.start".into(),
            data: Some(json!({"tool": tool_name, "callId": call_id, "args": args})),
            error: None,
        },
        StreamEvent::ToolResult {
            tool_name,
            call_id,
            output,
            display_output,
            success,
            metadata,
        } => {
            let mut data = json!({"tool": tool_name, "callId": call_id, "output": display_output.as_ref().unwrap_or(output), "success": success});
            if let Some(meta) = metadata {
                data["metadata"] = meta.clone();
            }
            WsResponse {
                id: req_id.clone(),
                msg_type: "chat.tool.done".into(),
                data: Some(data),
                error: None,
            }
        }
        StreamEvent::ToolProgress {
            tool_name,
            call_id,
            message,
            progress,
            partial_output,
        } => WsResponse {
            id: req_id.clone(),
            msg_type: "chat.tool.progress".into(),
            data: Some(json!({
                "tool": tool_name,
                "callId": call_id,
                "message": message,
                "progress": progress,
                "partialOutput": partial_output,
            })),
            error: None,
        },
        StreamEvent::Done {
            session_id,
            tool_calls_made,
            iterations,
            usage,
            elapsed_ms,
            ..
        } => {
            let _ = state.strm.ws_broadcast.send(
                json!({"type":"event","event":"sessions.changed","data":{"sessionId":session_id}})
                    .to_string(),
            );
            let mut data = json!({"sessionId": session_id, "toolCallsMade": tool_calls_made, "iterations": iterations, "elapsedMs": elapsed_ms});
            if let Some(ref u) = usage {
                data["usage"] = json!({"promptTokens": u.prompt_tokens, "completionTokens": u.completion_tokens, "totalTokens": u.total_tokens});
            }
            if let Some((est_tokens, ctx_window)) = context_estimate {
                let actual_prompt = usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0);
                data["contextTokens"] = json!(if actual_prompt > 0 {
                    actual_prompt
                } else {
                    est_tokens
                });
                if ctx_window > 0 {
                    data["contextWindow"] = json!(ctx_window);
                }
            }
            WsResponse {
                id: req_id.clone(),
                msg_type: "chat.complete".into(),
                data: Some(data),
                error: None,
            }
        }
        StreamEvent::AskQuestion {
            request_id,
            question,
            options,
            timeout_secs,
            allow_multiple,
        } => WsResponse {
            id: req_id.clone(),
            msg_type: "chat.ask_question".into(),
            data: Some(json!({
                "requestId": request_id,
                "question": question,
                "options": options,
                "timeoutSecs": timeout_secs,
                "allowMultiple": allow_multiple,
            })),
            error: None,
        },
        StreamEvent::Error(msg) => WsResponse {
            id: req_id.clone(),
            msg_type: "chat.error".into(),
            data: None,
            error: Some(json!({"message": msg})),
        },

        // ── Sub-agent streaming events ──────────────────────────────
        StreamEvent::SubAgentStart {
            run_id,
            agent_id,
            subagent_type,
            task,
            depth,
        } => WsResponse {
            id: req_id.clone(),
            msg_type: "chat.subagent.start".into(),
            data: Some(json!({
                "runId": run_id, "agentId": agent_id,
                "subagentType": subagent_type, "task": task, "depth": depth,
            })),
            error: None,
        },
        StreamEvent::SubAgentDelta { run_id, content } => WsResponse {
            id: req_id.clone(),
            msg_type: "chat.subagent.delta".into(),
            data: Some(json!({"runId": run_id, "content": content})),
            error: None,
        },
        StreamEvent::SubAgentToolExecuting {
            run_id,
            tool_name,
            call_id,
            args,
        } => WsResponse {
            id: req_id.clone(),
            msg_type: "chat.subagent.tool.start".into(),
            data: Some(json!({
                "runId": run_id, "tool": tool_name, "callId": call_id, "args": args,
            })),
            error: None,
        },
        StreamEvent::SubAgentToolResult {
            run_id,
            tool_name,
            call_id,
            output,
            success,
        } => WsResponse {
            id: req_id.clone(),
            msg_type: "chat.subagent.tool.done".into(),
            data: Some(json!({
                "runId": run_id, "tool": tool_name, "callId": call_id,
                "output": output, "success": success,
            })),
            error: None,
        },
        StreamEvent::SubAgentComplete {
            run_id,
            status,
            result,
            tool_calls_made,
            iterations,
            usage,
            elapsed_ms,
        } => {
            let mut data = json!({
                "runId": run_id, "status": status, "result": result,
                "toolCallsMade": tool_calls_made, "iterations": iterations,
                "elapsedMs": elapsed_ms,
            });
            if let Some(ref u) = usage {
                data["usage"] = json!({
                    "promptTokens": u.prompt_tokens,
                    "completionTokens": u.completion_tokens,
                    "totalTokens": u.total_tokens,
                });
            }
            WsResponse {
                id: req_id.clone(),
                msg_type: "chat.subagent.complete".into(),
                data: Some(data),
                error: None,
            }
        }
        StreamEvent::ContextLimitWarning {
            used_tokens,
            limit_tokens,
            message,
        } => WsResponse {
            id: req_id.clone(),
            msg_type: "chat.context.warning".into(),
            data: Some(json!({
                "usedTokens": used_tokens,
                "limitTokens": limit_tokens,
                "message": message,
            })),
            error: None,
        },
        StreamEvent::CompactWarning {
            used_tokens,
            limit_tokens,
            message,
        } => WsResponse {
            id: req_id.clone(),
            msg_type: "chat.compact.warning".into(),
            data: Some(json!({
                "usedTokens": used_tokens,
                "limitTokens": limit_tokens,
                "message": message,
            })),
            error: None,
        },
        StreamEvent::ContextUsageUpdate {
            used_tokens,
            limit_tokens,
            compressed,
            tokens_saved,
        } => WsResponse {
            id: req_id.clone(),
            msg_type: "chat.context.usage".into(),
            data: Some(json!({
                "usedTokens": used_tokens,
                "limitTokens": limit_tokens,
                "compressed": compressed,
                "tokensSaved": tokens_saved,
            })),
            error: None,
        },
        StreamEvent::BriefMessage {
            content,
            attachments,
            mode,
        } => WsResponse {
            id: req_id.clone(),
            msg_type: "chat.brief".into(),
            data: Some(json!({
                "content": content,
                "attachments": attachments,
                "mode": mode,
            })),
            error: None,
        },
        StreamEvent::ModeChange { from, to } => WsResponse {
            id: req_id.clone(),
            msg_type: "chat.mode_change".into(),
            data: Some(json!({
                "from": format!("{from}"),
                "to": format!("{to}"),
            })),
            error: None,
        },
        StreamEvent::PlanFileUpdate {
            session_id,
            path,
            exists,
        } => WsResponse {
            id: req_id.clone(),
            msg_type: "chat.plan_file".into(),
            data: Some(json!({
                "sessionId": session_id,
                "path": path,
                "exists": exists,
            })),
            error: None,
        },
        StreamEvent::Suggestions { items } => WsResponse {
            id: req_id.clone(),
            msg_type: "chat.suggestions".into(),
            data: Some(json!({ "items": items })),
            error: None,
        },
        StreamEvent::CompactBoundary {
            trigger,
            pre_compact_tokens,
            post_compact_tokens,
            messages_removed,
        } => WsResponse {
            id: req_id.clone(),
            msg_type: "chat.compact.boundary".into(),
            data: Some(json!({
                "trigger": format!("{trigger:?}"),
                "preCompactTokens": pre_compact_tokens,
                "postCompactTokens": post_compact_tokens,
                "messagesRemoved": messages_removed,
            })),
            error: None,
        },
    }
}
