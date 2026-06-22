use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use serde_json::json;

use crate::state::AppState;
use xiaolin_core::config_access::persist_config_key;
use xiaolin_protocol::McpAddParams;

use super::send_resp;
use super::types::WsResponse;

fn mask_value(s: &str) -> String {
    if s.len() <= 4 {
        "****".to_string()
    } else {
        format!("{}****", &s[..s.floor_char_boundary(4)])
    }
}

// ─── MCP WS handlers ───

pub async fn handle_mcp_status(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
) {
    let status: Vec<_> = state.ext.mcp_status.load().values().cloned().collect();
    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "mcp.status".into(),
            data: Some(json!({"servers": status})),
            error: None,
        },
    )
    .await;
}

pub async fn handle_mcp_reload(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
) {
    match state.reload_mcp_servers().await {
        Ok(()) => {
            let status: Vec<_> = state.ext.mcp_status.load().values().cloned().collect();
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "mcp.reload".into(),
                    data: Some(json!({"ok": true, "servers": status})),
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
                    error: Some(json!({"message": format!("{e}")})),
                },
            )
            .await;
        }
    }
}

pub async fn handle_mcp_add(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: McpAddParams,
) {
    let id = params.id;
    let command = params.command;
    let args = params.args;
    let url = params.url;
    let transport: xiaolin_core::agent_config::McpTransportType =
        match serde_json::from_value(json!(params.transport)) {
            Ok(t) => t,
            Err(_) => {
                send_resp(
                    sender,
                    &WsResponse {
                        id: req_id,
                        msg_type: "error".into(),
                        data: None,
                        error: Some(json!({
                            "message": format!(
                                "invalid transport '{}': expected stdio, sse, streamable_http, or http",
                                params.transport
                            )
                        })),
                    },
                )
                .await;
                return;
            }
        };

    let new_server = xiaolin_core::agent_config::McpServerConfig {
        id: id.clone(),
        command,
        args,
        enabled: Some(true),
        env: params.env,
        url,
        transport,
        startup_timeout_sec: None,
        bearer_token_env_var: params.bearer_token_env_var,
        http_headers: params.http_headers,
    };

    if let Err(e) = new_server.validate() {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"message": e})),
            },
        )
        .await;
        return;
    }

    {
        let mut live: serde_json::Value = (**state.cfg.config_live.load()).clone();
        let server_val = serde_json::to_value(&new_server).unwrap_or_default();
        if let Some(arr) = live.get_mut("mcpServers").and_then(|v| v.as_array_mut()) {
            arr.retain(|v| v.get("id").and_then(|i| i.as_str()) != Some(&id));
            arr.push(server_val);
        } else {
            live["mcpServers"] = json!([server_val]);
        }
        state.cfg.config_live.store(Arc::new(live));
    }

    {
        let live = state.cfg.config_live.load();
        let mcp_val = live.get("mcpServers").cloned().unwrap_or(json!([]));
        let _ = persist_config_key("mcpServers", &mcp_val);
    }

    match state.reload_mcp_servers().await {
        Ok(()) => {
            let status = state.ext.mcp_status.load().get(&id).cloned();
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "mcp.add".into(),
                    data: Some(json!({"ok": true, "id": id, "status": status})),
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
                    error: Some(json!({"message": format!("{e}")})),
                },
            )
            .await;
        }
    }
}

pub async fn handle_mcp_remove(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let id = match params.get("id").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": -32602, "message": "id required"})),
                },
            )
            .await;
            return;
        }
    };

    {
        let mut live: serde_json::Value = (**state.cfg.config_live.load()).clone();
        if let Some(arr) = live.get_mut("mcpServers").and_then(|v| v.as_array_mut()) {
            arr.retain(|v| v.get("id").and_then(|i| i.as_str()) != Some(&id));
        }
        state.cfg.config_live.store(Arc::new(live));
    }

    {
        let live = state.cfg.config_live.load();
        let mcp_val = live.get("mcpServers").cloned().unwrap_or(json!([]));
        let _ = persist_config_key("mcpServers", &mcp_val);
    }

    match state.reload_mcp_servers().await {
        Ok(()) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "mcp.remove".into(),
                    data: Some(json!({"ok": true, "id": id})),
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
                    error: Some(json!({"message": format!("{e}")})),
                },
            )
            .await;
        }
    }
}

pub async fn handle_mcp_detail(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    server_id: &str,
) {
    let status = state.ext.mcp_status.load().get(server_id).cloned();
    if status.is_none() {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"message": "server not found"})),
            },
        )
        .await;
        return;
    }
    let status = status.unwrap();

    let live = state.cfg.config_live.load();
    let config_from_live = live
        .get("mcpServers")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter()
                .find(|s| s.get("id").and_then(|i| i.as_str()) == Some(server_id))
        })
        .cloned();

    let (config_json, config_source) = if let Some(cfg) = config_from_live {
        (cfg, "user")
    } else if let Ok(cwd) = std::env::current_dir() {
        let ws_root = xiaolin_core::workspace::detect_workspace_root(&cwd);
        if let Some(project_mcp) =
            xiaolin_core::agent_config::load_project_mcp_config(&ws_root)
        {
            let cfgs = project_mcp.to_mcp_server_configs();
            if let Some(c) = cfgs.into_iter().find(|c| c.id == server_id) {
                (serde_json::to_value(&c).unwrap_or_default(), "project")
            } else {
                (json!({}), "unknown")
            }
        } else {
            (json!({}), "unknown")
        }
    } else {
        (json!({}), "unknown")
    };

    let masked_env: serde_json::Map<String, serde_json::Value> = config_json
        .get("env")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .map(|(k, v)| {
                    let masked = v.as_str().map(mask_value).unwrap_or_default();
                    (k.clone(), json!(masked))
                })
                .collect()
        })
        .unwrap_or_default();

    let config = json!({
        "command": config_json.get("command").and_then(|v| v.as_str()).unwrap_or(""),
        "args": config_json.get("args").cloned().unwrap_or(json!([])),
        "transport": config_json.get("transport").and_then(|v| v.as_str()).unwrap_or("stdio"),
        "url": config_json.get("url"),
        "env": masked_env,
        "source": config_source,
    });

    let tools = {
        let handles = state.ext.mcp_handles.lock().await;
        if let Some(client) = handles.get(server_id) {
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
            msg_type: "mcp.detail".into(),
            data: Some(json!({
                "id": status.id,
                "status": status.status,
                "error": status.error,
                "toolCount": status.tool_count,
                "connectedAt": status.connected_at,
                "config": config,
                "tools": tools,
            })),
            error: None,
        },
    )
    .await;
}
