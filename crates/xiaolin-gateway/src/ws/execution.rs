use axum::extract::ws::{Message, WebSocket};
use serde_json::json;

use crate::state::AppState;

use super::send_resp;
use super::types::WsResponse;

/// Set execution mode for a session (agent vs plan mode).
pub async fn handle_execution_set_mode(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
    bg_tx: Option<&tokio::sync::mpsc::Sender<WsResponse>>,
) {
    let Some(mode_str) = params.get("mode").and_then(|v| v.as_str()) else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "mode required ('agent' or 'plan')"})),
            },
        )
        .await;
        return;
    };

    let session_id = params
        .get("sessionId")
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

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "execution.set_mode".into(),
            data: Some(json!({"ok": true, "from": format!("{from}"), "to": format!("{to}")})),
            error: None,
        },
    )
    .await;

    if from != to {
        if let Some(tx) = bg_tx {
            let _ = tx
                .send(WsResponse {
                    id: None,
                    msg_type: "mode_change".into(),
                    data: Some(json!({
                        "sessionId": session_id,
                        "from": format!("{from}"),
                        "to": format!("{to}"),
                    })),
                    error: None,
                })
                .await;

            let plan_store = &state.rt.plan_file_store;
            let plan_path = plan_store.plan_path(session_id);
            let plan_exists = plan_path.exists();
            let _ = tx
                .send(WsResponse {
                    id: None,
                    msg_type: "plan_file_update".into(),
                    data: Some(json!({
                        "sessionId": session_id,
                        "path": plan_path.to_string_lossy().to_string(),
                        "exists": plan_exists,
                    })),
                    error: None,
                })
                .await;
        }
    }
}

/// Approve plan: transition mode and broadcast events.
pub async fn handle_execution_approve_plan(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
    bg_tx: &tokio::sync::mpsc::Sender<WsResponse>,
) {
    let session_id = params
        .get("sessionId")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let mode_str = params
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("agent");

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

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "execution.approve_plan".into(),
            data: Some(json!({"ok": true, "from": format!("{from}"), "to": format!("{to}")})),
            error: None,
        },
    )
    .await;

    if from != to {
        let _ = bg_tx
            .send(WsResponse {
                id: None,
                msg_type: "mode_change".into(),
                data: Some(json!({
                    "sessionId": session_id,
                    "from": format!("{from}"),
                    "to": format!("{to}"),
                })),
                error: None,
            })
            .await;

        let plan_store = &state.rt.plan_file_store;
        let plan_path = plan_store.plan_path(session_id);
        let plan_exists = plan_path.exists();
        let _ = bg_tx
            .send(WsResponse {
                id: None,
                msg_type: "plan_file_update".into(),
                data: Some(json!({
                    "sessionId": session_id,
                    "path": plan_path.to_string_lossy().to_string(),
                    "exists": plan_exists,
                })),
                error: None,
            })
            .await;
    }
}

/// Get plan file content for a session.
pub async fn handle_execution_get_plan(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let session_id = params
        .get("sessionId")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let plan_store = &state.rt.plan_file_store;
    let path = plan_store.plan_path(session_id);
    let exists = path.exists();
    let content = if exists {
        std::fs::read_to_string(&path).ok()
    } else {
        None
    };

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "execution.get_plan".into(),
            data: Some(json!({
                "path": path.to_string_lossy().to_string(),
                "content": content,
                "exists": exists,
            })),
            error: None,
        },
    )
    .await;
}