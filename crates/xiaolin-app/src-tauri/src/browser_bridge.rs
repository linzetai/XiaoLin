//! Bridge between `xiaolin-tools-browser` WebView engine and Tauri BrowserPanelManager.

use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, Wry};
use uuid::Uuid;
use xiaolin_tools_browser::BrowserBridge;

use crate::browser_eval::{self, cancel_eval, normalize_eval_result, register_eval};
use crate::browser_panel::{
    validate_browser_url, validate_js_payload, validate_page_id, BrowserPanelManager,
    BrowserPanelState, PageLoadState,
};

pub struct TauriBrowserBridge {
    app: AppHandle,
}

impl TauriBrowserBridge {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    fn with_manager<T>(&self, f: impl FnOnce(&mut BrowserPanelManager) -> Result<T, String>) -> Result<T, String> {
        let state = self.app.state::<BrowserPanelState>();
        let mut guard = state
            .0
            .lock()
            .map_err(|_| "browser manager lock poisoned".to_string())?;
        f(&mut guard)
    }

    fn resolve_page_id(&self, page_id: Option<&str>) -> Result<String, String> {
        if let Some(id) = page_id {
            validate_page_id(id)?;
            return Ok(id.to_string());
        }
        self.with_manager(|manager| {
            manager
                .active_page_id()
                .map(|s| s.to_string())
                .ok_or_else(|| "no active browser page".to_string())
        })
    }

    fn webview_label_for_page(&self, page_id: &str) -> Result<String, String> {
        self.with_manager(|manager| {
            manager
                .get_page(page_id)
                .map(|p| p.webview_label.clone())
                .ok_or_else(|| "page not found".to_string())
        })
    }

    fn get_webview(&self, label: &str) -> Result<tauri::Webview<Wry>, String> {
        self.app
            .get_webview(label)
            .ok_or_else(|| "browser webview not found".to_string())
    }

    fn eval_js_with_result(&self, page_id: Option<&str>, js: &str) -> Result<String, String> {
        validate_js_payload(js)?;
        let page_id = self.resolve_page_id(page_id)?;
        let label = self.webview_label_for_page(&page_id)?;
        let webview = self.get_webview(&label)?;

        let callback_id = Uuid::new_v4().to_string();
        let wrapped_js = format!(
            "(function(){{try{{var __r=({js});\
             window.__XIAOLIN__.send({{type:'eval_result',data:{{id:'{callback_id}',result:JSON.stringify(__r)}}}});\
             }}catch(e){{\
             window.__XIAOLIN__.send({{type:'eval_result',data:{{id:'{callback_id}',error:String(e.message||e)}}}});\
             }}}})()"
        );

        let rx = register_eval(callback_id.clone());
        webview
            .eval(&wrapped_js)
            .map_err(|_| "failed to evaluate script".to_string())?;

        match rx.recv_timeout(browser_eval::eval_timeout()) {
            Ok(Ok(result_json)) => normalize_eval_result(&result_json),
            Ok(Err(error)) => Err(error),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                cancel_eval(&callback_id);
                Err(format!(
                    "eval result timeout ({}s)",
                    browser_eval::eval_timeout().as_secs()
                ))
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                cancel_eval(&callback_id);
                Err("eval result channel closed".to_string())
            }
        }
    }
}

impl BrowserBridge for TauriBrowserBridge {
    fn eval_js(&self, page_id: Option<&str>, js: &str) -> Result<String, String> {
        self.eval_js_with_result(page_id, js)
    }

    fn navigate(&self, page_id: Option<&str>, url: &str) -> Result<(), String> {
        let page_id = self.resolve_page_id(page_id)?;
        let parsed = validate_browser_url(url)?;
        let label = self.webview_label_for_page(&page_id)?;

        self.with_manager(|manager| {
            if let Some(page) = manager.get_page_mut(&page_id) {
                page.url = url.to_string();
                page.load_state = PageLoadState::Loading;
            }
            Ok(())
        })?;

        self.get_webview(&label)?
            .navigate(parsed)
            .map_err(|_| "navigation failed".to_string())
    }

    fn go_back(&self, page_id: Option<&str>) -> Result<(), String> {
        let page_id = self.resolve_page_id(page_id)?;
        let label = self.webview_label_for_page(&page_id)?;
        self.get_webview(&label)?
            .eval("history.back()")
            .map_err(|_| "go_back failed".to_string())
    }

    fn go_forward(&self, page_id: Option<&str>) -> Result<(), String> {
        let page_id = self.resolve_page_id(page_id)?;
        let label = self.webview_label_for_page(&page_id)?;
        self.get_webview(&label)?
            .eval("history.forward()")
            .map_err(|_| "go_forward failed".to_string())
    }

    fn reload(&self, page_id: Option<&str>, ignore_cache: bool) -> Result<(), String> {
        let page_id = self.resolve_page_id(page_id)?;
        let label = self.webview_label_for_page(&page_id)?;
        let js = if ignore_cache {
            "location.reload(true)"
        } else {
            "location.reload()"
        };
        self.get_webview(&label)?
            .eval(js)
            .map_err(|_| "reload failed".to_string())
    }

    fn list_pages(&self) -> Result<String, String> {
        self.with_manager(|manager| {
            let pages: Vec<_> = manager
                .list_pages()
                .into_iter()
                .enumerate()
                .map(|(i, p)| {
                    serde_json::json!({
                        "pageId": i,
                        "page_id": p.page_id,
                        "url": p.url,
                        "title": p.title,
                    })
                })
                .collect();
            Ok(serde_json::json!({ "pages": pages }).to_string())
        })
    }

    fn select_page(&self, page_id: &str) -> Result<(), String> {
        validate_page_id(page_id)?;
        self.with_manager(|manager| manager.set_active(page_id))
    }

    fn open_page(&self, url: &str) -> Result<String, String> {
        validate_browser_url(url)?;
        // Delegate to IPC-less open: emit to frontend or call browser_open_page internals.
        // Stub: return instruction until full integration.
        Err("browser webview open_page: use Browser Panel UI or browser_open_page IPC (agent integration pending)".into())
    }

    fn close_page(&self, page_id: &str) -> Result<(), String> {
        validate_page_id(page_id)?;
        let label = self.webview_label_for_page(page_id)?;
        if let Ok(webview) = self.get_webview(&label) {
            let _ = webview.close();
        }
        self.with_manager(|manager| {
            manager.remove_page(page_id);
            Ok(())
        })
    }

    fn screenshot(&self, page_id: Option<&str>) -> Result<Vec<u8>, String> {
        let _ = self.resolve_page_id(page_id)?;
        Err("browser webview screenshot: native capture not wired yet (Phase 5 stub)".into())
    }

    fn set_agent_control(&self, page_id: Option<&str>, active: bool) -> Result<(), String> {
        let page_id = self.resolve_page_id(page_id)?;
        let _ = self.app.emit(
            "browser-agent-control",
            serde_json::json!({ "pageId": page_id, "active": active }),
        );
        Ok(())
    }

    fn active_browser_context(&self) -> Result<Option<serde_json::Value>, String> {
        self.with_manager(|manager| {
            let pages = manager.list_pages();
            if pages.is_empty() {
                return Ok(None);
            }
            let active = manager
                .active_page_id()
                .and_then(|id| manager.get_page(id))
                .or_else(|| {
                    pages
                        .first()
                        .and_then(|info| manager.get_page(&info.page_id))
                });
            let Some(page) = active else {
                return Ok(None);
            };
            Ok(Some(serde_json::json!({
                "url": page.url,
                "title": page.title,
                "page_count": pages.len(),
            })))
        })
    }

    fn emit_agent_op(
        &self,
        page_id: Option<&str>,
        action: &str,
        description: &str,
    ) -> Result<(), String> {
        let page_id = match page_id {
            Some(id) => id.to_string(),
            None => self.resolve_page_id(None).unwrap_or_default(),
        };
        let _ = self.app.emit(
            "browser-user-action",
            serde_json::json!({
                "pageId": page_id,
                "type": "agent_op",
                "data": {
                    "action": action,
                    "description": description,
                },
                "ts": chrono::Utc::now().timestamp_millis(),
            }),
        );
        Ok(())
    }
}

/// Register the Tauri browser bridge and prefer WebView engine for agent tools.
pub fn install_browser_bridge(app: &AppHandle) {
    std::env::set_var("XIAOLIN_BROWSER_ENGINE", "webview");
    let bridge = Arc::new(TauriBrowserBridge::new(app.clone()));
    if let Err(existing) = xiaolin_tools_browser::set_browser_bridge(bridge) {
        tracing::warn!("browser bridge already registered");
        drop(existing);
    } else {
        tracing::info!("browser bridge registered (WebView engine enabled)");
    }
}
