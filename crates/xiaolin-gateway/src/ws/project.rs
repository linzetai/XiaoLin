use axum::extract::ws::{Message, WebSocket};
use serde_json::json;
use std::path::Path;

use crate::state::AppState;
use xiaolin_session::ProjectPatch;

use super::send_resp;
use super::types::WsResponse;

pub async fn handle_projects_list(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    include_archived: Option<bool>,
) {
    let include = include_archived.unwrap_or(false);
    match state.store.session_store.list_projects(include).await {
        Ok(projects) => {
            let mut result = Vec::with_capacity(projects.len());
            for p in projects {
                let reachable = Path::new(&p.root_path).exists();
                let session_count = state
                    .store
                    .session_store
                    .count_sessions_for_project(&p.id)
                    .await
                    .unwrap_or(0);
                result.push(json!({
                    "id": p.id,
                    "name": p.name,
                    "rootPath": p.root_path,
                    "color": p.color,
                    "pinned": p.pinned != 0,
                    "archived": p.archived != 0,
                    "reachable": reachable,
                    "lastOpenedAt": p.last_opened_at,
                    "sessionCount": session_count,
                }));
            }
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "projects.list".into(),
                    data: Some(json!({ "projects": result })),
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
                    error: Some(json!({"code": -32000, "message": format!("{e}")})),
                },
            )
            .await;
        }
    }
}

pub async fn handle_projects_create(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    root_path: &str,
    name: Option<&str>,
    color: Option<&str>,
) {
    match state
        .store
        .session_store
        .create_project(root_path, name, color)
        .await
    {
        Ok(p) => {
            let reachable = Path::new(&p.root_path).exists();
            let resp_data = json!({
                "id": p.id,
                "name": p.name,
                "rootPath": p.root_path,
                "color": p.color,
                "pinned": p.pinned != 0,
                "archived": p.archived != 0,
                "reachable": reachable,
                "lastOpenedAt": p.last_opened_at,
                "sessionCount": 0,
            });
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "projects.create".into(),
                    data: Some(resp_data),
                    error: None,
                },
            )
            .await;

            broadcast_projects_changed(state, &p.id, "created").await;
        }
        Err(e) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": -32000, "message": format!("{e}")})),
                },
            )
            .await;
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn handle_projects_update(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    id: &str,
    name: Option<String>,
    color: Option<String>,
    pinned: Option<bool>,
    archived: Option<bool>,
) {
    let patch = ProjectPatch {
        name,
        color,
        pinned,
        archived,
    };
    match state.store.session_store.update_project(id, &patch).await {
        Ok(()) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "projects.update".into(),
                    data: Some(json!({"id": id, "success": true})),
                    error: None,
                },
            )
            .await;

            broadcast_projects_changed(state, id, "updated").await;
        }
        Err(e) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": -32000, "message": format!("{e}")})),
                },
            )
            .await;
        }
    }
}

pub async fn handle_projects_delete(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    id: &str,
) {
    match state.store.session_store.delete_project(id).await {
        Ok(()) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "projects.delete".into(),
                    data: Some(json!({"id": id, "success": true})),
                    error: None,
                },
            )
            .await;

            broadcast_projects_changed(state, id, "deleted").await;
        }
        Err(e) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": -32000, "message": format!("{e}")})),
                },
            )
            .await;
        }
    }
}

pub async fn handle_projects_detect(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    _state: &AppState,
    req_id: Option<String>,
    path: &str,
) {
    use xiaolin_core::workspace::detect_workspace_root;

    let root = detect_workspace_root(Path::new(path));
    let root_str = root.to_string_lossy().to_string();
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unnamed".to_string());

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "projects.detect".into(),
            data: Some(json!({
                "rootPath": root_str,
                "name": name,
                "hints": [],
            })),
            error: None,
        },
    )
    .await;
}

async fn broadcast_projects_changed(state: &AppState, project_id: &str, action: &str) {
    let event = json!({
        "type": "event",
        "event": "projects.changed",
        "data": {
            "projectId": project_id,
            "action": action,
        }
    });
    let _ = state
        .strm
        .ws_broadcast
        .send(serde_json::to_string(&event).unwrap_or_default());
}
