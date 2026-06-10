mod agents;
mod automations;
mod channels;
mod chat;
mod config;
mod cron;
mod execution;
mod git;
mod mcp;
mod notifications;
mod plugins;
mod project;
mod session;
mod skills;
mod types;

pub use types::WsQueryParams;

use types::{WsRequest, WsResponse};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Extension, Query, State,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

use crate::state::AppState;

use xiaolin_protocol::ClientOp;
use xiaolin_security::ApiKeyAuth;

static CONN_COUNTER: AtomicU64 = AtomicU64::new(0);

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(90);
const MAX_MESSAGE_SIZE: usize = 4 * 1024 * 1024;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Extension(auth): Extension<ApiKeyAuth>,
    Query(params): Query<WsQueryParams>,
) -> impl IntoResponse {
    let pre_authed = match &params.token {
        Some(token) => auth.validate_key(token),
        None => !auth.is_enabled(),
    };
    ws.max_message_size(MAX_MESSAGE_SIZE)
        .on_upgrade(move |socket| handle_socket(socket, state, auth, pre_authed))
}

async fn send_resp(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    resp: &WsResponse,
) -> bool {
    match serde_json::to_string(resp) {
        Ok(json) => sender.send(Message::Text(json)).await.is_ok(),
        Err(e) => {
            tracing::error!(error = %e, "failed to serialize WsResponse");
            false
        }
    }
}

async fn handle_socket(socket: WebSocket, state: AppState, auth: ApiKeyAuth, pre_authed: bool) {
    let conn_id = CONN_COUNTER.fetch_add(1, Ordering::SeqCst);
    xiaolin_observe::record_ws_connection(1);
    tracing::info!(conn_id, pre_authed, "websocket client connected");

    let (mut sender, mut receiver) = socket.split();
    let mut authenticated = pre_authed;
    let mut last_activity = Instant::now();
    let mut broadcast_rx = state.strm.ws_broadcast.subscribe();
    let mut subscriptions: HashSet<String> = HashSet::new();
    let mut owned_sessions: HashSet<String> = HashSet::new();
    let cancel = CancellationToken::new();
    let active_chat_sessions: Arc<tokio::sync::Mutex<HashMap<String, String>>> =
        Arc::new(tokio::sync::Mutex::new(HashMap::new()));

    // Channel for background tasks (chat streaming) to send responses back to the client.
    // This decouples streaming from the main loop so heartbeat/ping continue working.
    let (bg_tx, mut bg_rx) = tokio::sync::mpsc::channel::<WsResponse>(128);

    let auth_required = auth.is_enabled();
    send_resp(
        &mut sender,
        &WsResponse {
            id: None,
            msg_type: "connected".into(),
            data: Some(json!({
                "version": env!("CARGO_PKG_VERSION"),
                "connId": conn_id,
                "protocol": "xiaolin-ws/2",
                "capabilities": ["approval", "history_items", "turn_coordinator", "structured_errors"],
                "methods": ["ping", "chat", "agents", "auth",
                            "sessions.list", "sessions.get", "sessions.messages", "sessions.delete",
                            "sessions.new", "sessions.claim", "sessions.update_title",
                            "cancel", "answer", "set_mode",
                            "models.list", "config.get", "config.set",
                            "mcp.status", "mcp.reload", "mcp.add", "mcp.remove", "mcp.detail",
                            "plugins.list", "plugins.enable", "plugins.disable", "plugins.restart", "plugins.tools",
                            "channels.list", "channels.detail", "channels.connect", "channels.update", "channels.restore",
                            "channels.wechat_login", "channels.wechat_poll", "channels.wechat_verify", "channels.disconnect",
                            "sub_agents.list",
                            "agents.get", "agents.create", "agents.update", "agents.delete",
                            "tools.list", "tools.update", "tools.submit_answer",
                            "skills.list", "skills.refresh",
                            "permissions.get_presets", "permissions.get_session", "permissions.set_session",
                            "automations.list", "automations.create", "automations.update", "automations.delete", "automations.runs", "automations.run_now",
                            "execution.set_mode", "execution.get_plan", "execution.approve_plan",
                            "resolve_approval", "approval.resolve",
                            "chat.compact", "compact",
                            "chat.steer", "steer",
                            "projects.list", "projects.create", "projects.update", "projects.delete", "projects.detect",
                            "git.status", "git.diff", "git.branches", "git.log", "git.stage", "git.unstage", "git.commit", "git.revert",
                            "subscribe", "unsubscribe"],
                "authRequired": auth_required && !authenticated,
            })),
            error: None,
        },
    )
    .await;

    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.tick().await;

    loop {
        tokio::select! {
            biased;

            // Background task responses (streaming chat events)
            Some(resp) = bg_rx.recv() => {
                if resp.msg_type == "turn_end" {
                    if let Some(sid) = resp
                        .data
                        .as_ref()
                        .and_then(|d| d.get("session_id"))
                        .and_then(|v| v.as_str())
                    {
                        owned_sessions.insert(sid.to_string());
                    }
                }
                last_activity = Instant::now();
                if !send_resp(&mut sender, &resp).await {
                    break;
                }
            }

            // Heartbeat tick
            _ = heartbeat.tick() => {
                if last_activity.elapsed() > CLIENT_TIMEOUT {
                    tracing::info!(conn_id, "client timeout, closing");
                    let _ = sender.send(Message::Close(Some(axum::extract::ws::CloseFrame {
                        code: 4000,
                        reason: "timeout".into(),
                    }))).await;
                    break;
                }
                if send_resp(&mut sender, &WsResponse {
                    id: None, msg_type: "heartbeat".into(),
                    data: Some(json!({"ts": chrono::Utc::now().to_rfc3339()})),
                    error: None,
                }).await {
                    // Heartbeat sent successfully means client is still connected;
                    // keep the connection alive while LLM streaming is pending.
                    last_activity = Instant::now();
                }
            }

            // Broadcast events for subscriptions (wildcard "*" not allowed for security)
            Ok(event_json) = broadcast_rx.recv() => {
                if let Ok(event) = serde_json::from_str::<serde_json::Value>(&event_json) {
                    let name = event.get("event").and_then(|v| v.as_str()).unwrap_or("");
                    if subscriptions.contains(name) {
                        let _ = sender.send(Message::Text(event_json)).await;
                    }
                }
            }

            // Client messages
            msg = receiver.next() => {
                let msg = match msg {
                    Some(Ok(msg)) => msg,
                    Some(Err(e)) => {
                        tracing::warn!(conn_id, error = %e, "websocket receive error");
                        break;
                    }
                    None => break,
                };

                last_activity = Instant::now();

                let text = match msg {
                    Message::Text(t) => t.to_string(),
                    Message::Close(_) => break,
                    Message::Ping(data) => { let _ = sender.send(Message::Pong(data)).await; continue; }
                    Message::Pong(_) => continue,
                    _ => continue,
                };

                let req: WsRequest = match serde_json::from_str(&text) {
                    Ok(r) => r,
                    Err(e) => {
                        send_resp(&mut sender, &WsResponse {
                            id: None, msg_type: "error".into(), data: None,
                            error: Some(json!({"code": -32700, "message": format!("parse error: {e}")})),
                        }).await;
                        continue;
                    }
                };

                // Auth method always available
                if req.method == "auth" {
                    let token = req.params.get("token").and_then(|v| v.as_str()).unwrap_or("");
                    if auth.validate_key(token) {
                        authenticated = true;
                        send_resp(&mut sender, &WsResponse {
                            id: req.id, msg_type: "auth.ok".into(),
                            data: Some(json!({"authenticated": true})), error: None,
                        }).await;
                    } else {
                        send_resp(&mut sender, &WsResponse {
                            id: req.id, msg_type: "auth.failed".into(), data: None,
                            error: Some(json!({"code": 401, "message": "invalid token"})),
                        }).await;
                    }
                    continue;
                }

                // Auth gate (ping always allowed)
                if auth_required && !authenticated {
                    if req.method == "ping" {
                        send_resp(&mut sender, &WsResponse {
                            id: req.id, msg_type: "pong".into(),
                            data: Some(json!({"ts": chrono::Utc::now().to_rfc3339()})),
                            error: None,
                        }).await;
                        continue;
                    }
                    send_resp(&mut sender, &WsResponse {
                        id: req.id, msg_type: "error".into(), data: None,
                        error: Some(json!({"code": 401, "message": "authentication required"})),
                    }).await;
                    continue;
                }

                dispatch(
                    &mut sender,
                    &state,
                    &mut subscriptions,
                    &mut owned_sessions,
                    &bg_tx,
                    &cancel,
                    active_chat_sessions.clone(),
                    req,
                )
                .await;
            }
        }
    }

    // Interrupt all in-flight sessions on disconnect.
    {
        let sessions = active_chat_sessions.lock().await;
        for (_rid, sid) in sessions.iter() {
            if let Some(handle) = state
                .svc
                .session_manager
                .get(&xiaolin_protocol::SessionId::new(sid))
                .await
            {
                let _ = handle
                    .submit(xiaolin_session_actor::SessionOp::Interrupt)
                    .await;
            }
        }
    }
    cancel.cancel();
    xiaolin_observe::record_ws_connection(-1);
    tracing::info!(conn_id, "websocket session ended");
}

#[allow(clippy::too_many_arguments)]
async fn dispatch(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    subscriptions: &mut HashSet<String>,
    owned_sessions: &mut HashSet<String>,
    bg_tx: &tokio::sync::mpsc::Sender<WsResponse>,
    cancel: &CancellationToken,
    active_chat_sessions: Arc<tokio::sync::Mutex<HashMap<String, String>>>,
    req: WsRequest,
) {
    let id = req.id;

    let op = match ClientOp::parse_request(&req.method, req.params.clone()) {
        Ok(op) => op,
        Err(e) => {
            send_resp(
                sender,
                &WsResponse {
                    id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": -32601, "message": e})),
                },
            )
            .await;
            return;
        }
    };

    match op {
        ClientOp::Ping => {
            send_resp(
                sender,
                &WsResponse {
                    id,
                    msg_type: "pong".into(),
                    data: Some(json!({"ts": chrono::Utc::now().to_rfc3339()})),
                    error: None,
                },
            )
            .await;
        }
        ClientOp::SubAgentsList => {
            let defs = state.strm.subagent_manager.subagent_defs();
            let agents_json: Vec<serde_json::Value> = defs
                .iter()
                .map(crate::routes::subagent::subagent_def_to_json)
                .collect();
            send_resp(
                sender,
                &WsResponse {
                    id,
                    msg_type: "sub_agents.list".into(),
                    data: Some(json!({ "agents": agents_json })),
                    error: None,
                },
            )
            .await;
        }
        ClientOp::SubAgentsConcurrency => {
            let snapshot = state.strm.subagent_manager.controller().snapshot();
            send_resp(
                sender,
                &WsResponse {
                    id,
                    msg_type: "sub_agents.concurrency".into(),
                    data: Some(serde_json::to_value(snapshot).unwrap_or_default()),
                    error: None,
                },
            )
            .await;
        }
        ClientOp::AgentsList => agents::handle_agents(sender, state, id).await,
        ClientOp::Chat { params } => {
            chat::spawn_chat(
                state,
                owned_sessions,
                bg_tx.clone(),
                cancel.clone(),
                active_chat_sessions.clone(),
                id,
                params,
            )
            .await
        }
        ClientOp::ChatCancel { .. } => {
            chat::handle_chat_cancel(sender, state, id, req.params, active_chat_sessions.clone()).await
        }
        ClientOp::ChatAnswer { .. } | ClientOp::ToolsSubmitAnswer { .. } => {
            chat::handle_chat_answer(sender, state, id, req.params).await
        }
        ClientOp::ChatSetMode { .. } => {
            chat::handle_chat_set_mode(sender, state, id, req.params, &Some(bg_tx.clone()))
                .await
        }
        ClientOp::SessionsList { params } => {
            session::handle_sessions_list(sender, state, id, params).await
        }
        ClientOp::SessionsGet { .. } => {
            session::handle_session_scoped(sender, state, owned_sessions, id, req.params, "get")
                .await
        }
        ClientOp::SessionsMessages { .. } => {
            session::handle_session_scoped(
                sender,
                state,
                owned_sessions,
                id,
                req.params,
                "messages",
            )
            .await
        }
        ClientOp::SessionsDelete { .. } => {
            session::handle_session_scoped(sender, state, owned_sessions, id, req.params, "delete")
                .await
        }
        ClientOp::SessionsNew { params } => {
            session::handle_sessions_new(sender, state, owned_sessions, id, params).await
        }
        ClientOp::SessionsClaim { .. } => {
            session::handle_sessions_claim(sender, state, owned_sessions, id, req.params).await
        }
        ClientOp::SessionsUpdateTitle { .. } => {
            session::handle_session_scoped(
                sender,
                state,
                owned_sessions,
                id,
                req.params,
                "update_title",
            )
            .await
        }
        ClientOp::SessionsSetWorkDir { .. } => {
            session::handle_session_scoped(
                sender,
                state,
                owned_sessions,
                id,
                req.params,
                "set_work_dir",
            )
            .await
        }
        ClientOp::ModelsList => config::handle_models_list(sender, state, id).await,
        ClientOp::ConfigGet { .. } => {
            config::handle_config_get(sender, state, id, req.params).await
        }
        ClientOp::ConfigSet { .. } => {
            config::handle_config_set(sender, state, id, req.params).await
        }
        ClientOp::McpStatus => mcp::handle_mcp_status(sender, state, id).await,
        ClientOp::McpReload => mcp::handle_mcp_reload(sender, state, id).await,
        ClientOp::McpAdd { params } => {
            mcp::handle_mcp_add(sender, state, id, params).await
        }
        ClientOp::McpRemove { .. } => {
            mcp::handle_mcp_remove(sender, state, id, req.params).await
        }
        ClientOp::McpDetail { id: server_id } => {
            mcp::handle_mcp_detail(sender, state, id, &server_id).await
        }
        ClientOp::PluginsList => {
            plugins::handle_plugins_list(sender, state, id).await
        }
        ClientOp::PluginsEnable { id: plugin_id } => {
            plugins::handle_plugins_enable(sender, state, id, &plugin_id).await
        }
        ClientOp::PluginsDisable { id: plugin_id } => {
            plugins::handle_plugins_disable(sender, state, id, &plugin_id).await
        }
        ClientOp::PluginsRestart { id: plugin_id } => {
            plugins::handle_plugins_restart(sender, state, id, &plugin_id).await
        }
        ClientOp::PluginsTools { id: plugin_id } => {
            plugins::handle_plugins_tools(sender, state, id, &plugin_id).await
        }
        ClientOp::AgentsGet { .. } => {
            agents::handle_agents_get(sender, state, id, req.params).await
        }
        ClientOp::AgentsCreate { params } => {
            agents::handle_agents_create(sender, state, id, params).await
        }
        ClientOp::AgentsUpdate { params, .. } => {
            agents::handle_agents_update(sender, state, id, params).await
        }
        ClientOp::AgentsDelete { .. } => {
            agents::handle_agents_delete(sender, state, id, req.params).await
        }
        ClientOp::ToolsList { params } => {
            agents::handle_tools_list(sender, state, id, params).await
        }
        ClientOp::ToolsUpdate { params } => {
            agents::handle_tools_update(sender, state, id, params).await
        }
        ClientOp::SkillsList { params } => {
            skills::handle_skills_list(sender, state, id, params).await
        }
        ClientOp::SkillsRefresh => skills::handle_skills_refresh(sender, state, id).await,
        ClientOp::ExecutionSetMode { .. } => {
            execution::handle_execution_set_mode(sender, state, id, req.params, Some(bg_tx)).await
        }
        ClientOp::ExecutionGetPlan { .. } => {
            execution::handle_execution_get_plan(sender, state, id, req.params).await
        }
        ClientOp::ExecutionApprovePlan { .. } => {
            execution::handle_execution_approve_plan(sender, state, id, req.params, bg_tx).await
        }
        ClientOp::ChatCompact { session_id } => {
            chat::handle_chat_compact(sender, state, id, &session_id, bg_tx).await;
        }
        ClientOp::ChatSteer {
            session_id,
            messages,
        } => {
            chat::handle_chat_steer(sender, state, id, &session_id, messages).await;
        }
        ClientOp::ResolveApproval {
            approval_id,
            decision,
            session_id,
        } => {
            let mut resolved = false;
            if let Some(sid) = &session_id {
                if let Some(handle) = state
                    .svc
                    .session_manager
                    .get(&xiaolin_protocol::SessionId::new(sid))
                    .await
                {
                    resolved = handle
                        .submit(xiaolin_session_actor::SessionOp::ResolveApproval {
                            interaction_id: approval_id.clone(),
                            decision: decision.clone(),
                        })
                        .await
                        .is_ok();
                }
            }
            send_resp(
                sender,
                &WsResponse {
                    id,
                    msg_type: "approval.resolved".into(),
                    data: Some(json!({"approvalId": approval_id, "resolved": resolved})),
                    error: None,
                },
            )
            .await;
        }
        ClientOp::ChannelsList => {
            channels::handle_channels_list(sender, state, id).await;
        }
        ClientOp::ChannelsDetail { id: channel_id } => {
            channels::handle_channels_detail(sender, state, id, &channel_id).await;
        }
        ClientOp::ChannelsConnect { id: channel_id } => {
            channels::handle_channels_connect(sender, state, id, &channel_id).await;
        }
        ClientOp::ChannelsUpdate { id: channel_id, config } => {
            channels::handle_channels_update(sender, state, id, &channel_id, config).await;
        }
        ClientOp::ChannelsRestore { id: channel_id } => {
            channels::handle_channels_restore(sender, state, id, &channel_id).await;
        }
        ClientOp::ChannelsWechatLogin => {
            channels::handle_wechat_login(sender, state, id).await;
        }
        ClientOp::ChannelsWechatPoll { session_key } => {
            channels::handle_wechat_poll(sender, state, id, &session_key).await;
        }
        ClientOp::ChannelsWechatVerify { session_key, code } => {
            channels::handle_wechat_verify(sender, state, id, &session_key, &code).await;
        }
        ClientOp::ChannelsDisconnect { channel_id, account_id } => {
            channels::handle_channels_disconnect(sender, state, id, &channel_id, account_id.as_deref()).await;
        }
        ClientOp::PermissionsGetPresets => {
            let presets = state.rt.permission_preset_registry.list();
            let presets_json: Vec<serde_json::Value> = presets
                .iter()
                .map(|p| serde_json::to_value(p).unwrap_or_default())
                .collect();
            send_resp(
                sender,
                &WsResponse {
                    id,
                    msg_type: "permissions.presets".into(),
                    data: Some(json!({ "presets": presets_json })),
                    error: None,
                },
            )
            .await;
        }
        ClientOp::PermissionsGetSession { session_id } => {
            let preset_id = state
                .ext
                .session_preset_ids
                .get(&session_id)
                .map(|v| v.value().clone())
                .unwrap_or_default();
            let has_override = state.ext.session_behavior_overrides.contains_key(&session_id);
            send_resp(
                sender,
                &WsResponse {
                    id,
                    msg_type: "permissions.session".into(),
                    data: Some(json!({
                        "sessionId": session_id,
                        "hasOverride": has_override,
                        "presetId": preset_id,
                    })),
                    error: None,
                },
            )
            .await;
        }
        ClientOp::PermissionsSetSession {
            session_id,
            preset_id,
        } => {
            let registry = &state.rt.permission_preset_registry;
            if let Some(preset) = registry.get(&preset_id) {
                let base_behavior = {
                    let agents = state.cfg.last_good_agents.read().await;
                    agents.first().map(|a| a.behavior.clone()).unwrap_or_default()
                };
                let resolved = preset.resolve_behavior(&base_behavior);
                state
                    .ext
                    .session_behavior_overrides
                    .insert(session_id.clone(), resolved);
                state
                    .ext
                    .session_preset_ids
                    .insert(session_id.clone(), preset_id.clone());
                send_resp(
                    sender,
                    &WsResponse {
                        id,
                        msg_type: "permissions.session_updated".into(),
                        data: Some(json!({
                            "sessionId": session_id,
                            "presetId": preset_id,
                            "preset": serde_json::to_value(preset).unwrap_or_default(),
                        })),
                        error: None,
                    },
                )
                .await;
                let _ = state.strm.ws_broadcast.send(
                    json!({"type":"event","event":"permissions.changed","data":{"sessionId": session_id,"presetId": preset_id}}).to_string(),
                );
            } else {
                send_resp(
                    sender,
                    &WsResponse {
                        id,
                        msg_type: "error".into(),
                        data: None,
                        error: Some(json!({
                            "code": 404,
                            "message": format!("preset '{}' not found", preset_id),
                        })),
                    },
                )
                .await;
            }
        }
        ClientOp::WorkspaceInit { work_dir } => {
            session::handle_workspace_init(sender, state, id, work_dir).await
        }
        ClientOp::Subscribe { events } => {
            for e in &events {
                subscriptions.insert(e.clone());
            }
            send_resp(
                sender,
                &WsResponse {
                    id,
                    msg_type: "subscribe.ok".into(),
                    data: Some(json!({"subscriptions": subscriptions.iter().collect::<Vec<_>>()})),
                    error: None,
                },
            )
            .await;
        }
        ClientOp::Unsubscribe { events } => {
            for e in &events {
                subscriptions.remove(e);
            }
            send_resp(
                sender,
                &WsResponse {
                    id,
                    msg_type: "unsubscribe.ok".into(),
                    data: Some(json!({"subscriptions": subscriptions.iter().collect::<Vec<_>>()})),
                    error: None,
                },
            )
            .await;
        }
        ClientOp::CronListJobs { agent_id } => {
            cron::handle_cron_list_jobs(sender, state, id, agent_id).await;
        }
        ClientOp::CronGetJob { job_id } => {
            cron::handle_cron_get_job(sender, state, id, &job_id).await;
        }
        ClientOp::CronUpsertJob { params } => {
            cron::handle_cron_upsert_job(sender, state, id, params).await;
        }
        ClientOp::CronDeleteJob { job_id } => {
            cron::handle_cron_delete_job(sender, state, id, &job_id).await;
        }
        ClientOp::CronListRuns { job_id, limit } => {
            cron::handle_cron_list_runs(sender, state, id, &job_id, limit).await;
        }
        ClientOp::AutomationsList => {
            automations::handle_automations_list(sender, state, id).await;
        }
        ClientOp::AutomationsCreate { params } => {
            automations::handle_automations_create(sender, state, id, params).await;
        }
        ClientOp::AutomationsUpdate { job_id, params } => {
            automations::handle_automations_update(sender, state, id, &job_id, params).await;
        }
        ClientOp::AutomationsDelete { job_id } => {
            automations::handle_automations_delete(sender, state, id, &job_id).await;
        }
        ClientOp::AutomationsRuns { job_id, limit } => {
            automations::handle_automations_runs(sender, state, id, &job_id, limit).await;
        }
        ClientOp::AutomationsRunNow { job_id } => {
            automations::handle_automations_run_now(sender, state, id, &job_id).await;
        }
        ClientOp::NotificationsUnreadCount => {
            notifications::handle_unread_count(sender, state, id).await;
        }
        ClientOp::NotificationsList { limit } => {
            notifications::handle_list(sender, state, id, limit).await;
        }
        ClientOp::NotificationsMarkRead { notification_id } => {
            notifications::handle_mark_read(sender, state, id, &notification_id).await;
        }
        ClientOp::NotificationsMarkAllRead => {
            notifications::handle_mark_all_read(sender, state, id).await;
        }
        ClientOp::NotificationsDelete { notification_id } => {
            notifications::handle_delete(sender, state, id, &notification_id).await;
        }
        ClientOp::ProjectsList { include_archived } => {
            project::handle_projects_list(sender, state, id, include_archived).await;
        }
        ClientOp::ProjectsCreate {
            root_path,
            name,
            color,
        } => {
            project::handle_projects_create(
                sender,
                state,
                id,
                &root_path,
                name.as_deref(),
                color.as_deref(),
            )
            .await;
        }
        ClientOp::ProjectsUpdate {
            id: project_id,
            name,
            color,
            pinned,
            archived,
        } => {
            project::handle_projects_update(sender, state, id, &project_id, name, color, pinned, archived)
                .await;
        }
        ClientOp::ProjectsDelete { id: project_id } => {
            project::handle_projects_delete(sender, state, id, &project_id).await;
        }
        ClientOp::ProjectsDetect { path } => {
            project::handle_projects_detect(sender, state, id, &path).await;
        }
        ClientOp::GitStatus { project_id } => {
            git::handle_git_status(sender, state, id, &project_id).await;
        }
        ClientOp::GitDiff { project_id, path, staged } => {
            git::handle_git_diff(sender, state, id, &project_id, &path, staged).await;
        }
        ClientOp::GitBranches { project_id } => {
            git::handle_git_branches(sender, state, id, &project_id).await;
        }
        ClientOp::GitLog { project_id, limit } => {
            git::handle_git_log(sender, state, id, &project_id, limit).await;
        }
        ClientOp::GitStage { project_id, files } => {
            git::handle_git_stage(sender, state, id, &project_id, &files).await;
        }
        ClientOp::GitUnstage { project_id, files } => {
            git::handle_git_unstage(sender, state, id, &project_id, &files).await;
        }
        ClientOp::GitCommit { project_id, message } => {
            git::handle_git_commit(sender, state, id, &project_id, &message).await;
        }
        ClientOp::GitRevert { project_id, files } => {
            git::handle_git_revert(sender, state, id, &project_id, &files).await;
        }
        ClientOp::GitInit { project_id } => {
            git::handle_git_init(sender, state, id, &project_id).await;
        }
        ClientOp::GoalPause { session_id } => {
            chat::handle_goal_action(sender, state, id, &session_id, "pause", None).await;
        }
        ClientOp::GoalResume { session_id } => {
            chat::handle_goal_action(sender, state, id, &session_id, "resume", None).await;
        }
        ClientOp::GoalClear { session_id } => {
            chat::handle_goal_action(sender, state, id, &session_id, "clear", None).await;
        }
        ClientOp::GoalEdit {
            session_id,
            description,
        } => {
            let params = serde_json::json!({"description": description});
            chat::handle_goal_action(sender, state, id, &session_id, "edit", Some(&params))
                .await;
        }
        ClientOp::GoalAddBudget {
            session_id,
            amount,
        } => {
            let params = serde_json::json!({"amount": amount});
            chat::handle_goal_action(
                sender,
                state,
                id,
                &session_id,
                "add_budget",
                Some(&params),
            )
            .await;
        }
        _ => {
            send_resp(
                sender,
                &WsResponse {
                    id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(
                        json!({"code": -32601, "message": format!("unsupported operation: {}", req.method)}),
                    ),
                },
            )
            .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::chat::forward_event;
    use xiaolin_core::config_access::{
        filter_config_for_read, mask_secret_values, navigate_config, set_nested_key,
        CONFIG_READABLE_KEYS, CONFIG_WRITABLE_KEYS,
    };
    use xiaolin_protocol::{AgentEvent, AskQuestionOption, TurnId, TurnSummary};
    use serde_json::json;

    #[test]
    fn set_nested_key_simple() {
        let mut root = json!({"a": {"b": 1}});
        set_nested_key(&mut root, "a.b", json!(2)).unwrap();
        assert_eq!(root["a"]["b"], 2);
    }

    #[test]
    fn set_nested_key_creates_intermediate() {
        let mut root = json!({});
        set_nested_key(&mut root, "x.y", json!("hello")).unwrap();
        assert_eq!(root["x"]["y"], "hello");
    }

    #[test]
    fn navigate_config_returns_nested_value() {
        let cfg = json!({"gateway": {"port": 18789}});
        let val = navigate_config(&cfg, "gateway.port");
        assert_eq!(val, 18789);
    }

    #[test]
    fn navigate_config_missing_key_returns_null() {
        let cfg = json!({"gateway": {}});
        let val = navigate_config(&cfg, "gateway.missing");
        assert!(val.is_null());
    }

    #[test]
    fn filter_config_only_includes_readable_keys() {
        let cfg = json!({
            "gateway": {"port": 18789},
            "logging": {"level": "info"},
            "dangerousInternal": "should not appear"
        });
        let filtered = filter_config_for_read(&cfg);
        assert!(filtered.get("gateway").is_some());
        assert!(filtered.get("logging").is_some());
        assert!(filtered.get("dangerousInternal").is_none());
    }

    #[test]
    fn mask_secret_values_masks_api_keys() {
        let val = json!({
            "openai": {
                "apiKey": "sk-1234567890abcdef",
                "baseUrl": "https://api.openai.com/v1"
            }
        });
        let masked = mask_secret_values(&val);
        let key = masked["openai"]["apiKey"].as_str().unwrap();
        assert!(key.contains("…"));
        assert!(!key.contains("1234567890"));
        assert_eq!(masked["openai"]["baseUrl"], "https://api.openai.com/v1");
    }

    #[test]
    fn mask_secret_values_short_key() {
        let val = json!({"apiKey": "short"});
        let masked = mask_secret_values(&val);
        assert_eq!(masked["apiKey"], "****");
    }

    #[test]
    fn mask_secret_values_empty_key_unchanged() {
        let val = json!({"apiKey": ""});
        let masked = mask_secret_values(&val);
        assert_eq!(masked["apiKey"], "");
    }

    #[test]
    fn config_writable_keys_include_web_search() {
        assert!(CONFIG_WRITABLE_KEYS.contains(&"webSearch"));
        assert!(CONFIG_WRITABLE_KEYS.contains(&"credentials"));
        assert!(CONFIG_WRITABLE_KEYS.contains(&"models"));
    }

    #[test]
    fn config_readable_keys_include_web_search_and_credentials() {
        assert!(CONFIG_READABLE_KEYS.contains(&"webSearch"));
        assert!(CONFIG_READABLE_KEYS.contains(&"credentials"));
        assert!(CONFIG_READABLE_KEYS.contains(&"modelRouter"));
    }

    #[test]
    fn forward_event_ask_question_format() {
        let event = AgentEvent::AskQuestion {
            turn_id: TurnId::new("turn-1"),
            request_id: "q1".into(),
            question: "Pick one".into(),
            options: vec![
                AskQuestionOption {
                    id: "a".into(),
                    label: "Option A".into(),
                },
                AskQuestionOption {
                    id: "b".into(),
                    label: "Option B".into(),
                },
            ],
            timeout_secs: 30,
            allow_multiple: false,
            session_id: None,
        };
        let resp = forward_event(&event, &Some("r1".into()));
        assert_eq!(resp.msg_type, "ask_question");
        let data = resp.data.unwrap();
        assert_eq!(data["request_id"], "q1");
        assert_eq!(data["question"], "Pick one");
        assert_eq!(data["options"].as_array().unwrap().len(), 2);
        assert_eq!(data["timeout_secs"], 30);
    }

    #[test]
    fn forward_event_turn_end_includes_session_id() {
        let turn_id = TurnId::new("turn-2");
        let event = AgentEvent::TurnEnd {
            turn_id: turn_id.clone(),
            summary: TurnSummary {
                turn_id,
                tool_calls_made: 2,
                iterations: 1,
                usage: None,
                elapsed_ms: 0,
                context_tokens: None,
                context_window: None,
            },
            session_id: Some("sess-123".into()),
            final_tool_calls: None,
        };
        let resp = forward_event(&event, &Some("r2".into()));
        assert_eq!(resp.msg_type, "turn_end");
        let data = resp.data.unwrap();
        assert_eq!(data["session_id"], "sess-123");
        assert_eq!(data["summary"]["tool_calls_made"], 2);
        assert_eq!(data["summary"]["iterations"], 1);
    }
}
