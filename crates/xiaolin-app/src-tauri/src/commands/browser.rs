use crate::browser_network::BrowserNetworkState;
use crate::browser_panel::{
    browser_data_directory, default_download_directory, sanitize_download_filename,
    validate_browser_url, validate_js_payload, validate_page_id, BrowserPage, BrowserPanelManager,
    BrowserPanelState, PageLoadState, PageVisibility, BROWSER_INIT_SCRIPT, BROWSER_WEBVIEW_PREFIX,
    MAX_BROWSER_PAGES, OFFSCREEN_POSITION,
};
#[cfg(target_os = "macos")]
use crate::browser_panel::browser_data_store_identifier;
use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::utils::config::WebviewUrl;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, State, Url, Wry};
use uuid::Uuid;
use xiaolin_tools_browser::{CONTENT_EXTRACT_JS, SELECTION_TOOLBAR_JS};

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
) -> Result<(), String> {
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
                    "loadState": PageLoadState::Failed("navigation blocked".into()),
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
                    .and_then(|segments| segments.last())
                    .map(sanitize_download_filename)
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| "download".to_string());
                *destination = download_dir.join(filename);
                let _ = app_for_download.emit(
                    "browser-download-requested",
                    serde_json::json!({
                        "pageId": page_id_for_download,
                        "url": url.to_string(),
                        "destination": destination.display().to_string(),
                    }),
                );
                true
            }
            DownloadEvent::Finished { url, path, success } => {
                let _ = app_for_download.emit(
                    "browser-download-finished",
                    serde_json::json!({
                        "pageId": page_id_for_download,
                        "url": url.to_string(),
                        "path": path.as_ref().map(|p| p.display().to_string()),
                        "success": success,
                    }),
                );
                true
            }
            _ => true,
        }
    })
    .on_new_window(move |url, _features| {
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

pub(crate) fn create_browser_page(
    app: &AppHandle,
    manager: &mut BrowserPanelManager,
    url: &str,
) -> Result<String, String> {
    if manager.page_count() >= MAX_BROWSER_PAGES {
        return Err(format!("browser page limit reached ({MAX_BROWSER_PAGES})"));
    }

    let parsed_url = validate_browser_url(url)?;
    let page_id = Uuid::new_v4().to_string();
    let webview_label = format!("{BROWSER_WEBVIEW_PREFIX}{page_id}");

    let window = app
        .get_window("main")
        .ok_or_else(|| "main window not found".to_string())?;

    let builder = build_browser_webview(app, webview_label.clone(), page_id.clone(), parsed_url)?;
    let _webview = window
        .add_child(
            builder,
            LogicalPosition::new(OFFSCREEN_POSITION, OFFSCREEN_POSITION),
            LogicalSize::new(1.0, 1.0),
        )
        .map_err(|_| "failed to create browser webview".to_string())?;

    let page = BrowserPage {
        page_id: page_id.clone(),
        webview_label,
        url: url.to_string(),
        title: String::new(),
        visibility: PageVisibility::Hidden,
        load_state: PageLoadState::Loading,
        layout_x: 0.0,
        layout_y: 0.0,
        layout_width: 0.0,
        layout_height: 0.0,
    };

    manager.add_page(page)?;
    Ok(page_id)
}

async fn open_page_from_url(app: AppHandle, url: String) -> Result<String, String> {
    let state = app.state::<BrowserPanelState>();
    with_manager(&state, |manager| create_browser_page(&app, manager, &url))
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

    if let Ok(webview) = get_webview(&app, &webview_label) {
        if let Err(e) = webview.close() {
            tracing::warn!(error = %e, webview = %webview_label, "failed to close browser webview");
        }
    }

    with_manager(&state, |manager| {
        manager.remove_page(&page_id);
        Ok(())
    })
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
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    validate_page_id(&page_id)?;
    if !x.is_finite() || !y.is_finite() || !width.is_finite() || !height.is_finite() {
        return Err("invalid layout coordinates".into());
    }
    if width < 0.0 || height < 0.0 || width > 10000.0 || height > 10000.0 {
        return Err("invalid layout size".into());
    }

    let page_snapshot = with_manager(&state, |manager| {
        manager.update_layout(&page_id, x, y, width, height)?;
        manager
            .get_page(&page_id)
            .cloned()
            .ok_or_else(|| "page not found".to_string())
    })?;

    apply_webview_layout(&app, &page_snapshot)
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

    let pages_snapshot = with_manager(&state, |manager| {
        manager.set_active(&page_id)?;
        Ok(manager
            .list_pages()
            .iter()
            .filter_map(|info| manager.get_page(&info.page_id).cloned())
            .collect::<Vec<_>>())
    })?;

    for page in &pages_snapshot {
        apply_webview_layout(&app, page)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn browser_hide_all_pages(
    app: AppHandle,
    state: State<'_, BrowserPanelState>,
) -> Result<(), String> {
    let pages_snapshot = with_manager(&state, |manager| {
        manager.hide_all();
        Ok(manager
            .list_pages()
            .iter()
            .filter_map(|info| manager.get_page(&info.page_id).cloned())
            .collect::<Vec<_>>())
    })?;

    for page in &pages_snapshot {
        apply_webview_layout(&app, page)?;
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

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
