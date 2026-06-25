use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub(super) struct RuntimeQualityQuery {
    pub session_id: Option<String>,
    pub limit: Option<i64>,
}

pub(super) async fn list_runtime_quality_turns(
    State(state): State<AppState>,
    Query(q): Query<RuntimeQualityQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(100);
    let result = if let Some(session_id) = q.session_id.as_deref() {
        state
            .store
            .runtime_quality_store
            .query_session(session_id, limit)
            .await
    } else {
        state.store.runtime_quality_store.query_recent(limit).await
    };

    match result {
        Ok(rows) => Json(json!({ "data": rows })).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub(super) async fn get_runtime_quality_turn(
    State(state): State<AppState>,
    Path((session_id, turn_id)): Path<(String, String)>,
) -> impl IntoResponse {
    match state
        .store
        .runtime_quality_store
        .get_turn(&session_id, &turn_id)
        .await
    {
        Ok(Some(row)) => Json(json!(row)).into_response(),
        Ok(None) => (
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({ "error": "runtime quality turn not found" })),
        )
            .into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub(super) async fn export_runtime_quality_turns(
    State(state): State<AppState>,
    Query(q): Query<RuntimeQualityQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(1000);
    let result = if let Some(session_id) = q.session_id.as_deref() {
        state
            .store
            .runtime_quality_store
            .query_session(session_id, limit)
            .await
    } else {
        state.store.runtime_quality_store.query_recent(limit).await
    };

    match result {
        Ok(rows) => Json(json!({
            "generatedAt": chrono::Utc::now().to_rfc3339(),
            "format": "xiaolin.runtime_quality.v1",
            "data": rows,
        }))
        .into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
