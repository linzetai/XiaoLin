use super::helpers::get_state;
use crate::AppData;
use serde_json::json;

// ─── Sessions ───

#[tauri::command]
pub async fn list_sessions(
    state: tauri::State<'_, AppData>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let limit = limit.unwrap_or(50).clamp(1, 200);
    let offset = offset.unwrap_or(0).max(0);
    let sessions = app
        .store
        .session_store
        .list_sessions(limit, offset)
        .await
        .map_err(|e| e.to_string())?;
    let count = sessions.len();
    let data: Vec<_> = sessions
        .iter()
        .map(|s| {
            json!({
                "id": s.id, "agentId": s.agent_id, "title": s.title,
                "workDir": s.work_dir,
                "messageCount": s.message_count,
                "createdAt": s.created_at, "updatedAt": s.updated_at,
                "totalPromptTokens": s.total_prompt_tokens,
                "totalCompletionTokens": s.total_completion_tokens,
                "totalElapsedMs": s.total_elapsed_ms,
            })
        })
        .collect();
    Ok(json!({"sessions": data, "count": count}))
}

#[tauri::command]
pub async fn get_session(
    state: tauri::State<'_, AppData>,
    session_id: String,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    match app.store.session_store.get_session(&session_id).await {
        Ok(Some(s)) => Ok(json!({
            "id": s.id, "agentId": s.agent_id, "title": s.title,
            "workDir": s.work_dir,
            "messageCount": s.message_count,
            "createdAt": s.created_at, "updatedAt": s.updated_at,
            "totalPromptTokens": s.total_prompt_tokens,
            "totalCompletionTokens": s.total_completion_tokens,
            "totalElapsedMs": s.total_elapsed_ms,
        })),
        Ok(None) => Err("session not found".into()),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub async fn set_session_work_dir(
    state: tauri::State<'_, AppData>,
    session_id: String,
    work_dir: Option<String>,
) -> Result<(), String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    app.store
        .session_store
        .update_work_dir(&session_id, work_dir.as_deref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_session_messages(
    state: tauri::State<'_, AppData>,
    session_id: String,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let messages = app
        .store
        .session_store
        .load_messages(&session_id)
        .await
        .map_err(|e| e.to_string())?;
    let data: Vec<_> = messages
        .iter()
        .map(|m| {
            let mut obj = json!({
                "id": m.id,
                "role": m.role,
                "content": m.content.as_ref().and_then(|c| serde_json::from_str::<serde_json::Value>(c).ok()),
                "name": m.name,
                "toolCallId": m.tool_call_id,
                "toolCallsJson": m.tool_calls_json.as_ref().and_then(|tc| serde_json::from_str::<serde_json::Value>(tc).ok()),
                "createdAt": m.created_at,
            });
            if m.prompt_tokens > 0 || m.completion_tokens > 0 || m.elapsed_ms > 0 {
                obj["promptTokens"] = json!(m.prompt_tokens);
                obj["completionTokens"] = json!(m.completion_tokens);
                obj["totalTokens"] = json!(m.total_tokens);
                obj["elapsedMs"] = json!(m.elapsed_ms);
            }
            obj
        })
        .collect();
    Ok(json!({"messages": data}))
}

#[tauri::command]
pub async fn create_session(
    state: tauri::State<'_, AppData>,
    agent_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let agent_id = agent_id.as_deref().unwrap_or("main");
    let new_id = uuid::Uuid::new_v4().to_string();
    let work_dir = app
        .rt
        .workspaces
        .get(agent_id)
        .map(|ws| ws.root.to_string_lossy().to_string());
    app.store
        .session_store
        .create_session_with_work_dir(&new_id, agent_id, None, work_dir.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({"sessionId": new_id, "agentId": agent_id, "workDir": work_dir}))
}

#[tauri::command]
pub async fn update_session_title(
    state: tauri::State<'_, AppData>,
    session_id: String,
    title: String,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let updated = app
        .store
        .session_store
        .update_title(&session_id, &title)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({"updated": updated}))
}

#[tauri::command]
pub async fn delete_session(
    state: tauri::State<'_, AppData>,
    session_id: String,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let deleted = app
        .store
        .session_store
        .delete_session(&session_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({"deleted": deleted}))
}
