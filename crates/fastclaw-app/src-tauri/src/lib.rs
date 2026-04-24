pub mod commands;
pub mod embedded;

use embedded::{EmbeddedGateway, GatewayInfo};
use std::collections::HashMap;
use std::sync::Arc;
use serde_json::json;
use tauri::{Emitter, Manager};
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tokio::sync::Mutex;

/// Gateway 启动状态
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "status")]
pub enum GatewayStartupState {
    /// 正在启动
    Starting,
    /// 启动成功
    Running { info: GatewayInfo },
    /// 启动失败
    Failed { error: String },
}

pub struct AppData {
    pub gateway: Mutex<Option<EmbeddedGateway>>,
    pub stream_cancels: Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<()>>>>,
    /// Gateway 启动状态，用于前端轮询或通知
    pub gateway_startup_state: Arc<Mutex<GatewayStartupState>>,
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItemBuilder::with_id("show", "显示窗口").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "退出").build(app)?;
    let menu = MenuBuilder::new(app).item(&show).separator().item(&quit).build()?;

    let _tray = TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().cloned().unwrap_or_else(|| {
            tauri::image::Image::new(&[], 0, 0)
        }))
        .menu(&menu)
        .tooltip("FastClaw")
        .on_menu_event(move |app, event| {
            match event.id().as_ref() {
                "show" => {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }
                "quit" => {
                    app.exit(0);
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                button_state: tauri::tray::MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        })
        .build(app)?;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    fastclaw_observe::init_observability("pretty");

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_mcp_bridge::init());

    builder
        .manage(AppData {
            gateway: Mutex::new(None),
            stream_cancels: Arc::new(Mutex::new(HashMap::new())),
            gateway_startup_state: Arc::new(Mutex::new(GatewayStartupState::Starting)),
        })
        .setup(|app| {
            setup_tray(app)?;

            // Global shortcut: Ctrl+Shift+Space to toggle window
            if let Ok(shortcut) = "ctrl+shift+space".parse::<Shortcut>() {
                let handle_for_shortcut = app.handle().clone();
                if let Err(e) = app.global_shortcut().on_shortcut(shortcut, move |_app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        if let Some(w) = handle_for_shortcut.get_webview_window("main") {
                            if w.is_visible().unwrap_or(false) {
                                let _ = w.hide();
                            } else {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                    }
                }) {
                    tracing::warn!("Failed to register global shortcut: {e}");
                }
            }

            let handle = app.handle().clone();
            let startup_state = handle.state::<AppData>().gateway_startup_state.clone();

            tauri::async_runtime::spawn(async move {
                match EmbeddedGateway::start(false, None).await {
                    Ok(gw) => {
                        // Subscribe to gateway broadcast events and re-emit as Tauri events.
                        // This bridges cron notifications (and other push events) to the
                        // frontend in embedded (non-WS) mode.
                        let mut broadcast_rx = gw.app_state().ws_broadcast.subscribe();
                        let handle_for_broadcast = handle.clone();
                        tauri::async_runtime::spawn(async move {
                            loop {
                                match broadcast_rx.recv().await {
                                    Ok(event_json) => {
                                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&event_json) {
                                            let event_name = val.get("event")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("");
                                            if event_name.is_empty() { continue; }

                                            let data = val.get("data").cloned().unwrap_or(serde_json::Value::Null);
                                            let tauri_name = event_name.replace('.', "-");
                                            let _ = handle_for_broadcast.emit(tauri_name.as_str(), data.clone());

                                            // For new notifications: fire OS notification + update tray
                                            if event_name == "notification.new" {
                                                let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("FastClaw");
                                                let body = data.get("body").and_then(|v| v.as_str()).unwrap_or("");
                                                {
                                                    use tauri_plugin_notification::NotificationExt;
                                                    let _ = handle_for_broadcast.notification()
                                                        .builder()
                                                        .title(title)
                                                        .body(body)
                                                        .show();
                                                }
                                                // Update tray tooltip with unread count
                                                if let Some(uc) = data.get("unreadCount").and_then(|v| v.as_i64()) {
                                                    if let Some(tray) = handle_for_broadcast.tray_by_id("main-tray") {
                                                        let tooltip = if uc > 0 {
                                                            format!("FastClaw ({uc} 条未读)")
                                                        } else {
                                                            "FastClaw".to_string()
                                                        };
                                                        let _ = tray.set_tooltip(Some(&tooltip));
                                                    }
                                                }
                                            }

                                            // On read events, update tray tooltip too
                                            if event_name == "notification.read" {
                                                if let Some(uc) = data.get("unreadCount").and_then(|v| v.as_i64()) {
                                                    if let Some(tray) = handle_for_broadcast.tray_by_id("main-tray") {
                                                        let tooltip = if uc > 0 {
                                                            format!("FastClaw ({uc} 条未读)")
                                                        } else {
                                                            "FastClaw".to_string()
                                                        };
                                                        let _ = tray.set_tooltip(Some(&tooltip));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                                }
                            }
                        });

                        let state = handle.state::<AppData>();
                        let mut lock = state.gateway.lock().await;
                        let info = gw.info().clone();
                        *lock = Some(gw);
                        
                        // 更新状态为 Running
                        let mut startup_state = startup_state.lock().await;
                        *startup_state = GatewayStartupState::Running { info };
                        
                        tracing::info!("embedded gateway started successfully");
                        
                        // 发送通知到前端
                        let _ = handle.emit("gateway://started", json!({
                            "status": "success",
                            "message": "Gateway 启动成功"
                        }));
                    }
                    Err(e) => {
                        let error_msg = format!("failed to start embedded gateway: {e}");
                        tracing::error!("{}", error_msg);
                        
                        // 更新状态为 Failed
                        let mut startup_state = startup_state.lock().await;
                        *startup_state = GatewayStartupState::Failed { error: error_msg.clone() };
                        
                        // 发送通知到前端
                        let _ = handle.emit("gateway://started", json!({
                            "status": "error",
                            "message": error_msg
                        }));
                        
                        use tauri_plugin_notification::NotificationExt;
                        let _ = handle.notification()
                            .builder()
                            .title("FastClaw")
                            .body(format!("Gateway 启动失败：{e}"))
                            .show();
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::test_model_connection,
            commands::get_gateway_info,
            commands::health_check,
            commands::list_agents,
            commands::list_sessions,
            commands::get_session,
            commands::set_session_work_dir,
            commands::get_session_messages,
            commands::create_session,
            commands::update_session_title,
            commands::delete_session,
            commands::list_models,
            commands::get_config,
            commands::set_config,
            commands::list_skills,
            commands::refresh_skills,
            commands::upload_skill,
            commands::list_tools,
            commands::list_agent_tools,
            commands::get_agent,
            commands::update_agent,
            commands::create_agent,
            commands::delete_agent,
            commands::read_identity_files,
            commands::upload_agent_avatar,
            commands::list_channels,
            commands::bind_agent_channel,
            commands::unbind_agent_channel,
            commands::update_agent_tools,
            commands::chat_stream,
            commands::cancel_chat_stream,
            commands::submit_tool_answer,
            commands::get_mcp_status,
            commands::reload_mcp_servers,
            commands::add_mcp_server,
            commands::remove_mcp_server,
            commands::cron_list_jobs,
            commands::cron_get_job,
            commands::cron_upsert_job,
            commands::cron_delete_job,
            commands::cron_list_runs,
            commands::notification_list,
            commands::notification_get,
            commands::notification_mark_read,
            commands::notification_mark_all_read,
            commands::notification_unread_count,
            commands::notification_delete,
            commands::notification_clear_read,
        ])
        .run(tauri::generate_context!())
        .expect("error while running FastClaw app");
}
