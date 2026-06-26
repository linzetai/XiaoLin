use axum::extract::ws::{Message, WebSocket};
use serde_json::json;

use crate::state::AppState;

use super::send_resp;
use super::types::WsResponse;

fn broadcast_changed(state: &AppState, event: &str, job_id: &str, data: serde_json::Value) {
    let _ = state.strm.ws_broadcast.send(
        json!({
            "type": "event",
            "event": "automations.changed",
            "data": {
                "event": event,
                "jobId": job_id,
                "job": data,
            }
        })
        .to_string(),
    );
}

pub async fn handle_automations_list(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
) {
    match state.store.cron_store.list().await {
        Ok(jobs) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "automations.list".into(),
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

pub async fn handle_automations_create(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let mut job: xiaolin_cron::CronJob = match serde_json::from_value(params) {
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
            let job_val = serde_json::to_value(&job).unwrap_or_default();
            broadcast_changed(state, "created", &job.id, job_val.clone());
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "automations.created".into(),
                    data: Some(json!({ "job": job_val })),
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

pub async fn handle_automations_update(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    job_id: &str,
    params: serde_json::Value,
) {
    let existing = match state.store.cron_store.get(job_id).await {
        Ok(Some(j)) => j,
        Ok(None) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(
                        json!({"code": -32602, "message": format!("job not found: {job_id}")}),
                    ),
                },
            )
            .await;
            return;
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
            return;
        }
    };

    let mut merged = serde_json::to_value(&existing).unwrap_or_default();
    if let (Some(base), Some(patch)) = (merged.as_object_mut(), params.as_object()) {
        for (k, v) in patch {
            base.insert(k.clone(), v.clone());
        }
    }

    let job: xiaolin_cron::CronJob = match serde_json::from_value(merged) {
        Ok(j) => j,
        Err(e) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(
                        json!({"code": -32602, "message": format!("invalid merged job: {e}")}),
                    ),
                },
            )
            .await;
            return;
        }
    };

    match state.store.cron_store.upsert(&job).await {
        Ok(()) => {
            let job_val = serde_json::to_value(&job).unwrap_or_default();
            broadcast_changed(state, "updated", &job.id, job_val.clone());
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "automations.updated".into(),
                    data: Some(json!({ "job": job_val })),
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

pub async fn handle_automations_delete(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    job_id: &str,
) {
    match state.store.cron_store.delete(job_id).await {
        Ok(deleted) => {
            broadcast_changed(state, "deleted", job_id, json!(null));
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "automations.deleted".into(),
                    data: Some(json!({ "ok": deleted, "jobId": job_id })),
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

pub async fn handle_automations_run_now(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    job_id: &str,
) {
    let existing = match state.store.cron_store.get(job_id).await {
        Ok(Some(j)) => j,
        Ok(None) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(
                        json!({"code": -32602, "message": format!("job not found: {job_id}")}),
                    ),
                },
            )
            .await;
            return;
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
            return;
        }
    };

    let mut job = existing;
    job.next_run = Some(chrono::Utc::now().to_rfc3339());
    if !job.enabled {
        job.enabled = true;
    }
    match state.store.cron_store.upsert(&job).await {
        Ok(()) => {
            state.store.cron_wake.notify_one();
            let job_val = serde_json::to_value(&job).unwrap_or_default();
            broadcast_changed(state, "updated", &job.id, job_val.clone());
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "automations.run_now".into(),
                    data: Some(json!({ "ok": true, "jobId": job_id, "job": job_val })),
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

pub async fn handle_automations_runs(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    job_id: &str,
    limit: Option<i64>,
) {
    match state
        .store
        .cron_store
        .list_runs(job_id, limit.unwrap_or(20))
        .await
    {
        Ok(runs) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "automations.runs".into(),
                    data: Some(json!({ "runs": runs, "jobId": job_id })),
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
