use crate::AppData;
use serde_json::json;
use super::helpers::{
    cleanup_agent_channels_from_live, ensure_agent_workspace_bootstrap, get_state, sync_agent_channels_to_live,
    validate_agent_id,
};

// ─── Agents ───

#[tauri::command]
pub async fn list_agents(
    state: tauri::State<'_, AppData>,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let agents: Vec<_> = app
        .rt
        .router
        .read()
        .await
        .list_agents()
        .iter()
        .map(|a| json!({"agentId": a.agent_id, "name": a.name, "model": a.model.model, "avatar": a.avatar}))
        .collect();
    Ok(json!({"agents": agents}))
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
        let router = app.rt.router.read().await;
        router
            .agent_by_id(&agent_id)
            .cloned()
            .ok_or_else(|| format!("agent not found: {agent_id}"))?
    };
    let tools: Vec<serde_json::Value> = app
        .rt
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
    let mut agent = {
        let router = app.rt.router.read().await;
        router
            .agent_by_id(&agent_id)
            .cloned()
            .ok_or_else(|| format!("agent not found: {agent_id}"))?
    };

    // Merge channels from global config_live that are bound to this agent.
    // This ensures channels added via the `add_channel` tool are visible.
    let live_snapshot = app.cfg.config_live.load();
    if let Some(bindings) = live_snapshot.get("bindings").and_then(|v| v.as_array()) {
        let global_channels = live_snapshot.get("channels").and_then(|v| v.as_object());
        if let Some(ch_obj) = global_channels {
            for binding in bindings {
                let bound_agent = binding
                    .get("agentId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("main");
                if bound_agent != agent_id {
                    continue;
                }
                let ch_id = binding
                    .get("match")
                    .and_then(|m| m.get("channel"))
                    .and_then(|v| v.as_str());
                if let Some(ch_id) = ch_id {
                    if !agent.channels.contains_key(ch_id) {
                        if let Some(ch_val) = ch_obj.get(ch_id) {
                            if let Ok(ch_cfg) = serde_json::from_value(ch_val.clone()) {
                                agent.channels.insert(ch_id.to_string(), ch_cfg);
                            }
                        }
                    }
                }
            }
        }
    }

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
        let router = app.rt.router.read().await;
        router
            .agent_by_id(&agent_id)
            .cloned()
            .ok_or_else(|| format!("agent not found: {agent_id}"))?
    };

    let registry_names: Vec<String> = app
        .rt
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

    let dir = fastclaw_core::paths::resolve_agents_dir_from(Some(&app.cfg.config.paths));
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
    let tools = app.rt.tool_registry.definitions();
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
    if agent.agent_id.as_str() != agent_id {
        agent.agent_id = agent_id.clone().into();
    }
    // Fill channel defaults before persisting so key fields survive round-trip.
    for ch_cfg in agent.channels.values_mut() {
        ch_cfg.fill_defaults();
    }

    let dir = fastclaw_core::paths::resolve_agents_dir_from(Some(&app.cfg.config.paths));
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

    let dir = fastclaw_core::paths::resolve_agents_dir_from(Some(&app.cfg.config.paths));
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
        let router = app.rt.router.read().await;
        router.agent_count()
    };
    if count_before <= 1 {
        return Err("refusing to delete the last remaining agent".into());
    }

    let dir = fastclaw_core::paths::resolve_agents_dir_from(Some(&app.cfg.config.paths));
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

    let state_dir = fastclaw_core::paths::resolve_state_dir_from(Some(&app.cfg.config.paths));
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
    let agents_dir = fastclaw_core::paths::resolve_agents_dir_from(Some(&app.cfg.config.paths));
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

    let state_dir = fastclaw_core::paths::resolve_state_dir_from(Some(&app.cfg.config.paths));
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
        "tools": read("TOOLS.md"),
    }))
}
