use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Extension, Query, State,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

use crate::chat_pipeline::{
    after_chat, maybe_spawn_smart_title_background, setup_chat, SetupChatOptions,
};
use crate::state::AppState;
use fastclaw_core::config_access::{
    CONFIG_READABLE_KEYS, CONFIG_WRITABLE_KEYS, filter_config_for_read, navigate_config,
    set_nested_key,
};
use fastclaw_core::types::{ChatMessage, ChatRequest, StreamEvent};
use fastclaw_security::ApiKeyAuth;

static CONN_COUNTER: AtomicU64 = AtomicU64::new(0);

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(90);
const MAX_MESSAGE_SIZE: usize = 4 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WsRequest {
    #[serde(default)]
    id: Option<String>,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WsResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct WsQueryParams {
    #[serde(default)]
    token: Option<String>,
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Extension(auth): Extension<ApiKeyAuth>,
    Query(params): Query<WsQueryParams>,
) -> impl IntoResponse {
    let pre_authed = match &params.token {
        Some(token) => auth.validate_key(token),
        None => !auth.is_enabled(),
    };
    ws.max_message_size(MAX_MESSAGE_SIZE)
        .on_upgrade(move |socket| handle_socket(socket, state, auth, pre_authed))
}

async fn send_resp(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    resp: &WsResponse,
) -> bool {
    match serde_json::to_string(resp) {
        Ok(json) => sender.send(Message::Text(json.into())).await.is_ok(),
        Err(e) => {
            tracing::error!(error = %e, "failed to serialize WsResponse");
            false
        }
    }
}

async fn handle_socket(socket: WebSocket, state: AppState, auth: ApiKeyAuth, pre_authed: bool) {
    let conn_id = CONN_COUNTER.fetch_add(1, Ordering::SeqCst);
    fastclaw_observe::record_ws_connection(1);
    tracing::info!(conn_id, pre_authed, "websocket client connected");

    let (mut sender, mut receiver) = socket.split();
    let mut authenticated = pre_authed;
    let mut last_activity = Instant::now();
    let mut broadcast_rx = state.ws_broadcast.subscribe();
    let mut subscriptions: HashSet<String> = HashSet::new();
    let mut owned_sessions: HashSet<String> = HashSet::new();
    let cancel = CancellationToken::new();
    let active_chat_cancels: Arc<tokio::sync::Mutex<HashMap<String, CancellationToken>>> =
        Arc::new(tokio::sync::Mutex::new(HashMap::new()));

    // Channel for background tasks (chat streaming) to send responses back to the client.
    // This decouples streaming from the main loop so heartbeat/ping continue working.
    let (bg_tx, mut bg_rx) = tokio::sync::mpsc::channel::<WsResponse>(128);

    let auth_required = auth.is_enabled();
    send_resp(
        &mut sender,
        &WsResponse {
            id: None,
            msg_type: "connected".into(),
            data: Some(json!({
                "version": env!("CARGO_PKG_VERSION"),
                "connId": conn_id,
                "protocol": "fastclaw-ws/1",
                "methods": ["ping", "chat", "agents", "auth",
                            "sessions.list", "sessions.get", "sessions.messages", "sessions.delete",
                            "sessions.new", "sessions.claim", "sessions.update_title",
                            "chat.cancel", "chat.answer", "models.list", "config.get", "config.set",
                            "mcp.status", "mcp.reload", "mcp.add", "mcp.remove",
                            "subscribe", "unsubscribe"],
                "authRequired": auth_required && !authenticated,
            })),
            error: None,
        },
    )
    .await;

    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.tick().await;

    loop {
        tokio::select! {
            biased;

            // Background task responses (streaming chat events)
            Some(resp) = bg_rx.recv() => {
                if resp.msg_type == "chat.complete" {
                    if let Some(sid) = resp
                        .data
                        .as_ref()
                        .and_then(|d| d.get("sessionId"))
                        .and_then(|v| v.as_str())
                    {
                        owned_sessions.insert(sid.to_string());
                    }
                }
                last_activity = Instant::now();
                if !send_resp(&mut sender, &resp).await {
                    break;
                }
            }

            // Heartbeat tick
            _ = heartbeat.tick() => {
                if last_activity.elapsed() > CLIENT_TIMEOUT {
                    tracing::info!(conn_id, "client timeout, closing");
                    let _ = sender.send(Message::Close(Some(axum::extract::ws::CloseFrame {
                        code: 4000,
                        reason: "timeout".into(),
                    }))).await;
                    break;
                }
                send_resp(&mut sender, &WsResponse {
                    id: None, msg_type: "heartbeat".into(),
                    data: Some(json!({"ts": chrono::Utc::now().to_rfc3339()})),
                    error: None,
                }).await;
            }

            // Broadcast events for subscriptions (wildcard "*" not allowed for security)
            Ok(event_json) = broadcast_rx.recv() => {
                if let Ok(event) = serde_json::from_str::<serde_json::Value>(&event_json) {
                    let name = event.get("event").and_then(|v| v.as_str()).unwrap_or("");
                    if subscriptions.contains(name) {
                        let _ = sender.send(Message::Text(event_json.into())).await;
                    }
                }
            }

            // Client messages
            msg = receiver.next() => {
                let msg = match msg {
                    Some(Ok(msg)) => msg,
                    Some(Err(e)) => {
                        tracing::warn!(conn_id, error = %e, "websocket receive error");
                        break;
                    }
                    None => break,
                };

                last_activity = Instant::now();

                let text = match msg {
                    Message::Text(t) => t.to_string(),
                    Message::Close(_) => break,
                    Message::Ping(data) => { let _ = sender.send(Message::Pong(data)).await; continue; }
                    Message::Pong(_) => continue,
                    _ => continue,
                };

                let req: WsRequest = match serde_json::from_str(&text) {
                    Ok(r) => r,
                    Err(e) => {
                        send_resp(&mut sender, &WsResponse {
                            id: None, msg_type: "error".into(), data: None,
                            error: Some(json!({"code": -32700, "message": format!("parse error: {e}")})),
                        }).await;
                        continue;
                    }
                };

                // Auth method always available
                if req.method == "auth" {
                    let token = req.params.get("token").and_then(|v| v.as_str()).unwrap_or("");
                    if auth.validate_key(token) {
                        authenticated = true;
                        send_resp(&mut sender, &WsResponse {
                            id: req.id, msg_type: "auth.ok".into(),
                            data: Some(json!({"authenticated": true})), error: None,
                        }).await;
                    } else {
                        send_resp(&mut sender, &WsResponse {
                            id: req.id, msg_type: "auth.failed".into(), data: None,
                            error: Some(json!({"code": 401, "message": "invalid token"})),
                        }).await;
                    }
                    continue;
                }

                // Auth gate (ping always allowed)
                if auth_required && !authenticated {
                    if req.method == "ping" {
                        send_resp(&mut sender, &WsResponse {
                            id: req.id, msg_type: "pong".into(),
                            data: Some(json!({"ts": chrono::Utc::now().to_rfc3339()})),
                            error: None,
                        }).await;
                        continue;
                    }
                    send_resp(&mut sender, &WsResponse {
                        id: req.id, msg_type: "error".into(), data: None,
                        error: Some(json!({"code": 401, "message": "authentication required"})),
                    }).await;
                    continue;
                }

                dispatch(
                    &mut sender,
                    &state,
                    &mut subscriptions,
                    &mut owned_sessions,
                    &bg_tx,
                    &cancel,
                    active_chat_cancels.clone(),
                    req,
                )
                .await;
            }
        }
    }

    cancel.cancel();
    fastclaw_observe::record_ws_connection(-1);
    tracing::info!(conn_id, "websocket session ended");
}

async fn dispatch(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    subscriptions: &mut HashSet<String>,
    owned_sessions: &mut HashSet<String>,
    bg_tx: &tokio::sync::mpsc::Sender<WsResponse>,
    cancel: &CancellationToken,
    active_chat_cancels: Arc<tokio::sync::Mutex<HashMap<String, CancellationToken>>>,
    req: WsRequest,
) {
    let id = req.id;
    match req.method.as_str() {
        "ping" => {
            send_resp(
                sender,
                &WsResponse {
                    id,
                    msg_type: "pong".into(),
                    data: Some(json!({"ts": chrono::Utc::now().to_rfc3339()})),
                    error: None,
                },
            )
            .await;
        }
        "agents" => handle_agents(sender, state, id).await,
        "chat" => {
            spawn_chat(
                state,
                owned_sessions,
                bg_tx.clone(),
                cancel.clone(),
                active_chat_cancels.clone(),
                id,
                req.params,
            )
            .await
        }
        "chat.cancel" => {
            handle_chat_cancel(sender, id, req.params, active_chat_cancels.clone()).await
        }
        "chat.answer" => {
            handle_chat_answer(sender, state, id, req.params).await
        }
        "sessions.list" => handle_sessions_list(sender, state, id, req.params).await,
        "sessions.get" => handle_session_scoped(sender, state, owned_sessions, id, req.params, "get").await,
        "sessions.messages" => handle_session_scoped(sender, state, owned_sessions, id, req.params, "messages").await,
        "sessions.delete" => handle_session_scoped(sender, state, owned_sessions, id, req.params, "delete").await,
        "sessions.new" => handle_sessions_new(sender, state, owned_sessions, id, req.params).await,
        "sessions.claim" => handle_sessions_claim(sender, state, owned_sessions, id, req.params).await,
        "sessions.update_title" => handle_session_scoped(sender, state, owned_sessions, id, req.params, "update_title").await,
        "models.list" => handle_models_list(sender, state, id).await,
        "config.get" => handle_config_get(sender, state, id, req.params).await,
        "config.set" => handle_config_set(sender, state, id, req.params).await,
        "mcp.status" => handle_mcp_status(sender, state, id).await,
        "mcp.reload" => handle_mcp_reload(sender, state, id).await,
        "mcp.add" => handle_mcp_add(sender, state, id, req.params).await,
        "mcp.remove" => handle_mcp_remove(sender, state, id, req.params).await,
        "subscribe" => {
            let events: Vec<String> = req
                .params
                .get("events")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            for e in &events {
                subscriptions.insert(e.clone());
            }
            send_resp(
                sender,
                &WsResponse {
                    id,
                    msg_type: "subscribe.ok".into(),
                    data: Some(json!({"subscriptions": subscriptions.iter().collect::<Vec<_>>()})),
                    error: None,
                },
            )
            .await;
        }
        "unsubscribe" => {
            let events: Vec<String> = req
                .params
                .get("events")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            for e in &events {
                subscriptions.remove(e);
            }
            send_resp(
                sender,
                &WsResponse {
                    id,
                    msg_type: "unsubscribe.ok".into(),
                    data: Some(json!({"subscriptions": subscriptions.iter().collect::<Vec<_>>()})),
                    error: None,
                },
            )
            .await;
        }
        other => {
            send_resp(
                sender,
                &WsResponse {
                    id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(
                        json!({"code": -32601, "message": format!("unknown method: {other}")}),
                    ),
                },
            )
            .await;
        }
    }
}

async fn handle_chat_cancel(
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
async fn handle_chat_answer(
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

    let tx = {
        let mut pending = state.ask_question_pending.lock().await;
        pending.remove(request_id)
    };

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

/// Routes session-scoped operations through an ownership check.
async fn handle_session_scoped(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    owned_sessions: &HashSet<String>,
    req_id: Option<String>,
    params: serde_json::Value,
    op: &str,
) {
    let sid = params.get("sessionId").and_then(|v| v.as_str());
    if let Some(sid) = sid {
        if !owned_sessions.contains(sid) {
            tracing::warn!(
                session_id = %sid,
                operation = %op,
                owned_count = owned_sessions.len(),
                "ws: session access denied — not owned by this connection"
            );
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": 403, "message": "session not owned by this connection"})),
                },
            )
            .await;
            return;
        }
    }
    match op {
        "get" => handle_sessions_get(sender, state, req_id, params).await,
        "messages" => handle_sessions_messages(sender, state, req_id, params).await,
        "delete" => handle_sessions_delete(sender, state, req_id, params).await,
        "update_title" => handle_sessions_update_title(sender, state, req_id, params).await,
        _ => {}
    }
}

/// Spawns streaming chat on a background task that sends WsResponse messages
/// through `bg_tx`. Uses the same session/memory/compaction logic as the HTTP path.
/// Cancelled when the client disconnects (cancel token fires).
async fn spawn_chat(
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
                .map(String::from),
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

        let setup = match setup_chat(
            &state,
            &request,
            SetupChatOptions {
                chat_stream: true,
                propagate_context_ingest_errors: false,
                set_resolved_session_on_request: true,
                record_chat_observe: false,
                ..Default::default()
            },
        )
        .await
        {
            Ok(s) => s,
            Err(e) => {
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
            let _ = state.session_store.append_message(&session_id, msg).await;
        }

        let (mut reserved, budget_degraded) = (setup.reserved_cost, setup.budget_degraded);

        let start_model = setup
            .enriched_request
            .model
            .as_deref()
            .unwrap_or(agent_config.model.model.as_str());

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

        let runtime = state.runtime.clone();
        let tool_reg = state.tool_registry.clone();
        let cfg = agent_config;
        let enriched = setup.enriched_request.clone();
        let cancel2 = turn_cancel.clone();
        let llm_for_task = setup.llm_override.clone();
        {
            let mut tx_map = state.stream_event_tx.lock().await;
            tx_map.insert(stream_context_key.clone(), tx.clone());
        }
        let stream_context_key_for_task = stream_context_key.clone();
        let stream_event_map_for_task = state.stream_event_tx.clone();
        let confirm_pending_for_task = state.ask_question_pending.clone();

        let task = tokio::spawn(async move {
            tokio::select! {
                result = fastclaw_agent::builtin_tools::with_stream_context(
                    stream_context_key_for_task.clone(),
                    runtime.execute_stream_with_confirm(&cfg, &enriched, &tool_reg, tx, llm_for_task, confirm_pending_for_task),
                ) => result,
                _ = cancel2.cancelled() => Err(anyhow::anyhow!("cancelled")),
            }
        });

        // Forward stream events → WsResponse → bg_tx, collect assistant content.
        // On Done: persist assistant message BEFORE forwarding to client.
        let mut assistant_content = String::new();
        let mut pending_question_ids: Vec<String> = Vec::new();
        while let Some(event) = rx.recv().await {
            if turn_cancel.is_cancelled() {
                if reserved > 0.0 {
                    let _ = state.budget_tracker.release_reservation(reserved);
                    reserved = 0.0;
                }
                if !assistant_content.is_empty() {
                    let assistant_msg = ChatMessage {
                        role: fastclaw_core::types::Role::Assistant,
                        content: Some(serde_json::Value::String(assistant_content.clone())),
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
                    };
                    let _ = after_chat(&state, &setup, &assistant_msg, false).await;
                }
                break;
            }
            if matches!(&event, StreamEvent::Error(_)) {
                if reserved > 0.0 {
                    let _ = state.budget_tracker.release_reservation(reserved);
                    reserved = 0.0;
                }
                if !assistant_content.is_empty() {
                    let assistant_msg = ChatMessage {
                        role: fastclaw_core::types::Role::Assistant,
                        content: Some(serde_json::Value::String(assistant_content.clone())),
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
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
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                };
                let _ = after_chat(&state, &setup, &assistant_msg, false).await;
            }
            let mut resp = event_to_response(&event, &rid, &state);
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
                    let _ = state.budget_tracker.release_reservation(reserved);
                }
                if !assistant_content.is_empty() {
                    let assistant_msg = ChatMessage {
                        role: fastclaw_core::types::Role::Assistant,
                        content: Some(serde_json::Value::String(assistant_content.clone())),
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
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
                    let _ = state.budget_tracker.release_reservation(reserved);
                }
                if !assistant_content.is_empty() {
                    let assistant_msg = ChatMessage {
                        role: fastclaw_core::types::Role::Assistant,
                        content: Some(serde_json::Value::String(std::mem::take(&mut assistant_content))),
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
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
            _ => {}
        }
        if !pending_question_ids.is_empty() {
            let mut pending = state.ask_question_pending.lock().await;
            for request_id in pending_question_ids {
                pending.remove(&request_id);
            }
        }
        {
            let mut tx_map = stream_event_map_for_task.lock().await;
            tx_map.remove(&stream_context_key);
        }
        if let Some(rid) = &rid_for_cleanup {
            let mut guard = active_chat_cancels_for_task.lock().await;
            guard.remove(rid);
        }
    });
}

fn event_to_response(event: &StreamEvent, req_id: &Option<String>, state: &AppState) -> WsResponse {
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
        StreamEvent::ToolExecuting { tool_name, call_id, args } => WsResponse {
            id: req_id.clone(),
            msg_type: "chat.tool.start".into(),
            data: Some(json!({"tool": tool_name, "callId": call_id, "args": args})),
            error: None,
        },
        StreamEvent::ToolResult {
            tool_name,
            call_id,
            output,
            success,
        } => WsResponse {
            id: req_id.clone(),
            msg_type: "chat.tool.done".into(),
            data: Some(
                json!({"tool": tool_name, "callId": call_id, "output": output, "success": success}),
            ),
            error: None,
        },
        StreamEvent::Done {
            session_id,
            tool_calls_made,
            iterations,
            ..
        } => {
            let _ = state.ws_broadcast.send(
                json!({"type":"event","event":"sessions.changed","data":{"sessionId":session_id}})
                    .to_string(),
            );
            WsResponse {
                id: req_id.clone(),
                msg_type: "chat.complete".into(),
                data: Some(
                    json!({"sessionId": session_id, "toolCallsMade": tool_calls_made, "iterations": iterations}),
                ),
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
    }
}

async fn handle_agents(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
) {
    let agents: Vec<_> = state
        .router
        .read()
        .await
        .list_agents()
        .iter()
        .map(|a| json!({"agentId": a.agent_id, "name": a.name, "model": a.model.model}))
        .collect();
    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "agents".into(),
            data: Some(json!({"agents": agents})),
            error: None,
        },
    )
    .await;
}

async fn handle_sessions_list(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let limit = params
        .get("limit")
        .and_then(|v| v.as_i64())
        .unwrap_or(50)
        .clamp(1, 200);
    let offset = params
        .get("offset")
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        .max(0);
    match state.session_store.list_sessions(limit, offset).await {
        Ok(sessions) => {
            let count = sessions.len();
            let data: Vec<_> = sessions.iter().map(|s| json!({
                "id": s.id, "agentId": s.agent_id, "title": s.title,
                "messageCount": s.message_count, "createdAt": s.created_at, "updatedAt": s.updated_at,
            })).collect();
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "sessions.list".into(),
                    data: Some(json!({"sessions": data, "count": count})),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"message": format!("{e}")})),
                },
            )
            .await;
        }
    }
}

async fn handle_sessions_get(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let Some(sid) = params.get("sessionId").and_then(|v| v.as_str()) else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "sessionId required"})),
            },
        )
        .await;
        return;
    };
    match state.session_store.get_session(sid).await {
        Ok(Some(s)) => {
            send_resp(sender, &WsResponse {
                id: req_id, msg_type: "sessions.get".into(),
                data: Some(json!({
                    "id": s.id, "agentId": s.agent_id, "title": s.title,
                    "messageCount": s.message_count, "createdAt": s.created_at, "updatedAt": s.updated_at,
                })), error: None,
            }).await;
        }
        Ok(None) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": 404, "message": "session not found"})),
                },
            )
            .await;
        }
        Err(e) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"message": format!("{e}")})),
                },
            )
            .await;
        }
    }
}

async fn handle_sessions_messages(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let Some(sid) = params.get("sessionId").and_then(|v| v.as_str()) else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "sessionId required"})),
            },
        )
        .await;
        return;
    };
    match state.session_store.load_messages(sid).await {
        Ok(messages) => {
            let data: Vec<_> = messages
                .iter()
                .map(|m| {
                    json!({
                        "id": m.id,
                        "role": m.role,
                        "content": m.content.as_ref().and_then(|c| serde_json::from_str::<serde_json::Value>(c).ok()),
                        "name": m.name, "toolCallId": m.tool_call_id, "createdAt": m.created_at,
                    })
                })
                .collect();
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "sessions.messages".into(),
                    data: Some(json!({"messages": data})),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"message": format!("{e}")})),
                },
            )
            .await;
        }
    }
}

async fn handle_sessions_delete(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let Some(sid) = params.get("sessionId").and_then(|v| v.as_str()) else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "sessionId required"})),
            },
        )
        .await;
        return;
    };
    match state.session_store.delete_session(sid).await {
        Ok(deleted) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "sessions.delete".into(),
                    data: Some(json!({"deleted": deleted})),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"message": format!("{e}")})),
                },
            )
            .await;
        }
    }
}

/// Create a new (empty) session for the given agent and return its ID.
async fn handle_sessions_new(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    owned_sessions: &mut HashSet<String>,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let agent_id = params
        .get("agentId")
        .and_then(|v| v.as_str())
        .unwrap_or("main");
    let new_id = uuid::Uuid::new_v4().to_string();
    match state
        .session_store
        .create_session(&new_id, agent_id, None)
        .await
    {
        Ok(_) => {
            owned_sessions.insert(new_id.clone());
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "sessions.new".into(),
                    data: Some(json!({"sessionId": new_id, "agentId": agent_id})),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"message": format!("{e}")})),
                },
            )
            .await;
        }
    }
}

/// Claim an existing session so this connection can access it (resume flow).
/// Verifies the session exists in the DB before granting ownership.
async fn handle_sessions_claim(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    owned_sessions: &mut HashSet<String>,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let Some(sid) = params.get("sessionId").and_then(|v| v.as_str()) else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "sessionId required"})),
            },
        )
        .await;
        return;
    };
    match state.session_store.get_session(sid).await {
        Ok(Some(_)) => {
            owned_sessions.insert(sid.to_string());
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "sessions.claim".into(),
                    data: Some(json!({"sessionId": sid, "claimed": true})),
                    error: None,
                },
            )
            .await;
        }
        Ok(None) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": 404, "message": "session not found"})),
                },
            )
            .await;
        }
        Err(e) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"message": format!("{e}")})),
                },
            )
            .await;
        }
    }
}

async fn handle_sessions_update_title(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let Some(sid) = params.get("sessionId").and_then(|v| v.as_str()) else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "sessionId required"})),
            },
        )
        .await;
        return;
    };
    let Some(title) = params.get("title").and_then(|v| v.as_str()) else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "title required"})),
            },
        )
        .await;
        return;
    };
    match state.session_store.update_title(sid, title).await {
        Ok(updated) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "sessions.update_title".into(),
                    data: Some(json!({"updated": updated})),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"message": format!("{e}")})),
                },
            )
            .await;
        }
    }
}

async fn handle_models_list(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
) {
    let mut models: Vec<serde_json::Value> = Vec::new();
    let mut seen = std::collections::HashSet::<String>::new();
    let live = state
        .config_live
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| serde_json::to_value(&*state.config).unwrap_or_else(|_| json!({})));
    if let Some(models_obj) = live.get("models").and_then(|v| v.as_object()) {
        for (key, cfg) in models_obj {
            let model = cfg
                .get("model")
                .or_else(|| cfg.get("defaultModel"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            if model.is_empty() {
                continue;
            }
            let provider = key.clone();
            let dedupe_key = format!("{provider}::{model}");
            if !seen.insert(dedupe_key) {
                continue;
            }
            models.push(json!({
                "agentId": key,
                "model": model,
                "provider": provider,
                "contextWindow": cfg.get("contextWindow").cloned().unwrap_or(serde_json::Value::Null),
                "costPer1kInput": cfg.get("costPer1kInput").cloned().unwrap_or(serde_json::Value::Null),
                "costPer1kOutput": cfg.get("costPer1kOutput").cloned().unwrap_or(serde_json::Value::Null),
                "supportsReasoning": cfg.get("supportsReasoning").cloned().unwrap_or(serde_json::Value::Null),
            }));
        }
    }
    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "models.list".into(),
            data: Some(json!({"models": models})),
            error: None,
        },
    )
    .await;
}

// ---------- Config API: config.get / config.set ----------

async fn handle_config_get(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let key = params.get("key").and_then(|v| v.as_str()).unwrap_or("");
    let live = state
        .config_live
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| serde_json::to_value(&*state.config).unwrap_or_default());
    if key.is_empty() {
        let filtered = filter_config_for_read(&live);
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "config.get".into(),
                data: Some(filtered),
                error: None,
            },
        )
        .await;
        return;
    }

    let top_key = key.split('.').next().unwrap_or(key);
    if !CONFIG_READABLE_KEYS.contains(&top_key) {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({
                    "code": 403,
                    "message": format!("access denied: key '{}' is not readable", top_key)
                })),
            },
        )
        .await;
        return;
    }

    let value = navigate_config(&live, key);
    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "config.get".into(),
            data: Some(json!({ "key": key, "value": value })),
            error: None,
        },
    )
    .await;
}

async fn handle_config_set(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let key = match params.get("key").and_then(|v| v.as_str()) {
        Some(k) if !k.is_empty() => k,
        _ => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": -32602, "message": "key parameter required"})),
                },
            )
            .await;
            return;
        }
    };

    let value = match params.get("value") {
        Some(v) => v.clone(),
        None => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": -32602, "message": "value parameter required"})),
                },
            )
            .await;
            return;
        }
    };

    let top_key = key.split('.').next().unwrap_or(key);
    if !CONFIG_WRITABLE_KEYS.contains(&top_key) {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({
                    "code": 403,
                    "message": format!("access denied: key '{}' is read-only via WS", top_key)
                })),
            },
        )
        .await;
        return;
    }

    let mut cfg_value = state
        .config_live
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| serde_json::to_value(&*state.config).unwrap_or_default());
    if set_nested_key(&mut cfg_value, key, value.clone()).is_err() {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "failed to set nested key"})),
            },
        )
        .await;
        return;
    }

    match serde_json::from_value::<fastclaw_core::config::FastClawConfig>(cfg_value.clone()) {
        Ok(new_config) => {
            let persisted = persist_config_key(key, &value);
            let applied = persisted.is_ok();
            if let Err(ref e) = persisted {
                tracing::warn!(key, error = %e, "config.set: validated but failed to persist");
            } else {
                tracing::info!(key, "config.set persisted to user config");
                if let Ok(mut live) = state.config_live.write() {
                    *live = cfg_value;
                }
                if top_key == "security" {
                    fastclaw_security::ssrf::set_ssrf_allowed_hosts(
                        new_config.security.ssrf_allowed_hosts.clone(),
                    );
                    fastclaw_security::dangerous_ops::set_dangerous_ops_config(
                        new_config.security.dangerous_ops_policy,
                        &new_config.security.dangerous_patterns,
                    );
                    tracing::info!(
                        hosts = ?new_config.security.ssrf_allowed_hosts,
                        dangerous_ops_policy = ?new_config.security.dangerous_ops_policy,
                        "config.set: hot-reloaded security settings"
                    );
                }
            }
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "config.set".into(),
                    data: Some(json!({
                        "key": key,
                        "value": value,
                        "status": "validated",
                        "persisted": applied,
                        "pendingRestart": false,
                    })),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({
                        "code": -32602,
                        "message": format!("validation failed: {e}")
                    })),
                },
            )
            .await;
        }
    }
}

fn persist_config_key(key: &str, value: &serde_json::Value) -> anyhow::Result<()> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("cannot resolve home directory"))?;
    let cfg_path = home.join(".fastclaw/config/default.json");
    let mut cfg_value: serde_json::Value = if cfg_path.exists() {
        let text = std::fs::read_to_string(&cfg_path)?;
        json5::from_str(&text).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    set_nested_key(&mut cfg_value, key, value.clone())
        .map_err(|_| anyhow::anyhow!("failed to set nested key"))?;

    if let Some(parent) = cfg_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&cfg_path, serde_json::to_string_pretty(&cfg_value)?)?;
    Ok(())
}

// ─── MCP WS handlers ───

async fn handle_mcp_status(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
) {
    let status = state
        .mcp_status
        .read()
        .map(|g| g.values().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "mcp.status".into(),
            data: Some(json!({"servers": status})),
            error: None,
        },
    )
    .await;
}

async fn handle_mcp_reload(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
) {
    match state.reload_mcp_servers().await {
        Ok(()) => {
            let status = state
                .mcp_status
                .read()
                .map(|g| g.values().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "mcp.reload".into(),
                    data: Some(json!({"ok": true, "servers": status})),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"message": format!("{e}")})),
                },
            )
            .await;
        }
    }
}

async fn handle_mcp_add(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let id = match params.get("id").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => {
            send_resp(sender, &WsResponse {
                id: req_id, msg_type: "error".into(), data: None,
                error: Some(json!({"code": -32602, "message": "id required"})),
            }).await;
            return;
        }
    };
    let command = match params.get("command").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => {
            send_resp(sender, &WsResponse {
                id: req_id, msg_type: "error".into(), data: None,
                error: Some(json!({"code": -32602, "message": "command required"})),
            }).await;
            return;
        }
    };
    let args: Vec<String> = params
        .get("args")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let new_server = fastclaw_core::agent_config::McpServerConfig {
        id: id.clone(),
        command,
        args,
        enabled: Some(true),
        env: Default::default(),
    };

    {
        if let Ok(mut live) = state.config_live.write() {
            let server_val = serde_json::to_value(&new_server).unwrap_or_default();
            if let Some(arr) = live.get_mut("mcpServers").and_then(|v| v.as_array_mut()) {
                arr.retain(|v| v.get("id").and_then(|i| i.as_str()) != Some(&id));
                arr.push(server_val);
            } else {
                live["mcpServers"] = json!([server_val]);
            }
        }
    }

    if let Ok(live) = state.config_live.read() {
        let mcp_val = live.get("mcpServers").cloned().unwrap_or(json!([]));
        let _ = persist_config_key("mcpServers", &mcp_val);
    }

    match state.reload_mcp_servers().await {
        Ok(()) => {
            let status = state.mcp_status.read().ok().and_then(|g| g.get(&id).cloned());
            send_resp(sender, &WsResponse {
                id: req_id, msg_type: "mcp.add".into(),
                data: Some(json!({"ok": true, "id": id, "status": status})),
                error: None,
            }).await;
        }
        Err(e) => {
            send_resp(sender, &WsResponse {
                id: req_id, msg_type: "error".into(), data: None,
                error: Some(json!({"message": format!("{e}")})),
            }).await;
        }
    }
}

async fn handle_mcp_remove(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let id = match params.get("id").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => {
            send_resp(sender, &WsResponse {
                id: req_id, msg_type: "error".into(), data: None,
                error: Some(json!({"code": -32602, "message": "id required"})),
            }).await;
            return;
        }
    };

    if let Ok(mut live) = state.config_live.write() {
        if let Some(arr) = live.get_mut("mcpServers").and_then(|v| v.as_array_mut()) {
            arr.retain(|v| v.get("id").and_then(|i| i.as_str()) != Some(&id));
        }
    }

    if let Ok(live) = state.config_live.read() {
        let mcp_val = live.get("mcpServers").cloned().unwrap_or(json!([]));
        let _ = persist_config_key("mcpServers", &mcp_val);
    }

    match state.reload_mcp_servers().await {
        Ok(()) => {
            send_resp(sender, &WsResponse {
                id: req_id, msg_type: "mcp.remove".into(),
                data: Some(json!({"ok": true, "id": id})),
                error: None,
            }).await;
        }
        Err(e) => {
            send_resp(sender, &WsResponse {
                id: req_id, msg_type: "error".into(), data: None,
                error: Some(json!({"message": format!("{e}")})),
            }).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastclaw_core::config_access::mask_secret_values;

    #[test]
    fn set_nested_key_simple() {
        let mut root = json!({"a": {"b": 1}});
        set_nested_key(&mut root, "a.b", json!(2)).unwrap();
        assert_eq!(root["a"]["b"], 2);
    }

    #[test]
    fn set_nested_key_creates_intermediate() {
        let mut root = json!({});
        set_nested_key(&mut root, "x.y", json!("hello")).unwrap();
        assert_eq!(root["x"]["y"], "hello");
    }

    #[test]
    fn navigate_config_returns_nested_value() {
        let cfg = json!({"gateway": {"port": 18789}});
        let val = navigate_config(&cfg, "gateway.port");
        assert_eq!(val, 18789);
    }

    #[test]
    fn navigate_config_missing_key_returns_null() {
        let cfg = json!({"gateway": {}});
        let val = navigate_config(&cfg, "gateway.missing");
        assert!(val.is_null());
    }

    #[test]
    fn filter_config_only_includes_readable_keys() {
        let cfg = json!({
            "gateway": {"port": 18789},
            "logging": {"level": "info"},
            "dangerousInternal": "should not appear"
        });
        let filtered = filter_config_for_read(&cfg);
        assert!(filtered.get("gateway").is_some());
        assert!(filtered.get("logging").is_some());
        assert!(filtered.get("dangerousInternal").is_none());
    }

    #[test]
    fn mask_secret_values_masks_api_keys() {
        let val = json!({
            "openai": {
                "apiKey": "sk-1234567890abcdef",
                "baseUrl": "https://api.openai.com/v1"
            }
        });
        let masked = mask_secret_values(&val);
        let key = masked["openai"]["apiKey"].as_str().unwrap();
        assert!(key.contains("…"));
        assert!(!key.contains("1234567890"));
        assert_eq!(masked["openai"]["baseUrl"], "https://api.openai.com/v1");
    }

    #[test]
    fn mask_secret_values_short_key() {
        let val = json!({"apiKey": "short"});
        let masked = mask_secret_values(&val);
        assert_eq!(masked["apiKey"], "****");
    }

    #[test]
    fn mask_secret_values_empty_key_unchanged() {
        let val = json!({"apiKey": ""});
        let masked = mask_secret_values(&val);
        assert_eq!(masked["apiKey"], "");
    }

    #[test]
    fn config_writable_keys_include_web_search() {
        assert!(CONFIG_WRITABLE_KEYS.contains(&"webSearch"));
        assert!(CONFIG_WRITABLE_KEYS.contains(&"credentials"));
        assert!(CONFIG_WRITABLE_KEYS.contains(&"models"));
    }

    #[test]
    fn config_readable_keys_include_web_search_and_credentials() {
        assert!(CONFIG_READABLE_KEYS.contains(&"webSearch"));
        assert!(CONFIG_READABLE_KEYS.contains(&"credentials"));
        assert!(CONFIG_READABLE_KEYS.contains(&"modelRouter"));
    }

    #[test]
    fn event_to_response_ask_question_format() {
        use fastclaw_core::types::AskQuestionOption;
        let event = StreamEvent::AskQuestion {
            request_id: "q1".into(),
            question: "Pick one".into(),
            options: vec![
                AskQuestionOption { id: "a".into(), label: "Option A".into() },
                AskQuestionOption { id: "b".into(), label: "Option B".into() },
            ],
            timeout_secs: 30,
            allow_multiple: false,
        };
        let state = {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let tmp = tempfile::tempdir().unwrap();
            rt.block_on(async { AppState::for_test(Box::new(NullProvider), tmp.path()).await.unwrap() })
        };
        let resp = event_to_response(&event, &Some("r1".into()), &state);
        assert_eq!(resp.msg_type, "chat.ask_question");
        let data = resp.data.unwrap();
        assert_eq!(data["requestId"], "q1");
        assert_eq!(data["question"], "Pick one");
        assert_eq!(data["options"].as_array().unwrap().len(), 2);
        assert_eq!(data["timeoutSecs"], 30);
    }

    #[test]
    fn event_to_response_done_includes_session_id() {
        let event = StreamEvent::Done {
            session_id: Some("sess-123".into()),
            tool_calls_made: 2,
            iterations: 1,
            final_tool_calls: None,
        };
        let state = {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let tmp = tempfile::tempdir().unwrap();
            rt.block_on(async { AppState::for_test(Box::new(NullProvider), tmp.path()).await.unwrap() })
        };
        let resp = event_to_response(&event, &Some("r2".into()), &state);
        assert_eq!(resp.msg_type, "chat.complete");
        let data = resp.data.unwrap();
        assert_eq!(data["sessionId"], "sess-123");
        assert_eq!(data["toolCallsMade"], 2);
        assert_eq!(data["iterations"], 1);
    }

    struct NullProvider;
    #[async_trait::async_trait]
    impl fastclaw_agent::LlmProvider for NullProvider {
        async fn chat_completion(
            &self,
            _params: &fastclaw_agent::CompletionParams<'_>,
        ) -> anyhow::Result<fastclaw_core::types::ChatResponse> {
            Ok(fastclaw_core::types::ChatResponse {
                id: "null".into(),
                object: "chat.completion".into(),
                created: 0,
                model: "null".into(),
                choices: vec![],
                usage: None,
            })
        }
        async fn chat_completion_stream(
            &self,
            _params: &fastclaw_agent::CompletionParams<'_>,
        ) -> anyhow::Result<futures::stream::BoxStream<'static, anyhow::Result<fastclaw_core::types::StreamDelta>>> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }
}
