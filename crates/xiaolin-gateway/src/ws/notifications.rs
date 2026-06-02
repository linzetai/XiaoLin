use axum::extract::ws::{Message, WebSocket};
use serde_json::json;

use crate::state::AppState;

use super::send_resp;
use super::types::WsResponse;

pub async fn handle_unread_count(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
) {
    match state.store.notification_store.unread_count().await {
        Ok(count) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "notifications.unread_count".into(),
                    data: Some(json!({ "count": count })),
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

pub async fn handle_list(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    limit: Option<i64>,
) {
    let limit = limit.unwrap_or(50);
    let notifications = state.store.notification_store.list(limit, 0, false).await;
    let unread = state.store.notification_store.unread_count().await;

    match (notifications, unread) {
        (Ok(list), Ok(count)) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "notifications.list".into(),
                    data: Some(json!({ "notifications": list, "unreadCount": count })),
                    error: None,
                },
            )
            .await;
        }
        (Err(e), _) | (_, Err(e)) => {
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

pub async fn handle_mark_read(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    notification_id: &str,
) {
    let _ = state.store.notification_store.mark_read(notification_id).await;
    let unread = state
        .store
        .notification_store
        .unread_count()
        .await
        .unwrap_or(0);

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "notifications.mark_read".into(),
            data: Some(json!({ "unreadCount": unread })),
            error: None,
        },
    )
    .await;
}

pub async fn handle_mark_all_read(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
) {
    let _ = state.store.notification_store.mark_all_read().await;
    let unread = state
        .store
        .notification_store
        .unread_count()
        .await
        .unwrap_or(0);

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "notifications.mark_all_read".into(),
            data: Some(json!({ "unreadCount": unread })),
            error: None,
        },
    )
    .await;
}

pub async fn handle_delete(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    notification_id: &str,
) {
    let _ = state.store.notification_store.delete(notification_id).await;

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "notifications.delete".into(),
            data: Some(json!({ "ok": true })),
            error: None,
        },
    )
    .await;
}
