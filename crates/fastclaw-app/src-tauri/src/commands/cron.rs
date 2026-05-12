use super::helpers::get_state;
use crate::AppData;
use serde_json::json;

// ─── Cron job IPC commands ────────────────────────────────────────────────

#[tauri::command]
pub async fn cron_list_jobs(
    state: tauri::State<'_, AppData>,
    agent_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let jobs = if let Some(aid) = agent_id {
        app.store
            .cron_store
            .list_by_agent(&aid)
            .await
            .map_err(|e| format!("{e}"))?
    } else {
        app.store
            .cron_store
            .list()
            .await
            .map_err(|e| format!("{e}"))?
    };
    Ok(json!({ "jobs": jobs, "count": jobs.len() }))
}

#[tauri::command]
pub async fn cron_get_job(
    state: tauri::State<'_, AppData>,
    job_id: String,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let job = app
        .store
        .cron_store
        .get(&job_id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| format!("cron job not found: {job_id}"))?;
    serde_json::to_value(job).map_err(|e| format!("{e}"))
}

#[tauri::command]
pub async fn cron_upsert_job(
    state: tauri::State<'_, AppData>,
    mut job: fastclaw_cron::CronJob,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;

    if job.id.is_empty() {
        job.id = uuid::Uuid::new_v4().to_string();
    }
    if job.created_at.is_empty() {
        job.created_at = chrono::Utc::now().to_rfc3339();
    }
    if job.schedule.is_empty() {
        return Err("schedule must not be empty".into());
    }
    // Validate the cron expression
    use std::str::FromStr;
    cron::Schedule::from_str(&job.schedule).map_err(|e| format!("invalid cron expression: {e}"))?;

    if let fastclaw_cron::JobAction::Webhook { ref url, .. } = job.action {
        fastclaw_security::ssrf::ssrf_check_url(url)
            .map_err(|e| format!("webhook URL rejected: {e}"))?;
    }

    app.store
        .cron_store
        .upsert(&job)
        .await
        .map_err(|e| format!("{e}"))?;

    app.store.cron_wake.notify_one();

    Ok(json!({ "id": job.id, "ok": true }))
}

#[tauri::command]
pub async fn cron_delete_job(
    state: tauri::State<'_, AppData>,
    job_id: String,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let deleted = app
        .store
        .cron_store
        .delete(&job_id)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(json!({ "deleted": deleted }))
}

#[tauri::command]
pub async fn cron_list_runs(
    state: tauri::State<'_, AppData>,
    job_id: String,
    limit: Option<i64>,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let runs = app
        .store
        .cron_store
        .list_runs(&job_id, limit.unwrap_or(20))
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(json!({ "runs": runs, "count": runs.len() }))
}
