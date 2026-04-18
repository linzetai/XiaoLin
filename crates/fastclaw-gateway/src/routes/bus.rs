use axum::{extract::State, Json};
use serde_json::json;

use crate::extract::AppJson;
use crate::state::AppState;

use super::error::AppError;

pub(super) async fn bus_list_agents(
    State(state): State<AppState>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let agents = state.message_bus.registered_agents().await;
    Ok(Json(json!({
        "registered_agents": agents,
        "count": agents.len(),
    })))
}

pub(super) async fn bus_send_message(
    State(state): State<AppState>,
    AppJson(msg): AppJson<fastclaw_core::bus::AgentMessage>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    state
        .message_bus
        .send(msg)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;
    Ok(Json(json!({ "sent": true })))
}

#[derive(serde::Deserialize)]
pub(super) struct BusRequestBody {
    pub message: fastclaw_core::bus::AgentMessage,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    30_000
}

pub(super) async fn bus_request_reply(
    State(state): State<AppState>,
    AppJson(body): AppJson<BusRequestBody>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let timeout = std::time::Duration::from_millis(body.timeout_ms);
    let reply = state
        .message_bus
        .request(body.message, timeout)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;
    Ok(Json(json!({ "reply": reply })))
}
