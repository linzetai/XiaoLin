use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::extract::AppJson;
use crate::state::AppState;

use super::error::AppError;
use super::session::PaginationParams;

#[derive(Deserialize)]
pub(super) struct FeedbackBody {
    pub session_id: String,
    pub agent_id: String,
    pub message_id: Option<String>,
    pub kind: String,
    pub value: Option<serde_json::Value>,
}

pub(super) async fn submit_feedback(
    State(state): State<AppState>,
    AppJson(body): AppJson<FeedbackBody>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let kind = match body.kind.as_str() {
        "thumbs_up" => fastclaw_evolution::FeedbackKind::ThumbsUp,
        "thumbs_down" => fastclaw_evolution::FeedbackKind::ThumbsDown,
        "rating" => {
            let v = body.value.as_ref().and_then(|v| v.as_f64()).unwrap_or(3.0) as f32;
            fastclaw_evolution::FeedbackKind::Rating(v)
        }
        "correction" => {
            let text = body
                .value
                .as_ref()
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            fastclaw_evolution::FeedbackKind::Correction(text)
        }
        other => {
            return Err(AppError::BadRequest(format!(
                "unknown feedback kind: {other}"
            )));
        }
    };

    let id = state
        .feedback_store
        .record_feedback(
            &body.session_id,
            &body.agent_id,
            body.message_id.as_deref(),
            &kind,
        )
        .await?;

    if let Err(e) = state
        .skill_store
        .apply_feedback(&body.session_id, &kind)
        .await
    {
        tracing::warn!(error = %e, "skill_store apply_feedback failed");
    }

    Ok(Json(json!({ "id": id })))
}

pub(super) async fn get_feedback(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Query(params): Query<PaginationParams>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let feedback = state.feedback_store.recent(&agent_id, params.limit).await?;
    Ok(Json(json!({ "feedback": feedback })))
}

pub(super) async fn evaluate_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let eval = fastclaw_evolution::StrategyEvaluator::new(&state.feedback_store);
    let report = eval.evaluate(&agent_id).await?;
    Ok(Json(json!({ "report": report })))
}

#[derive(Deserialize)]
pub(super) struct DistillBody {
    pub current_prompt: String,
}

pub(super) async fn distill_prompt(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    AppJson(body): AppJson<DistillBody>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let eval = fastclaw_evolution::StrategyEvaluator::new(&state.feedback_store);
    let report = eval.evaluate(&agent_id).await?;
    let candidate = state
        .prompt_distiller
        .distill(&agent_id, &body.current_prompt, &report)
        .await?;
    Ok(Json(json!({ "candidate": candidate })))
}

pub(super) async fn list_candidates(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Query(params): Query<PaginationParams>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let candidates = state.prompt_distiller.list(&agent_id, params.limit).await?;
    Ok(Json(json!({ "candidates": candidates })))
}

pub(super) async fn accept_candidate(
    State(state): State<AppState>,
    Path(candidate_id): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let ok = state.prompt_distiller.accept(&candidate_id).await?;
    Ok(Json(json!({ "accepted": ok })))
}

pub(super) async fn reject_candidate(
    State(state): State<AppState>,
    Path(candidate_id): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let ok = state.prompt_distiller.reject(&candidate_id).await?;
    Ok(Json(json!({ "rejected": ok })))
}
