use axum::extract::ws::{Message, WebSocket};
use serde_json::json;

use crate::state::AppState;

use super::send_resp;
use super::types::WsResponse;

pub async fn handle_agents(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
) {
    let agents: Vec<_> = state
        .rt
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
