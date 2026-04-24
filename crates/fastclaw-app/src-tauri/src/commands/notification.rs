use crate::AppData;
use serde_json::json;
use super::helpers::get_state;

// ─── Notification center commands ─────────────────────────────────────────────

#[tauri::command]
pub async fn notification_list(
    state: tauri::State<'_, AppData>,
    limit: Option<i64>,
    offset: Option<i64>,
    unread_only: Option<bool>,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let items = app
        .store
        .notification_store
        .list(limit.unwrap_or(30), offset.unwrap_or(0), unread_only.unwrap_or(false))
        .await
        .map_err(|e| format!("{e}"))?;
    let unread = app.store.notification_store.unread_count().await.unwrap_or(0);
    Ok(json!({ "notifications": items, "count": items.len(), "unreadCount": unread }))
}

#[tauri::command]
pub async fn notification_get(
    state: tauri::State<'_, AppData>,
    id: String,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let n = app
        .store
        .notification_store
        .get(&id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| format!("notification not found: {id}"))?;
    Ok(serde_json::to_value(n).map_err(|e| format!("{e}"))?)
}

#[tauri::command]
pub async fn notification_mark_read(
    state: tauri::State<'_, AppData>,
    id: String,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    app.store.notification_store
        .mark_read(&id)
        .await
        .map_err(|e| format!("{e}"))?;
    let unread = app.store.notification_store.unread_count().await.unwrap_or(0);

    let event = serde_json::json!({
        "type": "event",
        "event": "notification.read",
        "data": { "id": id, "unreadCount": unread }
    });
    let _ = app.strm.ws_broadcast.send(event.to_string());

    Ok(json!({ "ok": true, "unreadCount": unread }))
}

#[tauri::command]
pub async fn notification_mark_all_read(
    state: tauri::State<'_, AppData>,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    app.store.notification_store
        .mark_all_read()
        .await
        .map_err(|e| format!("{e}"))?;
    let unread = app.store.notification_store.unread_count().await.unwrap_or(0);

    let event = serde_json::json!({
        "type": "event",
        "event": "notification.read",
        "data": { "unreadCount": unread }
    });
    let _ = app.strm.ws_broadcast.send(event.to_string());

    Ok(json!({ "ok": true, "unreadCount": unread }))
}

#[tauri::command]
pub async fn notification_unread_count(
    state: tauri::State<'_, AppData>,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let count = app.store.notification_store.unread_count().await.map_err(|e| format!("{e}"))?;
    Ok(json!({ "count": count }))
}

#[tauri::command]
pub async fn notification_delete(
    state: tauri::State<'_, AppData>,
    id: String,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let deleted = app.store.notification_store.delete(&id).await.map_err(|e| format!("{e}"))?;
    Ok(json!({ "ok": deleted }))
}

#[tauri::command]
pub async fn notification_clear_read(
    state: tauri::State<'_, AppData>,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let cleared = app.store.notification_store.clear_read().await.map_err(|e| format!("{e}"))?;
    Ok(json!({ "ok": true, "cleared": cleared }))
}
