use axum::extract::ws::{Message, WebSocket};
use serde_json::json;

use crate::routes::agents::{
    rebuild_behavior_tool_lists, tool_effective_enabled, write_agent_config_file,
};
use crate::state::AppState;
use xiaolin_protocol::{ToolsListParams, ToolsUpdateParams};

use super::send_resp;
use super::types::WsResponse;

pub async fn handle_agents(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
) {
    let agents: Vec<_> = state
        .rt
        .router
        .read()
        .await
        .list_agents()
        .iter()
        .map(|a| json!({"agentId": a.agent_id, "name": a.name, "model": a.model.model}))
        .collect();
    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "agents".into(),
            data: Some(json!({"agents": agents})),
            error: None,
        },
    )
    .await;
}

/// Get a single agent's configuration.
pub async fn handle_agents_get(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let Some(agent_id) = params.get("agentId").and_then(|v| v.as_str()) else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "agentId required"})),
            },
        )
        .await;
        return;
    };

    let agent = {
        let router = state.rt.router.read().await;
        router.agent_by_id(agent_id).cloned()
    };

    let Some(mut cfg) = agent else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(
                    json!({"code": 404, "message": format!("agent not found: {}", agent_id)}),
                ),
            },
        )
        .await;
        return;
    };

    // Merge channels from global config_live bound to this agent
    let live_snapshot = state.cfg.config_live.load();
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
                    if !cfg.channels.contains_key(ch_id) {
                        if let Ok(ch_cfg) =
                            serde_json::from_value(ch_obj.get(ch_id).cloned().unwrap_or_default())
                        {
                            cfg.channels.insert(ch_id.to_string(), ch_cfg);
                        }
                    }
                }
            }
        }
    }

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "agents.get".into(),
            data: Some(serde_json::to_value(&cfg).unwrap_or(json!({}))),
            error: None,
        },
    )
    .await;
}

/// Create a new agent.
pub async fn handle_agents_create(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let Some(config_val) = params.get("config") else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "config required"})),
            },
        )
        .await;
        return;
    };

    let Ok(config) =
        serde_json::from_value::<xiaolin_core::agent_config::AgentConfig>(config_val.clone())
    else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "invalid agent config"})),
            },
        )
        .await;
        return;
    };

    let agent_id = config.agent_id.clone();

    // Check if agent already exists
    {
        let router = state.rt.router.read().await;
        if router.agent_by_id(&agent_id).is_some() {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": 400, "message": format!("agent '{}' already exists", agent_id)})),
                },
            )
            .await;
            return;
        }
    }

    if let Err(e) = write_agent_config_file(state, &config).await {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(
                    json!({"code": 500, "message": format!("failed to write config: {}", e)}),
                ),
            },
        )
        .await;
        return;
    }

    let reloaded = state.reload_agents().await.unwrap_or(0);

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "agents.create".into(),
            data: Some(json!({"ok": true, "agentId": agent_id, "reloaded": reloaded})),
            error: None,
        },
    )
    .await;
}

/// Update an existing agent.
pub async fn handle_agents_update(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let Some(agent_id) = params.get("agentId").and_then(|v| v.as_str()) else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "agentId required"})),
            },
        )
        .await;
        return;
    };

    let Some(config_val) = params.get("config") else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "config required"})),
            },
        )
        .await;
        return;
    };

    let Ok(mut config) =
        serde_json::from_value::<xiaolin_core::agent_config::AgentConfig>(config_val.clone())
    else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "invalid agent config"})),
            },
        )
        .await;
        return;
    };

    // Ensure agentId matches
    config.agent_id = agent_id.to_string().into();

    // Verify agent exists
    {
        let router = state.rt.router.read().await;
        if router.agent_by_id(agent_id).is_none() {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(
                        json!({"code": 404, "message": format!("agent not found: {}", agent_id)}),
                    ),
                },
            )
            .await;
            return;
        }
    }

    if let Err(e) = write_agent_config_file(state, &config).await {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(
                    json!({"code": 500, "message": format!("failed to write config: {}", e)}),
                ),
            },
        )
        .await;
        return;
    }

    let reloaded = state.reload_agents().await.unwrap_or(0);

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "agents.update".into(),
            data: Some(json!({"ok": true, "agentId": agent_id, "reloaded": reloaded})),
            error: None,
        },
    )
    .await;
}

/// Delete an agent.
pub async fn handle_agents_delete(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let Some(agent_id) = params.get("agentId").and_then(|v| v.as_str()) else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "agentId required"})),
            },
        )
        .await;
        return;
    };

    // Check agent count
    let count_before = {
        let router = state.rt.router.read().await;
        router.agent_count()
    };
    if count_before <= 1 {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(
                    json!({"code": 400, "message": "refusing to delete the last remaining agent"}),
                ),
            },
        )
        .await;
        return;
    }

    // Find the config file
    let agents_dir = xiaolin_core::paths::resolve_agents_dir_from(Some(&state.cfg.config.paths));
    let path = agents_dir.join(format!("{agent_id}.json"));

    if !path.exists() {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": 404, "message": format!("agent config file not found for '{}'", agent_id)})),
            },
        )
        .await;
        return;
    }

    if let Err(e) = tokio::fs::remove_file(&path).await {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(
                    json!({"code": 500, "message": format!("failed to remove config: {}", e)}),
                ),
            },
        )
        .await;
        return;
    }

    let reloaded = state.reload_agents().await.unwrap_or(0);

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "agents.delete".into(),
            data: Some(json!({"ok": true, "deleted": agent_id, "reloaded": reloaded})),
            error: None,
        },
    )
    .await;
}

/// List tools available to an agent with their enabled/disabled status.
pub async fn handle_tools_list(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: ToolsListParams,
) {
    let Some(agent_id) = params
        .agent_id
        .as_deref()
        .or_else(|| params.extra.get("agentId").and_then(|v| v.as_str()))
    else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "agentId required"})),
            },
        )
        .await;
        return;
    };

    let agent = {
        let router = state.rt.router.read().await;
        router.agent_by_id(agent_id).cloned()
    };

    let Some(agent) = agent else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(
                    json!({"code": 404, "message": format!("agent not found: {}", agent_id)}),
                ),
            },
        )
        .await;
        return;
    };

    let tools: Vec<_> = state
        .rt
        .tool_registry
        .definitions()
        .iter()
        .map(|td| {
            let name = &td.function.name;
            let enabled = tool_effective_enabled(&agent, name);
            json!({
                "id": name,
                "enabled": enabled,
                "description": td.function.description,
            })
        })
        .collect();

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "tools.list".into(),
            data: Some(json!({"tools": tools})),
            error: None,
        },
    )
    .await;
}

/// Update tool enabled/disabled state for an agent.
pub async fn handle_tools_update(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: ToolsUpdateParams,
) {
    let params_value = serde_json::to_value(&params).unwrap_or_default();
    let Some(agent_id) = params_value.get("agentId").and_then(|v| v.as_str()) else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "agentId required"})),
            },
        )
        .await;
        return;
    };

    let Some(tools_val) = params_value.get("tools") else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "tools required"})),
            },
        )
        .await;
        return;
    };

    let Ok(toggles) = serde_json::from_value::<Vec<ToolToggle>>(tools_val.clone()) else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "invalid tools format"})),
            },
        )
        .await;
        return;
    };

    let agent = {
        let router = state.rt.router.read().await;
        match router.agent_by_id(agent_id).cloned() {
            Some(a) => a,
            None => {
                send_resp(
                    sender,
                    &WsResponse {
                        id: req_id,
                        msg_type: "error".into(),
                        data: None,
                        error: Some(json!({"code": 404, "message": format!("agent not found: {}", agent_id)})),
                    },
                )
                .await;
                return;
            }
        }
    };

    let registry_names: Vec<String> = state
        .rt
        .tool_registry
        .definitions()
        .iter()
        .map(|td| td.function.name.clone())
        .collect();

    let toggles_vec: Vec<(String, bool)> = toggles.into_iter().map(|t| (t.id, t.enabled)).collect();

    let mut agent = agent;
    rebuild_behavior_tool_lists(&mut agent, &registry_names, &toggles_vec);

    if let Err(e) = write_agent_config_file(state, &agent).await {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(
                    json!({"code": 500, "message": format!("failed to write config: {}", e)}),
                ),
            },
        )
        .await;
        return;
    }

    let reloaded = state.reload_agents().await.unwrap_or(0);

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "tools.update".into(),
            data: Some(json!({"ok": true, "reloaded": reloaded})),
            error: None,
        },
    )
    .await;
}

#[derive(serde::Deserialize)]
struct ToolToggle {
    id: String,
    enabled: bool,
}
