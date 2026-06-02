use axum::extract::ws::{Message, WebSocket};
use serde_json::json;

use crate::state::AppState;

use super::send_resp;
use super::types::WsResponse;

pub async fn handle_cron_list_jobs(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    agent_id: Option<String>,
) {
    let result = if let Some(aid) = agent_id {
        state.store.cron_store.list_by_agent(&aid).await
    } else {
        state.store.cron_store.list().await
    };

    match result {
        Ok(jobs) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "cron.list_jobs".into(),
                    data: Some(json!({ "jobs": jobs })),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": -32000, "message": format!("{e}")})),
                },
            )
            .await;
        }
    }
}

pub async fn handle_cron_get_job(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    job_id: &str,
) {
    match state.store.cron_store.get(job_id).await {
        Ok(Some(job)) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "cron.get_job".into(),
                    data: Some(serde_json::to_value(job).unwrap_or_default()),
                    error: None,
                },
            )
            .await;
        }
        Ok(None) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": -32602, "message": format!("cron job not found: {job_id}")})),
                },
            )
            .await;
        }
        Err(e) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": -32000, "message": format!("{e}")})),
                },
            )
            .await;
        }
    }
}

pub async fn handle_cron_upsert_job(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let job_value = params.get("job").cloned().unwrap_or(params.clone());
    let mut job: xiaolin_cron::CronJob = match serde_json::from_value(job_value) {
        Ok(j) => j,
        Err(e) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": -32602, "message": format!("invalid job: {e}")})),
                },
            )
            .await;
            return;
        }
    };

    if job.id.is_empty() {
        job.id = uuid::Uuid::new_v4().to_string();
    }
    if job.created_at.is_empty() {
        job.created_at = chrono::Utc::now().to_rfc3339();
    }

    match state.store.cron_store.upsert(&job).await {
        Ok(()) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "cron.upsert_job".into(),
                    data: Some(json!({ "ok": true, "jobId": job.id })),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": -32000, "message": format!("{e}")})),
                },
            )
            .await;
        }
    }
}

pub async fn handle_cron_delete_job(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    job_id: &str,
) {
    match state.store.cron_store.delete(job_id).await {
        Ok(deleted) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "cron.delete_job".into(),
                    data: Some(json!({ "ok": deleted })),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": -32000, "message": format!("{e}")})),
                },
            )
            .await;
        }
    }
}

pub async fn handle_cron_list_runs(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    job_id: &str,
    limit: Option<i64>,
) {
    match state.store.cron_store.list_runs(job_id, limit.unwrap_or(20)).await {
        Ok(runs) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "cron.list_runs".into(),
                    data: Some(json!({ "runs": runs })),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": -32000, "message": format!("{e}")})),
                },
            )
            .await;
        }
    }
}
