pub mod browser_panel;
pub mod commands;
pub mod embedded;

use embedded::{GatewayInfo, GatewayProcess};
use serde_json::json;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Listener, Manager};
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

/// Determine the ConfigMode from `XIAOLIN_PROFILE` env var.
/// - unset / empty / "dev" → Development
/// - "prod" / "production" → Production
/// - anything else → Profile(name)
fn resolve_config_mode() -> xiaolin_core::config::ConfigMode {
    match std::env::var("XIAOLIN_PROFILE").ok().filter(|s| !s.is_empty()) {
        None => {
            if cfg!(debug_assertions) {
                xiaolin_core::config::ConfigMode::Development
            } else {
                xiaolin_core::config::ConfigMode::Production
            }
        }
        Some(ref p) if p == "dev" || p == "development" => {
            xiaolin_core::config::ConfigMode::Development
        }
        Some(ref p) if p == "prod" || p == "production" => {
            xiaolin_core::config::ConfigMode::Production
        }
        Some(name) => xiaolin_core::config::ConfigMode::Profile(name),
    }
}

fn sanitize_profile_for_js(raw: &str) -> String {
    if raw
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        raw.to_string()
    } else {
        "default".to_string()
    }
}

fn local_storage_isolation_plugin() -> impl tauri::plugin::Plugin<tauri::Wry> {
    struct StorageIsolation {
        script: Option<String>,
    }
    impl tauri::plugin::Plugin<tauri::Wry> for StorageIsolation {
        fn name(&self) -> &'static str {
            "storage-isolation"
        }
        fn initialization_script(&self) -> Option<String> {
            self.script.clone()
        }
    }

    let script = std::env::var("XIAOLIN_PROFILE")
        .ok()
        .filter(|s| !s.is_empty() && s != "dev" && s != "development")
        .map(|profile| {
            let profile = sanitize_profile_for_js(&profile);
            format!(
                r#"const __P="xiaolin:{profile}:";const __S=window.localStorage;
Object.defineProperty(window,'localStorage',{{value:new Proxy(__S,{{get(t,p){{
if(p==='getItem')return k=>__S.getItem(__P+k);
if(p==='setItem')return(k,v)=>__S.setItem(__P+k,v);
if(p==='removeItem')return k=>__S.removeItem(__P+k);
if(p==='clear')return()=>{{for(let i=__S.length-1;i>=0;i--){{const k=__S.key(i);if(k&&k.startsWith(__P))__S.removeItem(k);}}}};
if(p==='length'){{let c=0;for(let i=0;i<__S.length;i++)if(__S.key(i)&&__S.key(i).startsWith(__P))c++;return c;}}
if(p==='key')return i=>{{let c=0;for(let j=0;j<__S.length;j++){{const k=__S.key(j);if(k&&k.startsWith(__P)){{if(c===i)return k.slice(__P.length);c++;}}}};return null;}};
return Reflect.get(t,p);}}}}),configurable:true}});"#,
                profile = profile
            )
        });

    StorageIsolation { script }
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

    let handle = app.handle().clone();
    app.listen("tray-pending-update", move |event: tauri::Event| {
        let has_pending: bool = event.payload().contains("true");
        if let Some(tray) = handle.tray_by_id("main-tray") {
            let tooltip = if has_pending {
                "XiaoLin (待处理)"
            } else {
                "XiaoLin"
            };
            let _ = tray.set_tooltip(Some(tooltip));
        }
    });

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
        .register_asynchronous_uri_scheme_protocol(
            "xiaolin-internal",
            browser_panel::handle_xiaolin_internal_protocol,
        )
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin({
            tauri_plugin_connector::ConnectorBuilder::new()
                .bind_address("127.0.0.1")
                .port_range(9555, 9556)
                .mcp_port_range(9556, 9557)
                .build()
        })
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init());

    let build_result = builder
        .plugin(local_storage_isolation_plugin())
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

            app.manage(browser_panel::BrowserPanelState(std::sync::Mutex::new(
                browser_panel::BrowserPanelManager::new(),
            )));

            setup_tray(app)?;

            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }

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
                let config_mode = resolve_config_mode();

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
                        tracing::error!(error = %e, "failed to start gateway");

                        let user_message = "Gateway failed to start".to_string();
                        let _ = watch_tx.send(GatewayStartupState::Failed {
                            error: user_message.clone(),
                        });

                        let _ = handle.emit(
                            "gateway://started",
                            json!({
                                "status": "error",
                                "message": user_message
                            }),
                        );

                        use tauri_plugin_notification::NotificationExt;
                        let _ = handle
                            .notification()
                            .builder()
                            .title("XiaoLin")
                            .body("Gateway 启动失败，请查看日志了解详情")
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
            commands::http_proxy::http_proxy,
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
            commands::file_viewer::read_file_for_viewer,
            commands::file_viewer::list_directory,
            commands::file_viewer::read_binary_for_viewer,
            commands::browser::browser_open_page,
            commands::browser::browser_close_page,
            commands::browser::browser_navigate,
            commands::browser::browser_go_back,
            commands::browser::browser_go_forward,
            commands::browser::browser_reload,
            commands::browser::browser_resize_webview,
            commands::browser::browser_list_pages,
            commands::browser::browser_show_page,
            commands::browser::browser_hide_all_pages,
            commands::browser::browser_eval_js,
        ])
        .build(tauri::generate_context!());

    let app = match build_result {
        Ok(app) => app,
        Err(e) => {
            eprintln!("error while building XiaoLin app: {e}");
            std::process::exit(1);
        }
    };

    app.run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                let state = app.state::<AppData>();
                let lock_result = tauri::async_runtime::block_on(async {
                    tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        state.gateway.lock(),
                    )
                    .await
                });
                match lock_result {
                    Ok(mut guard) => {
                        if let Some(ref mut process) = *guard {
                            process.shutdown();
                        }
                    }
                    Err(_) => {
                        tracing::warn!(
                            "gateway shutdown: failed to acquire lock within 5s, embedded gateway may not shut down cleanly"
                        );
                    }
                }
            }
        });
}