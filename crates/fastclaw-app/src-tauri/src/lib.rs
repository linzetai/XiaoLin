pub mod commands;
pub mod embedded;

use embedded::{EmbeddedGateway, GatewayInfo};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager};
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
    let menu = MenuBuilder::new(app)
        .item(&show)
        .separator()
        .item(&quit)
        .build()?;

    let _tray = TrayIconBuilder::with_id("main-tray")
        .icon(
            app.default_window_icon()
                .cloned()
                .unwrap_or_else(|| tauri::image::Image::new(&[], 0, 0)),
        )
        .menu(&menu)
        .tooltip("FastClaw")
        .on_menu_event(move |app, event| match event.id().as_ref() {
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
    let default_level = if cfg!(debug_assertions) {
        Some("info")
    } else {
        None
    };
    fastclaw_observe::init_observability_with_level("pretty", default_level);

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
        .plugin(tauri_plugin_mcp_bridge::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init());

    builder
        .manage(AppData {
            gateway: Mutex::new(None),
            stream_cancels: Arc::new(Mutex::new(HashMap::new())),
            gateway_startup_state: Arc::new(Mutex::new(GatewayStartupState::Starting)),
        })
        .setup(|app| {
            setup_tray(app)?;

            // macOS: re-enable native shadow on transparent window and set
            // framework-level rounded corners via windowEffects.
            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_shadow(true);
                let effects = tauri::window::EffectsBuilder::new()
                    .effects(vec![tauri::window::Effect::WindowBackground])
                    .radius(10.0)
                    .build();
                let _ = window.set_effects(effects);
            }

            // Global shortcut: Ctrl+Shift+Space to toggle window
            if let Ok(shortcut) = "ctrl+shift+space".parse::<Shortcut>() {
                let handle_for_shortcut = app.handle().clone();
                if let Err(e) =
                    app.global_shortcut()
                        .on_shortcut(shortcut, move |_app, _shortcut, event| {
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
                        })
                {
                    tracing::warn!("Failed to register global shortcut: {e}");
                }
            }

            let handle = app.handle().clone();
            let startup_state = handle.state::<AppData>().gateway_startup_state.clone();

            tauri::async_runtime::spawn(async move {
                // 根据编译模式自动选择配置模式
                let config_mode = if cfg!(debug_assertions) {
                    fastclaw_core::config::ConfigMode::Development
                } else {
                    fastclaw_core::config::ConfigMode::Production
                };

                match EmbeddedGateway::start(&config_mode).await {
                    Ok(gw) => {
                        // Subscribe to gateway broadcast events and re-emit as Tauri events.
                        // This bridges cron notifications (and other push events) to the
                        // frontend in embedded (non-WS) mode.
                        let mut broadcast_rx = gw.app_state().strm.ws_broadcast.subscribe();
                        let handle_for_broadcast = handle.clone();
                        tauri::async_runtime::spawn(async move {
                            loop {
                                match broadcast_rx.recv().await {
                                    Ok(event_json) => {
                                        if let Ok(val) =
                                            serde_json::from_str::<serde_json::Value>(&event_json)
                                        {
                                            let event_name = val
                                                .get("event")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("");
                                            if event_name.is_empty() {
                                                continue;
                                            }

                                            let data = val
                                                .get("data")
                                                .cloned()
                                                .unwrap_or(serde_json::Value::Null);
                                            let tauri_name = event_name.replace('.', "-");
                                            let _ = handle_for_broadcast
                                                .emit(tauri_name.as_str(), data.clone());

                                            // For new notifications: fire OS notification + update tray
                                            if event_name == "notification.new" {
                                                let title = data
                                                    .get("title")
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("FastClaw");
                                                let body = data
                                                    .get("body")
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("");
                                                {
                                                    use tauri_plugin_notification::NotificationExt;
                                                    let _ = handle_for_broadcast
                                                        .notification()
                                                        .builder()
                                                        .title(title)
                                                        .body(body)
                                                        .show();
                                                }
                                                // Update tray tooltip with unread count
                                                if let Some(uc) =
                                                    data.get("unreadCount").and_then(|v| v.as_i64())
                                                {
                                                    if let Some(tray) =
                                                        handle_for_broadcast.tray_by_id("main-tray")
                                                    {
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
                                                if let Some(uc) =
                                                    data.get("unreadCount").and_then(|v| v.as_i64())
                                                {
                                                    if let Some(tray) =
                                                        handle_for_broadcast.tray_by_id("main-tray")
                                                    {
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
                                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                        continue
                                    }
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
                        let _ = handle.emit(
                            "gateway://started",
                            json!({
                                "status": "success",
                                "message": "Gateway 启动成功"
                            }),
                        );
                    }
                    Err(e) => {
                        let error_msg = format!("failed to start embedded gateway: {e}");
                        tracing::error!("{}", error_msg);

                        // 更新状态为 Failed
                        let mut startup_state = startup_state.lock().await;
                        *startup_state = GatewayStartupState::Failed {
                            error: error_msg.clone(),
                        };

                        // 发送通知到前端
                        let _ = handle.emit(
                            "gateway://started",
                            json!({
                                "status": "error",
                                "message": error_msg
                            }),
                        );

                        use tauri_plugin_notification::NotificationExt;
                        let _ = handle
                            .notification()
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
            commands::config::test_model_connection,
            commands::config::get_gateway_info,
            commands::config::health_check,
            commands::agent::list_agents,
            commands::session::list_sessions,
            commands::session::get_session,
            commands::session::set_session_work_dir,
            commands::session::get_session_messages,
            commands::session::create_session,
            commands::session::update_session_title,
            commands::session::delete_session,
            commands::session::export_session_content,
            commands::config::list_models,
            commands::config::get_config,
            commands::config::set_config,
            commands::skill::list_skills,
            commands::skill::refresh_skills,
            commands::skill::upload_skill,
            commands::agent::list_tools,
            commands::agent::list_agent_tools,
            commands::agent::get_agent,
            commands::agent::update_agent,
            commands::agent::create_agent,
            commands::agent::delete_agent,
            commands::agent::read_identity_files,
            commands::agent::upload_agent_avatar,
            commands::channel::list_channels,
            commands::channel::bind_agent_channel,
            commands::channel::unbind_agent_channel,
            commands::channel::reload_channel,
            commands::agent::update_agent_tools,
            commands::chat::chat_stream,
            commands::chat::cancel_chat_stream,
            commands::chat::submit_tool_answer,
            commands::chat::set_execution_mode,
            commands::mcp::get_mcp_status,
            commands::mcp::reload_mcp_servers,
            commands::mcp::add_mcp_server,
            commands::mcp::remove_mcp_server,
            commands::cron::cron_list_jobs,
            commands::cron::cron_get_job,
            commands::cron::cron_upsert_job,
            commands::cron::cron_delete_job,
            commands::cron::cron_list_runs,
            commands::notification::notification_list,
            commands::notification::notification_get,
            commands::notification::notification_mark_read,
            commands::notification::notification_mark_all_read,
            commands::notification::notification_unread_count,
            commands::notification::notification_delete,
            commands::notification::notification_clear_read,
            commands::migration::import_data,
            commands::migration::export_data,
        ])
        .run(tauri::generate_context!())
        .expect("error while running FastClaw app");
}
