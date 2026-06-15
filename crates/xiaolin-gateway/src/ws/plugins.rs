use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use serde_json::json;

use crate::state::AppState;
use xiaolin_core::config_access::persist_config_key;

use super::send_resp;
use super::types::WsResponse;

pub async fn handle_plugins_list(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
) {
    let status_map = state.ext.mcp_status.load();
    let live = state.cfg.config_live.load();
    let user_servers: Vec<String> = live
        .get("mcpServers")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.get("id").and_then(|i| i.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let project_ids = get_project_server_ids();
    let mut plugins: Vec<serde_json::Value> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for (id, st) in status_map.iter() {
        seen.insert(id.clone());
        let scope = if !st.scope.is_empty() {
            st.scope.as_str()
        } else if project_ids.contains(id) {
            "project"
        } else {
            "user"
        };
        let enabled = st.status != xiaolin_core::types::McpStatus::Disabled
            && st.status != xiaolin_core::types::McpStatus::PendingApproval;
        plugins.push(json!({
            "id": id,
            "name": id,
            "scope": scope,
            "enabled": enabled,
            "status": st.status,
            "toolCount": st.tool_count,
            "lastError": st.error,
            "connectedAt": st.connected_at,
            "commandPreview": st.command_preview,
        }));
    }

    for uid in &user_servers {
        if !seen.contains(uid) {
            plugins.push(json!({
                "id": uid,
                "name": uid,
                "scope": "user",
                "enabled": false,
                "status": "disabled",
                "toolCount": 0,
                "lastError": null,
                "connectedAt": null,
            }));
        }
    }

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "plugins.list".into(),
            data: Some(json!({ "plugins": plugins })),
            error: None,
        },
    )
    .await;
}

pub async fn handle_plugins_enable(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    plugin_id: &str,
) {
    let project_ids = get_project_server_ids();
    if project_ids.contains(plugin_id) {
        if let Err(e) = set_project_mcp_disabled(plugin_id, false) {
            send_error(sender, req_id, &format!("project config error: {e}")).await;
            return;
        }
    } else {
        set_user_mcp_enabled(state, plugin_id, true);
    }

    match state.reload_mcp_servers().await {
        Ok(()) => {
            broadcast_status_changed(state);
            let st = state.ext.mcp_status.load().get(plugin_id).cloned();
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "plugins.enable".into(),
                    data: Some(json!({ "ok": true, "id": plugin_id, "status": st })),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => send_error(sender, req_id, &format!("{e}")).await,
    }
}

pub async fn handle_plugins_disable(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    plugin_id: &str,
) {
    let project_ids = get_project_server_ids();
    if project_ids.contains(plugin_id) {
        if let Err(e) = set_project_mcp_disabled(plugin_id, true) {
            send_error(sender, req_id, &format!("project config error: {e}")).await;
            return;
        }
    } else {
        set_user_mcp_enabled(state, plugin_id, false);
    }

    match state.reload_mcp_servers().await {
        Ok(()) => {
            broadcast_status_changed(state);
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "plugins.disable".into(),
                    data: Some(json!({ "ok": true, "id": plugin_id })),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => send_error(sender, req_id, &format!("{e}")).await,
    }
}

pub async fn handle_plugins_restart(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    plugin_id: &str,
) {
    if !state.ext.mcp_status.load().contains_key(plugin_id) {
        send_error(sender, req_id, "plugin not found").await;
        return;
    }

    match state.reload_mcp_servers().await {
        Ok(()) => {
            broadcast_status_changed(state);
            let st = state.ext.mcp_status.load().get(plugin_id).cloned();
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "plugins.restart".into(),
                    data: Some(json!({ "ok": true, "id": plugin_id, "status": st })),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => send_error(sender, req_id, &format!("{e}")).await,
    }
}

pub async fn handle_plugins_tools(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    plugin_id: &str,
) {
    let tools = {
        let handles = state.ext.mcp_handles.lock().await;
        if let Some(client) = handles.get(plugin_id) {
            client
                .tools()
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description.as_deref().unwrap_or(""),
                    })
                })
                .collect::<Vec<_>>()
        } else {
            vec![]
        }
    };

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "plugins.tools".into(),
            data: Some(json!({ "id": plugin_id, "tools": tools })),
            error: None,
        },
    )
    .await;
}

fn get_project_server_ids() -> std::collections::HashSet<String> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let ws_root = xiaolin_core::workspace::detect_workspace_root(&cwd);
    xiaolin_core::agent_config::load_project_mcp_config(&ws_root)
        .map(|p| {
            p.to_mcp_server_configs()
                .into_iter()
                .map(|c| c.id)
                .collect()
        })
        .unwrap_or_default()
}

fn set_user_mcp_enabled(state: &AppState, plugin_id: &str, enabled: bool) {
    let mut live: serde_json::Value = (**state.cfg.config_live.load()).clone();
    if let Some(arr) = live.get_mut("mcpServers").and_then(|v| v.as_array_mut()) {
        for s in arr.iter_mut() {
            if s.get("id").and_then(|i| i.as_str()) == Some(plugin_id) {
                s["enabled"] = json!(enabled);
            }
        }
    }
    state.cfg.config_live.store(Arc::new(live));
    let mcp_val = state
        .cfg
        .config_live
        .load()
        .get("mcpServers")
        .cloned()
        .unwrap_or(json!([]));
    let _ = persist_config_key("mcpServers", &mcp_val);
}

fn set_project_mcp_disabled(plugin_id: &str, disabled: bool) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let ws_root = xiaolin_core::workspace::detect_workspace_root(&cwd);
    let paths = [
        ws_root.join(".xiaolin/mcp.json"),
        ws_root.join(".cursor/mcp.json"),
    ];
    let path = match paths.iter().find(|p| p.exists()) {
        Some(p) => p.clone(),
        None => paths[0].clone(),
    };

    let mut doc: serde_json::Value = if path.exists() {
        serde_json::from_str(&std::fs::read_to_string(&path)?)?
    } else {
        json!({ "mcpServers": {} })
    };

    if let Some(servers) = doc.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
        if let Some(entry) = servers.get_mut(plugin_id) {
            if let Some(obj) = entry.as_object_mut() {
                if disabled {
                    obj.insert("disabled".to_string(), json!(true));
                } else {
                    obj.remove("disabled");
                }
            }
        }
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(&doc)?)?;
    Ok(())
}

pub async fn handle_plugins_approve(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    plugin_id: &str,
) {
    let cwd = std::env::current_dir().unwrap_or_default();
    let ws_root = xiaolin_core::workspace::detect_workspace_root(&cwd);

    if let Err(e) = xiaolin_core::project_mcp_approval::set_approval(
        &ws_root,
        plugin_id,
        xiaolin_core::project_mcp_approval::ProjectMcpApproval::Approved,
    ) {
        send_error(sender, req_id, &format!("failed to save approval: {e}")).await;
        return;
    }

    tracing::info!(mcp_id = %plugin_id, "project MCP server approved, connecting");

    match state.reload_mcp_servers().await {
        Ok(()) => {
            broadcast_status_changed(state);
            let st = state.ext.mcp_status.load().get(plugin_id).cloned();
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "plugins.approve".into(),
                    data: Some(json!({ "ok": true, "id": plugin_id, "status": st })),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => send_error(sender, req_id, &format!("{e}")).await,
    }
}

pub async fn handle_plugins_reject(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    plugin_id: &str,
) {
    let cwd = std::env::current_dir().unwrap_or_default();
    let ws_root = xiaolin_core::workspace::detect_workspace_root(&cwd);

    if let Err(e) = xiaolin_core::project_mcp_approval::set_approval(
        &ws_root,
        plugin_id,
        xiaolin_core::project_mcp_approval::ProjectMcpApproval::Rejected,
    ) {
        send_error(sender, req_id, &format!("failed to save rejection: {e}")).await;
        return;
    }

    tracing::info!(mcp_id = %plugin_id, "project MCP server rejected");

    // Disconnect the server if it was running (e.g. previously approved then rejected).
    {
        let mut handles = state.ext.mcp_handles.lock().await;
        if handles.contains_key(plugin_id) {
            let prefix = xiaolin_mcp::naming::mcp_server_prefix(plugin_id);
            let removed = state.rt.tool_registry.unregister_by_prefix(&prefix);
            handles.remove(plugin_id);
            tracing::info!(mcp_id = %plugin_id, tools_removed = removed, "disconnected rejected MCP server");
        }
    }

    match state.reload_mcp_servers().await {
        Ok(()) => {
            broadcast_status_changed(state);
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "plugins.reject".into(),
                    data: Some(json!({ "ok": true, "id": plugin_id })),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => send_error(sender, req_id, &format!("{e}")).await,
    }
}

fn broadcast_status_changed(state: &AppState) {
    let status: Vec<_> = state.ext.mcp_status.load().values().cloned().collect();
    let payload = json!({
        "type": "event",
        "event": "plugins.status_changed",
        "data": { "plugins": status },
    });
    let _ = state.strm.ws_broadcast.send(payload.to_string());
}

async fn send_error(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    req_id: Option<String>,
    msg: &str,
) {
    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "error".into(),
            data: None,
            error: Some(json!({"message": msg})),
        },
    )
    .await;
}
