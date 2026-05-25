use axum::extract::ws::{Message, WebSocket};
use serde_json::json;
use std::collections::HashSet;

use crate::state::AppState;
use fastclaw_protocol::{SessionsListParams, SessionsNewParams};

use super::send_resp;
use super::types::WsResponse;

pub async fn handle_sessions_list(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: SessionsListParams,
) {
    let limit = params
        .limit
        .unwrap_or(50)
        .clamp(1, 200) as i64;
    let offset = params.offset.unwrap_or(0) as i64;
    match state.store.session_store.list_sessions(limit, offset).await {
        Ok(sessions) => {
            let count = sessions.len();
            let data: Vec<_> = sessions.iter().map(|s| json!({
                "id": s.id, "agentId": s.agent_id, "title": s.title,
                "messageCount": s.message_count, "createdAt": s.created_at, "updatedAt": s.updated_at,
                "totalPromptTokens": s.total_prompt_tokens,
                "totalCompletionTokens": s.total_completion_tokens,
                "totalElapsedMs": s.total_elapsed_ms,
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

pub async fn handle_sessions_get(
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
    match state.store.session_store.get_session(sid).await {
        Ok(Some(s)) => {
            send_resp(sender, &WsResponse {
                id: req_id, msg_type: "sessions.get".into(),
                data: Some(json!({
                    "id": s.id, "agentId": s.agent_id, "title": s.title,
                    "messageCount": s.message_count, "createdAt": s.created_at, "updatedAt": s.updated_at,
                    "totalPromptTokens": s.total_prompt_tokens,
                    "totalCompletionTokens": s.total_completion_tokens,
                    "totalElapsedMs": s.total_elapsed_ms,
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

pub async fn handle_sessions_messages(
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
    match state.store.session_store.load_messages(sid).await {
        Ok(messages) => {
            let data: Vec<_> = messages
                .iter()
                .map(|m| {
                    let mut obj = json!({
                        "id": m.id,
                        "role": m.role,
                        "content": m.content.as_ref().and_then(|c| serde_json::from_str::<serde_json::Value>(c).ok()),
                        "name": m.name, "toolCallId": m.tool_call_id, "createdAt": m.created_at,
                        "toolCallsJson": m.tool_calls_json.as_ref().and_then(|tc| serde_json::from_str::<serde_json::Value>(tc).ok()),
                    });
                    if m.prompt_tokens > 0 || m.completion_tokens > 0 || m.elapsed_ms > 0 {
                        obj["promptTokens"] = json!(m.prompt_tokens);
                        obj["completionTokens"] = json!(m.completion_tokens);
                        obj["totalTokens"] = json!(m.total_tokens);
                        obj["elapsedMs"] = json!(m.elapsed_ms);
                    }
                    obj
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

pub async fn handle_sessions_delete(
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
    match state.store.session_store.delete_session(sid).await {
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
pub async fn handle_sessions_new(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    owned_sessions: &mut HashSet<String>,
    req_id: Option<String>,
    params: SessionsNewParams,
) {
    let agent_id = params
        .agent_id
        .as_ref()
        .map(|a| a.as_str())
        .unwrap_or("main");
    let new_id = uuid::Uuid::new_v4().to_string();
    let work_dir = state
        .rt
        .workspaces
        .get(agent_id)
        .map(|ws| ws.root.to_string_lossy().to_string());
    match state
        .store
        .session_store
        .create_session_with_work_dir(&new_id, agent_id, None, work_dir.as_deref())
        .await
    {
        Ok(_) => {
            owned_sessions.insert(new_id.clone());
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "sessions.new".into(),
                    data: Some(
                        json!({"sessionId": new_id, "agentId": agent_id, "workDir": work_dir}),
                    ),
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
pub async fn handle_sessions_claim(
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
    match state.store.session_store.get_session(sid).await {
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

pub async fn handle_sessions_update_title(
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
    match state.store.session_store.update_title(sid, title).await {
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
/// Routes session-scoped operations, auto-claiming ownership when needed.
///
/// Desktop clients reconnect across app restarts, so sessions created by a
/// previous WebSocket connection must be accessible.  Instead of rejecting
/// with 403, we verify the session exists in the DB and adopt it — the same
/// strategy `spawn_chat` already uses for the "chat" command.
pub async fn handle_session_scoped(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    owned_sessions: &mut HashSet<String>,
    req_id: Option<String>,
    params: serde_json::Value,
    op: &str,
) {
    let sid = params.get("sessionId").and_then(|v| v.as_str());
    if let Some(sid) = sid {
        if !owned_sessions.contains(sid) {
            match state.store.session_store.get_session(sid).await {
                Ok(Some(_)) => {
                    owned_sessions.insert(sid.to_string());
                    tracing::debug!(session_id = %sid, operation = %op, "auto-claimed session");
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
                    return;
                }
                Err(e) => {
                    tracing::warn!(session_id = %sid, error = %e, "failed to verify session for auto-claim");
                    send_resp(
                        sender,
                        &WsResponse {
                            id: req_id,
                            msg_type: "error".into(),
                            data: None,
                            error: Some(json!({"code": 500, "message": "internal error"})),
                        },
                    )
                    .await;
                    return;
                }
            }
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
