use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use fastclaw_core::config::BindingMatch;
use fastclaw_core::routing::RuntimeRouteBinding;

use crate::extract::AppJson;
use crate::state::AppState;

use super::error::AppError;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertRouteBody {
    pub agent_id: String,
    #[serde(rename = "match")]
    pub match_rule: BindingMatch,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RouteRow {
    id: String,
    agent_id: String,
    #[serde(rename = "match")]
    match_rule: BindingMatch,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RouteListResponse {
    routes: Vec<RouteRow>,
}

pub(super) async fn list_routes(State(state): State<AppState>) -> impl IntoResponse {
    let rows = {
        let rt = state.runtime_route_bindings.read().await;
        rt.iter()
            .map(|r| RouteRow {
                id: r.id.clone(),
                agent_id: r.binding.agent_id.clone(),
                match_rule: r.binding.match_rule.clone(),
            })
            .collect::<Vec<_>>()
    };
    Json(RouteListResponse { routes: rows })
}

pub(super) async fn add_route(
    State(state): State<AppState>,
    AppJson(body): AppJson<UpsertRouteBody>,
) -> Result<impl IntoResponse, AppError> {
    let id = Uuid::new_v4().to_string();
    let binding = fastclaw_core::config::BindingConfig {
        agent_id: body.agent_id,
        match_rule: body.match_rule,
    };
    let entry = RuntimeRouteBinding {
        id: id.clone(),
        binding,
    };
    let row = RouteRow {
        id: entry.id.clone(),
        agent_id: entry.binding.agent_id.clone(),
        match_rule: entry.binding.match_rule.clone(),
    };
    let mut rt = state.runtime_route_bindings.write().await;
    rt.push(entry);
    Ok(Json(row))
}

pub(super) async fn delete_route(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let mut rt = state.runtime_route_bindings.write().await;
    let pos = rt
        .iter()
        .position(|r| r.id == id)
        .ok_or_else(|| AppError::NotFound(format!("route binding not found: {id}")))?;
    rt.remove(pos);
    Ok(Json(serde_json::json!({ "deleted": true, "id": id })))
}

pub(super) async fn update_route(
    State(state): State<AppState>,
    Path(id): Path<String>,
    AppJson(body): AppJson<UpsertRouteBody>,
) -> Result<impl IntoResponse, AppError> {
    let mut rt = state.runtime_route_bindings.write().await;
    let entry = rt
        .iter_mut()
        .find(|r| r.id == id)
        .ok_or_else(|| AppError::NotFound(format!("route binding not found: {id}")))?;
    entry.binding.agent_id = body.agent_id;
    entry.binding.match_rule = body.match_rule;
    let row = RouteRow {
        id: entry.id.clone(),
        agent_id: entry.binding.agent_id.clone(),
        match_rule: entry.binding.match_rule.clone(),
    };
    Ok(Json(row))
}
