use axum::extract::{Path, State};
use axum::Json;
use serde_json::json;

use crate::extract::AppJson;
use crate::state::AppState;

use super::error::AppError;

pub(super) async fn list_cron_jobs(
    State(state): State<AppState>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let jobs = state.store.cron_store.list().await?;
    Ok(Json(json!({ "jobs": jobs, "count": jobs.len() })))
}

pub(super) async fn get_cron_job(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let job = state
        .store
        .cron_store
        .get(&job_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("cron job not found: {job_id}")))?;
    Ok(Json(json!(job)))
}

pub(super) async fn upsert_cron_job(
    State(state): State<AppState>,
    AppJson(mut job): AppJson<fastclaw_cron::CronJob>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    if job.id.is_empty() {
        job.id = uuid::Uuid::new_v4().to_string();
    }
    if job.created_at.is_empty() {
        job.created_at = chrono::Utc::now().to_rfc3339();
    }

    if job.schedule.is_empty() {
        return Err(AppError::BadRequest("schedule must not be empty".into()));
    }

    if let fastclaw_cron::JobAction::Webhook { ref url, .. } = job.action {
        fastclaw_security::ssrf::ssrf_check_url(url)
            .map_err(|e| AppError::BadRequest(format!("webhook URL rejected: {e}")))?;
    }

    state.store.cron_store.upsert(&job).await?;
    Ok(Json(json!({ "id": job.id, "ok": true })))
}

pub(super) async fn delete_cron_job(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let deleted = state.store.cron_store.delete(&job_id).await?;
    Ok(Json(json!({ "deleted": deleted })))
}
