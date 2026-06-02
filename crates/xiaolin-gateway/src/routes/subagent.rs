use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentRunResponse {
    run_id: String,
    parent_session_id: String,
    agent_id: String,
    subagent_type: String,
    task: String,
    status: String,
    result: Option<String>,
    tool_calls_made: u32,
    iterations: u32,
    depth: u32,
    elapsed_ms: Option<u64>,
    created_at: String,
    completed_at: Option<String>,
}

impl From<xiaolin_session::SubAgentRunRow> for SubAgentRunResponse {
    fn from(r: xiaolin_session::SubAgentRunRow) -> Self {
        Self {
            run_id: r.run_id,
            parent_session_id: r.parent_session_id,
            agent_id: r.agent_id,
            subagent_type: r.subagent_type,
            task: r.task,
            status: r.status,
            result: r.result,
            tool_calls_made: r.tool_calls_made as u32,
            iterations: r.iterations as u32,
            depth: r.depth as u32,
            elapsed_ms: r.elapsed_ms.map(|v| v as u64),
            created_at: r.created_at,
            completed_at: r.completed_at,
        }
    }
}

impl From<xiaolin_core::types::SubAgentRun> for SubAgentRunResponse {
    fn from(r: xiaolin_core::types::SubAgentRun) -> Self {
        let status = match &r.status {
            xiaolin_core::types::SubAgentStatus::Pending => "pending",
            xiaolin_core::types::SubAgentStatus::Running => "running",
            xiaolin_core::types::SubAgentStatus::Completed => "completed",
            xiaolin_core::types::SubAgentStatus::Failed(_) => "failed",
            xiaolin_core::types::SubAgentStatus::Cancelled => "cancelled",
        };
        Self {
            run_id: r.run_id,
            parent_session_id: r.parent_session_id,
            agent_id: r.agent_id.to_string(),
            subagent_type: r.subagent_type.to_string(),
            task: r.task,
            status: status.to_string(),
            result: r.result,
            tool_calls_made: r.tool_calls_made,
            iterations: r.iterations,
            depth: r.depth,
            elapsed_ms: r.elapsed_ms,
            created_at: String::new(),
            completed_at: None,
        }
    }
}

/// Serialize a `SubAgentDef` to a JSON value for API responses.
pub fn subagent_def_to_json(d: &xiaolin_core::agent_config::SubAgentDef) -> serde_json::Value {
    serde_json::json!({
        "id": d.id,
        "name": d.name,
        "description": d.description,
        "background": d.background,
        "concurrency_safe": d.concurrency_safe,
        "tools": {
            "allowed": d.tools.allowed,
            "denied": d.tools.denied,
        },
        "source": format!("{:?}", d.source),
    })
}

/// GET /api/v1/subagents/defs — list all sub-agent definitions
pub async fn list_subagent_defs(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let defs = state.strm.subagent_manager.subagent_defs();
    let agents: Vec<serde_json::Value> = defs.iter().map(subagent_def_to_json).collect();
    Json(serde_json::json!({ "agents": agents }))
}

/// GET /api/v1/subagents/runs?sessionId=...
pub async fn list_subagent_runs(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<ListRunsParams>,
) -> Result<Json<Vec<SubAgentRunResponse>>, axum::http::StatusCode> {
    if let Some(session_id) = &params.session_id {
        match state
            .store
            .session_store
            .list_subagent_runs(session_id)
            .await
        {
            Ok(rows) => Ok(Json(rows.into_iter().map(Into::into).collect())),
            Err(_) => Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
        }
    } else {
        let runs = state.strm.subagent_manager.list_runs(None);
        Ok(Json(runs.into_iter().map(Into::into).collect()))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListRunsParams {
    session_id: Option<String>,
}

/// GET /api/v1/subagents/runs/:run_id
pub async fn get_subagent_run(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<Json<SubAgentRunResponse>, axum::http::StatusCode> {
    if let Some(run) = state.strm.subagent_manager.get_run(&run_id) {
        return Ok(Json(run.into()));
    }

    match state.store.session_store.get_subagent_run(&run_id).await {
        Ok(Some(row)) => Ok(Json(row.into())),
        Ok(None) => Err(axum::http::StatusCode::NOT_FOUND),
        Err(_) => Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// DELETE /api/v1/subagents/runs/:run_id
pub async fn cancel_subagent_run(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> axum::http::StatusCode {
    if state.strm.subagent_manager.cancel(&run_id) {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::NOT_FOUND
    }
}

/// GET /api/v1/subagents/concurrency — real-time concurrency snapshot
pub async fn get_concurrency_snapshot(
    State(state): State<AppState>,
) -> Json<xiaolin_agent::ConcurrencySnapshot> {
    Json(state.strm.subagent_manager.controller().snapshot())
}
