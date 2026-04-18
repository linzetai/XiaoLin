use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolRegistry, ToolResult};

use super::network::{strip_html_tags, truncate_text};

/// Headless browser tool using Chrome DevTools Protocol.
/// Lazily launches Chrome on first use; reuses the same instance across calls.
pub struct BrowserTool {
    inner: Arc<Mutex<Option<headless_chrome::Browser>>>,
}

impl BrowserTool {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
        }
    }

    fn ensure_browser(inner: &Mutex<Option<headless_chrome::Browser>>) -> Result<(), String> {
        let mut guard = inner.lock().map_err(|e| {
            format!(
                "browser: could not lock the shared Chrome handle (poisoned or contended mutex): {e}. \
                 What to do next: retry once; if this repeats, the gateway process may need restart—report to the operator."
            )
        })?;
        if guard.is_none() {
            let launch_options = headless_chrome::LaunchOptions::default_builder()
                .headless(true)
                .sandbox(false)
                .build()
                .map_err(|e| {
                    format!(
                        "browser: invalid Chrome launch options: {e}. \
                         What to do next: check headless_chrome defaults and OS limits; ask the operator if custom flags are required."
                    )
                })?;
            let b = headless_chrome::Browser::new(launch_options).map_err(|e| {
                format!(
                    "browser: could not start headless Chrome/Chromium: {e}. \
                     What to do next: ensure google-chrome or chromium is installed and on PATH, the gateway user may launch browsers, and no sandbox policy blocks it; see operator docs for FASTCLAW_BROWSER dependencies."
                )
            })?;
            *guard = Some(b);
        }
        Ok(())
    }

    fn run_action(
        inner: &Mutex<Option<headless_chrome::Browser>>,
        action: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        use headless_chrome::protocol::cdp::Page;

        let guard = inner.lock().map_err(|e| {
            format!(
                "browser: mutex lock failed while running action '{action}': {e}. \
                 What to do next: retry; if poisoned, restart the gateway worker."
            )
        })?;
        let browser = guard.as_ref().ok_or_else(|| {
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
                let tab = browser.new_tab().map_err(|e| {
                    format!(
                        "browser navigate: could not open a new tab: {e}. \
                         What to do next: retry; if Chrome is unstable, restart the gateway browser pool."
                    )
                })?;
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
                let text = tab
                    .get_content()
                    .map_err(|e| {
                        format!(
                            "browser navigate: could not read DOM HTML for '{url}': {e}. \
                             What to do next: retry; if the site is SPA-only, prefer evaluate with a script that waits for selectors."
                        )
                    })?;
                    let cleaned = strip_html_tags(&text);
                    let truncated = truncate_text(&cleaned, 16_384);

                let _ = tab.close(true);
                Ok(serde_json::json!({
                    "url": url,
                    "title": title,
                    "content": truncated,
                    "content_length": truncated.len(),
                }))
            }
            "screenshot" => {
                let url = args
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        "browser screenshot: missing string field 'url'. \
                         Example: {\"action\": \"screenshot\", \"url\": \"https://example.com\", \"output_path\": \"/tmp/page.png\"}."
                            .to_string()
                    })?;
                let output_path = args
                    .get("output_path")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| {
                        let ts = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis())
                            .unwrap_or(0);
                        format!("/tmp/fastclaw_screenshot_{ts}.png")
                    });

                let tab = browser.new_tab().map_err(|e| {
                    format!(
                        "browser screenshot: could not open a new tab: {e}. \
                         What to do next: same as navigate—retry or restart gateway if Chrome wedged."
                    )
                })?;
                tab.navigate_to(url).map_err(|e| {
                    format!(
                        "browser screenshot: navigation to '{url}' failed: {e}. \
                         What to do next: fix URL or network, then retry."
                    )
                })?;
                tab.wait_until_navigated().map_err(|e| {
                    format!(
                        "browser screenshot: wait for '{url}' failed: {e}. \
                         What to do next: retry with a lighter page or after the site recovers."
                    )
                })?;

                let png = tab
                    .capture_screenshot(
                        Page::CaptureScreenshotFormatOption::Png,
                        None,
                        None,
                        true,
                    )
                    .map_err(|e| {
                        format!(
                            "browser screenshot: capture_screenshot failed for '{url}': {e}. \
                             What to do next: confirm the page finished painting; some sites block automation—try evaluate or web_fetch instead."
                        )
                    })?;

                std::fs::write(&output_path, &png).map_err(|e| {
                    format!(
                        "browser screenshot: could not write PNG to '{output_path}': {e}. \
                         What to do next: pick a writable directory (often /tmp), create parents, or omit output_path to use the default under /tmp."
                    )
                })?;

                let _ = tab.close(true);
                Ok(serde_json::json!({
                    "path": output_path,
                    "bytes": png.len(),
                }))
            }
            "evaluate" => {
                let url = args.get("url").and_then(|v| v.as_str());
                let script = args
                    .get("script")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        "browser evaluate: missing string field 'script'. \
                         Example: {\"action\": \"evaluate\", \"url\": \"https://example.com\", \"script\": \"document.title\"}. url is optional—omit to run in about:blank."
                            .to_string()
                    })?;

                let tab = browser.new_tab().map_err(|e| {
                    format!(
                        "browser evaluate: could not open a new tab: {e}. \
                         What to do next: retry or restart gateway if Chrome is wedged."
                    )
                })?;
                if let Some(u) = url {
                    tab.navigate_to(u).map_err(|e| {
                        format!(
                            "browser evaluate: navigation to '{u}' failed: {e}. \
                             What to do next: fix URL or try without url for pure JS experiments."
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

                let _ = tab.close(true);
                Ok(serde_json::json!({
                    "result": format!("{:?}", result.value),
                }))
            }
            other => Err(format!(
                "browser: unknown action '{other}'. \
                 Use exactly 'navigate', 'screenshot', or 'evaluate' (see tool schema), then retry with the required fields for that action."
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
        "Control headless Chrome/Chromium via CDP: navigate pages, extract tag-stripped text, take PNG screenshots, or run JavaScript and return Debug-formatted values. \
         The gateway lazily starts one shared browser—first call can be slow; later calls reuse it until restart. \
         navigate + url → JSON {url, title, content, content_length}; content is cleaned text truncated near 16KiB—use when JS-rendered DOM matters and web_fetch would miss it. \
         screenshot + url (optional output_path) saves a PNG—good for UI proof or visual checks, not generic binary downloads. \
         evaluate needs script; url optional (skip navigation for about:blank experiments). Results are Rust Debug strings—wrap with JSON.stringify in script when you need clean text. \
         Prefer web_fetch/http_fetch for static HTML or APIs; use browser for JS-heavy pages or pixels. \
         Respect the same automation/SSRF policies as other network tools—only approved URLs. \
         Anti-pattern: huge scraping loops inside one evaluate—batch with smaller scripts or other tools. \
         Example: {\"action\": \"navigate\", \"url\": \"https://example.com\"}."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "action".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["navigate", "screenshot", "evaluate"],
                "description": "One of: navigate (load url, return stripped text), screenshot (png of rendered page), evaluate (run script, optional url first). Required on every call."
            }),
        );
        props.insert(
            "url".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "HTTP(S) page to load. Required for navigate and screenshot. Optional for evaluate—omit only when your script does not need a loaded document."
            }),
        );
        props.insert(
            "script".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "JavaScript expression or snippet for evaluate only. Example: 'document.querySelector(\"h1\")?.textContent'. Returned value is formatted with Debug—design scripts to return JSON.stringify(...) when you need stable text."
            }),
        );
        props.insert("output_path".to_string(), serde_json::json!({
            "type": "string",
            "description": "Filesystem path for screenshot PNG. Example: '/tmp/login.png'. Defaults to /tmp/fastclaw_screenshot_<timestamp>.png when omitted. Parent dirs should exist or be creatable."
        }));
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["action".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "browser: arguments are not valid JSON: {e}. \
                 Pass e.g. {{\"action\": \"navigate\", \"url\": \"https://example.com\"}} with double-quoted keys, then retry."
            )),
        };

        let action = match args.get("action").and_then(|v| v.as_str()) {
            Some(a) => a.to_string(),
            None => return ToolResult::err(
                "browser is missing required string field 'action'. \
                 Example: {\"action\": \"screenshot\", \"url\": \"https://example.com\"}."
                    .to_string(),
            ),
        };

        let inner = self.inner.clone();

        let result = tokio::task::spawn_blocking(move || {
            Self::ensure_browser(&inner)?;
            Self::run_action(&inner, &action, &args)
        })
        .await;

        match result {
            Ok(Ok(v)) => ToolResult::ok(v.to_string()),
            Ok(Err(e)) => ToolResult::err(e),
            Err(e) => ToolResult::err(format!(
                "browser: the blocking worker task panicked or failed to join: {e}. \
                 What went wrong: spawn_blocking did not return a normal tool result (worker crash or runtime shutdown). \
                 What to do next: retry once with a smaller action; if it repeats, restart the gateway browser worker and report the panic to the operator."
            )),
        }
    }
}

pub fn register_browser_tool(registry: &mut ToolRegistry) {
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
        assert!(schema.required.contains(&"action".to_string()));
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
    async fn browser_screenshot_missing_url() {
        let tool = BrowserTool::new();
        let result = tool.execute(r#"{"action":"screenshot"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("missing"));
    }

    #[tokio::test]
    async fn browser_evaluate_missing_script() {
        let tool = BrowserTool::new();
        let result = tool.execute(r#"{"action":"evaluate","url":"https://example.com"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("missing"));
    }

    #[test]
    fn register_browser_tool_adds_to_registry() {
        let mut registry = ToolRegistry::new();
        register_browser_tool(&mut registry);
        assert!(registry.get("browser").is_some());
    }
}
