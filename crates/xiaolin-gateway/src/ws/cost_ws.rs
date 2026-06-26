use axum::extract::ws::{Message, WebSocket};
use serde_json::{json, Value};

use crate::state::AppState;

use super::send_resp;
use super::types::WsResponse;

pub async fn handle_cost_summary(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
) {
    match state.store.cost_store.query_summary(None).await {
        Ok(summary) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "cost.summary".into(),
                    data: Some(json!(summary)),
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
                    msg_type: "cost.summary".into(),
                    data: None,
                    error: Some(Value::String(e.to_string())),
                },
            )
            .await;
        }
    }
}

pub async fn handle_cost_daily(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    start: Option<String>,
    end: Option<String>,
) {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let from = start.unwrap_or_else(|| {
        (chrono::Local::now() - chrono::Duration::days(30))
            .format("%Y-%m-%d")
            .to_string()
    });
    let to = end.unwrap_or(today);

    match state.store.cost_store.query_daily_tokens(&from, &to).await {
        Ok(rows) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "cost.daily".into(),
                    data: Some(json!({ "items": rows })),
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
                    msg_type: "cost.daily".into(),
                    data: None,
                    error: Some(Value::String(e.to_string())),
                },
            )
            .await;
        }
    }
}

pub async fn handle_cost_tools(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    start: Option<String>,
    end: Option<String>,
) {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let from = start.unwrap_or_else(|| {
        (chrono::Local::now() - chrono::Duration::days(30))
            .format("%Y-%m-%d")
            .to_string()
    });
    let to = end.unwrap_or(today);

    match state.store.cost_store.query_tool_stats(&from, &to).await {
        Ok(rows) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "cost.tools".into(),
                    data: Some(json!({ "items": rows })),
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
                    msg_type: "cost.tools".into(),
                    data: None,
                    error: Some(Value::String(e.to_string())),
                },
            )
            .await;
        }
    }
}

pub async fn handle_cost_sessions(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    limit: Option<i64>,
) {
    match state
        .store
        .cost_store
        .query_sessions(limit.unwrap_or(50))
        .await
    {
        Ok(rows) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "cost.sessions".into(),
                    data: Some(json!({ "items": rows })),
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
                    msg_type: "cost.sessions".into(),
                    data: None,
                    error: Some(Value::String(e.to_string())),
                },
            )
            .await;
        }
    }
}
