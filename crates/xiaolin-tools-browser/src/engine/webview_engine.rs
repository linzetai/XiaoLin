use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

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

    /// User requested to take control back from the agent (desktop only).
    fn request_user_takeover(&self, page_id: Option<&str>) -> Result<(), String> {
        let _ = page_id;
        Err("browser user takeover: bridge not configured".into())
    }

    /// Clear user takeover so the agent may resume browser actions.
    fn clear_user_takeover(&self) -> Result<(), String> {
        Ok(())
    }

    /// Whether user takeover is active (agent actions must fail closed).
    fn is_user_takeover_active(&self) -> bool {
        false
    }

    /// Whether user takeover is active for a specific page (preferred over global check).
    fn is_user_takeover_active_for_page(&self, page_id: Option<&str>) -> bool {
        let _ = page_id;
        self.is_user_takeover_active()
    }

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

const DEVTOOLS_UNSUPPORTED: &str = "DevTools actions require CDP engine. \
In WebView mode, use Agent observe actions instead.";

struct AgentControlDebounce {
    exit_generation: u64,
}

static AGENT_CONTROL_DEBOUNCE: OnceLock<Mutex<AgentControlDebounce>> = OnceLock::new();

fn agent_control_debounce() -> &'static Mutex<AgentControlDebounce> {
    AGENT_CONTROL_DEBOUNCE.get_or_init(|| Mutex::new(AgentControlDebounce { exit_generation: 0 }))
}

/// Debounce agent-control overlay exit by 500ms to avoid UI flicker between consecutive actions.
fn schedule_debounced_exit_agent_control(page_id: Option<String>) {
    let generation = {
        let mut state = agent_control_debounce()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.exit_generation += 1;
        state.exit_generation
    };

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        let should_exit = {
            let state = agent_control_debounce()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.exit_generation == generation
        };
        if should_exit {
            let engine = TauriWebViewEngine;
            let _ = engine.exit_agent_control(page_id.as_deref());
        }
    });
}

pub fn browser_request_user_takeover(page_id: Option<&str>) -> Result<(), String> {
    bridge()?.request_user_takeover(page_id)
}

pub fn browser_clear_user_takeover() -> Result<(), String> {
    bridge()?.clear_user_takeover()
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
            crate::actions::validate_uid(u)?;
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
        self.eval(page_id, "window.__XIAOLIN_AGENT_ACTIVE__ = true;")?;
        self.eval(page_id, AGENT_CONTROL_INTERCEPT_JS)?;
        Ok(())
    }

    fn exit_agent_control(&self, page_id: Option<&str>) -> Result<(), String> {
        self.eval(page_id, "window.__XIAOLIN_AGENT_ACTIVE__ = false;")?;
        bridge()?.set_agent_control(page_id, false)?;
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

        if bridge.is_user_takeover_active_for_page(page_id_ref) {
            return Err("user_takeover: user has taken control".into());
        }

        let needs_agent_control = matches!(
            action,
            "click" | "fill"
                | "fill_form"
                | "type_text"
                | "press_key"
                | "hover"
                | "scroll"
                | "drag"
                | "select"
                | "type"
                | "upload_file"
                | "wait_for"
        );

        if needs_agent_control {
            self.enter_agent_control(page_id.as_deref())?;
        }

        let result = self.dispatch_sync_inner(action, args, &page_id, page_id_ref, bridge);

        if needs_agent_control {
            schedule_debounced_exit_agent_control(page_id.clone());
        }

        result
    }

    fn dispatch_sync_inner(
        &self,
        action: &str,
        args: &serde_json::Value,
        page_id: &Option<String>,
        page_id_ref: Option<&str>,
        bridge: &Arc<dyn BrowserBridge>,
    ) -> Result<EngineActionResult, String> {
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
                let pid = page_id_from_args_field(args)
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
                let pid = page_id_from_args_field(args)
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

            "list_console_messages" | "get_console_message" | "list_network_requests"
            | "get_network_request" => Err(DEVTOOLS_UNSUPPORTED.to_string()),

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

            "fill_form" => {
                let elements = args
                    .get("elements")
                    .and_then(|v| v.as_array())
                    .ok_or("browser fill_form: missing 'elements' array.")?;
                let mut pairs = Vec::new();
                for item in elements {
                    let uid = item.get("uid").and_then(|v| v.as_str()).unwrap_or("");
                    let value = item.get("value").and_then(|v| v.as_str()).unwrap_or("");
                    if uid.is_empty() {
                        continue;
                    }
                    crate::actions::validate_uid(uid)?;
                    pairs.push(serde_json::json!({ "uid": uid, "value": value }));
                }
                let pairs_json = serde_json::to_string(&pairs).unwrap_or_else(|_| "[]".into());
                let js = format!(
                    r#"(function(){{\
  var filled=0;var items={pairs_json};\
  for(var i=0;i<items.length;i++){{\
    var item=items[i];\
    var el=document.querySelector('[data-fc-uid="'+item.uid+'"]');\
    if(!el)continue;\
    var val=item.value||'';\
    if(el.tagName==='SELECT'){{el.value=val;}}else{{\
      el.focus();\
      var setter=(Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype,'value')||{{}}).set\
        ||(Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype,'value')||{{}}).set;\
      if(setter)setter.call(el,val);else el.value=val;\
      if(el._valueTracker)el._valueTracker.setValue('');\
    }}\
    el.dispatchEvent(new Event('input',{{bubbles:true}}));\
    el.dispatchEvent(new Event('change',{{bubbles:true}}));\
    filled++;\
  }}\
  return filled;\
}})()"#
                );
                let filled = self.eval(page_id.as_deref(), &js)?;
                self.log_agent_op(page_id_ref, "fill_form", &format!("{filled} fields"));
                Ok(EngineActionResult::text(
                    serde_json::json!({ "ok": true, "filled": filled.parse::<u32>().unwrap_or(0) })
                        .to_string(),
                ))
            }

            "type_text" => {
                let text = args
                    .get("text")
                    .and_then(|v| v.as_str())
                    .ok_or("browser type_text: missing 'text'.")?;
                let text_j = serde_json::to_string(text).unwrap_or_default();
                let find = element_find_expr(args, true)?;
                let js = format!(
                    r#"(function(){{
  var el={find}||document.activeElement||document.body;
  if(!el)throw new Error('no target element');
  el.focus();
  var text={text_j};
  var setter=(Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype,'value')||{{}}).set
    ||(Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype,'value')||{{}}).set;
  for(var i=0;i<text.length;i++){{
    var ch=text.charAt(i);
    el.dispatchEvent(new KeyboardEvent('keydown',{{key:ch,bubbles:true}}));
    el.dispatchEvent(new KeyboardEvent('keypress',{{key:ch,bubbles:true}}));
    if(setter&&('value'in el))setter.call(el,(el.value||'')+ch);
    else if('value'in el)el.value=(el.value||'')+ch;
    el.dispatchEvent(new Event('input',{{bubbles:true}}));
    el.dispatchEvent(new KeyboardEvent('keyup',{{key:ch,bubbles:true}}));
  }}
  if(el._valueTracker)el._valueTracker.setValue('');
  return text.length;
}})()"#
                );
                let typed = self.eval(page_id.as_deref(), &js)?;
                if let Some(submit) = args.get("submitKey").and_then(|v| v.as_str()) {
                    let submit_js = webview_press_key_js(submit)?;
                    self.eval(page_id.as_deref(), &submit_js)?;
                }
                self.log_agent_op(page_id_ref, "type_text", text);
                Ok(EngineActionResult::text(
                    serde_json::json!({ "ok": true, "typed": text, "chars": typed }).to_string(),
                ))
            }

            "press_key" => {
                let key = args
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or("browser press_key: missing 'key'.")?;
                let js = webview_press_key_js(key)?;
                self.eval(page_id.as_deref(), &js)?;
                self.log_agent_op(page_id_ref, "press_key", key);
                Ok(EngineActionResult::text(
                    serde_json::json!({ "ok": true, "key": key }).to_string(),
                ))
            }

            "type" => {
                let selector = actions::require_selector(args, "type")?;
                let text = args
                    .get("text")
                    .and_then(|v| v.as_str())
                    .ok_or("browser type: missing 'text'.")?;
                let sel_j = serde_json::to_string(selector).unwrap();
                let text_j = serde_json::to_string(text).unwrap();
                let js = format!(
                    r#"(function(){{
  var el=document.querySelector({sel_j});
  if(!el)throw new Error('not found');
  el.focus();
  var text={text_j};
  var setter=(Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype,'value')||{{}}).set
    ||(Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype,'value')||{{}}).set;
  if(setter)setter.call(el,text);else el.value=text;
  if(el._valueTracker)el._valueTracker.setValue('');
  el.dispatchEvent(new Event('input',{{bubbles:true}}));
  el.dispatchEvent(new Event('change',{{bubbles:true}}));
  return text.length;
}})()"#
                );
                self.eval(page_id.as_deref(), &js)?;
                self.log_agent_op(page_id_ref, "type", selector);
                Ok(EngineActionResult::text(
                    serde_json::json!({ "ok": true, "selector": selector, "text": text })
                        .to_string(),
                ))
            }

            "select" => {
                let selector = args
                    .get("selector")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let uid = args.get("uid").and_then(|v| v.as_str());
                let value = args
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or("browser select: missing 'value'.")?;
                let find = if let Some(u) = uid {
                    crate::actions::validate_uid(u)?;
                    format!("document.querySelector('[data-fc-uid=\"{u}\"]')")
                } else if let Some(sel) = selector {
                    format!("document.querySelector({})", serde_json::to_string(&sel).unwrap())
                } else {
                    return Err("browser select: missing uid or selector".into());
                };
                let val_j = serde_json::to_string(value).unwrap();
                let js = format!(
                    "(() => {{ var el = {find}; if (!el) throw new Error('not found'); \
                     el.value = {val_j}; el.dispatchEvent(new Event('input', {{bubbles: true}})); \
                     el.dispatchEvent(new Event('change', {{bubbles: true}})); return 'ok'; }})()"
                );
                self.eval(page_id.as_deref(), &js)?;
                self.log_agent_op(page_id_ref, "select", value);
                Ok(EngineActionResult::text(
                    serde_json::json!({ "ok": true, "value": value }).to_string(),
                ))
            }

            "drag" => {
                let from_uid = args
                    .get("from_uid")
                    .or(args.get("uid"))
                    .and_then(|v| v.as_str())
                    .ok_or("browser drag: missing 'from_uid'.")?;
                let to_uid = args
                    .get("to_uid")
                    .and_then(|v| v.as_str())
                    .ok_or("browser drag: missing 'to_uid'.")?;
                crate::actions::validate_uid(from_uid)?;
                crate::actions::validate_uid(to_uid)?;
                let js = format!(
                    r#"(function(){{
  var from=document.querySelector('[data-fc-uid="{from_uid}"]');
  var to=document.querySelector('[data-fc-uid="{to_uid}"]');
  if(!from||!to)throw new Error('elements not found');
  var fr=from.getBoundingClientRect();
  var tr=to.getBoundingClientRect();
  var dt=new DataTransfer();
  from.dispatchEvent(new DragEvent('dragstart',{{bubbles:true,dataTransfer:dt,clientX:fr.x+fr.width/2,clientY:fr.y+fr.height/2}}));
  to.dispatchEvent(new DragEvent('dragover',{{bubbles:true,dataTransfer:dt,clientX:tr.x+tr.width/2,clientY:tr.y+tr.height/2}}));
  to.dispatchEvent(new DragEvent('drop',{{bubbles:true,dataTransfer:dt,clientX:tr.x+tr.width/2,clientY:tr.y+tr.height/2}}));
  from.dispatchEvent(new DragEvent('dragend',{{bubbles:true,dataTransfer:dt}}));
  return 'ok';
}})()"#
                );
                self.eval(page_id.as_deref(), &js)?;
                self.log_agent_op(page_id_ref, "drag", &format!("{from_uid}->{to_uid}"));
                Ok(EngineActionResult::text(
                    serde_json::json!({ "ok": true, "from": from_uid, "to": to_uid }).to_string(),
                ))
            }

            "upload_file" => {
                let file_path = args
                    .get("filePath")
                    .and_then(|v| v.as_str())
                    .ok_or("browser upload_file: missing 'filePath'.")?;
                let path = actions::validate_upload_path(file_path)?;
                let bytes = std::fs::read(&path)
                    .map_err(|_| "browser upload_file: could not read file".to_string())?;
                const MAX_WEBVIEW_UPLOAD_BYTES: usize = 300 * 1024;
                if bytes.len() > MAX_WEBVIEW_UPLOAD_BYTES {
                    return Err(format!(
                        "browser upload_file: file too large (max {}KB for webview engine)",
                        MAX_WEBVIEW_UPLOAD_BYTES / 1024
                    ));
                }
                let b64 = base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    &bytes,
                );
                let filename = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("upload.bin");
                let name_j = serde_json::to_string(filename).unwrap();
                let find = element_find_expr(args, false)?;
                let mime = mime_from_path(&path);
                let mime_j = serde_json::to_string(&mime).unwrap();
                let js = format!(
                    r#"(function(){{
  var el={find};
  if(!el)throw new Error('file input not found');
  var b64="{b64}";
  var binary=atob(b64);
  var bytes=new Uint8Array(binary.length);
  for(var i=0;i<binary.length;i++)bytes[i]=binary.charCodeAt(i);
  var file=new File([bytes],{name_j},{{type:{mime_j}}});
  var dt=new DataTransfer();
  dt.items.add(file);
  el.files=dt.files;
  el.dispatchEvent(new Event('input',{{bubbles:true}}));
  el.dispatchEvent(new Event('change',{{bubbles:true}}));
  return 'ok';
}})()"#
                );
                self.eval(page_id.as_deref(), &js)?;
                self.log_agent_op(page_id_ref, "upload_file", filename);
                Ok(EngineActionResult::text(
                    serde_json::json!({ "ok": true, "filePath": file_path }).to_string(),
                ))
            }

            "handle_dialog" => {
                // NOTE: BROWSER_INIT_SCRIPT (xiaolin-app) hijacks confirm/prompt and always
                // returns true/default — it does not read __fc_dialog_* vars. Agent must call
                // handle_dialog before triggering the dialog, but full support requires
                // xiaolin-app BROWSER_INIT_SCRIPT changes.
                let dialog_action = args
                    .get("dialog_action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("accept");
                let prompt_text = args.get("promptText").and_then(|v| v.as_str());
                let js = if dialog_action == "dismiss" {
                    "window.__fc_dialog_dismiss = true; 'ok'".to_string()
                } else if let Some(pt) = prompt_text {
                    let escaped = serde_json::to_string(pt).unwrap();
                    format!("window.__fc_dialog_response = {escaped}; 'ok'")
                } else {
                    "'ok'".to_string()
                };
                self.eval(page_id.as_deref(), &js)?;
                Ok(EngineActionResult::text(
                    serde_json::json!({ "ok": true, "action": dialog_action }).to_string(),
                ))
            }

            "interact" => {
                self.exit_agent_control(page_id.as_deref())?;
                if let Some(u) = args.get("url").and_then(|v| v.as_str()) {
                    actions::validate_url_scheme(u)?;
                    bridge.navigate(page_id_ref, u)?;
                }
                let started_url = self.eval(page_id.as_deref(), "location.href")?;
                let wait_seconds = args
                    .get("wait_seconds")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(60);
                let deadline_ms = wait_seconds * 1000;
                let poll_js = format!(
                    r#"(function(){{
  var started={started};
  if(location.href!==started)return location.href;
  return '';
}})()"#,
                    started = serde_json::to_string(&started_url).unwrap()
                );
                let mut final_url = started_url.clone();
                let poll_interval = std::time::Duration::from_secs(2);
                let started = std::time::Instant::now();
                while started.elapsed().as_millis() < deadline_ms as u128 {
                    std::thread::sleep(poll_interval);
                    let current = self.eval(page_id.as_deref(), &poll_js)?;
                    if !current.is_empty() && current != started_url {
                        final_url = current.trim_matches('"').to_string();
                        break;
                    }
                }
                let title = self
                    .eval(page_id.as_deref(), "document.title")
                    .unwrap_or_default();
                self.log_agent_op(page_id_ref, "interact", &final_url);
                Ok(EngineActionResult::text(
                    serde_json::json!({
                        "started_url": started_url,
                        "final_url": final_url,
                        "title": title.trim_matches('"'),
                        "url_changed": started_url != final_url,
                    })
                    .to_string(),
                ))
            }

            other => Err(format!(
                "browser webview: action '{other}' not implemented yet"
            )),
        }
    }
}

fn webview_interaction_js(action: &str, args: &serde_json::Value) -> Result<String, String> {
    let uid = args.get("uid").and_then(|v| v.as_str());
    let selector = args.get("selector").and_then(|v| v.as_str());
    let find = if let Some(u) = uid {
        crate::actions::validate_uid(u)?;
        format!(
            "document.querySelector('[data-fc-uid=\"{}\"]')",
            u
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
                 if (el.tagName === 'SELECT') {{ el.value = {val_j}; }} else {{ \
                   el.focus(); \
                   var setter = (Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value') || {{}}).set \
                     || (Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value') || {{}}).set; \
                   if (setter) setter.call(el, {val_j}); else el.value = {val_j}; \
                   if (el._valueTracker) el._valueTracker.setValue(''); \
                 }} \
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

fn page_id_from_args_field(args: &serde_json::Value) -> Option<String> {
    args.get("pageId")
        .or(args.get("page_id"))
        .and_then(|v| {
            v.as_str()
                .map(String::from)
                .or_else(|| v.as_u64().map(|n| n.to_string()))
        })
}

fn element_find_expr(args: &serde_json::Value, optional: bool) -> Result<String, String> {
    let uid = args.get("uid").and_then(|v| v.as_str());
    let selector = args.get("selector").and_then(|v| v.as_str());
    if let Some(u) = uid {
        crate::actions::validate_uid(u)?;
        Ok(format!("document.querySelector('[data-fc-uid=\"{u}\"]')"))
    } else if let Some(sel) = selector {
        Ok(format!(
            "document.querySelector({})",
            serde_json::to_string(sel).unwrap()
        ))
    } else if optional {
        Ok("null".to_string())
    } else {
        Err("missing uid or selector".to_string())
    }
}

fn webview_press_key_js(key: &str) -> Result<String, String> {
    let parts: Vec<&str> = key.split('+').map(|s| s.trim()).collect();
    let main_key = parts.last().copied().unwrap_or(key);
    let main_j = serde_json::to_string(main_key).unwrap_or_default();
    let mut mods = Vec::new();
    for part in &parts[..parts.len().saturating_sub(1)] {
        match part.to_ascii_lowercase().as_str() {
            "control" | "ctrl" => mods.push("ctrlKey: true"),
            "shift" => mods.push("shiftKey: true"),
            "alt" => mods.push("altKey: true"),
            "meta" | "command" | "cmd" => mods.push("metaKey: true"),
            other => return Err(format!("browser press_key: unknown modifier '{other}'")),
        }
    }
    let mods_str = if mods.is_empty() {
        String::new()
    } else {
        format!(", {}", mods.join(", "))
    };
    Ok(format!(
        r#"(function(){{
  var el=document.activeElement||document.body;
  var opts={{key:{main_j},bubbles:true,cancelable:true{mods_str}}};
  el.dispatchEvent(new KeyboardEvent('keydown',opts));
  el.dispatchEvent(new KeyboardEvent('keypress',opts));
  el.dispatchEvent(new KeyboardEvent('keyup',opts));
  return 'ok';
}})()"#
    ))
}

fn mime_from_path(path: &std::path::Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("pdf") => "application/pdf",
        Some("txt") => "text/plain",
        Some("csv") => "text/csv",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
    .to_string()
}

static WEBVIEW_SUPPORTED_ACTIONS: &[&str] = &[
    "navigate",
    "go_back",
    "go_forward",
    "reload",
    "click",
    "fill",
    "fill_form",
    "type_text",
    "press_key",
    "hover",
    "scroll",
    "take_snapshot",
    "screenshot",
    "get_content",
    "evaluate",
    "wait_for",
    "list_pages",
    "select_page",
    "new_page",
    "close_page",
    "cookies",
    "interact",
];

#[async_trait]
impl BrowserEngine for TauriWebViewEngine {
    fn engine_type(&self) -> &str {
        "webview"
    }

    fn supported_actions(&self) -> &[&str] {
        WEBVIEW_SUPPORTED_ACTIONS
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
