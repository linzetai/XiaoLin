use axum::extract::{Path, Query, State};
use axum::Extension;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use xiaolin_security::ApiKeyAuth;

use crate::state::AppState;

use super::error::AppError;

#[derive(Deserialize)]
pub(super) struct TracePagination {
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

fn default_limit() -> u32 {
    50
}

pub(super) async fn list_traces(
    State(state): State<AppState>,
    Extension(_auth): Extension<ApiKeyAuth>,
    Query(params): Query<TracePagination>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let traces = state
        .store
        .session_store
        .list_traces(params.limit, params.offset)
        .await?;
    Ok(Json(json!({ "traces": traces })))
}

pub(super) async fn get_trace(
    State(state): State<AppState>,
    Extension(_auth): Extension<ApiKeyAuth>,
    Path(trace_id): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let trace = state.store.session_store.get_trace(&trace_id).await?;
    match trace {
        Some(t) => Ok(Json(json!(t))),
        None => Err(AppError::NotFound(format!("trace {trace_id} not found"))),
    }
}

pub(super) async fn delete_trace(
    State(state): State<AppState>,
    Extension(_auth): Extension<ApiKeyAuth>,
    Path(trace_id): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let deleted = state.store.session_store.delete_trace(&trace_id).await?;
    Ok(Json(json!({ "deleted": deleted })))
}
