use crate::embedded::GatewayInfo;
use crate::AppData;
use fastclaw_core::config_access::{
    CONFIG_READABLE_KEYS, CONFIG_WRITABLE_KEYS, filter_config_for_read, navigate_config,
    set_nested_key,
};
use fastclaw_gateway::AppState;
use serde_json::json;
use tauri::Emitter;

fn collect_available_models(app: &AppState) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::<String>::new();

    let live = app
        .config_live
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| serde_json::to_value(&*app.config).unwrap_or_else(|_| json!({})));

    if let Some(models_obj) = live.get("models").and_then(|v| v.as_object()) {
        for (key, cfg) in models_obj {
            let model = cfg
                .get("model")
                .or_else(|| cfg.get("defaultModel"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            if model.is_empty() {
                continue;
            }
            // Use the config key (e.g. "dashscope") as provider — it matches
            // credential lookup keys and is recognized by create_provider_chain.
            let provider = key.clone();
            let dedupe_key = format!("{provider}::{model}");
            if !seen.insert(dedupe_key) {
                continue;
            }
            out.push(json!({
                "agentId": key,
                "model": model,
                "provider": provider,
                "contextWindow": cfg.get("contextWindow").cloned().unwrap_or(serde_json::Value::Null),
                "costPer1kInput": cfg.get("costPer1kInput").cloned().unwrap_or(serde_json::Value::Null),
                "costPer1kOutput": cfg.get("costPer1kOutput").cloned().unwrap_or(serde_json::Value::Null),
                "supportsReasoning": cfg.get("supportsReasoning").cloned().unwrap_or(serde_json::Value::Null),
            }));
        }
    }

    out
}

fn get_state(gw: &Option<crate::embedded::EmbeddedGateway>) -> Result<&AppState, String> {
    gw.as_ref()
        .map(|g| g.app_state())
        .ok_or_else(|| "gateway not started".to_string())
}

fn ensure_agent_workspace_bootstrap(app: &AppState, agent_id: &str) -> Result<(), String> {
    let state_dir = fastclaw_core::paths::resolve_state_dir_from(Some(&app.config.paths));
    let ws_root = fastclaw_core::workspace::resolve_workspace_root(&state_dir, agent_id, None);
    let ws = fastclaw_core::workspace::AgentWorkspace::new(ws_root, agent_id.to_string());
    ws.ensure_bootstrap()
        .map_err(|e| format!("ensure workspace bootstrap failed: {e}"))
}

fn validate_agent_id(agent_id: &str) -> Result<(), String> {
    if agent_id.is_empty() {
        return Err("agent_id cannot be empty".to_string());
    }
    if agent_id.len() > 64 {
        return Err("agent_id too long (max 64 characters)".to_string());
    }
    if !agent_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err(
            "agent_id contains invalid characters; only [a-zA-Z0-9_-] are allowed".to_string(),
        );
    }
    Ok(())
}

// ─── Model connection test ───

#[tauri::command]
pub async fn test_model_connection(
    base_url: String,
    api_key: String,
    model: Option<String>,
) -> Result<serde_json::Value, String> {
    let base = base_url.trim_end_matches('/');
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    // Try /models first; fall back to a lightweight chat completion probe.
    let models_url = format!("{base}/models");
    let models_resp = client
        .get(&models_url)
        .header("Authorization", format!("Bearer {api_key}"))
        .send()
        .await;

    if let Ok(resp) = models_resp {
        if resp.status().is_success() {
            return Ok(json!({ "ok": true, "method": "models" }));
        }
        // 401/403 means auth failure — report immediately.
        let status = resp.status().as_u16();
        if status == 401 || status == 403 {
            let body = resp.text().await.unwrap_or_default();
            let snippet = if body.len() > 150 { &body[..150] } else { &body };
            return Err(format!("认证失败 (HTTP {status}): {snippet}"));
        }
    }

    // Fallback: lightweight chat completion with max_tokens=1 and a tiny prompt.
    let chat_url = format!("{base}/chat/completions");
    let model_name = model.unwrap_or_else(|| "gpt-3.5-turbo".to_string());
    let payload = json!({
        "model": model_name,
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 1,
        "stream": false,
    });
    let chat_resp = client
        .post(&chat_url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("连接失败: {e}"))?;

    let status = chat_resp.status().as_u16();
    if chat_resp.status().is_success() {
        Ok(json!({ "ok": true, "method": "chat" }))
    } else if status == 401 || status == 403 {
        let body = chat_resp.text().await.unwrap_or_default();
        let snippet = if body.len() > 150 { &body[..150] } else { &body };
        Err(format!("认证失败 (HTTP {status}): {snippet}"))
    } else {
        let body = chat_resp.text().await.unwrap_or_default();
        let snippet = if body.len() > 150 { &body[..150] } else { &body };
        Err(format!("HTTP {status}: {snippet}"))
    }
}

// ─── Gateway info & health ───

#[tauri::command]
pub async fn get_gateway_info(
    state: tauri::State<'_, AppData>,
) -> Result<GatewayInfo, String> {
    let gw = state.gateway.lock().await;
    match gw.as_ref() {
        Some(g) => Ok(g.info().clone()),
        None => Err("gateway not started".into()),
    }
}

#[tauri::command]
pub async fn health_check(
    state: tauri::State<'_, AppData>,
) -> Result<bool, String> {
    let gw = state.gateway.lock().await;
    Ok(gw.as_ref().is_some())
}

// ─── Agents ───

#[tauri::command]
pub async fn list_agents(
    state: tauri::State<'_, AppData>,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let agents: Vec<_> = app
        .router
        .read()
        .await
        .list_agents()
        .iter()
        .map(|a| json!({"agentId": a.agent_id, "name": a.name, "model": a.model.model}))
        .collect();
    Ok(json!({"agents": agents}))
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
    match app.session_store.get_session(&session_id).await {
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
    app.session_store
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
        .session_store
        .load_messages(&session_id)
        .await
        .map_err(|e| e.to_string())?;
    let data: Vec<_> = messages
        .iter()
        .map(|m| {
            json!({
                "id": m.id,
                "role": m.role,
                "content": m.content.as_ref().and_then(|c| serde_json::from_str::<serde_json::Value>(c).ok()),
                "name": m.name,
                "toolCallId": m.tool_call_id,
                "toolCallsJson": m.tool_calls_json.as_ref().and_then(|tc| serde_json::from_str::<serde_json::Value>(tc).ok()),
                "createdAt": m.created_at,
            })
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
        .workspaces
        .get(agent_id)
        .map(|ws| ws.root.to_string_lossy().to_string());
    app.session_store
        .create_session_with_work_dir(
            &new_id,
            agent_id,
            None,
            work_dir.as_deref(),
        )
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
        .session_store
        .delete_session(&session_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({"deleted": deleted}))
}

// ─── Models ───

#[tauri::command]
pub async fn list_models(
    state: tauri::State<'_, AppData>,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let models = collect_available_models(app);
    Ok(json!({"models": models}))
}

// ─── Config ───

#[tauri::command]
pub async fn get_config(
    state: tauri::State<'_, AppData>,
    key: Option<String>,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let key = key.as_deref().unwrap_or("");

    let live = app
        .config_live
        .read()
        .map_err(|e| format!("config_live lock poisoned: {e}"))?
        .clone();

    if key.is_empty() {
        let filtered = filter_config_for_read(&live);
        return Ok(filtered);
    }

    let top_key = key.split('.').next().unwrap_or(key);
    if !CONFIG_READABLE_KEYS.contains(&top_key) {
        return Err(format!("access denied: key '{}' is not readable", top_key));
    }

    let value = navigate_config(&live, key);
    Ok(json!({"key": key, "value": value}))
}

#[tauri::command]
pub async fn set_config(
    state: tauri::State<'_, AppData>,
    key: String,
    value: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;

    if key.is_empty() {
        return Err("key parameter required".into());
    }

    let top_key = key.split('.').next().unwrap_or(&key);
    if !CONFIG_WRITABLE_KEYS.contains(&top_key) {
        return Err(format!("access denied: key '{}' is read-only", top_key));
    }

    let mut cfg_value = app
        .config_live
        .read()
        .map_err(|e| format!("config_live lock poisoned: {e}"))?
        .clone();
    if set_nested_key(&mut cfg_value, &key, value.clone()).is_err() {
        return Err("failed to set nested key".into());
    }

    serde_json::from_value::<fastclaw_core::config::FastClawConfig>(cfg_value.clone())
        .map_err(|e| format!("validation failed: {e}"))?;

    let persisted = persist_config_key(&key, &value);
    let applied = persisted.is_ok();
    if let Err(ref e) = persisted {
        tracing::warn!(key = key.as_str(), error = %e, "config.set: validated but failed to persist");
    }

    if applied {
        if let Ok(mut live) = app.config_live.write() {
            *live = cfg_value;
        }
        tracing::info!(key = key.as_str(), "config.set: persisted and updated in-memory");
        if top_key == "credentials" || top_key == "models" {
            if let Err(e) = app.reload_agents().await {
                tracing::warn!(
                    key = key.as_str(),
                    error = %e,
                    "config.set: updated config but failed to refresh runtime providers"
                );
            }
        }
        if top_key == "security" {
            if let Ok(parsed) = serde_json::from_value::<fastclaw_core::config::FastClawConfig>(
                app.config_live.read().map(|g| g.clone()).unwrap_or_default(),
            ) {
                fastclaw_security::ssrf::set_ssrf_allowed_hosts(
                    parsed.security.ssrf_allowed_hosts.clone(),
                );
                fastclaw_security::dangerous_ops::set_dangerous_ops_config(
                    parsed.security.dangerous_ops_policy,
                    &parsed.security.dangerous_patterns,
                );
                tracing::info!(
                    hosts = ?parsed.security.ssrf_allowed_hosts,
                    dangerous_ops_policy = ?parsed.security.dangerous_ops_policy,
                    "config.set: hot-reloaded security settings"
                );
            }
        }
    }

    Ok(json!({
        "key": key,
        "persisted": applied,
        "pendingRestart": false,
    }))
}

// ─── Skills & Tools ───

#[tauri::command]
pub async fn list_skills(
    state: tauri::State<'_, AppData>,
    agent_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let aid = agent_id.clone().unwrap_or_else(|| "main".to_string());
    tracing::info!(agent_id = %aid, "IPC list_skills called");
    let registry = app.skill_registry_for(&aid);
    let skills: Vec<serde_json::Value> = registry
        .list()
        .into_iter()
        .filter(|s| s.frontmatter.enabled.unwrap_or(true))
        .map(|s| {
            json!({
                "id": s.id,
                "name": s.name,
                "description": s.description,
                "tags": s.frontmatter.tags,
            })
        })
        .collect();
    tracing::info!(count = skills.len(), "IPC list_skills returning");
    Ok(json!({ "skills": skills, "count": skills.len() }))
}

#[tauri::command]
pub async fn refresh_skills(
    state: tauri::State<'_, AppData>,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let count = app.reload_skills().map_err(|e| e.to_string())?;
    Ok(json!({ "refreshed": true, "count": count }))
}

#[tauri::command]
pub async fn upload_skill(
    state: tauri::State<'_, AppData>,
    source_path: String,
) -> Result<serde_json::Value, String> {
    let src = std::path::Path::new(&source_path);
    if !src.exists() {
        return Err(format!("path does not exist: {source_path}"));
    }

    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let skills_dir = fastclaw_core::paths::resolve_skills_dir_from(Some(&app.config.paths));

    if src.is_dir() {
        let skill_md = src.join("SKILL.md");
        if !skill_md.exists() {
            return Err("selected folder does not contain a SKILL.md file".into());
        }
        let dir_name = src
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("invalid directory name")?;
        let dest = skills_dir.join(dir_name);
        if dest.exists() {
            std::fs::remove_dir_all(&dest).map_err(|e| format!("failed to clean existing skill dir: {e}"))?;
        }
        copy_dir_recursive(src, &dest).map_err(|e| format!("failed to copy skill dir: {e}"))?;
        let count = app.reload_skills().map_err(|e| e.to_string())?;
        return Ok(json!({ "installed": dir_name, "count": count }));
    }

    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext == "zip" {
        let file = std::fs::File::open(src).map_err(|e| format!("failed to open zip: {e}"))?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("invalid zip: {e}"))?;
        let mut top_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut has_skill_md = false;
        for i in 0..archive.len() {
            let f = archive
                .by_index(i)
                .map_err(|e| format!("failed to read zip entry at index {i}: {e}"))?;
            let Some(enclosed) = f.enclosed_name() else {
                return Err(format!("zip contains unsafe path traversal entry: {}", f.name()));
            };
            if enclosed
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n == "SKILL.md")
            {
                has_skill_md = true;
            }
            if let Some(component) = enclosed.components().next() {
                top_dirs.insert(component.as_os_str().to_string_lossy().to_string());
            }
        }
        if !has_skill_md {
            return Err("zip archive does not contain a SKILL.md file".into());
        }

        let is_flat = top_dirs.len() == 1;
        let extract_to = if is_flat {
            skills_dir.clone()
        } else {
            let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("skill");
            skills_dir.join(stem)
        };
        std::fs::create_dir_all(&extract_to)
            .map_err(|e| format!("failed to create extraction dir: {e}"))?;
        for i in 0..archive.len() {
            let mut f = archive
                .by_index(i)
                .map_err(|e| format!("failed to read zip entry at index {i}: {e}"))?;
            let Some(enclosed) = f.enclosed_name().map(|p| p.to_path_buf()) else {
                return Err(format!("zip contains unsafe path traversal entry: {}", f.name()));
            };
            let out_path = extract_to.join(enclosed);
            if f.is_dir() {
                std::fs::create_dir_all(&out_path)
                    .map_err(|e| format!("failed to create dir during extraction: {e}"))?;
            } else {
                if let Some(parent) = out_path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("failed to create parent dir during extraction: {e}"))?;
                }
                let mut out_file = std::fs::File::create(&out_path)
                    .map_err(|e| format!("failed to create extracted file: {e}"))?;
                std::io::copy(&mut f, &mut out_file)
                    .map_err(|e| format!("failed to write extracted file: {e}"))?;
            }
        }

        let skill_name = if is_flat {
            top_dirs.into_iter().next().unwrap_or_default()
        } else {
            src.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("skill")
                .to_string()
        };
        let count = app.reload_skills().map_err(|e| e.to_string())?;
        return Ok(json!({ "installed": skill_name, "count": count }));
    }

    Err(format!("unsupported file type: .{ext} (expected a folder or .zip)"))
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn list_agent_tools(
    state: tauri::State<'_, AppData>,
    agent_id: String,
) -> Result<serde_json::Value, String> {
    validate_agent_id(&agent_id)?;
    tracing::info!(agent_id = %agent_id, "IPC list_agent_tools called");
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let agent = {
        let router = app.router.read().await;
        router
            .agent_by_id(&agent_id)
            .cloned()
            .ok_or_else(|| format!("agent not found: {agent_id}"))?
    };
    let tools: Vec<serde_json::Value> = app
        .tool_registry
        .definitions()
        .iter()
        .map(|td| {
            let name = &td.function.name;
            let enabled =
                fastclaw_gateway::routes::agents::tool_effective_enabled(&agent, name);
            json!({
                "id": name,
                "enabled": enabled,
                "description": td.function.description,
            })
        })
        .collect();
    tracing::info!(count = tools.len(), "IPC list_agent_tools returning");
    Ok(json!({ "agentId": agent_id, "tools": tools }))
}

#[tauri::command]
pub async fn get_agent(
    state: tauri::State<'_, AppData>,
    agent_id: String,
) -> Result<serde_json::Value, String> {
    validate_agent_id(&agent_id)?;
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let agent = {
        let router = app.router.read().await;
        router
            .agent_by_id(&agent_id)
            .cloned()
            .ok_or_else(|| format!("agent not found: {agent_id}"))?
    };
    serde_json::to_value(&agent).map_err(|e| format!("serialize: {e}"))
}

#[tauri::command]
pub async fn update_agent_tools(
    state: tauri::State<'_, AppData>,
    agent_id: String,
    tools: Vec<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    validate_agent_id(&agent_id)?;
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let mut agent = {
        let router = app.router.read().await;
        router
            .agent_by_id(&agent_id)
            .cloned()
            .ok_or_else(|| format!("agent not found: {agent_id}"))?
    };

    let registry_names: Vec<String> = app
        .tool_registry
        .definitions()
        .iter()
        .map(|td| td.function.name.clone())
        .collect();

    let toggles: Vec<(String, bool)> = tools
        .into_iter()
        .filter_map(|t| {
            let id = t.get("id")?.as_str()?.to_string();
            let enabled = t.get("enabled")?.as_bool()?;
            Some((id, enabled))
        })
        .collect();

    fastclaw_gateway::routes::agents::rebuild_behavior_tool_lists(
        &mut agent,
        &registry_names,
        &toggles,
    );

    let dir = fastclaw_core::paths::resolve_agents_dir_from(Some(&app.config.paths));
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("create dir: {e}"))?;
    let path = dir.join(format!("{agent_id}.json"));
    let bytes =
        serde_json::to_vec_pretty(&agent).map_err(|e| format!("serialize: {e}"))?;
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|e| format!("write: {e}"))?;

    let count = app.reload_agents().await.map_err(|e| format!("{e}"))?;
    Ok(json!({ "ok": true, "agentId": agent_id, "reloaded": count }))
}

#[tauri::command]
pub async fn list_tools(
    state: tauri::State<'_, AppData>,
) -> Result<serde_json::Value, String> {
    tracing::info!("IPC list_tools called");
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let tools = app.tool_registry.definitions();
    tracing::info!(count = tools.len(), "IPC list_tools returning");
    Ok(json!({ "tools": tools }))
}

#[tauri::command]
pub async fn update_agent(
    state: tauri::State<'_, AppData>,
    agent_id: String,
    config: serde_json::Value,
) -> Result<serde_json::Value, String> {
    validate_agent_id(&agent_id)?;
    tracing::info!(agent_id = %agent_id, "IPC update_agent called");
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;

    let mut agent: fastclaw_core::agent_config::AgentConfig =
        serde_json::from_value(config).map_err(|e| format!("invalid agent config: {e}"))?;
    if agent.agent_id != agent_id {
        agent.agent_id = agent_id.clone();
    }

    let dir = fastclaw_core::paths::resolve_agents_dir_from(Some(&app.config.paths));
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("create dir: {e}"))?;
    let path = dir.join(format!("{agent_id}.json"));
    let bytes = serde_json::to_vec_pretty(&agent).map_err(|e| format!("serialize: {e}"))?;
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|e| format!("write: {e}"))?;
    ensure_agent_workspace_bootstrap(app, &agent_id)?;

    // Sync per-agent channels → global config_live so the running gateway can route.
    sync_agent_channels_to_live(app, &agent_id, &agent.channels);

    let count = app.reload_agents().await.map_err(|e| format!("{e}"))?;
    tracing::info!(agent_id = %agent_id, reloaded = count, "IPC update_agent done");
    Ok(json!({ "ok": true, "agentId": agent_id, "reloaded": count }))
}

#[tauri::command]
pub async fn create_agent(
    state: tauri::State<'_, AppData>,
    config: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;

    let agent: fastclaw_core::agent_config::AgentConfig =
        serde_json::from_value(config).map_err(|e| format!("invalid agent config: {e}"))?;
    let aid = agent.agent_id.clone();
    validate_agent_id(&aid)?;
    tracing::info!(agent_id = %aid, "IPC create_agent called");

    let dir = fastclaw_core::paths::resolve_agents_dir_from(Some(&app.config.paths));
    let path = dir.join(format!("{aid}.json"));
    if path.exists() {
        return Err(format!("agent `{aid}` already exists"));
    }

    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("create dir: {e}"))?;
    let bytes = serde_json::to_vec_pretty(&agent).map_err(|e| format!("serialize: {e}"))?;
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|e| format!("write: {e}"))?;
    ensure_agent_workspace_bootstrap(app, &aid)?;

    let count = app.reload_agents().await.map_err(|e| format!("{e}"))?;
    tracing::info!(agent_id = %aid, reloaded = count, "IPC create_agent done");
    Ok(json!({ "ok": true, "agentId": aid, "reloaded": count }))
}

#[tauri::command]
pub async fn delete_agent(
    state: tauri::State<'_, AppData>,
    agent_id: String,
) -> Result<serde_json::Value, String> {
    validate_agent_id(&agent_id)?;
    tracing::info!(agent_id = %agent_id, "IPC delete_agent called");
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;

    let count_before = {
        let router = app.router.read().await;
        router.agent_count()
    };
    if count_before <= 1 {
        return Err("refusing to delete the last remaining agent".into());
    }

    let dir = fastclaw_core::paths::resolve_agents_dir_from(Some(&app.config.paths));
    let path = dir.join(format!("{agent_id}.json"));
    if !path.exists() {
        return Err(format!("agent config file not found for `{agent_id}`"));
    }

    tokio::fs::remove_file(&path)
        .await
        .map_err(|e| format!("delete: {e}"))?;

    // Clean up channels / bindings belonging to the deleted agent.
    cleanup_agent_channels_from_live(app, &agent_id);

    let count = app.reload_agents().await.map_err(|e| format!("{e}"))?;
    tracing::info!(agent_id = %agent_id, reloaded = count, "IPC delete_agent done");
    Ok(json!({ "ok": true, "agentId": agent_id, "reloaded": count }))
}

// ─── Avatar upload ───

#[tauri::command]
pub async fn upload_agent_avatar(
    state: tauri::State<'_, AppData>,
    agent_id: String,
    source_path: String,
) -> Result<serde_json::Value, String> {
    validate_agent_id(&agent_id)?;
    tracing::info!(agent_id = %agent_id, source = %source_path, "IPC upload_agent_avatar");
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;

    let state_dir = fastclaw_core::paths::resolve_state_dir_from(Some(&app.config.paths));
    let avatars_dir = state_dir.join("avatars");
    tokio::fs::create_dir_all(&avatars_dir)
        .await
        .map_err(|e| format!("create avatars dir: {e}"))?;

    let src = std::path::Path::new(&source_path);
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png");
    let dest = avatars_dir.join(format!("{agent_id}.{ext}"));
    tokio::fs::copy(src, &dest)
        .await
        .map_err(|e| format!("copy avatar: {e}"))?;

    let dest_str = dest.to_string_lossy().to_string();

    // Update agent config with avatar path
    let agents_dir = fastclaw_core::paths::resolve_agents_dir_from(Some(&app.config.paths));
    let cfg_path = agents_dir.join(format!("{agent_id}.json"));
    if cfg_path.exists() {
        let bytes = tokio::fs::read(&cfg_path)
            .await
            .map_err(|e| format!("read agent config: {e}"))?;
        let mut val = serde_json::from_slice::<serde_json::Value>(&bytes)
            .map_err(|e| format!("parse agent config: {e}"))?;
        val["avatar"] = json!(dest_str);
        let out = serde_json::to_vec_pretty(&val)
            .map_err(|e| format!("serialize agent config: {e}"))?;
        tokio::fs::write(&cfg_path, out)
            .await
            .map_err(|e| format!("write agent config: {e}"))?;
    }

    tracing::info!(agent_id = %agent_id, dest = %dest_str, "avatar uploaded");
    Ok(json!({ "ok": true, "path": dest_str }))
}

// ─── Identity files ───

#[tauri::command]
pub async fn read_identity_files(
    state: tauri::State<'_, AppData>,
    agent_id: String,
) -> Result<serde_json::Value, String> {
    validate_agent_id(&agent_id)?;
    tracing::info!(agent_id = %agent_id, "IPC read_identity_files");
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;

    let state_dir = fastclaw_core::paths::resolve_state_dir_from(Some(&app.config.paths));
    let ws_root =
        fastclaw_core::workspace::resolve_workspace_root(&state_dir, &agent_id, None);
    let ws = fastclaw_core::workspace::AgentWorkspace::new(&ws_root, &agent_id);
    let _ = ws.ensure_bootstrap();

    let read = |name: &str| -> serde_json::Value {
        let p = ws_root.join(name);
        match std::fs::read_to_string(&p) {
            Ok(s) if !s.trim().is_empty() => json!(s),
            _ => serde_json::Value::Null,
        }
    };

    Ok(json!({
        "soul": read("SOUL.md"),
        "user": read("USER.md"),
        "agents": read("AGENTS.md"),
    }))
}

// ─── Per-agent channel sync helpers ───

/// Sync an agent's per-agent channels into the global `config_live` so the
/// running gateway can route inbound messages.  Also ensures matching bindings
/// exist.
fn sync_agent_channels_to_live(
    app: &AppState,
    agent_id: &str,
    channels: &std::collections::HashMap<String, fastclaw_core::config::ChannelConfig>,
) {
    if let Ok(mut live) = app.config_live.write() {
        let live_val: &mut serde_json::Value = &mut *live;

        // Merge channels into global config_live.channels
        if let Some(obj) = live_val.get_mut("channels").and_then(|v: &mut serde_json::Value| v.as_object_mut()) {
            for (ch_id, ch_cfg) in channels {
                if let Ok(val) = serde_json::to_value(ch_cfg) {
                    obj.insert(ch_id.clone(), val);
                }
            }
        } else if !channels.is_empty() {
            let mut obj = serde_json::Map::new();
            for (ch_id, ch_cfg) in channels {
                if let Ok(val) = serde_json::to_value(ch_cfg) {
                    obj.insert(ch_id.clone(), val);
                }
            }
            live_val["channels"] = serde_json::Value::Object(obj);
        }

        // Ensure bindings exist for each channel
        if live_val.get("bindings").is_none() {
            live_val["bindings"] = json!([]);
        }
        if let Some(arr) = live_val.get_mut("bindings").and_then(|v: &mut serde_json::Value| v.as_array_mut()) {
            // Remove old bindings for this agent
            arr.retain(|b: &serde_json::Value| {
                b.get("agentId").and_then(|a: &serde_json::Value| a.as_str()) != Some(agent_id)
            });
            // Re-add for current channels
            for ch_id in channels.keys() {
                arr.push(json!({
                    "agentId": agent_id,
                    "match": { "channel": ch_id }
                }));
            }
        }

        tracing::info!(
            agent_id,
            channel_count = channels.len(),
            "synced per-agent channels to config_live"
        );
    }
}

/// Remove all channels and bindings belonging to a deleted agent from `config_live`.
fn cleanup_agent_channels_from_live(
    app: &AppState,
    agent_id: &str,
) {
    if let Ok(mut live) = app.config_live.write() {
        if let Some(arr) = live.get_mut("bindings").and_then(|v: &mut serde_json::Value| v.as_array_mut()) {
            arr.retain(|b: &serde_json::Value| {
                b.get("agentId").and_then(|a: &serde_json::Value| a.as_str()) != Some(agent_id)
            });
        }
        tracing::info!(agent_id, "cleaned up channel bindings for deleted agent");
    }
}

// ─── Channel bindings ───

#[tauri::command]
pub async fn list_channels(
    state: tauri::State<'_, AppData>,
) -> Result<serde_json::Value, String> {
    tracing::info!("IPC list_channels");
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;

    let channels_val = app
        .config_live
        .read()
        .map_err(|e| format!("lock: {e}"))?
        .get("channels")
        .cloned()
        .unwrap_or(json!({}));

    let bindings_val = app
        .config_live
        .read()
        .map_err(|e| format!("lock: {e}"))?
        .get("bindings")
        .cloned()
        .unwrap_or(json!([]));

    Ok(json!({ "channels": channels_val, "bindings": bindings_val }))
}

#[tauri::command]
pub async fn bind_agent_channel(
    state: tauri::State<'_, AppData>,
    agent_id: String,
    channel_id: String,
) -> Result<serde_json::Value, String> {
    validate_agent_id(&agent_id)?;
    tracing::info!(agent_id = %agent_id, channel_id = %channel_id, "IPC bind_agent_channel");
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;

    let serialized = {
        let mut live = app.config_live.write().map_err(|e| format!("lock: {e}"))?;

        let new_binding = json!({
            "agentId": agent_id,
            "match": { "channel": channel_id }
        });

        let bindings = live.get_mut("bindings").and_then(|v| v.as_array_mut());
        if let Some(arr) = bindings {
            let already = arr.iter().any(|b| {
                b.get("agentId").and_then(|a| a.as_str()) == Some(&agent_id)
                    && b.get("match")
                        .and_then(|m| m.get("channel"))
                        .and_then(|c| c.as_str())
                        == Some(&channel_id)
            });
            if !already {
                arr.push(new_binding);
            }
        } else {
            live["bindings"] = json!([new_binding]);
        }

        serde_json::to_vec_pretty(&*live).map_err(|e| format!("serialize config: {e}"))?
    };

    let cfg_dir = fastclaw_core::paths::resolve_config_dir_from(Some(&app.config.paths));
    let cfg_path = cfg_dir.join("default.json");
    tokio::fs::write(&cfg_path, serialized)
        .await
        .map_err(|e| format!("write config: {e}"))?;

    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub async fn unbind_agent_channel(
    state: tauri::State<'_, AppData>,
    agent_id: String,
    channel_id: String,
) -> Result<serde_json::Value, String> {
    validate_agent_id(&agent_id)?;
    tracing::info!(agent_id = %agent_id, channel_id = %channel_id, "IPC unbind_agent_channel");
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;

    let serialized = {
        let mut live = app.config_live.write().map_err(|e| format!("lock: {e}"))?;

        if let Some(arr) = live.get_mut("bindings").and_then(|v| v.as_array_mut()) {
            arr.retain(|b| {
                !(b.get("agentId").and_then(|a| a.as_str()) == Some(&agent_id)
                    && b.get("match")
                        .and_then(|m| m.get("channel"))
                        .and_then(|c| c.as_str())
                        == Some(&channel_id))
            });
        }

        serde_json::to_vec_pretty(&*live).map_err(|e| format!("serialize config: {e}"))?
    };

    let cfg_dir = fastclaw_core::paths::resolve_config_dir_from(Some(&app.config.paths));
    let cfg_path = cfg_dir.join("default.json");
    tokio::fs::write(&cfg_path, serialized)
        .await
        .map_err(|e| format!("write config: {e}"))?;

    Ok(json!({ "ok": true }))
}

// ─── Chat streaming via Tauri Channel ───

#[tauri::command]
pub async fn chat_stream(
    state: tauri::State<'_, AppData>,
    app_handle: tauri::AppHandle,
    channel: tauri::ipc::Channel<serde_json::Value>,
    messages: Vec<serde_json::Value>,
    agent_id: Option<String>,
    session_id: Option<String>,
    model: Option<String>,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    work_dir: Option<String>,
    request_id: Option<String>,
) -> Result<(), String> {
    let work_dir = work_dir;
    use fastclaw_core::types::{ChatMessage, ChatRequest, Role, StreamEvent};
    use fastclaw_gateway::chat_pipeline::{
        after_chat, maybe_spawn_smart_title_background, setup_chat, SetupChatOptions,
    };
    use fastclaw_gateway::routes::record_chat_budget_stream_estimate;

    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?.clone();
    drop(gw);
    let stream_request_id = request_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
    {
        let mut cancels = state.stream_cancels.lock().await;
        if let Some(prev) = cancels.insert(stream_request_id.clone(), cancel_tx) {
            let _ = prev.send(());
        }
    }

    let chat_messages: Vec<ChatMessage> = messages
        .into_iter()
        .map(|m| {
            let role_str = m.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            let role = match role_str {
                "system" => Role::System,
                "assistant" => Role::Assistant,
                "tool" => Role::Tool,
                _ => Role::User,
            };
            let content = m.get("content").cloned();
            let name = m.get("name").and_then(|v| v.as_str()).map(String::from);
            let tool_call_id = m
                .get("tool_call_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            ChatMessage {
                role,
                content,
                name,
                tool_calls: None,
                tool_call_id,
            }
        })
        .collect();

    let request = ChatRequest {
        messages: chat_messages,
        model,
        stream: true,
        max_tokens,
        temperature,
        agent_id,
        session_id,
        tools: None,
        slash_intent: None,
        work_dir,
    };

    let setup = setup_chat(
        &app,
        &request,
        SetupChatOptions {
            chat_stream: true,
            propagate_context_ingest_errors: false,
            set_resolved_session_on_request: true,
            record_chat_observe: false,
        },
    )
    .await
    .map_err(|e| e.to_string())?;

    let session_id = setup.session_id.clone();
    let agent_id = setup.agent_id.clone();
    let needs_title = setup.needs_title;
    let model_for_budget = setup.model_for_budget.clone();
    let input_estimate = setup.input_estimate;
    let budget_degraded = setup.budget_degraded;
    let mut reserved = setup.reserved_cost;
    let agent_config = setup.agent_config.clone();
    let enriched = setup.enriched_request.clone();
    let after_turn_messages = setup.enriched_request.messages.clone();
    let context_tokens_est = setup.context_tokens_estimate;

    for msg in &setup.user_messages {
        if let Err(e) = app.session_store.append_message(&session_id, msg).await {
            tracing::error!(session_id = %session_id, error = %e, "failed to persist user message");
        }
    }

    let start_model = enriched
        .model
        .as_deref()
        .unwrap_or(agent_config.model.model.as_str());

    let mut start_payload = json!({
        "model": start_model,
        "sessionId": &session_id,
        "resolvedAgent": &agent_id,
    });
    if budget_degraded {
        start_payload["budgetDegraded"] = json!(true);
    }
    let _ = channel.send(json!({"type": "chat.start", "data": start_payload}));

    let (tx, mut rx) = tokio::sync::mpsc::channel::<StreamEvent>(64);

    let stream_context_key = uuid::Uuid::new_v4().to_string();
    {
        let mut slot = app.stream_event_tx.lock().await;
        slot.insert(stream_context_key.clone(), tx.clone());
    }

    let runtime = app.runtime.clone();
    let tool_reg = app.tool_registry.clone();
    let llm_override = setup.llm_override.clone();
    let stream_event_tx_ref = app.stream_event_tx.clone();
    let stream_context_key_for_task = stream_context_key.clone();
    let stream_request_id_for_task = stream_request_id.clone();
    let stream_cancel_map_for_task = state.stream_cancels.clone();
    let confirm_pending_for_task = app.ask_question_pending.clone();

    let task = tokio::spawn(async move {
        let result = tokio::select! {
            result = fastclaw_agent::builtin_tools::with_stream_context(
                stream_context_key_for_task.clone(),
                runtime.execute_stream_with_confirm(&agent_config, &enriched, &tool_reg, tx, llm_override, confirm_pending_for_task),
            ) => result,
            _ = &mut cancel_rx => Err(anyhow::anyhow!("cancelled")),
        };
        stream_event_tx_ref
            .lock()
            .await
            .remove(&stream_context_key_for_task);
        stream_cancel_map_for_task
            .lock()
            .await
            .remove(&stream_request_id_for_task);
        result
    });

    let mut assistant_content = String::new();
    let mut pending_question_ids: Vec<String> = Vec::new();
    let mut last_checkpoint = std::time::Instant::now();
    let checkpoint_interval = std::time::Duration::from_secs(5);
    // (call_id, tool_name, args_json, result_output, success)
    let mut accumulated_tool_calls: Vec<(String, String, Option<String>, Option<String>, bool)> = Vec::new();
    while let Some(event) = rx.recv().await {
        match &event {
            StreamEvent::Delta(delta) => {
                if let Some(text) = delta
                    .choices
                    .first()
                    .and_then(|c| c.delta.content.as_deref())
                {
                    assistant_content.push_str(text);
                }
                if !assistant_content.is_empty() && last_checkpoint.elapsed() >= checkpoint_interval {
                    last_checkpoint = std::time::Instant::now();
                    let _ = app.session_store.save_partial_assistant(&session_id, &assistant_content).await;
                }
                let _ = channel.send(json!({
                    "type": "chat.delta",
                    "data": {"content": delta.choices.first().and_then(|c| c.delta.content.as_deref()), "model": delta.model}
                }));
            }
            StreamEvent::ToolExecuting {
                tool_name,
                call_id,
                args,
            } => {
                accumulated_tool_calls.push((call_id.clone(), tool_name.clone(), args.clone(), None, true));
                let _ = channel.send(json!({
                    "type": "chat.tool.start",
                    "data": {"tool": tool_name, "callId": call_id, "args": args}
                }));
            }
            StreamEvent::ToolResult {
                tool_name,
                call_id,
                output,
                success,
            } => {
                if let Some(tc) = accumulated_tool_calls.iter_mut().find(|(cid, _, _, _, _)| cid == call_id) {
                    tc.3 = Some(output.clone());
                    tc.4 = *success;
                }
                let _ = channel.send(json!({
                    "type": "chat.tool.done",
                    "data": {"tool": tool_name, "callId": call_id, "output": output, "success": success}
                }));
            }
            StreamEvent::Done {
                session_id: sid,
                tool_calls_made,
                iterations,
                usage,
                elapsed_ms,
                ..
            } => {
                record_chat_budget_stream_estimate(
                    &app,
                    model_for_budget.as_str(),
                    input_estimate,
                    assistant_content.len(),
                );

                if !assistant_content.is_empty() {
                    let _ = app.session_store.remove_partial_assistant(&session_id).await;
                    let saved_tool_calls: Option<Vec<fastclaw_core::types::ToolCall>> = if accumulated_tool_calls.is_empty() {
                        None
                    } else {
                        Some(accumulated_tool_calls.iter().map(|(cid, tname, args, output, success)| {
                            fastclaw_core::types::ToolCall {
                                id: cid.clone(),
                                call_type: "function".to_string(),
                                function: fastclaw_core::types::FunctionCall {
                                    name: tname.clone(),
                                    arguments: args.clone().unwrap_or_default(),
                                },
                                output: output.clone(),
                                success: Some(*success),
                                duration_ms: None,
                            }
                        }).collect())
                    };
                    let assistant_msg = ChatMessage {
                        role: Role::Assistant,
                        content: Some(serde_json::Value::String(assistant_content.clone())),
                        name: None,
                        tool_calls: saved_tool_calls,
                        tool_call_id: None,
                    };
                    let _ = after_chat(&app, &setup, &assistant_msg, false).await;
                }

                let mut complete_data = json!({
                    "sessionId": sid,
                    "toolCallsMade": tool_calls_made,
                    "iterations": iterations,
                    "elapsedMs": elapsed_ms,
                });
                if let Some(ref u) = usage {
                    complete_data["usage"] = json!({
                        "promptTokens": u.prompt_tokens,
                        "completionTokens": u.completion_tokens,
                        "totalTokens": u.total_tokens,
                    });
                }
                if let Some((est_tokens, ctx_window)) = context_tokens_est {
                    let actual_prompt = usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0);
                    complete_data["contextTokens"] = json!(if actual_prompt > 0 { actual_prompt } else { est_tokens });
                    if ctx_window > 0 {
                        complete_data["contextWindow"] = json!(ctx_window);
                    }
                }
                let _ = channel.send(json!({
                    "type": "chat.complete",
                    "data": complete_data,
                }));

                // Persist usage metrics
                if let Some(ref sid_str) = sid {
                    let pt = usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0);
                    let ct = usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0);
                    let _ = app.session_store.accumulate_usage(sid_str, pt, ct, *elapsed_ms).await;
                }

                // Emit Tauri event for session change
                let _ = app_handle.emit(
                    "sessions-changed",
                    json!({"sessionId": sid}),
                );
            }
            StreamEvent::AskQuestion {
                request_id,
                question,
                options,
                timeout_secs,
                allow_multiple,
            } => {
                pending_question_ids.push(request_id.clone());
                let _ = channel.send(json!({
                    "type": "chat.ask_question",
                    "data": {
                        "requestId": request_id,
                        "question": question,
                        "options": options,
                        "timeoutSecs": timeout_secs,
                        "allowMultiple": allow_multiple,
                    }
                }));
            }
            StreamEvent::Error(e) => {
                if reserved > 0.0 {
                    let _ = app.budget_tracker.release_reservation(reserved);
                    reserved = 0.0;
                }
                if !assistant_content.is_empty() {
                    let _ = app.session_store.remove_partial_assistant(&session_id).await;
                    let err_tc = if accumulated_tool_calls.is_empty() { None } else {
                        Some(accumulated_tool_calls.iter().map(|(cid, tname, args, o, s)| {
                            fastclaw_core::types::ToolCall { id: cid.clone(), call_type: "function".into(), function: fastclaw_core::types::FunctionCall { name: tname.clone(), arguments: args.clone().unwrap_or_default() }, output: o.clone(), success: Some(*s), duration_ms: None }
                        }).collect())
                    };
                    let assistant_msg = ChatMessage {
                        role: Role::Assistant,
                        content: Some(serde_json::Value::String(assistant_content.clone())),
                        name: None,
                        tool_calls: err_tc,
                        tool_call_id: None,
                    };
                    let _ = after_chat(&app, &setup, &assistant_msg, false).await;
                }
                let _ = channel.send(json!({
                    "type": "chat.error",
                    "error": {"message": e.to_string()}
                }));
            }
        }
    }

    if !assistant_content.is_empty() {
        let _ = app
            .context_engine
            .after_turn(&after_turn_messages, &agent_id, &session_id)
            .await;
    }

    if needs_title && !assistant_content.is_empty() {
        maybe_spawn_smart_title_background(&app, &setup, &assistant_content);
    }

    let build_tc_for_persist = || -> Option<Vec<fastclaw_core::types::ToolCall>> {
        if accumulated_tool_calls.is_empty() { return None; }
        Some(accumulated_tool_calls.iter().map(|(cid, tname, args, o, s)| {
            fastclaw_core::types::ToolCall { id: cid.clone(), call_type: "function".into(), function: fastclaw_core::types::FunctionCall { name: tname.clone(), arguments: args.clone().unwrap_or_default() }, output: o.clone(), success: Some(*s), duration_ms: None }
        }).collect())
    };

    match task.await {
        Ok(Err(e)) => {
            if reserved > 0.0 {
                let _ = app.budget_tracker.release_reservation(reserved);
            }
            if !assistant_content.is_empty() {
                let _ = app.session_store.remove_partial_assistant(&session_id).await;
                let assistant_msg = ChatMessage {
                    role: Role::Assistant,
                    content: Some(serde_json::Value::String(assistant_content.clone())),
                    name: None,
                    tool_calls: build_tc_for_persist(),
                    tool_call_id: None,
                };
                let _ = after_chat(&app, &setup, &assistant_msg, false).await;
            }
            let _ = channel.send(json!({
                "type": "chat.error",
                "error": {"message": format!("{e}")}
            }));
        }
        Err(e) => {
            if reserved > 0.0 {
                let _ = app.budget_tracker.release_reservation(reserved);
            }
            if !assistant_content.is_empty() {
                let _ = app.session_store.remove_partial_assistant(&session_id).await;
                let assistant_msg = ChatMessage {
                    role: Role::Assistant,
                    content: Some(serde_json::Value::String(std::mem::take(&mut assistant_content))),
                    name: None,
                    tool_calls: build_tc_for_persist(),
                    tool_call_id: None,
                };
                let _ = after_chat(&app, &setup, &assistant_msg, false).await;
            }
            let _ = channel.send(json!({
                "type": "chat.error",
                "error": {"message": format!("task panic: {e}")}
            }));
        }
        _ => {}
    }

    if !pending_question_ids.is_empty() {
        let mut pending = app.ask_question_pending.lock().await;
        for request_id in pending_question_ids {
            pending.remove(&request_id);
        }
    }
    state.stream_cancels.lock().await.remove(&stream_request_id);

    Ok(())
}

#[tauri::command]
pub async fn cancel_chat_stream(
    state: tauri::State<'_, AppData>,
    request_id: String,
) -> Result<serde_json::Value, String> {
    let sender = state.stream_cancels.lock().await.remove(&request_id);
    let cancelled = if let Some(tx) = sender {
        tx.send(()).is_ok()
    } else {
        false
    };
    Ok(json!({ "ok": true, "cancelled": cancelled }))
}

// ─── Ask Question answer submission ───

#[tauri::command]
pub async fn submit_tool_answer(
    state: tauri::State<'_, AppData>,
    request_id: String,
    answer: String,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let sender = app.ask_question_pending.lock().await.remove(&request_id);
    if let Some(tx) = sender {
        let _ = tx.send(answer);
        Ok(json!({ "ok": true }))
    } else {
        Ok(json!({ "ok": false, "reason": "request not found or already answered" }))
    }
}

fn persist_config_key(key: &str, value: &serde_json::Value) -> anyhow::Result<()> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("cannot resolve home directory"))?;
    let cfg_path = home.join(".fastclaw/config/default.json");
    let mut cfg_value: serde_json::Value = if cfg_path.exists() {
        let text = std::fs::read_to_string(&cfg_path)?;
        json5::from_str(&text).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    set_nested_key(&mut cfg_value, key, value.clone())
        .map_err(|_| anyhow::anyhow!("failed to set nested key"))?;

    if let Some(parent) = cfg_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&cfg_path, serde_json::to_string_pretty(&cfg_value)?)?;
    Ok(())
}

// ─── MCP server management ───

#[tauri::command]
pub async fn get_mcp_status(
    state: tauri::State<'_, AppData>,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let status = app
        .mcp_status
        .read()
        .map_err(|e| format!("lock: {e}"))?;
    let list: Vec<&fastclaw_core::types::McpServerStatus> = status.values().collect();
    serde_json::to_value(&list).map_err(|e| format!("serialize: {e}"))
}

#[tauri::command]
pub async fn reload_mcp_servers(
    state: tauri::State<'_, AppData>,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?.clone();
    drop(gw);
    app.reload_mcp_servers()
        .await
        .map_err(|e| format!("{e}"))?;
    let status = app
        .mcp_status
        .read()
        .map_err(|e| format!("lock: {e}"))?;
    let list: Vec<&fastclaw_core::types::McpServerStatus> = status.values().collect();
    serde_json::to_value(&list).map_err(|e| format!("serialize: {e}"))
}

#[tauri::command]
pub async fn add_mcp_server(
    state: tauri::State<'_, AppData>,
    id: String,
    command: String,
    args: Option<Vec<String>>,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?.clone();
    drop(gw);

    let new_server = fastclaw_core::agent_config::McpServerConfig {
        id: id.clone(),
        command,
        args: args.unwrap_or_default(),
        enabled: Some(true),
        env: Default::default(),
    };

    {
        let mut live = app
            .config_live
            .write()
            .map_err(|e| format!("lock: {e}"))?;
        let arr = live
            .get_mut("mcpServers")
            .and_then(|v| v.as_array_mut());
        let server_val =
            serde_json::to_value(&new_server).map_err(|e| format!("serialize: {e}"))?;
        if let Some(arr) = arr {
            arr.retain(|v| v.get("id").and_then(|i| i.as_str()) != Some(&id));
            arr.push(server_val);
        } else {
            live["mcpServers"] = json!([server_val]);
        }
    }

    if let Err(e) = persist_config_key("mcpServers", &{
        let live = app
            .config_live
            .read()
            .map_err(|e| format!("lock: {e}"))?;
        live.get("mcpServers").cloned().unwrap_or(json!([]))
    }) {
        tracing::warn!(error = %e, "failed to persist mcpServers");
    }

    app.reload_mcp_servers()
        .await
        .map_err(|e| format!("{e}"))?;

    let status = app
        .mcp_status
        .read()
        .map_err(|e| format!("lock: {e}"))?;
    let server_status = status.get(&id).cloned();
    Ok(json!({
        "ok": true,
        "id": id,
        "status": server_status,
    }))
}

#[tauri::command]
pub async fn remove_mcp_server(
    state: tauri::State<'_, AppData>,
    id: String,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?.clone();
    drop(gw);

    {
        let mut live = app
            .config_live
            .write()
            .map_err(|e| format!("lock: {e}"))?;
        if let Some(arr) = live.get_mut("mcpServers").and_then(|v| v.as_array_mut()) {
            arr.retain(|v| v.get("id").and_then(|i| i.as_str()) != Some(&id));
        }
    }

    if let Err(e) = persist_config_key("mcpServers", &{
        let live = app
            .config_live
            .read()
            .map_err(|e| format!("lock: {e}"))?;
        live.get("mcpServers").cloned().unwrap_or(json!([]))
    }) {
        tracing::warn!(error = %e, "failed to persist mcpServers");
    }

    app.reload_mcp_servers()
        .await
        .map_err(|e| format!("{e}"))?;

    Ok(json!({ "ok": true, "id": id }))
}

// ─── Cron job IPC commands ────────────────────────────────────────────────

#[tauri::command]
pub async fn cron_list_jobs(
    state: tauri::State<'_, AppData>,
    agent_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let jobs = if let Some(aid) = agent_id {
        app.cron_store
            .list_by_agent(&aid)
            .await
            .map_err(|e| format!("{e}"))?
    } else {
        app.cron_store.list().await.map_err(|e| format!("{e}"))?
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
        .cron_store
        .get(&job_id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| format!("cron job not found: {job_id}"))?;
    Ok(serde_json::to_value(job).map_err(|e| format!("{e}"))?)
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
    cron::Schedule::from_str(&job.schedule)
        .map_err(|e| format!("invalid cron expression: {e}"))?;

    if let fastclaw_cron::JobAction::Webhook { ref url, .. } = job.action {
        fastclaw_security::ssrf::ssrf_check_url(url)
            .map_err(|e| format!("webhook URL rejected: {e}"))?;
    }

    app.cron_store
        .upsert(&job)
        .await
        .map_err(|e| format!("{e}"))?;

    app.cron_wake.notify_one();

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
        .cron_store
        .list_runs(&job_id, limit.unwrap_or(20))
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(json!({ "runs": runs, "count": runs.len() }))
}

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
        .notification_store
        .list(limit.unwrap_or(30), offset.unwrap_or(0), unread_only.unwrap_or(false))
        .await
        .map_err(|e| format!("{e}"))?;
    let unread = app.notification_store.unread_count().await.unwrap_or(0);
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
    app.notification_store
        .mark_read(&id)
        .await
        .map_err(|e| format!("{e}"))?;
    let unread = app.notification_store.unread_count().await.unwrap_or(0);

    let event = serde_json::json!({
        "type": "event",
        "event": "notification.read",
        "data": { "id": id, "unreadCount": unread }
    });
    let _ = app.ws_broadcast.send(event.to_string());

    Ok(json!({ "ok": true, "unreadCount": unread }))
}

#[tauri::command]
pub async fn notification_mark_all_read(
    state: tauri::State<'_, AppData>,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    app.notification_store
        .mark_all_read()
        .await
        .map_err(|e| format!("{e}"))?;
    let unread = app.notification_store.unread_count().await.unwrap_or(0);

    let event = serde_json::json!({
        "type": "event",
        "event": "notification.read",
        "data": { "unreadCount": unread }
    });
    let _ = app.ws_broadcast.send(event.to_string());

    Ok(json!({ "ok": true, "unreadCount": unread }))
}

#[tauri::command]
pub async fn notification_unread_count(
    state: tauri::State<'_, AppData>,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let count = app.notification_store.unread_count().await.map_err(|e| format!("{e}"))?;
    Ok(json!({ "count": count }))
}

#[tauri::command]
pub async fn notification_delete(
    state: tauri::State<'_, AppData>,
    id: String,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let deleted = app.notification_store.delete(&id).await.map_err(|e| format!("{e}"))?;
    Ok(json!({ "ok": deleted }))
}

#[tauri::command]
pub async fn notification_clear_read(
    state: tauri::State<'_, AppData>,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let cleared = app.notification_store.clear_read().await.map_err(|e| format!("{e}"))?;
    Ok(json!({ "ok": true, "cleared": cleared }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastclaw_core::config_access::mask_secret_values;

    // ═══════════════════════════════════════════════════════════════════
    // navigate_config
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn navigate_config_single_level() {
        let cfg = json!({"gateway": {"port": 18789}});
        assert_eq!(navigate_config(&cfg, "gateway"), json!({"port": 18789}));
    }

    #[test]
    fn navigate_config_nested_path() {
        let cfg = json!({"gateway": {"port": 18789, "host": "127.0.0.1"}});
        assert_eq!(navigate_config(&cfg, "gateway.port"), json!(18789));
    }

    #[test]
    fn navigate_config_deeply_nested() {
        let cfg = json!({"a": {"b": {"c": {"d": 42}}}});
        assert_eq!(navigate_config(&cfg, "a.b.c.d"), json!(42));
    }

    #[test]
    fn navigate_config_missing_intermediate_returns_null() {
        let cfg = json!({"a": {"b": 1}});
        assert!(navigate_config(&cfg, "a.x.y").is_null());
    }

    #[test]
    fn navigate_config_missing_leaf_returns_null() {
        let cfg = json!({"gateway": {}});
        assert!(navigate_config(&cfg, "gateway.missing").is_null());
    }

    #[test]
    fn navigate_config_empty_key_returns_null() {
        let cfg = json!({"x": 1});
        let result = navigate_config(&cfg, "");
        assert!(result.is_null(), "empty key matches no object key → null");
    }

    #[test]
    fn navigate_config_non_object_root() {
        let cfg = json!("just a string");
        assert!(navigate_config(&cfg, "anything").is_null());
    }

    #[test]
    fn navigate_config_array_value() {
        let cfg = json!({"items": [1, 2, 3]});
        assert_eq!(navigate_config(&cfg, "items"), json!([1, 2, 3]));
    }

    // ═══════════════════════════════════════════════════════════════════
    // set_nested_key
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn set_nested_key_simple_overwrite() {
        let mut root = json!({"a": {"b": 1}});
        set_nested_key(&mut root, "a.b", json!(2)).unwrap();
        assert_eq!(root["a"]["b"], 2);
    }

    #[test]
    fn set_nested_key_creates_intermediate_objects() {
        let mut root = json!({});
        set_nested_key(&mut root, "x.y.z", json!("hello")).unwrap();
        assert_eq!(root["x"]["y"]["z"], "hello");
    }

    #[test]
    fn set_nested_key_top_level() {
        let mut root = json!({"a": 1});
        set_nested_key(&mut root, "b", json!(2)).unwrap();
        assert_eq!(root["b"], 2);
        assert_eq!(root["a"], 1);
    }

    #[test]
    fn set_nested_key_overwrites_non_object_intermediate() {
        let mut root = json!({"a": "string"});
        set_nested_key(&mut root, "a.b", json!(1)).unwrap();
        assert_eq!(root["a"]["b"], 1);
    }

    #[test]
    fn set_nested_key_preserves_siblings() {
        let mut root = json!({"a": {"b": 1, "c": 2}});
        set_nested_key(&mut root, "a.b", json!(99)).unwrap();
        assert_eq!(root["a"]["b"], 99);
        assert_eq!(root["a"]["c"], 2);
    }

    #[test]
    fn set_nested_key_array_value() {
        let mut root = json!({});
        set_nested_key(&mut root, "items", json!([1, 2, 3])).unwrap();
        assert_eq!(root["items"], json!([1, 2, 3]));
    }

    // ═══════════════════════════════════════════════════════════════════
    // mask_secret_values
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn mask_secrets_long_api_key() {
        let val = json!({"openai": {"apiKey": "sk-1234567890abcdef", "baseUrl": "https://api.openai.com/v1"}});
        let masked = mask_secret_values(&val);
        let key = masked["openai"]["apiKey"].as_str().unwrap();
        assert!(key.contains("…"), "long key should be partially masked");
        assert!(key.starts_with("sk-1"));
        assert!(key.ends_with("cdef"));
        assert_eq!(masked["openai"]["baseUrl"], "https://api.openai.com/v1");
    }

    #[test]
    fn mask_secrets_short_api_key() {
        let val = json!({"apiKey": "short"});
        let masked = mask_secret_values(&val);
        assert_eq!(masked["apiKey"], "****");
    }

    #[test]
    fn mask_secrets_empty_key_unchanged() {
        let val = json!({"apiKey": ""});
        let masked = mask_secret_values(&val);
        assert_eq!(masked["apiKey"], "");
    }

    #[test]
    fn mask_secrets_app_secret_field() {
        let val = json!({"appSecret": "0123456789abcdef"});
        let masked = mask_secret_values(&val);
        let s = masked["appSecret"].as_str().unwrap();
        assert!(s.contains("…"), "appSecret should be masked");
    }

    #[test]
    fn mask_secrets_api_key_snake_case() {
        let val = json!({"api_key": "a1b2c3d4e5f6g7h8"});
        let masked = mask_secret_values(&val);
        let s = masked["api_key"].as_str().unwrap();
        assert!(s.contains("…"));
    }

    #[test]
    fn mask_secrets_non_string_value_unchanged() {
        let val = json!({"apiKey": 12345});
        let masked = mask_secret_values(&val);
        assert_eq!(masked["apiKey"], 12345);
    }

    #[test]
    fn mask_secrets_nested_arrays() {
        let val = json!({"providers": [
            {"name": "openai", "apiKey": "sk-xxxxxxxxxxxxxxxx"},
            {"name": "anthropic", "apiKey": "sk-ant-yyyyyyyyyyyy"}
        ]});
        let masked = mask_secret_values(&val);
        for item in masked["providers"].as_array().unwrap() {
            let key = item["apiKey"].as_str().unwrap();
            assert!(key.contains("…") || key == "****");
        }
    }

    #[test]
    fn mask_secrets_non_secret_fields_untouched() {
        let val = json!({"name": "test", "baseUrl": "http://example.com", "model": "gpt-4"});
        let masked = mask_secret_values(&val);
        assert_eq!(masked, val);
    }

    // ═══════════════════════════════════════════════════════════════════
    // filter_config_for_read
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn filter_config_includes_all_readable_keys() {
        let mut cfg = json!({});
        for key in CONFIG_READABLE_KEYS {
            cfg[key] = json!({"dummy": true});
        }
        cfg["dangerousInternal"] = json!("should not appear");
        let filtered = filter_config_for_read(&cfg);
        for key in CONFIG_READABLE_KEYS {
            assert!(filtered.get(key).is_some(), "should include {key}");
        }
        assert!(filtered.get("dangerousInternal").is_none());
    }

    #[test]
    fn filter_config_masks_credentials() {
        let cfg = json!({
            "credentials": {"openai": {"apiKey": "sk-1234567890abcdef"}},
            "models": {"openai": {"apiKey": "sk-9876543210fedcba"}},
            "gateway": {"port": 18789}
        });
        let filtered = filter_config_for_read(&cfg);
        let cred_key = filtered["credentials"]["openai"]["apiKey"].as_str().unwrap();
        assert!(cred_key.contains("…"), "credentials should be masked");
        let model_key = filtered["models"]["openai"]["apiKey"].as_str().unwrap();
        assert!(model_key.contains("…"), "models should be masked");
        assert_eq!(filtered["gateway"]["port"], 18789, "gateway not masked");
    }

    #[test]
    fn filter_config_missing_keys_omitted() {
        let cfg = json!({"gateway": {"port": 18789}});
        let filtered = filter_config_for_read(&cfg);
        assert!(filtered.get("gateway").is_some());
        assert!(filtered.get("logging").is_none());
    }

    #[test]
    fn filter_config_empty_config() {
        let cfg = json!({});
        let filtered = filter_config_for_read(&cfg);
        assert!(filtered.as_object().unwrap().is_empty());
    }

    #[test]
    fn filter_config_non_object_returns_empty() {
        let cfg = json!("not an object");
        let filtered = filter_config_for_read(&cfg);
        assert!(filtered.as_object().unwrap().is_empty());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Config key ACL
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn writable_keys_are_subset_of_readable() {
        for key in CONFIG_WRITABLE_KEYS {
            assert!(
                CONFIG_READABLE_KEYS.contains(key),
                "writable key '{key}' must also be readable"
            );
        }
    }

    #[test]
    fn readable_keys_include_expected_entries() {
        for expected in ["gateway", "logging", "session", "memory", "models",
                         "credentials", "modelRouter", "evolution", "webSearch", "security"] {
            assert!(CONFIG_READABLE_KEYS.contains(&expected), "missing readable key: {expected}");
        }
    }

    #[test]
    fn writable_keys_include_expected_entries() {
        for expected in ["logging", "session", "memory", "credentials", "models",
                         "modelRouter", "evolution", "webSearch", "security"] {
            assert!(CONFIG_WRITABLE_KEYS.contains(&expected), "missing writable key: {expected}");
        }
    }

    #[test]
    fn gateway_is_not_writable() {
        assert!(
            !CONFIG_WRITABLE_KEYS.contains(&"gateway"),
            "gateway should be read-only"
        );
    }

    #[test]
    fn security_is_readable_and_writable() {
        assert!(CONFIG_READABLE_KEYS.contains(&"security"));
        assert!(CONFIG_WRITABLE_KEYS.contains(&"security"));
    }

    // ═══════════════════════════════════════════════════════════════════
    // get_state helper
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn get_state_none_returns_error() {
        let gw: Option<crate::embedded::EmbeddedGateway> = None;
        let result = get_state(&gw);
        assert!(result.is_err());
        match result {
            Err(msg) => assert_eq!(msg, "gateway not started"),
            Ok(_) => panic!("expected error"),
        }
    }
}
