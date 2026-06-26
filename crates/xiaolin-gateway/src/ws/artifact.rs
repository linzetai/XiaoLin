use axum::extract::ws::{Message, WebSocket};
use serde_json::json;
use std::collections::HashSet;

use crate::state::AppState;

use super::send_resp;
use super::types::WsResponse;

pub async fn handle_artifacts_list(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    owned_sessions: &mut HashSet<String>,
    req_id: Option<String>,
    session_id: &str,
) {
    if !owned_sessions.contains(session_id) {
        match state.store.session_store.get_session(session_id).await {
            Ok(Some(_)) => {
                owned_sessions.insert(session_id.to_string());
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
                tracing::warn!(session_id = %session_id, error = %e, "failed to verify session for artifacts.list");
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

    match state
        .store
        .artifact_store
        .get_session_artifacts(session_id)
        .await
    {
        Ok(artifacts) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "artifacts.list".into(),
                    data: Some(json!(artifacts)),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                session_id = %session_id,
                "failed to list file artifacts"
            );
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "artifacts.list".into(),
                    data: None,
                    error: Some(json!({"code": 500, "message": "failed to load artifacts"})),
                },
            )
            .await;
        }
    }
}
