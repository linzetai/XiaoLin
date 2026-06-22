use axum::extract::ws::{Message, WebSocket};
use serde_json::json;

use crate::state::AppState;

use super::send_resp;
use super::types::WsResponse;

pub async fn handle_artifacts_list(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    session_id: &str,
) {
    match state.store.artifact_store.get_session_artifacts(session_id).await {
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
                    data: Some(json!([])),
                    error: None,
                },
            )
            .await;
        }
    }
}
