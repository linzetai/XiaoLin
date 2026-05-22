pub mod commands;
pub mod embedded;

use embedded::{GatewayInfo, GatewayProcess};
use serde_json::json;
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

/// App state for Tauri.
///
/// Only holds the gateway process manager. All business logic
/// (chat, sessions, agents, etc.) goes through WebSocket to the Gateway.
pub struct AppData {
    pub gateway: Mutex<Option<GatewayProcess>>,
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

            if let Ok(shortcut) = "ctrl+shift+space".parse::<Shortcut>() {
                let gs = app.global_shortcut();
                let _ = gs.unregister_all();
                let handle_for_shortcut = app.handle().clone();
                if let Err(e) = gs.on_shortcut(shortcut, move |_app, _shortcut, event| {
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
                    tracing::debug!("Global shortcut registration skipped: {e}");
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

                match GatewayProcess::start(&config_mode).await {
                    Ok(gw) => {
                        let state = handle.state::<AppData>();
                        let mut lock = state.gateway.lock().await;
                        let info = gw.info().clone();
                        *lock = Some(gw);

                        // 更新状态为 Running
                        let mut startup_state = startup_state.lock().await;
                        *startup_state = GatewayStartupState::Running { info };

                        tracing::info!("gateway process ready");

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
                        let error_msg = format!("failed to start gateway: {e}");
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
        // Only register IPC commands for local file operations
        // All business logic goes through WebSocket
        .invoke_handler(tauri::generate_handler![
            commands::config::get_gateway_info,
            commands::session::export_session_content,
            commands::agent::upload_agent_avatar,
            commands::agent::read_identity_files,
            commands::skill::upload_skill,
            commands::migration::import_data,
            commands::migration::export_data,
            commands::clipboard::clipboard_read_image,
            commands::clipboard::read_image_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running FastClaw app");
}