use axum::{
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use serde_json::json;

use crate::state::AppState;

pub(super) async fn serve_ui() -> impl IntoResponse {
    Json(json!({
        "name": "FastClaw",
        "description": "AI Agent Orchestration Engine",
        "docs": "/health"
    }))
}

pub(super) async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

pub(super) async fn readiness(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> impl IntoResponse {
    let agent_count = state.router.read().await.list_agents().len();
    let status = if agent_count > 0 {
        "ready"
    } else {
        "not_ready"
    };
    let code = if status == "ready" {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        code,
        Json(json!({ "status": status, "agents": agent_count })),
    )
}

pub(super) async fn auth_status(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> impl IntoResponse {
    let auth_required = !state.config.security.api_keys.is_empty();
    Json(json!({ "authRequired": auth_required }))
}

pub(super) async fn metrics_endpoint() -> impl IntoResponse {
    let body = fastclaw_observe::render_metrics();
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
}

/// In-memory structured metrics (Prometheus text); see [`fastclaw_observe::MetricsCollector`].
pub(super) async fn structured_metrics_v1() -> impl IntoResponse {
    let body = fastclaw_observe::render_structured_metrics_prometheus();
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
}
