use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use serde_json::json;

use crate::state::AppState;
use fastclaw_core::config_access::persist_config_key;

use super::send_resp;
use super::types::WsResponse;

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
    let command = match params.get("command").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": -32602, "message": "command required"})),
                },
            )
            .await;
            return;
        }
    };
    let args: Vec<String> = params
        .get("args")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let new_server = fastclaw_core::agent_config::McpServerConfig {
        id: id.clone(),
        command,
        args,
        enabled: Some(true),
        env: Default::default(),
        url: None,
        transport: "stdio".to_string(),
    };

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
