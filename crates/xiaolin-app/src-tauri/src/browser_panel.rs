use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::http::{self, header, StatusCode};
use tauri::{Emitter, Manager, UriSchemeContext, UriSchemeResponder, Url, Wry};

pub const MAX_BROWSER_PAGES: usize = 8;
pub const MAX_IPC_MESSAGE_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_BROWSER_URL_LEN: usize = 8192;
pub const MAX_BROWSER_JS_LEN: usize = 512 * 1024;
pub const OFFSCREEN_POSITION: f64 = -9999.0;
pub const BROWSER_WEBVIEW_PREFIX: &str = "browser-";

/// Whitelist message types for xiaolin-internal://callback.
const ALLOWED_INTERNAL_MESSAGE_TYPES: &[&str] = &[
    "ready",
    "snapshot",
    "console",
    "network",
    "selection",
    "dialog",
    "eval_result",
    "user_action_blocked",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PageVisibility {
    Active,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase", tag = "state")]
pub enum PageLoadState {
    Loading,
    Ready,
    Failed(String),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserPageInfo {
    pub page_id: String,
    pub url: String,
    pub title: String,
    pub visibility: PageVisibility,
    pub load_state: PageLoadState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BrowserPage {
    pub page_id: String,
    pub webview_label: String,
    pub url: String,
    pub title: String,
    pub visibility: PageVisibility,
    pub load_state: PageLoadState,
    pub layout_x: f64,
    pub layout_y: f64,
    pub layout_width: f64,
    pub layout_height: f64,
}

impl BrowserPage {
    pub fn to_info(&self) -> BrowserPageInfo {
        BrowserPageInfo {
            page_id: self.page_id.clone(),
            url: self.url.clone(),
            title: self.title.clone(),
            visibility: self.visibility,
            load_state: self.load_state.clone(),
        }
    }
}

pub struct BrowserPanelManager {
    pages: HashMap<String, BrowserPage>,
    active_page_id: Option<String>,
}

impl BrowserPanelManager {
    pub fn new() -> Self {
        Self {
            pages: HashMap::new(),
            active_page_id: None,
        }
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub(crate) fn active_page_id(&self) -> Option<&str> {
        self.active_page_id.as_deref()
    }

    pub fn add_page(&mut self, page: BrowserPage) -> Result<(), String> {
        if self.pages.len() >= MAX_BROWSER_PAGES {
            return Err(format!("browser page limit reached ({MAX_BROWSER_PAGES})"));
        }
        let page_id = page.page_id.clone();
        self.pages.insert(page_id.clone(), page);
        self.active_page_id = Some(page_id);
        self.normalize_active_visibility();
        Ok(())
    }

    pub fn remove_page(&mut self, page_id: &str) -> Option<BrowserPage> {
        let removed = self.pages.remove(page_id);
        if self.active_page_id.as_deref() == Some(page_id) {
            self.active_page_id = self.pages.keys().next().cloned();
            if let Some(active_id) = self.active_page_id.clone() {
                let _ = self.set_active(&active_id);
            }
        }
        removed
    }

    pub fn get_page(&self, page_id: &str) -> Option<&BrowserPage> {
        self.pages.get(page_id)
    }

    pub fn get_page_mut(&mut self, page_id: &str) -> Option<&mut BrowserPage> {
        self.pages.get_mut(page_id)
    }

    pub fn get_page_by_webview_label(&self, label: &str) -> Option<&BrowserPage> {
        self.pages.values().find(|p| p.webview_label == label)
    }

    pub fn list_pages(&self) -> Vec<BrowserPageInfo> {
        self.pages.values().map(BrowserPage::to_info).collect()
    }

    pub fn set_active(&mut self, page_id: &str) -> Result<(), String> {
        if !self.pages.contains_key(page_id) {
            return Err("page not found".into());
        }
        self.active_page_id = Some(page_id.to_string());
        self.normalize_active_visibility();
        Ok(())
    }

    pub fn hide_all(&mut self) {
        for page in self.pages.values_mut() {
            page.visibility = PageVisibility::Hidden;
        }
    }

    pub fn update_layout(
        &mut self,
        page_id: &str,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> Result<(), String> {
        let page = self
            .pages
            .get_mut(page_id)
            .ok_or_else(|| "page not found".to_string())?;
        page.layout_x = x;
        page.layout_y = y;
        page.layout_width = width;
        page.layout_height = height;
        Ok(())
    }

    fn normalize_active_visibility(&mut self) {
        let active = self.active_page_id.clone();
        for (id, page) in &mut self.pages {
            page.visibility = if active.as_deref() == Some(id.as_str()) {
                PageVisibility::Active
            } else {
                PageVisibility::Hidden
            };
        }
    }
}

impl Default for BrowserPanelManager {
    fn default() -> Self {
        Self::new()
    }
}

pub struct BrowserPanelState(pub std::sync::Mutex<BrowserPanelManager>);

/// Shared browser data directory for cookie/storage persistence (D2).
pub fn browser_data_directory() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("xiaolin")
        .join("browser-data")
}

#[cfg(target_os = "macos")]
pub fn browser_data_store_identifier() -> [u8; 16] {
    *b"xiaolin-browser\x00"
}

/// Deny-by-default navigation whitelist (Layer 3 / rule #28).
pub fn is_navigation_allowed(url: &Url) -> bool {
    match url.scheme() {
        "http" | "https" => true,
        "file" | "javascript" | "data" | "tauri" | "ipc" | "asset" => {
            tracing::warn!(url = %url, scheme = url.scheme(), "blocked browser navigation");
            false
        }
        other => {
            tracing::warn!(url = %url, scheme = other, "blocked browser navigation (unknown protocol)");
            false
        }
    }
}

pub fn validate_browser_url(url: &str) -> Result<Url, String> {
    if url.is_empty() {
        return Err("url required".into());
    }
    if url.len() > MAX_BROWSER_URL_LEN {
        return Err("url too long".into());
    }
    if url.contains('\0') {
        return Err("invalid url".into());
    }
    let parsed = Url::parse(url).map_err(|_| "invalid url".to_string())?;
    if !is_navigation_allowed(&parsed) {
        return Err("url not allowed".into());
    }
    Ok(parsed)
}

pub fn validate_page_id(page_id: &str) -> Result<(), String> {
    if page_id.is_empty() || page_id.len() > 64 {
        return Err("invalid page id".into());
    }
    if !page_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("invalid page id".into());
    }
    Ok(())
}

pub fn validate_js_payload(js: &str) -> Result<(), String> {
    if js.is_empty() {
        return Err("script required".into());
    }
    if js.len() > MAX_BROWSER_JS_LEN {
        return Err("script too long".into());
    }
    if js.contains('\0') {
        return Err("invalid script".into());
    }
    Ok(())
}

/// Layer 0-3 initialization script injected into every browser page (D13).
pub const BROWSER_INIT_SCRIPT: &str = r#"(function(){
'use strict';
function send(msg){return fetch('xiaolin-internal://callback',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(msg)}).then(function(r){return r.json().catch(function(){return{};});}).catch(function(){return{};});}
function notify(type,data){send({type:type,data:data,ts:Date.now()});}
var api={send:send,notify:notify,version:1};
Object.freeze(api);
Object.defineProperty(window,'__XIAOLIN__',{value:api,writable:false,configurable:false,enumerable:false});
notify('ready',{url:location.href});
['log','warn','error','info','debug'].forEach(function(level){var orig=console[level].bind(console);console[level]=function(){var args=Array.prototype.slice.call(arguments);notify('console',{level:level,args:args.map(String).slice(0,10)});return orig.apply(console,arguments);};});
window.addEventListener('error',function(e){notify('console',{level:'error',args:[e.message,(e.filename||'')+':'+(e.lineno||0)]});});
window.addEventListener('unhandledrejection',function(e){notify('console',{level:'error',args:['Unhandled rejection: '+String(e.reason)]});});
var origFetch=window.fetch;
window.fetch=function(input,init){var url=typeof input==='string'?input:(input&&input.url)||String(input);var method=(init&&init.method)||'GET';var t0=Date.now();return origFetch.apply(this,arguments).then(function(resp){notify('network',{type:'fetch',method:method,url:url,status:resp.status,timing:Date.now()-t0});return resp;}).catch(function(err){notify('network',{type:'fetch',method:method,url:url,status:0,error:String(err),timing:Date.now()-t0});throw err;});};
var XHR=XMLHttpRequest.prototype,origOpen=XHR.open,origSend=XHR.send;
XHR.open=function(method,url){this.__xl_method=method;this.__xl_url=url;this.__xl_t0=Date.now();return origOpen.apply(this,arguments);};
XHR.send=function(){var self=this;this.addEventListener('loadend',function(){notify('network',{type:'xhr',method:self.__xl_method,url:self.__xl_url,status:self.status,timing:Date.now()-(self.__xl_t0||0)});});return origSend.apply(this,arguments);};
window.alert=function(msg){notify('dialog',{kind:'alert',message:String(msg)});};
window.confirm=function(msg){notify('dialog',{kind:'confirm',message:String(msg)});return true;};
window.prompt=function(msg,def){notify('dialog',{kind:'prompt',message:String(msg),default:def!=null?String(def):''});return def!=null?String(def):'';};
})();"#;

fn http_response(status: StatusCode, body: &[u8]) -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body.to_vec())
        .unwrap_or_else(|_| {
            http::Response::builder()
                .status(status)
                .body(body.to_vec())
                .expect("response without headers")
        })
}

fn is_browser_webview_label(label: &str) -> bool {
    label.starts_with(BROWSER_WEBVIEW_PREFIX)
}

/// Global handler for `xiaolin-internal://` custom protocol (D12).
pub fn handle_xiaolin_internal_protocol(
    ctx: UriSchemeContext<'_, Wry>,
    request: http::Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let app = ctx.app_handle().clone();
    let webview_label = ctx.webview_label().to_string();

    if request.method() != http::Method::POST {
        responder.respond(http_response(
            StatusCode::METHOD_NOT_ALLOWED,
            br#"{"ok":false,"error":"method not allowed"}"#,
        ));
        return;
    }

    if !is_browser_webview_label(&webview_label) {
        tracing::warn!(
            webview = %webview_label,
            "blocked xiaolin-internal request from non-browser webview"
        );
        responder.respond(http_response(
            StatusCode::FORBIDDEN,
            br#"{"ok":false,"error":"forbidden"}"#,
        ));
        return;
    }

    let body = request.body();
    if body.len() > MAX_IPC_MESSAGE_BYTES {
        tracing::warn!(
            webview = %webview_label,
            bytes = body.len(),
            "xiaolin-internal payload too large"
        );
        responder.respond(http_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            br#"{"ok":false,"error":"payload too large"}"#,
        ));
        return;
    }

    let payload: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "invalid xiaolin-internal JSON body");
            responder.respond(http_response(
                StatusCode::BAD_REQUEST,
                br#"{"ok":false,"error":"invalid json"}"#,
            ));
            return;
        }
    };

    let msg_type = payload
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if !ALLOWED_INTERNAL_MESSAGE_TYPES.contains(&msg_type) {
        tracing::warn!(
            webview = %webview_label,
            msg_type = msg_type,
            "blocked unknown xiaolin-internal message type"
        );
        responder.respond(http_response(
            StatusCode::FORBIDDEN,
            br#"{"ok":false,"error":"forbidden type"}"#,
        ));
        return;
    }

    let page_id = {
        let state = app.state::<BrowserPanelState>();
        let guard = match state.0.lock() {
            Ok(g) => g,
            Err(_) => {
                responder.respond(http_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    br#"{"ok":false,"error":"internal error"}"#,
                ));
                return;
            }
        };
        guard
            .get_page_by_webview_label(&webview_label)
            .map(|p| p.page_id.clone())
    };

    let Some(page_id) = page_id else {
        responder.respond(http_response(
            StatusCode::NOT_FOUND,
            br#"{"ok":false,"error":"page not found"}"#,
        ));
        return;
    };

    if msg_type == "eval_result" {
        let data = payload.get("data");
        let id = data.and_then(|d| d.get("id")).and_then(|v| v.as_str());
        if let Some(id) = id {
            let outcome = match (
                data.and_then(|d| d.get("result")).and_then(|v| v.as_str()),
                data.and_then(|d| d.get("error")).and_then(|v| v.as_str()),
            ) {
                (Some(result), _) => Ok(result.to_string()),
                (_, Some(error)) => Err(error.to_string()),
                _ => Err("eval_result missing result and error".to_string()),
            };
            crate::browser_eval::complete_eval(id, outcome);
        }
        responder.respond(http_response(
            StatusCode::OK,
            br#"{"ok":true}"#,
        ));
        return;
    }

    let event_name = match msg_type {
        "ready" => "browser-page-ready",
        "snapshot" => "browser-snapshot",
        "console" => "browser-console",
        "network" => "browser-network",
        "selection" | "user_action_blocked" => "browser-user-action",
        "dialog" => "browser-dialog",
        _ => unreachable!("validated above"),
    };

    let emit_payload = serde_json::json!({
        "pageId": page_id,
        "type": msg_type,
        "data": payload.get("data").cloned().unwrap_or(serde_json::Value::Null),
        "ts": payload.get("ts"),
    });

    if let Err(e) = app.emit(event_name, emit_payload) {
        tracing::warn!(error = %e, event = event_name, "failed to emit browser internal event");
    }

    responder.respond(http_response(
        StatusCode::OK,
        br#"{"ok":true}"#,
    ));
}

pub fn default_download_directory() -> PathBuf {
    dirs::download_dir().unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("Downloads")
    })
}

pub fn sanitize_download_filename(raw: &str) -> String {
    let name = raw
        .rsplit('/')
        .next()
        .unwrap_or("download")
        .rsplit('\\')
        .next()
        .unwrap_or("download");
    let sanitized: String = name
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0'))
        .collect();
    if sanitized.is_empty() {
        "download".to_string()
    } else {
        sanitized.chars().take(200).collect()
    }
}
