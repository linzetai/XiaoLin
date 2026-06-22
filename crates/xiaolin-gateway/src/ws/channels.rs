use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use serde_json::json;

use xiaolin_core::channel::ChannelPlugin;
use xiaolin_core::config_access::persist_config_key;

use crate::state::AppState;

use super::send_resp;
use super::types::WsResponse;

pub async fn handle_channels_list(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
) {
    let registry = state.ext.channel_registry.read().await;
    let mut channels = Vec::new();

    for ch in registry.all_plugins() {
        let meta = ch.meta();
        let caps = ch.capabilities();
        let mode = ch.connection_mode();
        let healthy = ch.probe().await.unwrap_or(false);

        channels.push(json!({
            "id": meta.id,
            "name": meta.name,
            "description": meta.description,
            "aliases": meta.aliases,
            "status": if healthy { "connected" } else { "disconnected" },
            "connectionMode": mode,
            "capabilities": {
                "directMessage": caps.direct_message,
                "groupChat": caps.group_chat,
                "media": caps.media,
                "streaming": caps.streaming,
                "reactions": caps.reactions,
                "threads": caps.threads,
            },
        }));
    }

    // Also list known channel types that are not registered (available but not connected).
    let registered_ids: Vec<String> = channels
        .iter()
        .filter_map(|c| c.get("id").and_then(|v| v.as_str()).map(String::from))
        .collect();
    drop(registry);

    let known_types = [
        ("wechat", "WeChat", "微信消息通道 — 通过扫码连接", "longpoll"),
        ("feishu", "飞书 / Lark", "飞书消息通道 — 需配置应用凭证", "websocket"),
    ];

    for (id, name, desc, mode) in &known_types {
        if !registered_ids.iter().any(|r| r == id) {
            let configured = state.cfg.config.channels.contains_key(*id);
            channels.push(json!({
                "id": id,
                "name": name,
                "description": desc,
                "aliases": [],
                "status": if configured { "configured" } else { "available" },
                "connectionMode": mode,
                "capabilities": {},
            }));
        }
    }

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "channels.list".into(),
            data: Some(json!({ "channels": channels })),
            error: None,
        },
    )
    .await;
}

fn mask_sensitive(s: &str) -> String {
    if s.len() <= 4 {
        "****".to_string()
    } else {
        format!("{}****", &s[..s.floor_char_boundary(4)])
    }
}

pub async fn handle_channels_detail(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    channel_id: &str,
) {
    let registry = state.ext.channel_registry.read().await;

    if let Some(plugin) = registry.get(channel_id) {
        let meta = plugin.meta().clone();
        let caps = plugin.capabilities();
        let mode = plugin.connection_mode().to_string();
        let healthy = plugin.probe().await.unwrap_or(false);
        let tool_list: Vec<serde_json::Value> = plugin
            .tools()
            .iter()
            .map(|t| {
                json!({
                    "name": t.name(),
                    "description": t.description(),
                })
            })
            .collect();
        drop(registry);

        let config = build_masked_channel_config(state, channel_id);
        let has_backup = has_channel_backup(state, channel_id);

        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "channels.detail".into(),
                data: Some(json!({
                    "id": meta.id,
                    "name": meta.name,
                    "description": meta.description,
                    "aliases": meta.aliases,
                    "status": if healthy { "connected" } else { "disconnected" },
                    "connectionMode": mode,
                    "capabilities": {
                        "directMessage": caps.direct_message,
                        "groupChat": caps.group_chat,
                        "media": caps.media,
                        "streaming": caps.streaming,
                        "reactions": caps.reactions,
                        "threads": caps.threads,
                    },
                    "tools": tool_list,
                    "config": config,
                    "hasBackup": has_backup,
                })),
                error: None,
            },
        )
        .await;
        return;
    }
    drop(registry);

    let known_types = [
        ("wechat", "WeChat", "微信消息通道 — 通过扫码连接", "longpoll"),
        ("feishu", "飞书 / Lark", "飞书消息通道 — 需配置应用凭证", "websocket"),
    ];

    if let Some((id, name, desc, mode)) = known_types.iter().find(|(kid, ..)| *kid == channel_id) {
        let configured = is_channel_configured_live(state, id);
        let config = build_masked_channel_config(state, id);
        let has_backup = has_channel_backup(state, id);
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "channels.detail".into(),
                data: Some(json!({
                    "id": id,
                    "name": name,
                    "description": desc,
                    "aliases": [],
                    "status": if configured { "configured" } else { "available" },
                    "connectionMode": mode,
                    "capabilities": {},
                    "tools": [],
                    "config": config,
                    "hasBackup": has_backup,
                })),
                error: None,
            },
        )
        .await;
        return;
    }

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "error".into(),
            data: None,
            error: Some(json!({"message": format!("channel '{}' not found", channel_id)})),
        },
    )
    .await;
}

fn build_masked_channel_config(state: &AppState, channel_id: &str) -> serde_json::Value {
    let live = state.cfg.config_live.load();
    let ch_val = live
        .get("channels")
        .and_then(|c| c.get(channel_id));

    let ch_val = match ch_val {
        Some(v) => v,
        None => {
            let boot = state.cfg.config.channels.get(channel_id);
            return match boot {
                Some(c) => {
                    let mask_opt = |v: &Option<String>| -> serde_json::Value {
                        match v {
                            Some(s) if !s.is_empty() => json!(mask_sensitive(s)),
                            _ => serde_json::Value::Null,
                        }
                    };
                    json!({
                        "appId": c.app_id,
                        "appSecret": mask_opt(&c.app_secret),
                        "verificationToken": mask_opt(&c.verification_token),
                        "encryptKey": mask_opt(&c.encrypt_key),
                        "domain": c.domain,
                        "replyMode": c.reply_mode,
                        "accountCount": c.accounts.len(),
                    })
                }
                None => json!({}),
            };
        }
    };

    let obj = ch_val.as_object();
    let mut result = serde_json::Map::new();

    let sensitive_keys = ["appSecret", "app_secret", "verificationToken", "verification_token", "encryptKey", "encrypt_key"];

    if let Some(obj) = obj {
        for (k, v) in obj {
            if sensitive_keys.iter().any(|s| s == k) {
                if let Some(s) = v.as_str() {
                    if !s.is_empty() {
                        result.insert(k.clone(), json!(mask_sensitive(s)));
                        continue;
                    }
                }
                result.insert(k.clone(), serde_json::Value::Null);
            } else {
                result.insert(k.clone(), v.clone());
            }
        }
    }

    serde_json::Value::Object(result)
}

fn has_channel_backup(state: &AppState, channel_id: &str) -> bool {
    let live = state.cfg.config_live.load();
    live.get("_backup")
        .and_then(|b| b.get("channels"))
        .and_then(|c| c.get(channel_id))
        .is_some()
}

fn is_channel_configured_live(state: &AppState, channel_id: &str) -> bool {
    let live = state.cfg.config_live.load();
    if let Some(channels) = live.get("channels") {
        if channels.get(channel_id).is_some() {
            return true;
        }
    }
    state.cfg.config.channels.contains_key(channel_id)
}

pub async fn handle_wechat_login(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
) {
    let existing_tokens: Vec<String> = xiaolin_wechat::auth::credential::list_credentials()
        .into_iter()
        .map(|(_, cred)| cred.token)
        .collect();

    match xiaolin_wechat::auth::qr_login::start_login(&existing_tokens).await {
        Ok(session) => {
            let key = session.session_key.clone();
            let qr_url = session.qr_url.clone();
            state.ext.wechat_login_sessions.insert(key.clone(), session);
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "channels.wechat_login".into(),
                    data: Some(json!({
                        "sessionKey": key,
                        "qrUrl": qr_url,
                        "status": "waiting",
                    })),
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
                    error: Some(json!({"message": format!("failed to start wechat login: {e}")})),
                },
            )
            .await;
        }
    }
}

pub async fn handle_wechat_poll(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    session_key: &str,
) {
    let mut entry = match state.ext.wechat_login_sessions.get_mut(session_key) {
        Some(e) => e,
        None => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"message": "login session not found"})),
                },
            )
            .await;
            return;
        }
    };

    let status = xiaolin_wechat::auth::qr_login::poll_login(&mut entry).await;
    use xiaolin_wechat::auth::qr_login::LoginStatus;

    match status {
        LoginStatus::Waiting => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "channels.wechat_poll".into(),
                    data: Some(json!({"status": "waiting"})),
                    error: None,
                },
            )
            .await;
        }
        LoginStatus::Scanned => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "channels.wechat_poll".into(),
                    data: Some(json!({"status": "scanned"})),
                    error: None,
                },
            )
            .await;
        }
        LoginStatus::NeedVerifyCode => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "channels.wechat_poll".into(),
                    data: Some(json!({"status": "need_verify_code", "message": "请输入手机微信显示的配对数字"})),
                    error: None,
                },
            )
            .await;
        }
        LoginStatus::Confirmed {
            bot_token,
            account_id,
            base_url,
            user_id,
        } => {
            let normalized_id =
                xiaolin_wechat::auth::credential::normalize_account_id(&account_id);
            let cred = xiaolin_wechat::auth::credential::WechatCredential {
                token: bot_token,
                base_url,
                user_id,
                cdn_base_url: None,
                created_at: Some(chrono::Utc::now().to_rfc3339()),
            };
            let _ = xiaolin_wechat::auth::credential::save_credential(&normalized_id, &cred);

            drop(entry);
            state.ext.wechat_login_sessions.remove(session_key);

            // Restart the wechat channel plugin to pick up new credentials.
            restart_wechat_channel(state).await;

            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "channels.wechat_poll".into(),
                    data: Some(json!({
                        "status": "confirmed",
                        "accountId": normalized_id,
                        "message": "微信连接成功",
                    })),
                    error: None,
                },
            )
            .await;

            let _ = state.strm.ws_broadcast.send(
                json!({"type": "event", "event": "channels.changed", "data": {"channelId": "wechat", "action": "connected"}}).to_string(),
            );
        }
        LoginStatus::AlreadyConnected => {
            drop(entry);
            state.ext.wechat_login_sessions.remove(session_key);
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "channels.wechat_poll".into(),
                    data: Some(json!({"status": "already_connected", "message": "已连接过此 XiaoLin"})),
                    error: None,
                },
            )
            .await;
        }
        LoginStatus::Expired => {
            let existing_tokens: Vec<String> =
                xiaolin_wechat::auth::credential::list_credentials()
                    .into_iter()
                    .map(|(_, c)| c.token)
                    .collect();
            let refresh_result =
                xiaolin_wechat::auth::qr_login::refresh_qr(&mut entry, &existing_tokens).await;
            match refresh_result {
                Ok(()) => {
                    let new_qr_url = entry.qr_url.clone();
                    send_resp(
                        sender,
                        &WsResponse {
                            id: req_id,
                            msg_type: "channels.wechat_poll".into(),
                            data: Some(json!({
                                "status": "expired_refreshed",
                                "qrUrl": new_qr_url,
                                "message": "二维码已刷新",
                            })),
                            error: None,
                        },
                    )
                    .await;
                }
                Err(e) => {
                    drop(entry);
                    state.ext.wechat_login_sessions.remove(session_key);
                    send_resp(
                        sender,
                        &WsResponse {
                            id: req_id,
                            msg_type: "error".into(),
                            data: None,
                            error: Some(json!({"message": format!("refresh QR failed: {e}")})),
                        },
                    )
                    .await;
                }
            }
        }
        LoginStatus::VerifyCodeBlocked => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "channels.wechat_poll".into(),
                    data: Some(json!({"status": "verify_blocked", "message": "验证码被拒绝，请重试"})),
                    error: None,
                },
            )
            .await;
        }
        LoginStatus::Error(msg) => {
            drop(entry);
            state.ext.wechat_login_sessions.remove(session_key);
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
    }
}

pub async fn handle_wechat_verify(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    session_key: &str,
    code: &str,
) {
    if let Some(mut entry) = state.ext.wechat_login_sessions.get_mut(session_key) {
        xiaolin_wechat::auth::qr_login::set_verify_code(&mut entry, code);
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "channels.wechat_verify".into(),
                data: Some(json!({"ok": true, "message": "验证码已提交，请继续轮询"})),
                error: None,
            },
        )
        .await;
    } else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"message": "login session not found"})),
            },
        )
        .await;
    }
}

pub async fn handle_channels_connect(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    channel_id: &str,
) {
    match state.reload_channel(channel_id).await {
        Ok(()) => {
            let _ = state.strm.ws_broadcast.send(
                json!({"type": "event", "event": "channels.changed", "data": {"channelId": channel_id, "action": "connected"}}).to_string(),
            );
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "channels.connect".into(),
                    data: Some(json!({"ok": true, "channelId": channel_id})),
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
                    error: Some(json!({"message": format!("failed to connect channel: {e}")})),
                },
            )
            .await;
        }
    }
}

pub async fn handle_channels_update(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    channel_id: &str,
    new_config: serde_json::Value,
) {
    let config_key = format!("channels.{channel_id}");

    // 1. Backup current config before overwriting
    {
        let live = state.cfg.config_live.load();
        if let Some(channels) = live.get("channels") {
            if let Some(old_cfg) = channels.get(channel_id) {
                let backup_key = format!("_backup.channels.{channel_id}");
                let _ = persist_config_key(&backup_key, old_cfg);

                let mut live_clone: serde_json::Value = (**live).clone();
                if live_clone.get("_backup").is_none() {
                    live_clone["_backup"] = json!({});
                }
                if live_clone["_backup"].get("channels").is_none() {
                    live_clone["_backup"]["channels"] = json!({});
                }
                live_clone["_backup"]["channels"][channel_id] = old_cfg.clone();
                state.cfg.config_live.store(Arc::new(live_clone));
            }
        }
    }

    // 2. Merge new config: take existing, overlay with provided fields
    let merged_config = {
        let live = state.cfg.config_live.load();
        let mut base = live
            .get("channels")
            .and_then(|c| c.get(channel_id))
            .cloned()
            .unwrap_or(json!({}));
        if let (Some(base_obj), Some(new_obj)) = (base.as_object_mut(), new_config.as_object()) {
            for (k, v) in new_obj {
                base_obj.insert(k.clone(), v.clone());
            }
        }
        base
    };

    // 3. Persist + update config_live
    if let Err(e) = persist_config_key(&config_key, &merged_config) {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"message": format!("failed to persist config: {e}")})),
            },
        )
        .await;
        return;
    }

    {
        let mut live_clone: serde_json::Value = (**state.cfg.config_live.load()).clone();
        if live_clone.get("channels").is_none() {
            live_clone["channels"] = json!({});
        }
        live_clone["channels"][channel_id] = merged_config;
        state.cfg.config_live.store(Arc::new(live_clone));
    }

    // 4. Hot-reload the channel plugin
    let reload_result = state.reload_channel(channel_id).await;
    let reload_ok = reload_result.is_ok();
    let reload_err = reload_result.err().map(|e| e.to_string());

    // 5. Broadcast change event
    let _ = state.strm.ws_broadcast.send(
        json!({"type": "event", "event": "channels.changed", "data": {"channelId": channel_id, "action": "updated"}}).to_string(),
    );

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "channels.update".into(),
            data: Some(json!({
                "ok": reload_ok,
                "channelId": channel_id,
                "reloadError": reload_err,
                "hasBackup": true,
            })),
            error: None,
        },
    )
    .await;
}

pub async fn handle_channels_restore(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    channel_id: &str,
) {
    let backup = {
        let live = state.cfg.config_live.load();
        live.get("_backup")
            .and_then(|b| b.get("channels"))
            .and_then(|c| c.get(channel_id))
            .cloned()
    };

    let backup = match backup {
        Some(b) => b,
        None => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"message": format!("no backup found for channel '{channel_id}'")})),
                },
            )
            .await;
            return;
        }
    };

    let config_key = format!("channels.{channel_id}");
    if let Err(e) = persist_config_key(&config_key, &backup) {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"message": format!("failed to persist restored config: {e}")})),
            },
        )
        .await;
        return;
    }

    {
        let mut live_clone: serde_json::Value = (**state.cfg.config_live.load()).clone();
        live_clone["channels"][channel_id] = backup;
        state.cfg.config_live.store(Arc::new(live_clone));
    }

    let reload_result = state.reload_channel(channel_id).await;
    let reload_ok = reload_result.is_ok();
    let reload_err = reload_result.err().map(|e| e.to_string());

    let _ = state.strm.ws_broadcast.send(
        json!({"type": "event", "event": "channels.changed", "data": {"channelId": channel_id, "action": "restored"}}).to_string(),
    );

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "channels.restore".into(),
            data: Some(json!({
                "ok": reload_ok,
                "channelId": channel_id,
                "reloadError": reload_err,
            })),
            error: None,
        },
    )
    .await;
}

pub async fn handle_channels_disconnect(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    channel_id: &str,
    _account_id: Option<&str>,
) {
    let mut registry = state.ext.channel_registry.write().await;
    if let Some(plugin) = registry.unregister(channel_id) {
        if let Err(e) = plugin.stop().await {
            tracing::warn!(channel_id, error = %e, "error stopping channel plugin");
        }
        drop(registry);

        let _ = state.strm.ws_broadcast.send(
            json!({"type": "event", "event": "channels.changed", "data": {"channelId": channel_id, "action": "disconnected"}}).to_string(),
        );

        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "channels.disconnect".into(),
                data: Some(json!({"ok": true, "channelId": channel_id})),
                error: None,
            },
        )
        .await;
    } else {
        drop(registry);
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"message": format!("channel '{channel_id}' not found")})),
            },
        )
        .await;
    }
}

async fn restart_wechat_channel(state: &AppState) {
    let mut registry = state.ext.channel_registry.write().await;
    if let Some(old_plugin) = registry.unregister("wechat") {
        let _ = old_plugin.stop().await;
    }

    let wechat_config = state
        .cfg
        .config
        .channels
        .get("wechat")
        .and_then(xiaolin_wechat::WechatChannelConfig::from_channel_config)
        .unwrap_or_default();

    let plugin = std::sync::Arc::new(xiaolin_wechat::WechatPlugin::new(wechat_config));
    if let Err(e) = plugin.start(state.ext.channel_inbound_tx.clone()).await {
        tracing::error!(error = %e, "failed to restart wechat channel plugin");
        return;
    }

    registry.register(plugin);
    tracing::info!("wechat channel plugin restarted with new credentials");
}
