pub mod commands;
pub mod embedded;

use embedded::{GatewayInfo, GatewayProcess};
use serde_json::json;
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
    pub startup_watch: tokio::sync::watch::Receiver<GatewayStartupState>,
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
        .tooltip("XiaoLin")
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
    xiaolin_observe::init_observability_with_level("pretty", default_level);

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
        .setup(|app| {
            let (watch_tx, watch_rx) = tokio::sync::watch::channel(GatewayStartupState::Starting);

            app.manage(AppData {
                gateway: Mutex::new(None),
                startup_watch: watch_rx,
            });

            app.manage(commands::clipboard::ClipboardState(
                std::sync::Mutex::new(None),
            ));

            app.manage(commands::audio_capture::AudioCaptureState::new());

            setup_tray(app)?;

            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_shadow(true);
                let effects = tauri::window::EffectsBuilder::new()
                    .effects(vec![tauri::window::Effect::WindowBackground])
                    .radius(10.0)
                    .build();
                let _ = window.set_effects(effects);
            }

            let gs = app.global_shortcut();
            let _ = gs.unregister_all();

            if let Ok(shortcut) = "ctrl+shift+space".parse::<Shortcut>() {
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
                    tracing::debug!("Global shortcut Ctrl+Shift+Space skipped: {e}");
                }
            }

            if let Ok(shortcut) = "ctrl+shift+l".parse::<Shortcut>() {
                let handle_for_qa = app.handle().clone();
                if let Err(e) = gs.on_shortcut(shortcut, move |_app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        if let Some(w) = handle_for_qa.get_webview_window("quick-action") {
                            if w.is_visible().unwrap_or(false) {
                                let _ = w.hide();
                            } else {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                    }
                }) {
                    tracing::debug!("Global shortcut Ctrl+Shift+L skipped: {e}");
                }
            }

            let handle = app.handle().clone();

            tauri::async_runtime::spawn(async move {
                let config_mode = if cfg!(debug_assertions) {
                    xiaolin_core::config::ConfigMode::Development
                } else {
                    xiaolin_core::config::ConfigMode::Production
                };

                match GatewayProcess::start(&config_mode).await {
                    Ok(gw) => {
                        let state = handle.state::<AppData>();
                        let info = gw.info().clone();
                        *state.gateway.lock().await = Some(gw);

                        let _ = watch_tx.send(GatewayStartupState::Running { info });
                        tracing::info!("gateway process ready");

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

                        let _ = watch_tx.send(GatewayStartupState::Failed {
                            error: error_msg.clone(),
                        });

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
                            .title("XiaoLin")
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
            commands::clipboard::clipboard_read_text,
            commands::clipboard::clipboard_write_text,
            commands::clipboard::clipboard_read_image,
            commands::clipboard::clipboard_write_image,
            commands::clipboard::read_image_file,
            commands::voice::transcribe_audio,
            commands::voice::stt_available,
            commands::audio_capture::native_audio_available,
            commands::audio_capture::start_native_recording,
            commands::audio_capture::stop_native_recording,
        ])
        .build(tauri::generate_context!())
        .expect("error while building XiaoLin app")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                let state = app.state::<AppData>();
                let mut guard = match state.gateway.try_lock() {
                    Ok(g) => g,
                    Err(_) => return,
                };
                if let Some(ref mut process) = *guard {
                    process.shutdown();
                }
            }
        });
}