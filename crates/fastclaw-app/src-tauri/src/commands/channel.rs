use std::sync::Arc;

use super::helpers::{get_state, validate_agent_id};
use crate::AppData;
use serde_json::json;

// ─── Hot-reload channel ───

#[tauri::command]
pub async fn reload_channel(
    state: tauri::State<'_, AppData>,
    channel_id: String,
) -> Result<serde_json::Value, String> {
    tracing::info!(channel_id = %channel_id, "IPC reload_channel");
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;

    app.reload_channel(&channel_id)
        .await
        .map_err(|e| format!("reload channel: {e}"))?;

    Ok(json!({ "ok": true, "channelId": channel_id }))
}

// ─── Channel bindings ───

#[tauri::command]
pub async fn list_channels(state: tauri::State<'_, AppData>) -> Result<serde_json::Value, String> {
    tracing::info!("IPC list_channels");
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;

    let live: serde_json::Value = (**app.cfg.config_live.load()).clone();
    let channels_val = live.get("channels").cloned().unwrap_or(json!({}));
    let bindings_val = live.get("bindings").cloned().unwrap_or(json!([]));

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
        let mut live: serde_json::Value = (**app.cfg.config_live.load()).clone();

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

        let bytes =
            serde_json::to_vec_pretty(&live).map_err(|e| format!("serialize config: {e}"))?;
        app.cfg.config_live.store(Arc::new(live));
        bytes
    };

    let cfg_dir = fastclaw_core::paths::resolve_config_dir_from(Some(&app.cfg.config.paths));
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
        let mut live: serde_json::Value = (**app.cfg.config_live.load()).clone();

        if let Some(arr) = live.get_mut("bindings").and_then(|v| v.as_array_mut()) {
            arr.retain(|b| {
                !(b.get("agentId").and_then(|a| a.as_str()) == Some(&agent_id)
                    && b.get("match")
                        .and_then(|m| m.get("channel"))
                        .and_then(|c| c.as_str())
                        == Some(&channel_id))
            });
        }

        let bytes =
            serde_json::to_vec_pretty(&live).map_err(|e| format!("serialize config: {e}"))?;
        app.cfg.config_live.store(Arc::new(live));
        bytes
    };

    let cfg_dir = fastclaw_core::paths::resolve_config_dir_from(Some(&app.cfg.config.paths));
    let cfg_path = cfg_dir.join("default.json");
    tokio::fs::write(&cfg_path, serialized)
        .await
        .map_err(|e| format!("write config: {e}"))?;

    Ok(json!({ "ok": true }))
}
