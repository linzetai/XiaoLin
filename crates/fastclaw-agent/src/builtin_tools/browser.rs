use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use base64::Engine as _;
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolRegistry, ToolResult};

use super::network::{strip_html_tags, truncate_text};

struct BrowserState {
    browser: headless_chrome::Browser,
    /// Persistent tab reused across actions to preserve session/cookies visually.
    /// Created on first use; recreated if the tab becomes invalid.
    persistent_tab: Option<Arc<headless_chrome::Tab>>,
}

/// Browser tool using Chrome DevTools Protocol.
/// Launches a **visible** Chrome window by default so the user can interact
/// (log in, solve CAPTCHAs, etc.).  Set `FASTCLAW_BROWSER_HEADLESS=true` to
/// revert to headless mode for CI/server environments.
///
/// A single persistent tab is reused across calls—session cookies, localStorage,
/// and login state carry over automatically.
pub struct BrowserTool {
    inner: Arc<Mutex<Option<BrowserState>>>,
}

impl BrowserTool {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
        }
    }

    fn is_headless() -> bool {
        std::env::var("FASTCLAW_BROWSER_HEADLESS")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    }

    fn profile_dir() -> std::path::PathBuf {
        if let Ok(dir) = std::env::var("FASTCLAW_BROWSER_PROFILE") {
            return std::path::PathBuf::from(dir);
        }
        let base = dirs::data_local_dir()
            .unwrap_or_else(|| std::env::temp_dir().join("fastclaw"));
        base.join("fastclaw").join("browser-profile")
    }

    fn ensure_browser(inner: &Mutex<Option<BrowserState>>) -> Result<(), String> {
        let mut guard = inner.lock().map_err(|e| {
            format!(
                "browser: could not lock the shared Chrome handle (poisoned or contended mutex): {e}. \
                 What to do next: retry once; if this repeats, the gateway process may need restart—report to the operator."
            )
        })?;
        if guard.is_none() {
            let headless = Self::is_headless();
            let profile = Self::profile_dir();
            std::fs::create_dir_all(&profile).ok();
            let launch_options = headless_chrome::LaunchOptions::default_builder()
                .headless(headless)
                .sandbox(false)
                .window_size(Some((1280, 900)))
                .user_data_dir(Some(profile))
                .build()
                .map_err(|e| {
                    format!(
                        "browser: invalid Chrome launch options: {e}. \
                         What to do next: check headless_chrome defaults and OS limits; ask the operator if custom flags are required."
                    )
                })?;
            let browser = headless_chrome::Browser::new(launch_options).map_err(|e| {
                format!(
                    "browser: could not start Chrome/Chromium: {e}. \
                     What to do next: ensure google-chrome or chromium is installed and on PATH, the gateway user may launch browsers, and no sandbox policy blocks it; see operator docs for FASTCLAW_BROWSER dependencies."
                )
            })?;
            *guard = Some(BrowserState {
                browser,
                persistent_tab: None,
            });
        }
        Ok(())
    }

    /// Returns the persistent tab, creating one if needed or if the old one died.
    fn get_or_create_tab(state: &mut BrowserState) -> Result<Arc<headless_chrome::Tab>, String> {
        let tab_alive = state
            .persistent_tab
            .as_ref()
            .and_then(|t| t.get_title().ok())
            .is_some();

        if tab_alive {
            return Ok(state.persistent_tab.as_ref().unwrap().clone());
        }

        let tab = state.browser.new_tab().map_err(|e| {
            format!(
                "browser: could not open a new tab: {e}. \
                 What to do next: retry; if Chrome is unstable, restart the gateway browser pool."
            )
        })?;
        state.persistent_tab = Some(tab.clone());
        Ok(tab)
    }

    fn run_action(
        inner: &Mutex<Option<BrowserState>>,
        action: &str,
        args: &serde_json::Value,
    ) -> Result<String, String> {
        use headless_chrome::protocol::cdp::Page;

        let mut guard = inner.lock().map_err(|e| {
            format!(
                "browser: mutex lock failed while running action '{action}': {e}. \
                 What to do next: retry; if poisoned, restart the gateway worker."
            )
        })?;
        let state = guard.as_mut().ok_or_else(|| {
            "browser: internal state has no Chrome instance after ensure_browser—this should not happen. \
             What to do next: retry the tool once; if it persists, restart the gateway and report a bug."
                .to_string()
        })?;

        match action {
            "navigate" => {
                let url = args
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        "browser navigate: missing string field 'url'. \
                         Example: {\"action\": \"navigate\", \"url\": \"https://example.com\"}."
                            .to_string()
                    })?;
                let tab = Self::get_or_create_tab(state)?;
                tab.navigate_to(url).map_err(|e| {
                    format!(
                        "browser navigate: navigation to '{url}' failed: {e}. \
                         What to do next: verify the URL scheme/host, TLS trust, and network reachability from the gateway host."
                    )
                })?;
                tab.wait_until_navigated().map_err(|e| {
                    format!(
                        "browser navigate: timed out or failed waiting for '{url}' to finish loading: {e}. \
                         What to do next: retry, try a simpler page, or increase wait at the operator level if pages are legitimately slow."
                    )
                })?;

                let title = tab.get_title().unwrap_or_default();
                let text = tab.get_content().map_err(|e| {
                    format!(
                        "browser navigate: could not read DOM HTML for '{url}': {e}. \
                         What to do next: retry; if the site is SPA-only, prefer evaluate with a script that waits for selectors."
                    )
                })?;
                let cleaned = strip_html_tags(&text);
                let truncated = truncate_text(&cleaned, 16_384);

                Ok(serde_json::json!({
                    "url": url,
                    "title": title,
                    "content": truncated,
                    "content_length": truncated.len(),
                }).to_string())
            }
            "screenshot" => {
                let url = args.get("url").and_then(|v| v.as_str());
                let save_to_disk = args.get("output_path").is_some();
                let output_path = args
                    .get("output_path")
                    .and_then(|v| v.as_str())
                    .map(String::from);

                let tab = Self::get_or_create_tab(state)?;

                if let Some(u) = url {
                    tab.navigate_to(u).map_err(|e| {
                        format!(
                            "browser screenshot: navigation to '{u}' failed: {e}. \
                             What to do next: fix URL or network, then retry."
                        )
                    })?;
                    tab.wait_until_navigated().map_err(|e| {
                        format!(
                            "browser screenshot: wait for '{u}' failed: {e}. \
                             What to do next: retry with a lighter page or after the site recovers."
                        )
                    })?;
                }

                let png = tab
                    .capture_screenshot(
                        Page::CaptureScreenshotFormatOption::Png,
                        None,
                        None,
                        true,
                    )
                    .map_err(|e| {
                        format!(
                            "browser screenshot: capture_screenshot failed: {e}. \
                             What to do next: confirm the page finished painting; some sites block automation—try evaluate or web_fetch instead."
                        )
                    })?;

                if let Some(ref path) = output_path {
                    if save_to_disk {
                        std::fs::write(path, &png).map_err(|e| {
                            format!(
                                "browser screenshot: could not write PNG to '{path}': {e}. \
                                 What to do next: pick a writable directory or omit output_path to display inline."
                            )
                        })?;
                    }
                }

                let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
                let current_url = tab.get_url();
                let title = tab.get_title().unwrap_or_default();

                let mut parts = Vec::new();
                parts.push(format!("url: {current_url}"));
                parts.push(format!("title: {title}"));
                parts.push(format!("bytes: {}", png.len()));
                if let Some(ref path) = output_path {
                    parts.push(format!("saved: {path}"));
                }
                parts.push(format!("![image](data:image/png;base64,{b64})"));

                Ok(parts.join("\n"))
            }
            "evaluate" => {
                let url = args.get("url").and_then(|v| v.as_str());
                let script = args
                    .get("script")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        "browser evaluate: missing string field 'script'. \
                         Example: {\"action\": \"evaluate\", \"url\": \"https://example.com\", \"script\": \"document.title\"}. url is optional—omit to run on the current page."
                            .to_string()
                    })?;

                let tab = Self::get_or_create_tab(state)?;
                if let Some(u) = url {
                    tab.navigate_to(u).map_err(|e| {
                        format!(
                            "browser evaluate: navigation to '{u}' failed: {e}. \
                             What to do next: fix URL or try without url to run on the current page."
                        )
                    })?;
                    tab.wait_until_navigated().map_err(|e| {
                        format!(
                            "browser evaluate: wait for '{u}' failed: {e}. \
                             What to do next: retry or simplify the page load."
                        )
                    })?;
                }

                let result = tab.evaluate(script, false).map_err(|e| {
                    format!(
                        "browser evaluate: JavaScript evaluation failed: {e}. \
                         What to do next: fix syntax/runtime errors in script, ensure prior navigation finished when url was set, and avoid long-running dialogs."
                    )
                })?;

                Ok(serde_json::json!({
                    "result": format!("{:?}", result.value),
                }).to_string())
            }
            "interact" => {
                let url = args.get("url").and_then(|v| v.as_str());
                let wait_seconds = args
                    .get("wait_seconds")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(60);

                if Self::is_headless() {
                    return Err(
                        "browser interact: this action requires a visible browser window. \
                         Set FASTCLAW_BROWSER_HEADLESS to false (or unset it) and restart the gateway."
                            .to_string(),
                    );
                }

                let tab = Self::get_or_create_tab(state)?;

                if let Some(u) = url {
                    tab.navigate_to(u).map_err(|e| {
                        format!(
                            "browser interact: navigation to '{u}' failed: {e}. \
                             What to do next: fix URL or network, then retry."
                        )
                    })?;
                    tab.wait_until_navigated().map_err(|e| {
                        format!(
                            "browser interact: wait for '{u}' failed: {e}. \
                             What to do next: retry or simplify the page load."
                        )
                    })?;
                }

                let started_url = tab.get_url();

                let poll_interval = std::time::Duration::from_secs(2);
                let deadline =
                    std::time::Instant::now() + std::time::Duration::from_secs(wait_seconds);

                while std::time::Instant::now() < deadline {
                    std::thread::sleep(poll_interval);
                    let current_url = tab.get_url();
                    if !started_url.is_empty() && current_url != started_url {
                        break;
                    }
                }

                let final_url = tab.get_url();
                let title = tab.get_title().unwrap_or_default();
                let text = tab.get_content().unwrap_or_default();
                let cleaned = strip_html_tags(&text);
                let truncated = truncate_text(&cleaned, 16_384);

                Ok(serde_json::json!({
                    "started_url": started_url.clone(),
                    "final_url": final_url.clone(),
                    "title": title,
                    "content": truncated,
                    "url_changed": started_url != final_url,
                }).to_string())
            }
            "get_content" => {
                let tab = Self::get_or_create_tab(state)?;
                let current_url = tab.get_url();
                let title = tab.get_title().unwrap_or_default();
                let text = tab.get_content().unwrap_or_default();
                let cleaned = strip_html_tags(&text);
                let truncated = truncate_text(&cleaned, 16_384);

                Ok(serde_json::json!({
                    "url": current_url,
                    "title": title,
                    "content": truncated,
                    "content_length": truncated.len(),
                }).to_string())
            }
            other => Err(format!(
                "browser: unknown action '{other}'. \
                 Use exactly 'navigate', 'screenshot', 'evaluate', 'interact', or 'get_content' (see tool schema), then retry with the required fields for that action."
            )),
        }
    }
}

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &str {
        "browser"
    }

    fn description(&self) -> &str {
        "Control a visible Chrome window via CDP—supports login, CAPTCHAs, and interactive flows. \
         The browser launches **non-headless** by default (set FASTCLAW_BROWSER_HEADLESS=true for CI). \
         A single persistent tab is reused across calls so cookies and login state carry over. \
         Actions: \
         navigate + url → {url, title, content, content_length}; content is tag-stripped text ≤16KiB. \
         screenshot (optional url, optional output_path) → PNG of the current or navigated page. \
         evaluate + script (optional url) → run JS on the current page; wrap with JSON.stringify for clean output. \
         interact (optional url, optional wait_seconds default 60) → open a page for the user to interact with (login, CAPTCHA); polls until the URL changes or timeout, then returns page content. \
         get_content → read the current page's URL, title, and text without navigating. \
         Prefer web_fetch for static HTML/APIs; use browser for JS-heavy pages, login-gated content, or visual tasks. \
         Example: {\"action\": \"interact\", \"url\": \"https://accounts.google.com\", \"wait_seconds\": 120}."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "action".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["navigate", "screenshot", "evaluate", "interact", "get_content"],
                "description": "navigate: load url, return text. screenshot: PNG of current/navigated page. evaluate: run JS. interact: open page for user interaction (login etc.), wait for URL change or timeout. get_content: read current page without navigating."
            }),
        );
        props.insert(
            "url".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "HTTP(S) page to load. Required for navigate. Optional for screenshot (screenshots current page if omitted), evaluate, and interact."
            }),
        );
        props.insert(
            "script".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "JavaScript expression for evaluate action only. Returned value is Debug-formatted—use JSON.stringify in script for clean text."
            }),
        );
        props.insert(
            "output_path".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Filesystem path for screenshot PNG. Defaults to OS temp directory when omitted."
            }),
        );
        props.insert(
            "wait_seconds".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "For interact action: max seconds to wait for user interaction (default 60). The tool returns early if the page URL changes (e.g. after login redirect)."
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
                    "browser: arguments are not valid JSON: {e}. \
                     Pass e.g. {{\"action\": \"navigate\", \"url\": \"https://example.com\"}} with double-quoted keys, then retry."
                ))
            }
        };

        let action = match args.get("action").and_then(|v| v.as_str()) {
            Some(a) => a.to_string(),
            None => {
                return ToolResult::err(
                    "browser is missing required string field 'action'. \
                     Example: {\"action\": \"interact\", \"url\": \"https://accounts.google.com\"}."
                        .to_string(),
                )
            }
        };

        let inner = self.inner.clone();

        let result = tokio::task::spawn_blocking(move || {
            Self::ensure_browser(&inner)?;
            Self::run_action(&inner, &action, &args)
        })
        .await;

        match result {
            Ok(Ok(v)) => ToolResult::ok(v),
            Ok(Err(e)) => ToolResult::err(e),
            Err(e) => ToolResult::err(format!(
                "browser: the blocking worker task panicked or failed to join: {e}. \
                 What went wrong: spawn_blocking did not return a normal tool result (worker crash or runtime shutdown). \
                 What to do next: retry once with a smaller action; if it repeats, restart the gateway browser worker and report the panic to the operator."
            )),
        }
    }
}

pub fn register_browser_tool(registry: &ToolRegistry) {
    registry.register(Arc::new(BrowserTool::new()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastclaw_core::tool::Tool;

    #[test]
    fn browser_tool_metadata() {
        let tool = BrowserTool::new();
        assert_eq!(tool.name(), "browser");
        assert!(!tool.description().is_empty());
        let schema = tool.parameters_schema();
        assert_eq!(schema.schema_type, "object");
        assert!(schema.properties.contains_key("action"));
        assert!(schema.properties.contains_key("url"));
        assert!(schema.properties.contains_key("script"));
        assert!(schema.properties.contains_key("output_path"));
        assert!(schema.properties.contains_key("wait_seconds"));
        assert!(schema.required.contains(&"action".to_string()));
    }

    #[test]
    fn browser_description_mentions_visible() {
        let tool = BrowserTool::new();
        let desc = tool.description();
        assert!(desc.contains("visible"));
        assert!(desc.contains("interact"));
        assert!(desc.contains("login"));
    }

    #[test]
    fn browser_schema_has_all_actions() {
        let tool = BrowserTool::new();
        let schema = tool.parameters_schema();
        let action_prop = &schema.properties["action"];
        let enum_vals = action_prop["enum"].as_array().unwrap();
        let actions: Vec<&str> = enum_vals.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(actions.contains(&"navigate"));
        assert!(actions.contains(&"screenshot"));
        assert!(actions.contains(&"evaluate"));
        assert!(actions.contains(&"interact"));
        assert!(actions.contains(&"get_content"));
    }

    #[test]
    fn is_headless_defaults_to_false() {
        std::env::remove_var("FASTCLAW_BROWSER_HEADLESS");
        assert!(!BrowserTool::is_headless());
    }

    #[test]
    fn is_headless_respects_env() {
        std::env::set_var("FASTCLAW_BROWSER_HEADLESS", "true");
        assert!(BrowserTool::is_headless());
        std::env::set_var("FASTCLAW_BROWSER_HEADLESS", "1");
        assert!(BrowserTool::is_headless());
        std::env::set_var("FASTCLAW_BROWSER_HEADLESS", "false");
        assert!(!BrowserTool::is_headless());
        std::env::remove_var("FASTCLAW_BROWSER_HEADLESS");
    }

    #[tokio::test]
    async fn browser_tool_rejects_missing_action() {
        let tool = BrowserTool::new();
        let result = tool.execute(r#"{"url":"https://example.com"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("missing"));
    }

    #[tokio::test]
    async fn browser_tool_rejects_unknown_action() {
        let tool = BrowserTool::new();
        let result = tool.execute(r#"{"action":"destroy"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("unknown action"));
    }

    #[tokio::test]
    async fn browser_tool_rejects_bad_json() {
        let tool = BrowserTool::new();
        let result = tool.execute("not json").await;
        assert!(!result.success);
        assert!(result.output.contains("not valid JSON"));
    }

    #[tokio::test]
    async fn browser_navigate_missing_url() {
        let tool = BrowserTool::new();
        let result = tool.execute(r#"{"action":"navigate"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("missing"));
    }

    #[tokio::test]
    async fn browser_evaluate_missing_script() {
        let tool = BrowserTool::new();
        let result = tool
            .execute(r#"{"action":"evaluate","url":"https://example.com"}"#)
            .await;
        assert!(!result.success);
        assert!(result.output.contains("missing"));
    }

    #[test]
    fn register_browser_tool_adds_to_registry() {
        let registry = ToolRegistry::new();
        register_browser_tool(&registry);
        assert!(registry.get("browser").is_some());
    }
}
