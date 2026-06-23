use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use xiaolin_core::tool::ToolImage;

use super::{BrowserEngine, EngineActionResult};
use crate::actions;
use crate::js::{AGENT_CONTROL_INTERCEPT_JS, HIGHLIGHT_COMPLETE_JS, HIGHLIGHT_ELEMENT_JS};

/// Bridge from browser engine to Tauri BrowserPanelManager IPC (registered at app startup).
pub trait BrowserBridge: Send + Sync {
    fn eval_js(&self, page_id: Option<&str>, js: &str) -> Result<String, String>;
    fn navigate(&self, page_id: Option<&str>, url: &str) -> Result<(), String>;
    fn go_back(&self, page_id: Option<&str>) -> Result<(), String>;
    fn go_forward(&self, page_id: Option<&str>) -> Result<(), String>;
    fn reload(&self, page_id: Option<&str>, ignore_cache: bool) -> Result<(), String>;
    fn list_pages(&self) -> Result<String, String>;
    fn select_page(&self, page_id: &str) -> Result<(), String>;
    fn open_page(&self, url: &str) -> Result<String, String>;
    fn close_page(&self, page_id: &str) -> Result<(), String>;
    fn screenshot(&self, page_id: Option<&str>) -> Result<Vec<u8>, String>;
    fn set_agent_control(&self, page_id: Option<&str>, active: bool) -> Result<(), String>;

    /// Active built-in browser page summary for agent context injection (desktop only).
    fn active_browser_context(&self) -> Result<Option<serde_json::Value>, String> {
        let _ = self;
        Ok(None)
    }

    /// Emit agent operation log entry to the main WebView (desktop only).
    fn emit_agent_op(
        &self,
        _page_id: Option<&str>,
        _action: &str,
        _description: &str,
    ) -> Result<(), String> {
        Ok(())
    }
}

/// Build a system-prompt snippet describing the active built-in browser page (desktop only).
pub fn browser_context_for_prompt() -> Option<String> {
    let ctx = bridge_for_context()?.active_browser_context().ok()??;
    let url = ctx.get("url").and_then(|v| v.as_str()).unwrap_or("");
    let title = ctx.get("title").and_then(|v| v.as_str()).unwrap_or("");
    let page_count = ctx.get("page_count").and_then(|v| v.as_u64()).unwrap_or(0);
    if url.is_empty() && title.is_empty() {
        return None;
    }
    Some(format!(
        "[Browser Context]\n\
Active page: {title}\n\
URL: {url}\n\
Open tabs: {page_count}\n\
\n\
The user may be viewing this page in the built-in browser panel. \
Use browser tools or `__xiaolin_extract.text()` / `.tables()` / `.links()` / `.metadata()` \
(via evaluate) when you need page content."
    ))
}

static BROWSER_BRIDGE: OnceLock<Arc<dyn BrowserBridge>> = OnceLock::new();

/// Register the Tauri bridge before gateway startup (desktop mode).
pub fn set_browser_bridge(bridge: Arc<dyn BrowserBridge>) -> Result<(), Arc<dyn BrowserBridge>> {
    BROWSER_BRIDGE.set(bridge)
}

pub(crate) fn bridge_is_configured() -> bool {
    BROWSER_BRIDGE.get().is_some()
}

/// Used by [`crate::browser_context_for_prompt`] (non-action read path).
pub fn bridge_for_context() -> Option<&'static Arc<dyn BrowserBridge>> {
    BROWSER_BRIDGE.get()
}

fn bridge() -> Result<&'static Arc<dyn BrowserBridge>, String> {
    BROWSER_BRIDGE.get().ok_or_else(|| {
        "browser webview: bridge not configured. Running in gateway-only mode? \
         Set XIAOLIN_BROWSER_ENGINE=cdp or register bridge at Tauri startup."
            .to_string()
    })
}

/// Built-in WebView engine (Tauri child WebView via BrowserPanelManager).
pub struct TauriWebViewEngine;

impl Default for TauriWebViewEngine {
    fn default() -> Self {
        Self
    }
}

impl TauriWebViewEngine {
    pub fn new() -> Self {
        Self
    }

    fn page_id_from_args(args: &serde_json::Value) -> Option<String> {
        args.get("pageId")
            .or(args.get("page_id"))
            .and_then(|v| v.as_str().map(String::from))
            .or_else(|| args.get("pageId").and_then(|v| v.as_u64()).map(|n| n.to_string()))
    }

    fn eval(&self, page_id: Option<&str>, js: &str) -> Result<String, String> {
        bridge()?.eval_js(page_id, js)
    }

    fn with_highlight<F>(&self, args: &serde_json::Value, op: F) -> Result<String, String>
    where
        F: FnOnce() -> Result<String, String>,
    {
        let uid = args.get("uid").and_then(|v| v.as_str());
        let selector = args.get("selector").and_then(|v| v.as_str());
        let page_id = Self::page_id_from_args(args);

        if let Some(u) = uid {
            let _ = self.eval(
                page_id.as_deref(),
                &format!("{HIGHLIGHT_ELEMENT_JS}({:?}, null)", u),
            );
            std::thread::sleep(std::time::Duration::from_millis(300));
        } else if let Some(sel) = selector {
            let sel_json = serde_json::to_string(sel).unwrap_or_default();
            let _ = self.eval(
                page_id.as_deref(),
                &format!("{HIGHLIGHT_ELEMENT_JS}(null, {sel_json})"),
            );
            std::thread::sleep(std::time::Duration::from_millis(300));
        }

        let result = op()?;

        if let Some(u) = uid {
            let _ = self.eval(
                page_id.as_deref(),
                &format!("{HIGHLIGHT_COMPLETE_JS}({:?}, null)", u),
            );
        } else if let Some(sel) = selector {
            let sel_json = serde_json::to_string(sel).unwrap_or_default();
            let _ = self.eval(
                page_id.as_deref(),
                &format!("{HIGHLIGHT_COMPLETE_JS}(null, {sel_json})"),
            );
        }

        Ok(result)
    }

    fn enter_agent_control(&self, page_id: Option<&str>) -> Result<(), String> {
        bridge()?.set_agent_control(page_id, true)?;
        self.eval(page_id, AGENT_CONTROL_INTERCEPT_JS)?;
        Ok(())
    }

    fn log_agent_op(&self, page_id: Option<&str>, action: &str, description: &str) {
        if let Ok(b) = bridge() {
            let _ = b.emit_agent_op(page_id, action, description);
        }
    }

    fn dispatch_sync(
        &self,
        action: &str,
        args: &serde_json::Value,
    ) -> Result<EngineActionResult, String> {
        let page_id = Self::page_id_from_args(args);
        let bridge = bridge()?;
        let page_id_ref = page_id.as_deref();

        match action {
            "click" | "fill" | "fill_form" | "type_text" | "press_key" | "hover" | "scroll"
            | "drag" | "select" | "type" | "upload_file" | "wait_for" => {
                self.enter_agent_control(page_id.as_deref())?;
            }
            _ => {}
        }

        match action {
            "navigate" | "go_back" | "go_forward" | "reload" => {
                let nav_type = if action == "navigate" {
                    args.get("type").and_then(|v| v.as_str()).unwrap_or("url")
                } else {
                    match action {
                        "go_back" => "back",
                        "go_forward" => "forward",
                        "reload" => "reload",
                        _ => "url",
                    }
                };
                match nav_type {
                    "back" => bridge.go_back(page_id.as_deref())?,
                    "forward" => bridge.go_forward(page_id.as_deref())?,
                    "reload" => {
                        let ignore = args
                            .get("ignoreCache")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        bridge.reload(page_id.as_deref(), ignore)?;
                    }
                    _ => {
                        let url = args
                            .get("url")
                            .and_then(|v| v.as_str())
                            .ok_or("browser navigate: missing 'url'.")?;
                        actions::validate_url_scheme(url)?;
                        bridge.navigate(page_id_ref, url)?;
                    }
                }
                self.log_agent_op(page_id_ref, "navigate", nav_type);
                Ok(EngineActionResult::text(
                    bridge.list_pages().unwrap_or_else(|_| "{}".to_string()),
                ))
            }

            "list_pages" => Ok(EngineActionResult::text(bridge.list_pages()?)),

            "select_page" => {
                let pid = args
                    .get("pageId")
                    .and_then(|v| v.as_u64())
                    .map(|n| n.to_string())
                    .or_else(|| args.get("page_id").and_then(|v| v.as_str()).map(String::from))
                    .ok_or("browser select_page: missing pageId")?;
                bridge.select_page(&pid)?;
                self.log_agent_op(page_id_ref, "select_page", &pid);
                Ok(EngineActionResult::text(
                    serde_json::json!({ "ok": true, "pageId": pid }).to_string(),
                ))
            }

            "new_page" => {
                let url = args
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or("browser new_page: missing 'url'.")?;
                actions::validate_url_scheme(url)?;
                let info = bridge.open_page(url)?;
                self.log_agent_op(page_id_ref, "new_page", url);
                Ok(EngineActionResult::text(info))
            }

            "close_page" => {
                let pid = args
                    .get("pageId")
                    .and_then(|v| v.as_u64())
                    .map(|n| n.to_string())
                    .or_else(|| args.get("page_id").and_then(|v| v.as_str()).map(String::from))
                    .ok_or("browser close_page: missing pageId")?;
                bridge.close_page(&pid)?;
                self.log_agent_op(page_id_ref, "close_page", &pid);
                Ok(EngineActionResult::text(
                    serde_json::json!({ "ok": true, "closed": pid }).to_string(),
                ))
            }

            "take_snapshot" | "get_content" => {
                let script = if action == "take_snapshot" {
                    include_str!("../js/snapshot_stub.js")
                } else {
                    "JSON.stringify({ url: location.href, title: document.title, content: (document.body && document.body.innerText || '').substring(0, 16384) })"
                };
                let raw = self.eval(page_id.as_deref(), script)?;
                let mut val: serde_json::Value =
                    serde_json::from_str(&raw).unwrap_or(serde_json::json!({ "raw": raw }));
                if let Some(obj) = val.as_object_mut() {
                    obj.insert(
                        "source".to_string(),
                        serde_json::json!(crate::js::UNTRUSTED_SOURCE),
                    );
                    obj.insert(
                        "warning".to_string(),
                        serde_json::json!(crate::js::UNTRUSTED_WARNING),
                    );
                }
                self.log_agent_op(page_id_ref, action, "page content captured");
                Ok(EngineActionResult::text(val.to_string()))
            }

            "screenshot" => {
                let png = bridge.screenshot(page_id.as_deref())?;
                let summary = format!("Screenshot captured ({} bytes, webview engine).", png.len());
                self.log_agent_op(page_id_ref, "screenshot", &summary);
                Ok(EngineActionResult::with_images(
                    summary,
                    vec![ToolImage {
                        mime_type: "image/png".into(),
                        data: png,
                    }],
                ))
            }

            "evaluate" => {
                let script = args
                    .get("function")
                    .or(args.get("script"))
                    .and_then(|v| v.as_str())
                    .ok_or("browser evaluate: missing 'script'.")?;
                let result = self.eval(page_id.as_deref(), script)?;
                let preview: String = script.chars().take(80).collect();
                self.log_agent_op(page_id_ref, "evaluate", &preview);
                Ok(EngineActionResult::text(
                    serde_json::json!({ "result": result }).to_string(),
                ))
            }

            "click" | "fill" | "hover" => {
                let detail = args
                    .get("uid")
                    .or(args.get("selector"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("element")
                    .to_string();
                let result = self
                    .with_highlight(args, || {
                        let js = webview_interaction_js(action, args)?;
                        self.eval(page_id.as_deref(), &js)
                    })
                    .map(EngineActionResult::text)?;
                self.log_agent_op(page_id_ref, action, &detail);
                Ok(result)
            }

            "list_console_messages" => {
                let raw = self.eval(
                    page_id.as_deref(),
                    "JSON.stringify((window.__XIAOLIN__ && window.__XIAOLIN__.getConsoleLog && window.__XIAOLIN__.getConsoleLog()) || [])",
                )?;
                Ok(EngineActionResult::text(format!("{{\"messages\":{raw}}}")))
            }

            "list_network_requests" => {
                let raw = self.eval(
                    page_id.as_deref(),
                    "JSON.stringify((window.__XIAOLIN__ && window.__XIAOLIN__.getNetworkLog && window.__XIAOLIN__.getNetworkLog()) || [])",
                )?;
                Ok(EngineActionResult::text(format!("{{\"requests\":{raw}}}")))
            }

            "cookies" => {
                let op = args
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("get");
                match op {
                    "get" => {
                        let raw = self.eval(
                            page_id.as_deref(),
                            "JSON.stringify({ cookies: document.cookie, note: 'HttpOnly cookies are not accessible via document.cookie' })",
                        )?;
                        Ok(EngineActionResult::text(raw))
                    }
                    "set" => {
                        let name = args
                            .get("cookie_name")
                            .and_then(|v| v.as_str())
                            .ok_or("missing cookie_name")?;
                        let value = args
                            .get("cookie_value")
                            .and_then(|v| v.as_str())
                            .ok_or("missing cookie_value")?;
                        let name_j = serde_json::to_string(name).unwrap();
                        let value_j = serde_json::to_string(value).unwrap();
                        self.eval(
                            page_id.as_deref(),
                            &format!("document.cookie = {name_j} + '=' + {value_j}; 'ok'"),
                        )?;
                        Ok(EngineActionResult::text(
                            serde_json::json!({ "ok": true, "operation": "set", "cookie_name": name })
                                .to_string(),
                        ))
                    }
                    other => Err(format!(
                        "browser cookies webview: operation '{other}' not fully implemented yet"
                    )),
                }
            }

            "wait_for" => {
                if let Some(selector) = args.get("selector").and_then(|v| v.as_str()) {
                    let timeout = actions::parse_timeout(args).as_millis();
                    let sel_j = serde_json::to_string(selector).unwrap();
                    let script = format!(
                        "new Promise(function(resolve, reject) {{ \
                           var sel = {sel_j}; var deadline = Date.now() + {timeout}; \
                           (function tick() {{ \
                             if (document.querySelector(sel)) return resolve('ok'); \
                             if (Date.now() > deadline) return reject('timeout'); \
                             setTimeout(tick, 200); \
                           }})(); \
                         }})"
                    );
                    self.eval(page_id.as_deref(), &script)?;
                    Ok(EngineActionResult::text(
                        serde_json::json!({ "ok": true, "selector": selector }).to_string(),
                    ))
                } else {
                    Err("browser wait_for webview: text wait not implemented yet".to_string())
                }
            }

            "scroll" => {
                let direction = args
                    .get("direction")
                    .and_then(|v| v.as_str())
                    .unwrap_or("down");
                let amount = args.get("amount").and_then(|v| v.as_i64()).unwrap_or(300);
                let delta = if direction == "up" {
                    -amount.abs()
                } else {
                    amount.abs()
                };
                self.eval(
                    page_id.as_deref(),
                    &format!("window.scrollBy(0, {delta}); 'ok'"),
                )?;
                self.log_agent_op(page_id_ref, "scroll", direction);
                Ok(EngineActionResult::text(
                    serde_json::json!({ "ok": true, "direction": direction }).to_string(),
                ))
            }

            other => Err(format!(
                "browser webview: action '{other}' not implemented yet (stub — use CDP fallback or wait for frontend integration)"
            )),
        }
    }
}

fn webview_interaction_js(action: &str, args: &serde_json::Value) -> Result<String, String> {
    let uid = args.get("uid").and_then(|v| v.as_str());
    let selector = args.get("selector").and_then(|v| v.as_str());
    let find = if let Some(u) = uid {
        format!(
            "document.querySelector('[data-fc-uid=\"{}\"]')",
            u.replace('"', "")
        )
    } else if let Some(sel) = selector {
        format!(
            "document.querySelector({})",
            serde_json::to_string(sel).unwrap()
        )
    } else {
        return Err("missing uid or selector".to_string());
    };

    match action {
        "click" => Ok(format!(
            "(() => {{ var el = {find}; if (!el) throw new Error('not found'); el.click(); return 'ok'; }})()"
        )),
        "fill" => {
            let value = args
                .get("value")
                .and_then(|v| v.as_str())
                .ok_or("missing value")?;
            let val_j = serde_json::to_string(value).unwrap();
            Ok(format!(
                "(() => {{ var el = {find}; if (!el) throw new Error('not found'); \
                 if (el.tagName === 'SELECT') {{ el.value = {val_j}; }} else {{ el.focus(); el.value = {val_j}; }} \
                 el.dispatchEvent(new Event('input', {{bubbles: true}})); \
                 el.dispatchEvent(new Event('change', {{bubbles: true}})); return 'ok'; }})()"
            ))
        }
        "hover" => Ok(format!(
            "(() => {{ var el = {find}; if (!el) throw new Error('not found'); \
             el.dispatchEvent(new MouseEvent('mouseover', {{bubbles: true}})); return 'ok'; }})()"
        )),
        _ => Err(format!("unsupported interaction: {action}")),
    }
}

#[async_trait]
impl BrowserEngine for TauriWebViewEngine {
    fn engine_type(&self) -> &str {
        "webview"
    }

    async fn shutdown(&self) {}

    async fn execute_action(
        &self,
        action: &str,
        args: &serde_json::Value,
    ) -> Result<EngineActionResult, String> {
        let action = action.to_string();
        let args = args.clone();
        let engine = TauriWebViewEngine;
        tokio::task::spawn_blocking(move || engine.dispatch_sync(&action, &args))
        .await
        .map_err(|e| format!("browser webview: worker panicked: {e}"))?
    }
}
