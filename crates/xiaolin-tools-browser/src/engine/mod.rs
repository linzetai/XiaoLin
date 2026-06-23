use std::sync::Arc;

mod cdp_engine;
pub(crate) mod webview_engine;

pub use cdp_engine::CdpEngine;
pub use webview_engine::{
    browser_clear_user_takeover, browser_context_for_prompt, browser_request_user_takeover,
    set_browser_bridge, BrowserBridge, TauriWebViewEngine,
};

use async_trait::async_trait;
use xiaolin_core::tool::ToolImage;

/// Result of a browser engine action (text + optional images for multimodal).
#[derive(Debug, Clone)]
pub struct EngineActionResult {
    pub text: String,
    pub images: Vec<ToolImage>,
}

impl EngineActionResult {
    pub fn text(s: impl Into<String>) -> Self {
        Self {
            text: s.into(),
            images: vec![],
        }
    }

    pub fn with_images(text: impl Into<String>, images: Vec<ToolImage>) -> Self {
        Self {
            text: text.into(),
            images,
        }
    }
}

/// Cookie parameter for browser cookie operations.
#[derive(Debug, Clone)]
pub struct CookieParam {
    pub name: String,
    pub value: String,
    pub domain: Option<String>,
    pub path: Option<String>,
}

/// Abstract browser automation engine (CDP headless Chrome or Tauri built-in WebView).
#[async_trait]
pub trait BrowserEngine: Send + Sync {
    /// Primary dispatch entry — all browser tool actions route through here.
    async fn execute_action(
        &self,
        action: &str,
        args: &serde_json::Value,
    ) -> Result<EngineActionResult, String>;

    fn engine_type(&self) -> &str;

    /// Synchronous cleanup (safe to call from `Drop` and non-async contexts).
    fn shutdown_sync(&self) {}

    async fn shutdown(&self) {
        self.shutdown_sync();
    }

    // ── Navigation (default wrappers) ─────────────────────────────────────

    async fn navigate(&self, url: &str) -> Result<EngineActionResult, String> {
        self.execute_action(
            "navigate",
            &serde_json::json!({ "type": "url", "url": url }),
        )
        .await
    }

    async fn go_back(&self) -> Result<EngineActionResult, String> {
        self.execute_action("go_back", &serde_json::json!({})).await
    }

    async fn go_forward(&self) -> Result<EngineActionResult, String> {
        self.execute_action("go_forward", &serde_json::json!({}))
            .await
    }

    async fn reload(&self) -> Result<EngineActionResult, String> {
        self.execute_action("reload", &serde_json::json!({})).await
    }

    // ── Interaction ───────────────────────────────────────────────────────

    async fn click(
        &self,
        selector: &str,
        uid: Option<&str>,
    ) -> Result<EngineActionResult, String> {
        let mut args = serde_json::json!({});
        if let Some(u) = uid {
            args["uid"] = serde_json::json!(u);
        } else {
            args["selector"] = serde_json::json!(selector);
        }
        self.execute_action("click", &args).await
    }

    async fn fill(&self, selector: &str, value: &str) -> Result<EngineActionResult, String> {
        self.execute_action(
            "fill",
            &serde_json::json!({ "selector": selector, "value": value }),
        )
        .await
    }

    async fn type_text(&self, text: &str) -> Result<EngineActionResult, String> {
        self.execute_action("type_text", &serde_json::json!({ "text": text }))
            .await
    }

    async fn press_key(&self, key: &str) -> Result<EngineActionResult, String> {
        self.execute_action("press_key", &serde_json::json!({ "key": key }))
            .await
    }

    async fn hover(&self, selector: &str) -> Result<EngineActionResult, String> {
        self.execute_action("hover", &serde_json::json!({ "selector": selector }))
            .await
    }

    async fn scroll(
        &self,
        direction: &str,
        amount: Option<i32>,
    ) -> Result<EngineActionResult, String> {
        let mut args = serde_json::json!({ "direction": direction });
        if let Some(a) = amount {
            args["amount"] = serde_json::json!(a);
        }
        self.execute_action("scroll", &args).await
    }

    // ── Observation ───────────────────────────────────────────────────────

    async fn take_snapshot(&self) -> Result<EngineActionResult, String> {
        self.execute_action("take_snapshot", &serde_json::json!({}))
            .await
    }

    async fn get_content(&self) -> Result<EngineActionResult, String> {
        self.execute_action("get_content", &serde_json::json!({})).await
    }

    async fn screenshot(&self) -> Result<EngineActionResult, String> {
        self.execute_action("screenshot", &serde_json::json!({})).await
    }

    async fn evaluate(&self, js: &str) -> Result<EngineActionResult, String> {
        self.execute_action("evaluate", &serde_json::json!({ "script": js }))
            .await
    }

    async fn wait_for(
        &self,
        selector: &str,
        timeout: Option<u64>,
    ) -> Result<EngineActionResult, String> {
        let mut args = serde_json::json!({ "selector": selector });
        if let Some(t) = timeout {
            args["timeout"] = serde_json::json!(t);
        }
        self.execute_action("wait_for", &args).await
    }

    // ── Page management ───────────────────────────────────────────────────

    async fn list_pages(&self) -> Result<EngineActionResult, String> {
        self.execute_action("list_pages", &serde_json::json!({})).await
    }

    async fn select_page(&self, page_id: &str) -> Result<EngineActionResult, String> {
        self.execute_action(
            "select_page",
            &serde_json::json!({ "pageId": page_id.parse::<u64>().unwrap_or(0) }),
        )
        .await
    }

    async fn new_page(&self, url: Option<&str>) -> Result<EngineActionResult, String> {
        let mut args = serde_json::json!({});
        if let Some(u) = url {
            args["url"] = serde_json::json!(u);
        }
        self.execute_action("new_page", &args).await
    }

    async fn close_page(&self, page_id: Option<&str>) -> Result<EngineActionResult, String> {
        let mut args = serde_json::json!({});
        if let Some(id) = page_id {
            args["pageId"] = serde_json::json!(id.parse::<u64>().unwrap_or(0));
        }
        self.execute_action("close_page", &args).await
    }

    async fn cookies(
        &self,
        operation: &str,
        cookies: Option<&[CookieParam]>,
    ) -> Result<EngineActionResult, String> {
        let mut args = serde_json::json!({ "operation": operation });
        if let Some(list) = cookies {
            if let Some(first) = list.first() {
                args["cookie_name"] = serde_json::json!(first.name);
                args["cookie_value"] = serde_json::json!(first.value);
            }
        }
        self.execute_action("cookies", &args).await
    }
}

/// Select engine kind from environment. Tauri desktop sets `XIAOLIN_BROWSER_ENGINE=webview`.
pub fn engine_kind_from_env() -> &'static str {
    if let Ok(raw) = std::env::var("XIAOLIN_BROWSER_ENGINE") {
        let s = raw.trim().to_ascii_lowercase();
        if s == "webview" || s == "tauri" {
            return "webview";
        }
        if s == "cdp" || s == "chrome" {
            return "cdp";
        }
    }
    if webview_engine::bridge_is_configured() {
        "webview"
    } else {
        "cdp"
    }
}

/// Build the default browser engine for the current runtime environment.
pub fn default_engine() -> Arc<dyn BrowserEngine> {
    match engine_kind_from_env() {
        "webview" => Arc::new(TauriWebViewEngine::new()),
        _ => Arc::new(CdpEngine::new()),
    }
}
