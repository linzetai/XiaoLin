use std::sync::Arc;

use super::helpers::get_state;
use crate::AppData;
use fastclaw_core::config_access::persist_config_key;
use serde_json::json;

// ─── MCP server management ───

#[tauri::command]
pub async fn get_mcp_status(state: tauri::State<'_, AppData>) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let status = app.ext.mcp_status.load();
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
    app.reload_mcp_servers().await.map_err(|e| format!("{e}"))?;
    let status = app.ext.mcp_status.load();
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
        url: None,
        transport: "stdio".to_string(),
    };

    {
        let mut live: serde_json::Value = (**app.cfg.config_live.load()).clone();
        let arr = live.get_mut("mcpServers").and_then(|v| v.as_array_mut());
        let server_val =
            serde_json::to_value(&new_server).map_err(|e| format!("serialize: {e}"))?;
        if let Some(arr) = arr {
            arr.retain(|v| v.get("id").and_then(|i| i.as_str()) != Some(&id));
            arr.push(server_val);
        } else {
            live["mcpServers"] = json!([server_val]);
        }
        app.cfg.config_live.store(Arc::new(live));
    }

    if let Err(e) = persist_config_key("mcpServers", &{
        let live = app.cfg.config_live.load();
        live.get("mcpServers").cloned().unwrap_or(json!([]))
    }) {
        tracing::warn!(error = %e, "failed to persist mcpServers");
    }

    app.reload_mcp_servers().await.map_err(|e| format!("{e}"))?;

    let status = app.ext.mcp_status.load();
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
        let mut live: serde_json::Value = (**app.cfg.config_live.load()).clone();
        if let Some(arr) = live.get_mut("mcpServers").and_then(|v| v.as_array_mut()) {
            arr.retain(|v| v.get("id").and_then(|i| i.as_str()) != Some(&id));
        }
        app.cfg.config_live.store(Arc::new(live));
    }

    if let Err(e) = persist_config_key("mcpServers", &{
        let live = app.cfg.config_live.load();
        live.get("mcpServers").cloned().unwrap_or(json!([]))
    }) {
        tracing::warn!(error = %e, "failed to persist mcpServers");
    }

    app.reload_mcp_servers().await.map_err(|e| format!("{e}"))?;

    Ok(json!({ "ok": true, "id": id }))
}
