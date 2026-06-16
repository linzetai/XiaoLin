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
        plugins.push(enrich_status(id, st, &project_ids));
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

    match state.restart_single_mcp_server(plugin_id).await {
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

pub fn broadcast_status_changed(state: &AppState) {
    let status_map = state.ext.mcp_status.load();
    let project_ids = get_project_server_ids();
    let mut seen = std::collections::HashSet::new();
    let mut plugins: Vec<serde_json::Value> = status_map
        .iter()
        .map(|(id, st)| { seen.insert(id.clone()); enrich_status(id, st, &project_ids) })
        .collect();

    let live = state.cfg.config_live.load();
    if let Some(arr) = live.get("mcpServers").and_then(|v| v.as_array()) {
        for s in arr {
            if let Some(uid) = s.get("id").and_then(|i| i.as_str()) {
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
        }
    }

    let payload = json!({
        "type": "event",
        "event": "plugins.status_changed",
        "data": { "plugins": plugins },
    });
    let _ = state.strm.ws_broadcast.send(payload.to_string());
}

pub async fn handle_plugins_oauth_login(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    plugin_id: &str,
) {
    use xiaolin_core::agent_config::McpServerConfig;
    use xiaolin_mcp::oauth;

    let cfg: Option<McpServerConfig> = {
        let live = state.cfg.config_live.load();
        let mcp_val = live
            .get("mcpServers")
            .cloned()
            .unwrap_or(serde_json::Value::Array(vec![]));
        let user_servers: Vec<McpServerConfig> =
            serde_json::from_value(mcp_val).unwrap_or_default();
        user_servers
            .into_iter()
            .find(|c| c.id == plugin_id)
            .or_else(|| {
                let cwd = std::env::current_dir().unwrap_or_default();
                let ws_root = xiaolin_core::workspace::detect_workspace_root(&cwd);
                xiaolin_core::agent_config::load_project_mcp_config(&ws_root).and_then(|p| {
                    p.to_mcp_server_configs()
                        .into_iter()
                        .find(|c| c.id == plugin_id)
                })
            })
    };

    let cfg = match cfg {
        Some(c) => c,
        None => {
            send_error(sender, req_id, "plugin not found in config").await;
            return;
        }
    };

    let url = match cfg.url.as_deref() {
        Some(u) if !u.is_empty() => u.to_string(),
        _ => {
            send_error(sender, req_id, "server has no URL, OAuth not applicable").await;
            return;
        }
    };

    xiaolin_mcp::clear_needs_auth_cache(plugin_id);

    let mut oauth_client = oauth::McpOAuthClient::new(&url);
    if let Err(e) = oauth_client.discover_metadata().await {
        send_error(
            sender,
            req_id,
            &format!("OAuth metadata discovery failed: {e}"),
        )
        .await;
        return;
    }

    let (redirect_uri, code_rx) = match oauth::start_callback_server().await {
        Ok(pair) => pair,
        Err(e) => {
            send_error(
                sender,
                req_id,
                &format!("failed to start OAuth callback server: {e}"),
            )
            .await;
            return;
        }
    };

    let pkce = oauth::PkceChallenge::generate();
    let state_param = format!("xiaolin_{}", chrono::Utc::now().timestamp_millis());

    let auth_url = match oauth_client.build_authorization_url(&pkce, &redirect_uri, &state_param, None) {
        Ok(u) => u,
        Err(e) => {
            send_error(
                sender,
                req_id,
                &format!("failed to build auth URL: {e}"),
            )
            .await;
            return;
        }
    };

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "plugins.oauth_login".into(),
            data: Some(json!({
                "ok": true,
                "id": plugin_id,
                "auth_url": auth_url,
            })),
            error: None,
        },
    )
    .await;

    let plugin_id = plugin_id.to_string();
    let state = state.clone();
    tokio::spawn(async move {
        let oauth_result: Result<(), String> = async {
            let code_state = tokio::time::timeout(std::time::Duration::from_secs(300), code_rx)
                .await
                .map_err(|_| "OAuth callback timed out (5 min)".to_string())?
                .map_err(|_| "OAuth callback channel closed".to_string())?;

            let (code, recv_state) = code_state;

            if recv_state != state_param {
                return Err(format!(
                    "OAuth state mismatch: expected {state_param}, got {recv_state}"
                ));
            }

            let token = oauth_client
                .exchange_code(&code, &pkce.code_verifier, &redirect_uri)
                .await
                .map_err(|e| format!("OAuth code exchange failed: {e}"))?;

            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let stored = oauth::StoredToken {
                access_token: token.access_token,
                refresh_token: token.refresh_token,
                expires_at: token.expires_in.map(|e| now_secs + e),
                server_url: url,
            };
            oauth::save_stored_token(&plugin_id, &stored)
                .map_err(|e| format!("failed to save OAuth token: {e}"))?;

            tracing::info!(server = %plugin_id, "OAuth token obtained, reconnecting server");
            if let Err(e) = state.restart_single_mcp_server(&plugin_id).await {
                tracing::error!(server = %plugin_id, error = %e, "failed to reconnect after OAuth");
            }
            Ok(())
        }
        .await;

        if let Err(reason) = &oauth_result {
            tracing::warn!(server = %plugin_id, error = %reason, "OAuth flow failed");
            let _ = state.strm.ws_broadcast.send(
                json!({
                    "type": "event",
                    "event": "plugins.oauth_failed",
                    "data": {
                        "id": plugin_id,
                        "error": reason,
                    }
                })
                .to_string(),
            );
        }
        broadcast_status_changed(&state);
    });
}

fn enrich_status(
    id: &str,
    st: &xiaolin_core::types::McpServerStatus,
    project_ids: &std::collections::HashSet<String>,
) -> serde_json::Value {
    let scope = if !st.scope.is_empty() {
        st.scope.as_str()
    } else if project_ids.contains(id) {
        "project"
    } else {
        "user"
    };
    let enabled = st.status != xiaolin_core::types::McpStatus::Disabled
        && st.status != xiaolin_core::types::McpStatus::PendingApproval;
    json!({
        "id": id,
        "name": id,
        "scope": scope,
        "enabled": enabled,
        "status": st.status,
        "toolCount": st.tool_count,
        "lastError": st.error,
        "connectedAt": st.connected_at,
        "commandPreview": st.command_preview,
        "transport": st.transport,
    })
}

pub async fn handle_plugins_prompts(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
) {
    let prompt_clients: Vec<(String, std::sync::Arc<xiaolin_mcp::McpClient>)> = {
        let handles = state.ext.mcp_handles.lock().await;
        handles
            .iter()
            .filter(|(_, c)| c.has_prompts())
            .map(|(id, c)| (id.clone(), c.clone()))
            .collect()
    };

    let mut all_prompts = Vec::new();
    for (server_id, client) in &prompt_clients {
        match client.list_prompts().await {
            Ok(prompts) => {
                for p in prompts {
                    all_prompts.push(json!({
                        "server": server_id,
                        "name": p.name,
                        "description": p.description,
                        "arguments": p.arguments,
                    }));
                }
            }
            Err(e) => {
                tracing::warn!(server = %server_id, error = %e, "failed to list prompts");
            }
        }
    }

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "plugins.prompts".into(),
            data: Some(json!({ "prompts": all_prompts })),
            error: None,
        },
    )
    .await;
}

pub async fn handle_plugins_elicitation_reply(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    elicitation_id: &str,
    action: &str,
    content: Option<serde_json::Value>,
) {
    let entry = state.strm.pending_elicitations.remove(elicitation_id);
    match entry {
        Some((_, pending)) => {
            let reply = crate::state::ElicitationReply {
                action: action.to_string(),
                content,
            };
            let _ = pending.reply_tx.send(reply);
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "plugins.elicitation_reply".into(),
                    data: Some(json!({"status": "ok"})),
                    error: None,
                },
            )
            .await;
        }
        None => {
            send_error(
                sender,
                req_id,
                &format!("elicitation '{elicitation_id}' not found or already expired"),
            )
            .await;
        }
    }
}

pub async fn handle_plugins_get_prompt(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    server_name: &str,
    prompt_name: &str,
    arguments: Option<std::collections::HashMap<String, String>>,
) {
    let client = {
        let handles = state.ext.mcp_handles.lock().await;
        handles.get(server_name).cloned()
    };
    let client = match client {
        Some(c) => c,
        None => {
            send_error(sender, req_id, &format!("server '{server_name}' not found")).await;
            return;
        }
    };

    match client.get_prompt(prompt_name, arguments).await {
        Ok(messages) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "plugins.get_prompt".into(),
                    data: Some(json!({
                        "server": server_name,
                        "prompt": prompt_name,
                        "messages": messages,
                    })),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => {
            send_error(sender, req_id, &format!("prompts/get failed: {e}")).await;
        }
    }
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
