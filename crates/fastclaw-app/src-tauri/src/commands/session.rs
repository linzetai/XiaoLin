use super::helpers::get_state;
use crate::AppData;
use serde_json::json;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportFormat {
    Markdown,
    Json,
}

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

#[tauri::command]
pub async fn export_session_content(
    state: tauri::State<'_, AppData>,
    session_id: String,
    format: ExportFormat,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;

    let session = app
        .store
        .session_store
        .get_session(&session_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "session not found".to_string())?;

    let messages = app
        .store
        .session_store
        .load_messages(&session_id)
        .await
        .map_err(|e| e.to_string())?;

    match format {
        ExportFormat::Json => {
            let export_data = json!({
                "session": {
                    "id": session.id,
                    "agentId": session.agent_id,
                    "title": session.title,
                    "workDir": session.work_dir,
                    "source": session.source,
                    "createdAt": session.created_at,
                    "updatedAt": session.updated_at,
                    "messageCount": session.message_count,
                    "totalPromptTokens": session.total_prompt_tokens,
                    "totalCompletionTokens": session.total_completion_tokens,
                    "totalElapsedMs": session.total_elapsed_ms,
                },
                "messages": messages.iter().map(|m| {
                    let content = m.content.as_ref().and_then(|c| serde_json::from_str::<serde_json::Value>(c).ok());
                    let tool_calls = m.tool_calls_json.as_ref().and_then(|tc| serde_json::from_str::<serde_json::Value>(tc).ok());
                    json!({
                        "id": m.id,
                        "role": m.role,
                        "content": content,
                        "name": m.name,
                        "toolCallId": m.tool_call_id,
                        "toolCalls": tool_calls,
                        "createdAt": m.created_at,
                        "promptTokens": m.prompt_tokens,
                        "completionTokens": m.completion_tokens,
                        "totalTokens": m.total_tokens,
                        "elapsedMs": m.elapsed_ms,
                    })
                }).collect::<Vec<_>>(),
                "exportedAt": chrono::Utc::now().to_rfc3339(),
            });
            let content = serde_json::to_string_pretty(&export_data).map_err(|e| e.to_string())?;
            let filename = format!(
                "{}.json",
                session.title.as_deref().unwrap_or("session").replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_")
            );
            Ok(json!({"content": content, "filename": filename, "mimeType": "application/json"}))
        }
        ExportFormat::Markdown => {
            let mut md = String::new();
            let title = session.title.as_deref().unwrap_or("未命名会话");
            md.push_str(&format!("# {}\n\n", title));
            md.push_str(&format!("- **会话 ID**: `{}`\n", session.id));
            md.push_str(&format!("- **Agent**: `{}`\n", session.agent_id));
            if let Some(ref wd) = session.work_dir {
                md.push_str(&format!("- **工作目录**: `{}`\n", wd));
            }
            md.push_str(&format!("- **创建时间**: {}\n", session.created_at));
            md.push_str(&format!("- **消息数**: {}\n", session.message_count));
            if session.total_prompt_tokens > 0 || session.total_completion_tokens > 0 {
                md.push_str(&format!(
                    "- **Token 用量**: prompt {} / completion {} / 总耗时 {}ms\n",
                    session.total_prompt_tokens, session.total_completion_tokens, session.total_elapsed_ms
                ));
            }
            md.push_str("\n---\n\n");

            for m in &messages {
                let role_label = match m.role.as_str() {
                    "user" => "👤 User",
                    "assistant" => "🤖 Assistant",
                    "system" => "⚙️ System",
                    "tool" => "🔧 Tool",
                    other => other,
                };
                md.push_str(&format!("## {}\n", role_label));
                md.push_str(&format!("_{}_\n\n", m.created_at));

                if let Some(ref content_str) = m.content {
                    let text = match serde_json::from_str::<serde_json::Value>(content_str) {
                        Ok(serde_json::Value::String(s)) => s,
                        Ok(serde_json::Value::Array(arr)) => {
                            arr.iter()
                                .filter_map(|part| {
                                    if part.get("type")?.as_str()? == "text" {
                                        part.get("text")?.as_str().map(|s| s.to_string())
                                    } else {
                                        None
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join("\n")
                        }
                        Ok(other) => other.to_string(),
                        Err(_) => content_str.clone(),
                    };
                    md.push_str(&text);
                    md.push_str("\n\n");
                }

                if let Some(ref tc_json) = m.tool_calls_json {
                    if let Ok(tool_calls) = serde_json::from_str::<Vec<serde_json::Value>>(tc_json) {
                        for tc in &tool_calls {
                            let name = tc.get("function")
                                .and_then(|f| f.get("name"))
                                .and_then(|n| n.as_str())
                                .unwrap_or("unknown");
                            let args = tc.get("function")
                                .and_then(|f| f.get("arguments"))
                                .and_then(|a| a.as_str())
                                .unwrap_or("");
                            md.push_str(&format!("<details>\n<summary>🔧 Tool Call: <code>{}</code></summary>\n\n", name));
                            md.push_str("```json\n");
                            if let Ok(pretty) = serde_json::from_str::<serde_json::Value>(args) {
                                md.push_str(&serde_json::to_string_pretty(&pretty).unwrap_or_else(|_| args.to_string()));
                            } else {
                                md.push_str(args);
                            }
                            md.push_str("\n```\n\n</details>\n\n");
                        }
                    }
                }

                if m.prompt_tokens > 0 || m.completion_tokens > 0 || m.elapsed_ms > 0 {
                    md.push_str(&format!(
                        "> tokens: {} prompt / {} completion | {}ms\n\n",
                        m.prompt_tokens, m.completion_tokens, m.elapsed_ms
                    ));
                }

                md.push_str("---\n\n");
            }

            let filename = format!(
                "{}.md",
                title.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_")
            );
            Ok(json!({"content": md, "filename": filename, "mimeType": "text/markdown"}))
        }
    }
}
