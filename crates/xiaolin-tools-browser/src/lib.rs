mod actions;
mod engine;
mod js;
mod network;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use xiaolin_core::tool::{Tool, ToolParameterSchema, ToolRegistry, ToolResult};

pub use network::{
    execute_network_action, network_bridge_configured, set_browser_network_bridge,
    set_network_ws_broadcast, broadcast_network_event, validate_network_action,
    BrowserNetworkBridge,
};
pub use engine::{
    browser_clear_user_takeover, browser_context_for_prompt, browser_request_user_takeover,
    default_engine, engine_kind_from_env, set_browser_bridge, BrowserBridge, BrowserEngine,
    CdpEngine, EngineActionResult, TauriWebViewEngine,
};
pub use js::{CONTENT_EXTRACT_JS, SELECTION_TOOLBAR_JS, UNTRUSTED_SOURCE, UNTRUSTED_WARNING};

const ACTION_TIMEOUT: Duration = Duration::from_secs(60);

/// Browser automation tool — delegates to a [`BrowserEngine`] implementation.
///
/// Default engine selection (see [`engine_kind_from_env`]):
/// - Tauri desktop with bridge registered → `TauriWebViewEngine`
/// - Pure gateway / CI → `CdpEngine` (headless Chrome)
pub struct BrowserTool {
    engine: Arc<dyn BrowserEngine>,
}

impl Default for BrowserTool {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserTool {
    pub fn new() -> Self {
        Self {
            engine: default_engine(),
        }
    }

    pub fn with_engine(engine: Arc<dyn BrowserEngine>) -> Self {
        Self { engine }
    }

    pub fn engine_type(&self) -> &str {
        self.engine.engine_type()
    }

    /// Shut down the browser engine and release resources.
    pub fn shutdown(&self) {
        self.engine.shutdown_sync();
    }

    // Re-exports for tests
    pub fn parse_timeout(args: &serde_json::Value) -> Duration {
        actions::parse_timeout(args)
    }

    pub fn require_selector<'a>(
        args: &'a serde_json::Value,
        action: &str,
    ) -> Result<&'a str, String> {
        actions::require_selector(args, action)
    }

    pub fn validate_args(action: &str, args: &serde_json::Value) -> Result<(), String> {
        actions::validate_args(action, args)
    }

    pub fn validate_url_scheme(url: &str) -> Result<(), String> {
        actions::validate_url_scheme(url)
    }

    pub fn validate_output_path(path: &str) -> Result<std::path::PathBuf, String> {
        actions::validate_output_path(path)
    }

    pub fn workspace_root_for_paths() -> std::path::PathBuf {
        actions::workspace_root_for_paths()
    }

    pub fn is_headless() -> bool {
        CdpEngine::is_headless()
    }
}

impl Drop for BrowserTool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &str {
        "browser"
    }

    fn search_hint(&self) -> &str {
        "chrome headless automation web page click screenshot navigate form"
    }

    fn is_deferred(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Browser automation via built-in WebView (Tauri) or Chrome DevTools Protocol fallback.\n\n\
         ## Core Workflow\n\
         1. **screenshot** → visually see the page (image returned directly to you)\n\
         2. **take_snapshot** → get accessibility tree with element UIDs\n\
         3. Use **uid-based actions** (click, fill, hover, etc.) to interact\n\
         4. **screenshot** again to verify the result\n\n\
         ALWAYS take a screenshot after navigation or interaction to visually confirm the outcome. \
         The screenshot image is sent directly to you for visual understanding — use it to verify layout, errors, content, and success/failure of actions.\n\n\
         ## Actions\n\
         **Navigation**: navigate (type: url/back/forward/reload), wait_for (text/selector)\n\
         **Observation**: take_snapshot (a11y tree → UIDs), screenshot (visual, supports uid/fullPage/format/quality), get_content (HTML), pdf\n\
         **Interaction**: click (uid, dblClick), fill (uid + value), fill_form (batch), type_text (+ submitKey), press_key (combos), hover, select, scroll, drag (from_uid → to_uid), upload_file (uid + filePath), handle_dialog, interact (manual CAPTCHA/login)\n\
         **Tabs**: list_pages, select_page (pageId), new_page (url), close_page\n\
         **DevTools**: evaluate (JS), list_network_requests, get_network_request (reqid), list_console_messages, get_console_message (msgid)\n\
         **Emulation**: emulate (userAgent, colorScheme), resize_page\n\
         **Cookies**: cookies (operation: get/set/delete/clear)\n\n\
         ## Action-Parameter Quick Reference\n\
         | action | required params | optional params |\n\
         |--------|-----------------|------------------|\n\
         | navigate | url (or type=back/forward/reload) | timeout |\n\
         | click | uid or selector | dblClick, includeSnapshot |\n\
         | fill | uid + value | — |\n\
         | fill_form | elements (array) | — |\n\
         | type_text | text | submitKey |\n\
         | press_key | key | — |\n\
         | screenshot | — | uid, fullPage, format, quality |\n\
         | take_snapshot | — | verbose |\n\
         | wait_for | text or selector | timeout |\n\
         | scroll | direction (up/down) | uid, amount |\n\
         | evaluate | expression | — |\n\
         | cookies | operation (get/set/delete/clear) | name, value, domain |\n\
         | select | uid + value(s) | — |\n\
         | drag | from_uid + to_uid | — |\n\
         | upload_file | uid + filePath | — |\n\n\
         ## Key Rules\n\
         - Prefer take_snapshot + uid over CSS selectors — UIDs are stable and unambiguous\n\
         - Use screenshot FREQUENTLY: after every navigation, after interactions, before making decisions about page content\n\
         - Persistent tab preserves session/cookies across actions — no need to re-login\n\
         - Set XIAOLIN_BROWSER_HEADLESS=true for CI/CDP environments"
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert("action".to_string(), serde_json::json!({
            "type": "string",
            "enum": [
                "navigate", "take_snapshot", "screenshot", "evaluate",
                "click", "fill", "fill_form", "type_text", "press_key",
                "hover", "select", "wait_for", "scroll", "drag",
                "handle_dialog", "interact", "get_content", "pdf",
                "list_pages", "select_page", "new_page", "close_page",
                "cookies", "list_network_requests", "list_console_messages",
                "get_console_message", "get_network_request",
                "upload_file", "emulate", "resize_page",
                "set_hosts", "set_proxy", "get_network_config", "clear_hosts"
            ],
            "description": "Action to perform. Workflow: screenshot → take_snapshot → uid-based actions → screenshot to verify. \
             Use navigate with type=back/forward/reload instead of separate go_back/go_forward/reload actions."
        }));
        props.insert(
            "type".to_string(),
            serde_json::json!({
                "type": "string", "enum": ["url", "back", "forward", "reload"],
                "description": "Navigate sub-type (for action=navigate). Default: url."
            }),
        );
        props.insert("url".to_string(), serde_json::json!({
            "type": "string",
            "description": "URL for navigate(type=url), new_page, or optional in screenshot/evaluate/interact."
        }));
        props.insert("uid".to_string(), serde_json::json!({
            "type": "string",
            "description": "Element UID from take_snapshot (e.g. 'e5'). Used by click, fill, hover, drag, screenshot (element capture), upload_file."
        }));
        props.insert(
            "dblClick".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "For click: double-click if true. Default false."
            }),
        );
        props.insert("includeSnapshot".to_string(), serde_json::json!({
            "type": "boolean",
            "description": "Include a11y snapshot in response after action. Works with click, fill, fill_form, hover, drag, press_key, upload_file. Default false."
        }));
        props.insert(
            "format".to_string(),
            serde_json::json!({
                "type": "string", "enum": ["png", "jpeg", "webp"],
                "description": "For screenshot: image format. Default png."
            }),
        );
        props.insert(
            "quality".to_string(),
            serde_json::json!({
                "type": "number",
                "description": "For screenshot: JPEG/WebP compression quality (0-100)."
            }),
        );
        props.insert(
            "msgid".to_string(),
            serde_json::json!({
                "type": "number",
                "description": "For get_console_message: message index from list_console_messages."
            }),
        );
        props.insert(
            "reqid".to_string(),
            serde_json::json!({
                "type": "number",
                "description": "For get_network_request: request index from list_network_requests."
            }),
        );
        props.insert("selector".to_string(), serde_json::json!({
            "type": "string",
            "description": "CSS selector (fallback when uid not available). Used by click, fill, hover, wait_for, legacy type/select."
        }));
        props.insert(
            "value".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Value for fill, fill_form elements, or legacy select."
            }),
        );
        props.insert(
            "text".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Text for type_text, or array of texts for wait_for."
            }),
        );
        props.insert(
            "script".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "JS code for evaluate action."
            }),
        );
        props.insert(
            "key".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Key for press_key (e.g. Enter, Tab, Control+A)."
            }),
        );
        props.insert(
            "submitKey".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Key to press after type_text (e.g. Enter)."
            }),
        );
        props.insert(
            "elements".to_string(),
            serde_json::json!({
                "type": "array",
                "description": "Array of {uid, value} for fill_form."
            }),
        );
        props.insert(
            "from_uid".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Source element UID for drag."
            }),
        );
        props.insert(
            "to_uid".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Target element UID for drag."
            }),
        );
        props.insert(
            "direction".to_string(),
            serde_json::json!({
                "type": "string", "description": "Scroll direction: up or down (default down)."
            }),
        );
        props.insert(
            "amount".to_string(),
            serde_json::json!({
                "type": "integer", "description": "Scroll pixels (default 300)."
            }),
        );
        props.insert(
            "timeout".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Timeout in ms for wait_for, navigate (default 10000)."
            }),
        );
        props.insert("verbose".to_string(), serde_json::json!({
            "type": "boolean",
            "description": "For take_snapshot: include full a11y tree. Default false (interactive-only)."
        }));
        props.insert(
            "fullPage".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "For screenshot: capture full page. Default false."
            }),
        );
        props.insert(
            "filePath".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "File path to save screenshot/snapshot output."
            }),
        );
        props.insert(
            "ignoreCache".to_string(),
            serde_json::json!({
                "type": "boolean", "description": "For navigate(reload): bypass cache."
            }),
        );
        props.insert(
            "pageId".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Tab index for select_page, close_page."
            }),
        );
        props.insert(
            "dialog_action".to_string(),
            serde_json::json!({
                "type": "string", "enum": ["accept", "dismiss"],
                "description": "For handle_dialog: accept or dismiss."
            }),
        );
        props.insert(
            "promptText".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Response text for handle_dialog on window.prompt."
            }),
        );
        props.insert(
            "operation".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "For cookies: get, set, delete, or clear."
            }),
        );
        props.insert(
            "cookie_name".to_string(),
            serde_json::json!({
                "type": "string", "description": "Cookie name for set/delete."
            }),
        );
        props.insert(
            "cookie_value".to_string(),
            serde_json::json!({
                "type": "string", "description": "Cookie value for set."
            }),
        );
        props.insert(
            "wait_seconds".to_string(),
            serde_json::json!({
                "type": "integer", "description": "Max seconds for interact (default 60)."
            }),
        );
        props.insert(
            "width".to_string(),
            serde_json::json!({
                "type": "number", "description": "Page width for resize_page."
            }),
        );
        props.insert(
            "height".to_string(),
            serde_json::json!({
                "type": "number", "description": "Page height for resize_page."
            }),
        );
        props.insert(
            "userAgent".to_string(),
            serde_json::json!({
                "type": "string", "description": "For emulate: user agent string."
            }),
        );
        props.insert(
            "colorScheme".to_string(),
            serde_json::json!({
                "type": "string", "enum": ["dark", "light", "auto"],
                "description": "For emulate: dark/light mode."
            }),
        );
        props.insert(
            "mappings".to_string(),
            serde_json::json!({
                "type": "array",
                "description": "For set_hosts: [{pattern, target_ip}] host mappings.",
                "items": {
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string" },
                        "target_ip": { "type": "string" }
                    }
                }
            }),
        );
        props.insert(
            "temporary".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "For set_hosts: session-only mapping (default true when agent-initiated)."
            }),
        );
        props.insert(
            "temporary_only".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "For clear_hosts: only remove session mappings."
            }),
        );
        props.insert(
            "reason".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Agent-provided reason shown in user confirmation panel."
            }),
        );
        props.insert(
            "require_confirm".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "Require user confirmation (default true for agent set_hosts/set_proxy)."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["action".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::err(format!(
                    "browser: invalid JSON: {e}. Example: {{\"action\": \"navigate\", \"type\": \"url\", \"url\": \"https://example.com\"}}"
                ))
            }
        };

        let action = match args.get("action").and_then(|v| v.as_str()) {
            Some(a) => a.to_string(),
            None => {
                return ToolResult::err(
                    "browser: missing required 'action' field. \
                     Start with take_snapshot to see the page, then use uid-based actions."
                        .to_string(),
                )
            }
        };

        if let Err(e) = actions::validate_args(&action, &args) {
            return ToolResult::err(e);
        }

        if matches!(
            action.as_str(),
            "set_hosts" | "set_proxy" | "get_network_config" | "clear_hosts"
        ) {
            let result = tokio::time::timeout(
                ACTION_TIMEOUT,
                network::execute_network_action(&action, &args),
            )
            .await;
            return match result {
                Ok(Ok(ar)) => ToolResult::ok(ar.text),
                Ok(Err(e)) => ToolResult::err(e),
                Err(_) => ToolResult::err(
                    "browser network: action timed out (60s). User may not have responded to confirmation."
                        .to_string(),
                ),
            };
        }

        let engine = self.engine.clone();
        let result = tokio::time::timeout(
            ACTION_TIMEOUT,
            async move { engine.execute_action(&action, &args).await },
        )
        .await;

        match result {
            Ok(Ok(ar)) => {
                if ar.images.is_empty() {
                    ToolResult::ok(ar.text)
                } else {
                    ToolResult::ok_with_images(ar.text, ar.images)
                }
            }
            Ok(Err(e)) => ToolResult::err(e),
            Err(_) => ToolResult::err(
                "browser: action timed out (60s). The page may be unresponsive.".to_string(),
            ),
        }
    }
}

/// Register browser tool with environment-appropriate engine (CDP or WebView).
pub fn register_browser_tool(registry: &ToolRegistry) {
    let engine = default_engine();
    tracing::info!(engine = engine.engine_type(), "registering browser tool");
    registry.register_deferred(Arc::new(BrowserTool::with_engine(engine)));
}

/// Register browser tool with an explicit engine implementation.
pub fn register_browser_tool_with_engine(
    registry: &ToolRegistry,
    engine: Arc<dyn BrowserEngine>,
) {
    tracing::info!(engine = engine.engine_type(), "registering browser tool");
    registry.register_deferred(Arc::new(BrowserTool::with_engine(engine)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaolin_core::tool::Tool;

    #[test]
    fn browser_tool_metadata() {
        let tool = BrowserTool::new();
        assert_eq!(tool.name(), "browser");
        assert!(!tool.description().is_empty());
        let schema = tool.parameters_schema();
        assert_eq!(schema.schema_type, "object");
        assert!(schema.properties.contains_key("action"));
        assert!(schema.properties.contains_key("url"));
        assert!(schema.properties.contains_key("uid"));
        assert!(schema.properties.contains_key("selector"));
        assert!(schema.properties.contains_key("script"));
        assert!(schema.required.contains(&"action".to_string()));
    }

    #[test]
    fn parse_timeout_defaults() {
        let args = serde_json::json!({});
        assert_eq!(BrowserTool::parse_timeout(&args), actions::DEFAULT_ELEMENT_TIMEOUT);
    }

    #[test]
    fn parse_timeout_custom() {
        let args = serde_json::json!({"timeout": 5000});
        assert_eq!(
            BrowserTool::parse_timeout(&args),
            Duration::from_millis(5000)
        );
    }

    #[test]
    fn require_selector_present() {
        let args = serde_json::json!({"selector": "#main"});
        assert_eq!(
            BrowserTool::require_selector(&args, "click").unwrap(),
            "#main"
        );
    }

    #[test]
    fn require_selector_missing() {
        let args = serde_json::json!({});
        let err = BrowserTool::require_selector(&args, "click").unwrap_err();
        assert!(err.contains("missing"));
        assert!(err.contains("selector"));
    }

    #[test]
    fn browser_description_mentions_snapshot() {
        let tool = BrowserTool::new();
        let desc = tool.description();
        assert!(desc.contains("take_snapshot"));
        assert!(desc.contains("uid"));
        assert!(desc.contains("screenshot"));
    }

    #[test]
    fn browser_schema_has_new_actions() {
        let tool = BrowserTool::new();
        let schema = tool.parameters_schema();
        let action_prop = &schema.properties["action"];
        let enum_vals = action_prop["enum"].as_array().unwrap();
        let actions_list: Vec<&str> = enum_vals.iter().map(|v| v.as_str().unwrap()).collect();
        for a in [
            "navigate", "take_snapshot", "screenshot", "evaluate", "click", "fill",
            "fill_form", "type_text", "press_key", "hover", "select", "wait_for",
            "scroll", "drag", "handle_dialog", "interact", "get_content", "pdf",
            "list_pages", "select_page", "new_page", "close_page", "cookies",
            "list_network_requests", "list_console_messages", "get_console_message",
            "get_network_request", "upload_file", "emulate", "resize_page",
        ] {
            assert!(actions_list.contains(&a), "enum missing action: {a}");
        }
    }

    #[test]
    fn is_headless_env_var() {
        std::env::remove_var("XIAOLIN_BROWSER_HEADLESS");
        assert!(!BrowserTool::is_headless());
        std::env::set_var("XIAOLIN_BROWSER_HEADLESS", "true");
        assert!(BrowserTool::is_headless());
        std::env::set_var("XIAOLIN_BROWSER_HEADLESS", "1");
        assert!(BrowserTool::is_headless());
        std::env::set_var("XIAOLIN_BROWSER_HEADLESS", "false");
        assert!(!BrowserTool::is_headless());
        std::env::remove_var("XIAOLIN_BROWSER_HEADLESS");
    }

    #[tokio::test]
    async fn browser_tool_rejects_missing_action() {
        let tool = BrowserTool::with_engine(Arc::new(CdpEngine::new()));
        let result = tool.execute(r#"{"url":"https://example.com"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("missing"));
    }

    #[tokio::test]
    async fn browser_tool_rejects_unknown_action() {
        let tool = BrowserTool::with_engine(Arc::new(CdpEngine::new()));
        let result = tool.execute(r#"{"action":"destroy"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("unknown action"));
    }

    #[tokio::test]
    async fn browser_tool_rejects_bad_json() {
        let tool = BrowserTool::with_engine(Arc::new(CdpEngine::new()));
        let result = tool.execute("not json").await;
        assert!(!result.success);
        assert!(result.output.contains("invalid JSON"));
    }

    #[tokio::test]
    async fn browser_navigate_missing_url_for_url_type() {
        let tool = BrowserTool::with_engine(Arc::new(CdpEngine::new()));
        let result = tool.execute(r#"{"action":"navigate","type":"url"}"#).await;
        assert!(!result.success || result.output.contains("missing"));
    }

    #[tokio::test]
    async fn browser_evaluate_missing_script() {
        let tool = BrowserTool::with_engine(Arc::new(CdpEngine::new()));
        let result = tool.execute(r#"{"action":"evaluate"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("script") || result.output.contains("function"));
    }

    #[tokio::test]
    async fn browser_click_missing_uid_and_selector() {
        let tool = BrowserTool::with_engine(Arc::new(CdpEngine::new()));
        let result = tool.execute(r#"{"action":"click"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("uid") || result.output.contains("selector"));
    }

    #[tokio::test]
    async fn browser_fill_missing_value() {
        let tool = BrowserTool::with_engine(Arc::new(CdpEngine::new()));
        let result = tool.execute(r#"{"action":"fill","uid":"e0"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("value"));
    }

    #[tokio::test]
    async fn browser_type_text_missing_text() {
        let tool = BrowserTool::with_engine(Arc::new(CdpEngine::new()));
        let result = tool.execute(r#"{"action":"type_text"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("text"));
    }

    #[tokio::test]
    async fn browser_press_key_missing_key() {
        let tool = BrowserTool::with_engine(Arc::new(CdpEngine::new()));
        let result = tool.execute(r#"{"action":"press_key"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("key"));
    }

    #[tokio::test]
    async fn browser_new_page_missing_url() {
        let tool = BrowserTool::with_engine(Arc::new(CdpEngine::new()));
        let result = tool.execute(r#"{"action":"new_page"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("url"));
    }

    #[tokio::test]
    async fn browser_pdf_missing_output_path() {
        let tool = BrowserTool::with_engine(Arc::new(CdpEngine::new()));
        let result = tool.execute(r#"{"action":"pdf"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("output_path"));
    }

    #[test]
    fn browser_validate_passes_simple_actions() {
        let args = serde_json::json!({});
        assert!(BrowserTool::validate_args("take_snapshot", &args).is_ok());
        assert!(BrowserTool::validate_args("list_pages", &args).is_ok());
        assert!(BrowserTool::validate_args("screenshot", &args).is_ok());
        assert!(BrowserTool::validate_args("scroll", &args).is_ok());
        assert!(BrowserTool::validate_args("interact", &args).is_ok());
        assert!(BrowserTool::validate_args("get_content", &args).is_ok());
        assert!(BrowserTool::validate_args("list_network_requests", &args).is_ok());
        assert!(BrowserTool::validate_args("list_console_messages", &args).is_ok());
        assert!(BrowserTool::validate_args("emulate", &args).is_ok());
    }

    #[test]
    fn register_browser_tool_adds_to_registry() {
        let registry = ToolRegistry::new();
        register_browser_tool(&registry);
        assert!(registry.get("browser").is_some());
    }

    #[test]
    fn validate_url_scheme_allows_http_https() {
        assert!(BrowserTool::validate_url_scheme("https://example.com").is_ok());
        assert!(BrowserTool::validate_url_scheme("http://example.com/path").is_ok());
        assert!(BrowserTool::validate_url_scheme("  HTTPS://Example.COM  ").is_ok());
    }

    #[test]
    fn validate_url_scheme_rejects_dangerous_schemes() {
        for url in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "ftp://example.com",
            "/etc/passwd",
        ] {
            let err = BrowserTool::validate_url_scheme(url).unwrap_err();
            assert!(
                err.contains("scheme not allowed") || err.contains("http://"),
                "unexpected error for {url}: {err}"
            );
        }
    }

    #[test]
    fn validate_output_path_rejects_outside_workspace() {
        let err = BrowserTool::validate_output_path("/etc/cron.d/evil").unwrap_err();
        assert!(err.contains("outside the workspace"));
    }

    #[test]
    fn validate_output_path_allows_workspace_relative() {
        let root = BrowserTool::workspace_root_for_paths();
        let rel = root.join("browser-out/test.png");
        let rel_str = rel
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let validated = BrowserTool::validate_output_path(&rel_str).unwrap();
        assert!(validated.starts_with(root.canonicalize().unwrap_or(root)));
    }

    #[test]
    fn validate_output_path_allows_temp_dir() {
        let tmp = std::env::temp_dir().join("xiaolin-browser-test-out.pdf");
        let validated = BrowserTool::validate_output_path(tmp.to_str().unwrap()).unwrap();
        assert!(validated.starts_with(
            std::env::temp_dir()
                .canonicalize()
                .unwrap_or_else(|_| std::env::temp_dir())
        ));
    }

    #[test]
    fn default_engine_is_cdp_without_webview_env() {
        std::env::remove_var("XIAOLIN_BROWSER_ENGINE");
        let tool = BrowserTool::new();
        assert_eq!(tool.engine_type(), "cdp");
    }

    #[tokio::test]
    #[ignore]
    async fn browser_smoke_navigate() {
        let tool = BrowserTool::with_engine(Arc::new(CdpEngine::new()));
        let result = tool
            .execute(r#"{"action":"navigate","type":"url","url":"https://example.com"}"#)
            .await;
        assert!(result.success, "navigate failed: {}", result.output);
    }

    #[tokio::test]
    #[ignore]
    async fn browser_smoke_take_snapshot() {
        let tool = BrowserTool::with_engine(Arc::new(CdpEngine::new()));
        let _ = tool
            .execute(r#"{"action":"navigate","type":"url","url":"https://example.com"}"#)
            .await;
        let result = tool.execute(r#"{"action":"take_snapshot"}"#).await;
        assert!(result.success);
        assert!(result.output.contains("e"));
    }
}
