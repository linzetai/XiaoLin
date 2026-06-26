use crate::browser_network::BrowserNetworkState;
use crate::browser_panel::{
    browser_data_directory, default_download_directory, sanitize_download_filename,
    validate_browser_url, validate_js_payload, validate_page_id, BrowserPage, BrowserPanelManager,
    BrowserPanelState, PageLoadState, PageVisibility, BROWSER_INIT_SCRIPT, BROWSER_WEBVIEW_PREFIX,
    FAVICON_EXTRACT_JS, OFFSCREEN_POSITION,
};
#[cfg(target_os = "macos")]
use crate::browser_panel::browser_data_store_identifier;
use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::utils::config::WebviewUrl;
use serde::Deserialize;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, State, Url, Wry};
use uuid::Uuid;
use xiaolin_tools_browser::{CONTENT_EXTRACT_JS, SELECTION_TOOLBAR_JS};
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserWebviewLayout {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    scale_factor: Option<f64>,
}

impl BrowserWebviewLayout {
    fn validate(&self) -> Result<(), String> {
        if !self.x.is_finite()
            || !self.y.is_finite()
            || !self.width.is_finite()
            || !self.height.is_finite()
        {
            return Err("invalid layout coordinates".into());
        }
        if self.width < 0.0
            || self.height < 0.0
            || self.width > 10000.0
            || self.height > 10000.0
        {
            return Err("invalid layout size".into());
        }
        Ok(())
    }
}


fn with_manager<T>(
    state: &State<'_, BrowserPanelState>,
    f: impl FnOnce(&mut BrowserPanelManager) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = state
        .0
        .lock()
        .map_err(|_| "browser manager lock poisoned".to_string())?;
    f(&mut guard)
}

fn get_webview(app: &AppHandle, label: &str) -> Result<tauri::Webview<Wry>, String> {
    app.get_webview(label)
        .ok_or_else(|| "browser webview not found".to_string())
}

fn apply_webview_layout(
    app: &AppHandle,
    page: &BrowserPage,
    scale_factor: f64,
) -> Result<(), String> {
    apply_webview_layouts_batch(app, std::slice::from_ref(page), scale_factor)
}

fn apply_webview_layouts_batch(
    app: &AppHandle,
    pages: &[BrowserPage],
    scale_factor: f64,
) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        apply_webview_layouts_gtk_batch(app, pages, scale_factor)
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = scale_factor;
        for page in pages {
            let webview = get_webview(app, &page.webview_label)?;
            if page.visibility == PageVisibility::Active
                && page.layout_width > 0.0
                && page.layout_height > 0.0
            {
                webview
                    .set_position(LogicalPosition::new(page.layout_x, page.layout_y))
                    .map_err(|_| "failed to position browser webview".to_string())?;
                webview
                    .set_size(LogicalSize::new(page.layout_width, page.layout_height))
                    .map_err(|_| "failed to resize browser webview".to_string())?;
            } else {
                webview
                    .set_position(LogicalPosition::new(OFFSCREEN_POSITION, OFFSCREEN_POSITION))
                    .map_err(|_| "failed to hide browser webview".to_string())?;
            }
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn apply_webview_layouts_gtk_batch(
    app: &AppHandle,
    pages: &[BrowserPage],
    scale_factor: f64,
) -> Result<(), String> {
    if pages.is_empty() {
        return Ok(());
    }

    let sf = if scale_factor > 0.0 { scale_factor } else { 1.0 };

    let positions: Vec<(String, i32, i32, i32, i32, bool)> = pages
        .iter()
        .map(|page| {
            let visible = page.visibility == PageVisibility::Active
                && page.layout_width > 0.0
                && page.layout_height > 0.0;
            let x = if visible { (page.layout_x * sf) as i32 } else { -9999 };
            let y = if visible { (page.layout_y * sf) as i32 } else { -9999 };
            let w = if visible { (page.layout_width * sf).ceil() as i32 } else { 1 };
            let h = if visible { (page.layout_height * sf).ceil() as i32 } else { 1 };
            (page.webview_label.clone(), x, y, w, h, visible)
        })
        .collect();

    let window = app
        .get_window("main")
        .ok_or_else(|| "main window not found".to_string())?;

    let (tx, rx) = std::sync::mpsc::channel();
    window
        .run_on_main_thread(move || {
            for (label, x, y, w, h, visible) in &positions {
                crate::browser_gtk::position_child(label, *x, *y, *w, *h, *visible);
            }
            let _ = tx.send(());
        })
        .map_err(|_| "failed to dispatch to main thread".to_string())?;

    rx.recv()
        .map_err(|_| "main thread channel closed".to_string())?;
    Ok(())
}

fn build_browser_webview(
    app: &AppHandle,
    webview_label: String,
    page_id: String,
    parsed_url: Url,
) -> Result<WebviewBuilder<Wry>, String> {
    let app_for_nav = app.clone();
    let app_for_load = app.clone();
    let app_for_title = app.clone();
    let app_for_download = app.clone();
    let app_for_new_window = app.clone();
    let page_id_for_load = page_id.clone();
    let page_id_for_title = page_id.clone();
    let page_id_for_download = page_id.clone();

    #[allow(unused_mut)]
    let mut builder = WebviewBuilder::new(
        webview_label.clone(),
        WebviewUrl::External(parsed_url.clone()),
    )
    .initialization_script(BROWSER_INIT_SCRIPT)
    .on_navigation(move |url| {
        if !crate::browser_panel::is_navigation_allowed(url) {
            if let Ok(mut guard) = app_for_nav.state::<BrowserPanelState>().0.lock() {
                if let Some(page) = guard.get_page_mut(&page_id) {
                    page.load_state = PageLoadState::Failed("navigation blocked".into());
                }
            }
            let _ = app_for_nav.emit(
                "browser-loading",
                serde_json::json!({
                    "pageId": page_id,
                    "loading": false,
                    "loadState": {"state": "failed", "message": "navigation blocked"},
                }),
            );
            return false;
        }
        let _ = app_for_nav.emit(
            "browser-url-changed",
            serde_json::json!({
                "pageId": page_id,
                "url": url.to_string(),
            }),
        );
        true
    })
    .on_page_load(move |webview, payload| {
        let url = payload.url().to_string();
        let (load_state, loading) = match payload.event() {
            PageLoadEvent::Started => (PageLoadState::Loading, true),
            PageLoadEvent::Finished => (PageLoadState::Ready, false),
        };

        {
            if let Ok(mut guard) = app_for_load.state::<BrowserPanelState>().0.lock() {
                if let Some(page) = guard.get_page_mut(&page_id_for_load) {
                    page.url = url.clone();
                    page.load_state = load_state.clone();
                }
            }
        }

        let _ = app_for_load.emit(
            "browser-loading",
            serde_json::json!({
                "pageId": page_id_for_load,
                "url": url,
                "loading": loading,
                "loadState": load_state,
            }),
        );

        if matches!(payload.event(), PageLoadEvent::Finished) {
            let _ = webview.eval(SELECTION_TOOLBAR_JS);
            let _ = webview.eval(CONTENT_EXTRACT_JS);
            if let Err(e) = webview.eval(FAVICON_EXTRACT_JS) {
                tracing::warn!(error = %e, page_id = %page_id_for_load, "favicon extract eval failed");
            }
        }
    })
    .on_document_title_changed(move |_webview, title| {
        {
            if let Ok(mut guard) = app_for_title.state::<BrowserPanelState>().0.lock() {
                if let Some(page) = guard.get_page_mut(&page_id_for_title) {
                    page.title = title.clone();
                }
            }
        }

        let _ = app_for_title.emit(
            "browser-title-changed",
            serde_json::json!({
                "pageId": page_id_for_title,
                "title": title,
            }),
        );
    })
    .on_download(move |_webview, event| {
        match event {
            DownloadEvent::Requested { url, destination } => {
                let download_dir = default_download_directory();
                let filename = url
                    .path_segments()
                    .and_then(|mut segments| segments.next_back())
                    .map(sanitize_download_filename)
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| "download".to_string());
                *destination = download_dir.join(filename);
                // Full filesystem path is required by the frontend "Open file"/"Open folder"
                // actions (shell.open). Only emit within the trusted main-window event channel.
                let destination_path = destination.to_string_lossy().to_string();
                let _ = app_for_download.emit(
                    "browser-download-requested",
                    serde_json::json!({
                        "pageId": page_id_for_download,
                        "url": url.to_string(),
                        "destination": destination_path,
                    }),
                );
                true
            }
            DownloadEvent::Finished { url, path, success } => {
                // Full path required for shell.open after download completes (see above).
                let path_str = path.as_ref().map(|p| p.to_string_lossy().to_string());
                let _ = app_for_download.emit(
                    "browser-download-finished",
                    serde_json::json!({
                        "pageId": page_id_for_download,
                        "url": url.to_string(),
                        "path": path_str,
                        "success": success,
                    }),
                );
                true
            }
            _ => true,
        }
    })
    .on_new_window(move |url, _features| {
        tracing::debug!(url = %url, "on_new_window triggered");
        if !crate::browser_panel::is_navigation_allowed(&url) {
            return NewWindowResponse::Deny;
        }
        let app = app_for_new_window.clone();
        let url_string = url.to_string();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = open_page_from_url(app, url_string).await {
                tracing::warn!(error = %e, "failed to open window.open target in browser");
            }
        });
        NewWindowResponse::Deny
    });

    // On Linux, do NOT set data_directory: it causes wry to create a separate
    // WebKitWebContext, which breaks the custom protocol (xiaolin-internal://)
    // registered on the default context. Cookie persistence is already handled
    // via FFI in browser_gtk::configure_webview_cookies.
    #[cfg(not(target_os = "linux"))]
    {
        let data_dir = browser_data_directory();
        if let Err(e) = std::fs::create_dir_all(&data_dir) {
            tracing::warn!(error = %e, path = %data_dir.display(), "failed to create browser data directory");
        }
        builder = builder.data_directory(data_dir);
    }

    #[cfg(target_os = "macos")]
    {
        builder = builder.data_store_identifier(browser_data_store_identifier());
    }

    // On Linux/WebKitGTK 2.52+, wry's `proxy_url()` calls
    // `set_network_proxy_settings` BEFORE cookies are configured, which breaks
    // the cookie jar (BUG-E2E-7). We skip it here and apply the proxy via FFI
    // AFTER configuring cookies in the GTK main thread block below.
    #[cfg(not(target_os = "linux"))]
    if let Some(net_state) = app.try_state::<BrowserNetworkState>() {
        if let Some(proxy_url) = net_state.manager().webview_proxy_url_sync() {
            match proxy_url.parse::<Url>() {
                Ok(parsed) => {
                    builder = builder.proxy_url(parsed);
                }
                Err(e) => {
                    tracing::warn!(proxy_url = %proxy_url, error = %e, "invalid webview proxy URL");
                }
            }
        }
    }

    Ok(builder)
}

/// Create a browser page. MUST NOT hold the BrowserPanelState mutex across
/// `window.add_child()` because it dispatches to the GTK main thread and the
/// WebView's `on_navigation` callback also acquires this mutex — holding the
/// lock across `add_child` causes a deadlock.
///
/// Page slot reservation (`add_page`) happens under lock *before* WebView
/// creation so concurrent `window.open` calls cannot exceed `MAX_BROWSER_PAGES`.
pub(crate) fn create_browser_page(
    app: &AppHandle,
    state: &State<'_, BrowserPanelState>,
    url: &str,
) -> Result<String, String> {
    let parsed_url = validate_browser_url(url)?;
    let page_id = Uuid::new_v4().to_string();
    let webview_label = format!("{BROWSER_WEBVIEW_PREFIX}{page_id}");

    let page = BrowserPage {
        page_id: page_id.clone(),
        webview_label: webview_label.clone(),
        url: url.to_string(),
        title: String::new(),
        visibility: PageVisibility::Hidden,
        load_state: PageLoadState::Loading,
        layout_x: 0.0,
        layout_y: 0.0,
        layout_width: 0.0,
        layout_height: 0.0,
    };

    with_manager(state, |manager| manager.add_page(page))?;

    let create_result = create_browser_webview_inner(
        app,
        state,
        &page_id,
        &webview_label,
        &parsed_url,
        url,
    );

    if let Err(e) = create_result {
        let _ = with_manager(state, |manager| {
            manager.remove_page(&page_id);
            Ok(())
        });
        return Err(e);
    }

    let _ = app.emit(
        "browser-page-created",
        serde_json::json!({
            "pageId": page_id,
            "url": url,
        }),
    );

    Ok(page_id)
}

fn create_browser_webview_inner(
    app: &AppHandle,
    _state: &State<'_, BrowserPanelState>,
    page_id: &str,
    webview_label: &str,
    parsed_url: &Url,
    _url: &str,
) -> Result<(), String> {
    let window = app
        .get_window("main")
        .ok_or_else(|| "main window not found".to_string())?;

    // On Linux, create the WebView with about:blank first so that cookie
    // persistence can be configured BEFORE the first real navigation.
    // WebKitGTK 2.52 ignores set_persistent_storage calls made after a page
    // has already been loaded.
    #[cfg(target_os = "linux")]
    let initial_url = "about:blank".parse::<Url>().unwrap();
    #[cfg(not(target_os = "linux"))]
    let initial_url = parsed_url.clone();

    let builder = build_browser_webview(
        app,
        webview_label.to_string(),
        page_id.to_string(),
        initial_url,
    )?;

    // add_child dispatches to GTK main thread which may synchronously fire
    // on_navigation (which acquires BrowserPanelState mutex). We MUST NOT hold
    // the mutex here.
    let webview = window
        .add_child(
            builder,
            LogicalPosition::new(OFFSCREEN_POSITION, OFFSCREEN_POSITION),
            LogicalSize::new(1.0, 1.0),
        )
        .map_err(|_| "failed to create browser webview".to_string())?;

    // On Linux/GTK, reparent the child WebView from the GtkBox (where add_child
    // placed it) into a GtkFixed for absolute positioning. Without this, GTK
    // splits window space equally among all WebViews in the GtkBox.
    // Configure cookies first, then proxy (order matters on WebKitGTK 2.52+),
    // then navigate to the target URL.
    #[cfg(target_os = "linux")]
    {
        let proxy_setting_for_gtk = app
            .try_state::<BrowserNetworkState>()
            .map(|ns| ns.manager().webview_proxy_setting_sync());

        let (tx, rx) = std::sync::mpsc::channel();
        let window_clone = window.clone();
        let label_for_gtk = webview_label.to_string();
        let data_dir = browser_data_directory();
        window
            .run_on_main_thread(move || {
                if let Ok(vbox) = window_clone.default_vbox() {
                    crate::browser_gtk::ensure_fixed_container(&vbox);
                    crate::browser_gtk::reparent_child_webview(&vbox, &label_for_gtk);
                    crate::browser_gtk::configure_webview_cookies(&label_for_gtk, &data_dir);
                    crate::browser_gtk::configure_webview_cors(&label_for_gtk);
                    if let Some(setting) = proxy_setting_for_gtk {
                        crate::browser_gtk::reapply_webview_proxy(&label_for_gtk, &setting);
                    }
                }
                let _ = tx.send(());
            })
            .map_err(|_| "failed to dispatch GTK reparent".to_string())?;
        rx.recv()
            .map_err(|_| "GTK reparent channel closed".to_string())?;

        webview
            .navigate(parsed_url.clone())
            .map_err(|_| "failed to navigate after cookie setup".to_string())?;
    }

    Ok(())
}

async fn open_page_from_url(app: AppHandle, url: String) -> Result<String, String> {
    let state = app.state::<BrowserPanelState>();
    create_browser_page(&app, &state, &url)
}

#[tauri::command]
pub async fn browser_request_takeover(page_id: String) -> Result<(), String> {
    validate_page_id(&page_id)?;
    xiaolin_tools_browser::browser_request_user_takeover(Some(&page_id))
}

#[tauri::command]
pub async fn browser_clear_user_takeover() -> Result<(), String> {
    xiaolin_tools_browser::browser_clear_user_takeover()
}

#[tauri::command]
pub async fn browser_open_page(
    app: AppHandle,
    _state: State<'_, BrowserPanelState>,
    url: String,
) -> Result<String, String> {
    open_page_from_url(app, url).await
}

#[tauri::command]
pub async fn browser_close_page(
    app: AppHandle,
    state: State<'_, BrowserPanelState>,
    page_id: String,
) -> Result<(), String> {
    validate_page_id(&page_id)?;

    let webview_label = with_manager(&state, |manager| {
        let page = manager
            .get_page(&page_id)
            .ok_or_else(|| "page not found".to_string())?;
        Ok(page.webview_label.clone())
    })?;

    #[cfg(target_os = "linux")]
    {
        let label_for_gtk = webview_label.clone();
        let window = app
            .get_window("main")
            .ok_or_else(|| "main window not found".to_string())?;
        let (tx, rx) = std::sync::mpsc::channel();
        window
            .run_on_main_thread(move || {
                crate::browser_gtk::remove_child(&label_for_gtk);
                let _ = tx.send(());
            })
            .map_err(|_| "failed to dispatch GTK remove".to_string())?;
        rx.recv()
            .map_err(|_| "GTK remove channel closed".to_string())?;
    }

    if let Ok(webview) = get_webview(&app, &webview_label) {
        if let Err(e) = webview.close() {
            tracing::warn!(error = %e, webview = %webview_label, "failed to close browser webview");
        }
    }

    with_manager(&state, |manager| {
        manager.remove_page(&page_id);
        Ok(())
    })?;

    let _ = app.emit(
        "browser-page-closed",
        serde_json::json!({
            "pageId": page_id,
        }),
    );

    Ok(())
}

#[tauri::command]
pub async fn browser_navigate(
    app: AppHandle,
    state: State<'_, BrowserPanelState>,
    page_id: String,
    url: String,
) -> Result<(), String> {
    validate_page_id(&page_id)?;
    let parsed = validate_browser_url(&url)?;

    let webview_label = with_manager(&state, |manager| {
        let page = manager
            .get_page_mut(&page_id)
            .ok_or_else(|| "page not found".to_string())?;
        page.url = url.clone();
        page.load_state = PageLoadState::Loading;
        Ok(page.webview_label.clone())
    })?;

    let webview = get_webview(&app, &webview_label)?;
    webview
        .navigate(parsed)
        .map_err(|_| "navigation failed".to_string())?;

    let _ = app.emit(
        "browser-loading",
        serde_json::json!({
            "pageId": page_id,
            "url": url,
            "loading": true,
            "loadState": PageLoadState::Loading,
        }),
    );

    Ok(())
}

#[tauri::command]
pub async fn browser_go_back(
    app: AppHandle,
    state: State<'_, BrowserPanelState>,
    page_id: String,
) -> Result<(), String> {
    browser_history_action(&app, &state, &page_id, "history.back()").await
}

#[tauri::command]
pub async fn browser_go_forward(
    app: AppHandle,
    state: State<'_, BrowserPanelState>,
    page_id: String,
) -> Result<(), String> {
    browser_history_action(&app, &state, &page_id, "history.forward()").await
}

#[tauri::command]
pub async fn browser_reload(
    app: AppHandle,
    state: State<'_, BrowserPanelState>,
    page_id: String,
) -> Result<(), String> {
    browser_history_action(&app, &state, &page_id, "location.reload()").await
}

async fn browser_history_action(
    app: &AppHandle,
    state: &State<'_, BrowserPanelState>,
    page_id: &str,
    js: &str,
) -> Result<(), String> {
    validate_page_id(page_id)?;
    let webview_label = with_manager(state, |manager| {
        manager
            .get_page(page_id)
            .map(|p| p.webview_label.clone())
            .ok_or_else(|| "page not found".to_string())
    })?;
    let webview = get_webview(app, &webview_label)?;
    webview
        .eval(js)
        .map_err(|_| "failed to execute browser action".to_string())
}

#[tauri::command]
pub async fn browser_resize_webview(
    app: AppHandle,
    state: State<'_, BrowserPanelState>,
    page_id: String,
    layout: BrowserWebviewLayout,
) -> Result<(), String> {
    validate_page_id(&page_id)?;
    layout.validate()?;

    let (page_snapshot, sf) = with_manager(&state, |manager| {
        if let Some(sf) = layout.scale_factor {
            manager.set_gtk_scale_factor(sf);
        }
        manager.update_layout(&page_id, layout.x, layout.y, layout.width, layout.height)?;
        let page = manager
            .get_page(&page_id)
            .cloned()
            .ok_or_else(|| "page not found".to_string())?;
        Ok((page, manager.gtk_scale_factor()))
    })?;

    apply_webview_layout(&app, &page_snapshot, sf)
}

#[tauri::command]
pub async fn browser_list_pages(
    state: State<'_, BrowserPanelState>,
) -> Result<Vec<crate::browser_panel::BrowserPageInfo>, String> {
    with_manager(&state, |manager| Ok(manager.list_pages()))
}

#[tauri::command]
pub async fn browser_show_page(
    app: AppHandle,
    state: State<'_, BrowserPanelState>,
    page_id: String,
) -> Result<(), String> {
    validate_page_id(&page_id)?;

    let (pages_snapshot, sf) = with_manager(&state, |manager| {
        manager.set_active(&page_id)?;
        let pages = manager
            .list_pages()
            .iter()
            .filter_map(|info| manager.get_page(&info.page_id).cloned())
            .collect::<Vec<_>>();
        Ok((pages, manager.gtk_scale_factor()))
    })?;

    apply_webview_layouts_batch(&app, &pages_snapshot, sf)
}

#[tauri::command]
pub async fn browser_hide_all_pages(
    app: AppHandle,
    state: State<'_, BrowserPanelState>,
) -> Result<(), String> {
    let (pages_snapshot, sf) = with_manager(&state, |manager| {
        manager.hide_all();
        let pages = manager
            .list_pages()
            .iter()
            .filter_map(|info| manager.get_page(&info.page_id).cloned())
            .collect::<Vec<_>>();
        Ok((pages, manager.gtk_scale_factor()))
    })?;

    apply_webview_layouts_batch(&app, &pages_snapshot, sf)
}

#[tauri::command]
pub async fn browser_eval_js(
    app: AppHandle,
    state: State<'_, BrowserPanelState>,
    page_id: String,
    script: String,
) -> Result<(), String> {
    validate_page_id(&page_id)?;
    validate_js_payload(&script)?;

    let webview_label = with_manager(&state, |manager| {
        manager
            .get_page(&page_id)
            .map(|p| p.webview_label.clone())
            .ok_or_else(|| "page not found".to_string())
    })?;

    let webview = get_webview(&app, &webview_label)?;
    webview
        .eval(script)
        .map_err(|_| "failed to evaluate script".to_string())
}

/// IPC-based notification from browser child WebViews.
/// Replacement for the `xiaolin-internal://callback` custom protocol which
/// doesn't work on Linux/WebKitGTK (fetch to custom schemes fails with
/// "Load failed" regardless of CORS settings).
///
/// Tauri 2 child WebViews inherit the parent window's capability set (see
/// `capabilities/default.json` — only `main`/`quick-action` are listed; browser
/// `browser-*` labels are not separately scoped). Defense-in-depth: reject any
/// webview whose label does not match `browser-*`, and whitelist message types.
#[tauri::command]
pub async fn browser_webview_notify(
    app: AppHandle,
    webview: tauri::Webview<Wry>,
    msg_type: String,
    data: serde_json::Value,
) -> Result<serde_json::Value, String> {
    use crate::browser_panel::{
        is_browser_webview_label, ALLOWED_INTERNAL_MESSAGE_TYPES, MAX_IPC_MESSAGE_BYTES,
    };

    let data_bytes = serde_json::to_vec(&data).map_err(|_| "invalid payload".to_string())?;
    if data_bytes.len() > MAX_IPC_MESSAGE_BYTES {
        tracing::warn!(
            bytes = data_bytes.len(),
            max = MAX_IPC_MESSAGE_BYTES,
            "browser_webview_notify payload too large"
        );
        return Err("payload too large".into());
    }

    let webview_label = webview.label().to_string();

    if !is_browser_webview_label(&webview_label) {
        return Err("forbidden: not a browser webview".into());
    }

    if !ALLOWED_INTERNAL_MESSAGE_TYPES.contains(&msg_type.as_str()) {
        return Err(format!("forbidden message type: {msg_type}"));
    }

    let page_id = {
        let state = app.state::<BrowserPanelState>();
        let guard = state.0.lock().map_err(|_| "lock poisoned".to_string())?;
        guard
            .get_page_by_webview_label(&webview_label)
            .map(|p| p.page_id.clone())
    };

    let Some(page_id) = page_id else {
        return Err("page not found".into());
    };

    if msg_type == "eval_result" {
        let id = data.get("id").and_then(|v| v.as_str());
        if let Some(id) = id {
            let outcome = match (
                data.get("result").and_then(|v| v.as_str()),
                data.get("error").and_then(|v| v.as_str()),
            ) {
                (Some(result), _) => Ok(result.to_string()),
                (_, Some(error)) => Err(error.to_string()),
                _ => Err("eval_result missing result and error".to_string()),
            };
            crate::browser_eval::complete_eval(id, outcome);
        }
        return Ok(serde_json::json!({"ok": true}));
    }

    if msg_type == "favicon" {
        let _ = app.emit(
            "browser-favicon-changed",
            serde_json::json!({
                "pageId": page_id,
                "dataUrl": data.get("dataUrl").cloned().unwrap_or(serde_json::Value::Null),
                "url": data.get("url").cloned().unwrap_or(serde_json::Value::Null),
            }),
        );
        return Ok(serde_json::json!({"ok": true}));
    }

    let event_name = match msg_type.as_str() {
        "ready" => "browser-page-ready",
        "snapshot" => "browser-snapshot",
        "console" => "browser-console",
        "network" => "browser-network",
        "selection" | "user_action_blocked" => "browser-user-action",
        "dialog" => "browser-dialog",
        _ => return Err(format!("unhandled type: {msg_type}")),
    };

    let _ = app.emit(
        event_name,
        serde_json::json!({
            "pageId": page_id,
            "type": msg_type,
            "data": data,
        }),
    );

    Ok(serde_json::json!({"ok": true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_panel::{is_navigation_allowed, MAX_BROWSER_PAGES};

    #[test]
    fn about_blank_allowed_other_about_schemes_blocked() {
        assert!(is_navigation_allowed(&"about:blank".parse().unwrap()));
        assert!(!is_navigation_allowed(&"about:srcdoc".parse().unwrap()));
        assert!(!is_navigation_allowed(&"about:config".parse().unwrap()));
    }

    #[test]
    fn manager_enforces_page_limit() {
        let mut manager = BrowserPanelManager::new();
        for i in 0..MAX_BROWSER_PAGES {
            let page = BrowserPage {
                page_id: format!("page-{i}"),
                webview_label: format!("browser-page-{i}"),
                url: "https://example.com".into(),
                title: String::new(),
                visibility: PageVisibility::Hidden,
                load_state: PageLoadState::Loading,
                layout_x: 0.0,
                layout_y: 0.0,
                layout_width: 0.0,
                layout_height: 0.0,
            };
            manager.add_page(page).expect("add page");
        }
        let overflow = BrowserPage {
            page_id: "overflow".into(),
            webview_label: "browser-overflow".into(),
            url: "https://example.com".into(),
            title: String::new(),
            visibility: PageVisibility::Hidden,
            load_state: PageLoadState::Loading,
            layout_x: 0.0,
            layout_y: 0.0,
            layout_width: 0.0,
            layout_height: 0.0,
        };
        assert!(manager.add_page(overflow).is_err());
    }
}
