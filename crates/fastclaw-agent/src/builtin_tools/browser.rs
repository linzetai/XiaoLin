use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolRegistry, ToolResult};

use super::network::{strip_html_tags, truncate_text};

const DEFAULT_ELEMENT_TIMEOUT: Duration = Duration::from_secs(10);
const BROWSER_LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);
const ACTION_TIMEOUT: Duration = Duration::from_secs(60);

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
            Self::cleanup_profile(&profile);
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

            let (tx, rx) = std::sync::mpsc::channel();
            let opts = launch_options;
            std::thread::spawn(move || {
                let _ = tx.send(headless_chrome::Browser::new(opts));
            });
            let browser = rx
                .recv_timeout(BROWSER_LAUNCH_TIMEOUT)
                .map_err(|_| {
                    "browser: Chrome launch timed out (30s). \
                     What to do next: ensure Chrome/Chromium is installed and on PATH; \
                     check that no other process holds the profile lock at the FASTCLAW_BROWSER_PROFILE directory."
                        .to_string()
                })?
                .map_err(|e| {
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

    fn cleanup_profile(profile: &std::path::Path) {
        for name in &[
            "SingletonLock",
            "SingletonSocket",
            "SingletonCookie",
            "lockfile",
        ] {
            let p = profile.join(name);
            if p.exists() {
                std::fs::remove_file(&p).ok();
            }
        }
        let default_lock = profile.join("Default").join("LOCK");
        if default_lock.exists() {
            std::fs::remove_file(&default_lock).ok();
        }

        Self::kill_orphan_chrome(profile);
    }

    #[cfg(target_os = "windows")]
    fn kill_orphan_chrome(profile: &std::path::Path) {
        let profile_str = profile.to_string_lossy().replace('/', "\\");
        let output = std::process::Command::new("wmic")
            .args(["process", "where", &format!("commandline like '%{profile_str}%' and name='chrome.exe'"), "get", "processid"])
            .output();
        if let Ok(out) = output {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                if let Ok(pid) = line.trim().parse::<u32>() {
                    let _ = std::process::Command::new("taskkill")
                        .args(["/F", "/PID", &pid.to_string()])
                        .output();
                }
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    fn kill_orphan_chrome(profile: &std::path::Path) {
        let profile_str = profile.to_string_lossy();
        let output = std::process::Command::new("pgrep")
            .args(["-f", &format!("chrome.*{profile_str}")])
            .output();
        if let Ok(out) = output {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                if let Ok(pid) = line.trim().parse::<i32>() {
                    let _ = std::process::Command::new("kill")
                        .args(["-9", &pid.to_string()])
                        .output();
                }
            }
        }
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
        tab.set_default_timeout(Duration::from_secs(30));
        state.persistent_tab = Some(tab.clone());
        Ok(tab)
    }

    /// Pre-validate required parameters before launching Chrome.
    fn validate_args(action: &str, args: &serde_json::Value) -> Result<(), String> {
        match action {
            "navigate" => {
                if args.get("url").and_then(|v| v.as_str()).is_none() {
                    return Err(
                        "browser navigate: missing string field 'url'. \
                         Example: {\"action\": \"navigate\", \"url\": \"https://example.com\"}."
                            .to_string(),
                    );
                }
            }
            "evaluate" => {
                if args.get("script").and_then(|v| v.as_str()).is_none() {
                    return Err(
                        "browser evaluate: missing string field 'script'. \
                         Example: {\"action\": \"evaluate\", \"script\": \"document.title\"}."
                            .to_string(),
                    );
                }
            }
            "click" | "hover" | "select" | "wait_for" => {
                if args.get("selector").and_then(|v| v.as_str()).is_none() {
                    return Err(format!(
                        "browser {action}: missing string field 'selector'. \
                         Example: {{\"action\": \"{action}\", \"selector\": \"button.submit\"}}."
                    ));
                }
            }
            "type" => {
                if args.get("selector").and_then(|v| v.as_str()).is_none() {
                    return Err(
                        "browser type: missing string field 'selector'. \
                         Example: {\"action\": \"type\", \"selector\": \"input#email\", \"text\": \"user@example.com\"}."
                            .to_string(),
                    );
                }
                if args.get("text").and_then(|v| v.as_str()).is_none() {
                    return Err(
                        "browser type: missing string field 'text'. \
                         Example: {\"action\": \"type\", \"selector\": \"input#email\", \"text\": \"user@example.com\"}."
                            .to_string(),
                    );
                }
            }
            "press_key" => {
                if args.get("key").and_then(|v| v.as_str()).is_none() {
                    return Err(
                        "browser press_key: missing string field 'key'. \
                         Example: {\"action\": \"press_key\", \"key\": \"Enter\"}."
                            .to_string(),
                    );
                }
            }
            "cookies" => {
                let op = args.get("operation").and_then(|v| v.as_str()).unwrap_or("get");
                if (op == "set" || op == "delete")
                    && args.get("cookie_name").and_then(|v| v.as_str()).is_none()
                {
                    return Err(format!(
                        "browser cookies {op}: missing string field 'cookie_name'. \
                         Example: {{\"action\": \"cookies\", \"operation\": \"{op}\", \"cookie_name\": \"token\"}}."
                    ));
                }
            }
            "pdf" => {
                if args.get("output_path").and_then(|v| v.as_str()).is_none() {
                    return Err(
                        "browser pdf: missing string field 'output_path'. \
                         Example: {\"action\": \"pdf\", \"output_path\": \"page.pdf\"}."
                            .to_string(),
                    );
                }
            }
            "screenshot" | "scroll" | "interact" | "get_content"
            | "go_back" | "go_forward" | "reload" => {}
            other => {
                return Err(format!(
                    "browser: unknown action '{other}'. \
                     Valid actions: navigate, screenshot, evaluate, click, type, press_key, hover, select, wait_for, scroll, go_back, go_forward, reload, cookies, pdf, interact, get_content."
                ));
            }
        }
        Ok(())
    }

    fn parse_timeout(args: &serde_json::Value) -> Duration {
        args.get("timeout_ms")
            .and_then(|v| v.as_u64())
            .map(Duration::from_millis)
            .unwrap_or(DEFAULT_ELEMENT_TIMEOUT)
    }

    fn require_selector<'a>(args: &'a serde_json::Value, action: &str) -> Result<&'a str, String> {
        args.get("selector")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                format!(
                    "browser {action}: missing string field 'selector'. \
                     Example: {{\"action\": \"{action}\", \"selector\": \"button.submit\"}}."
                )
            })
    }

    fn run_action(
        inner: &Mutex<Option<BrowserState>>,
        action: &str,
        args: &serde_json::Value,
    ) -> Result<String, String> {
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

        if Self::is_browser_dead(state) {
            drop(guard);
            Self::reset_and_relaunch(inner)?;
            let mut guard = inner.lock().map_err(|e| format!("browser: re-lock failed: {e}"))?;
            let state = guard.as_mut().ok_or("browser: state empty after relaunch")?;
            return Self::dispatch_action(state, action, args);
        }

        Self::dispatch_action(state, action, args)
    }

    fn is_browser_dead(state: &BrowserState) -> bool {
        state
            .persistent_tab
            .as_ref()
            .map(|t| t.get_url().is_empty() && t.get_title().is_err())
            .unwrap_or(false)
            || state.browser.get_tabs().lock().map(|tabs| tabs.is_empty()).unwrap_or(true)
    }

    fn reset_and_relaunch(inner: &Mutex<Option<BrowserState>>) -> Result<(), String> {
        {
            let mut guard = inner.lock().map_err(|e| format!("browser: lock failed during reset: {e}"))?;
            *guard = None;
        }
        Self::ensure_browser(inner)
    }

    fn dispatch_action(
        state: &mut BrowserState,
        action: &str,
        args: &serde_json::Value,
    ) -> Result<String, String> {
        use headless_chrome::protocol::cdp::{Network, Page};

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
            "click" => {
                let tab = Self::get_or_create_tab(state)?;
                let selector = Self::require_selector(args, "click")?;
                let el = tab.find_element(selector).map_err(|e| {
                    format!(
                        "browser click: no element matched selector '{selector}': {e}. \
                         What to do next: correct the CSS selector, call wait_for until the control exists, or navigate to the right page first."
                    )
                })?;
                el.click().map_err(|e| {
                    format!(
                        "browser click: could not perform click on selector '{selector}': {e}. \
                         What to do next: dismiss modals, try hover then click, or use evaluate to run the same handler in JavaScript if the control is not reachable."
                    )
                })?;
                Ok(serde_json::json!({ "ok": true, "selector": selector }).to_string())
            }
            "type" => {
                let tab = Self::get_or_create_tab(state)?;
                let selector = Self::require_selector(args, "type")?;
                let text = args
                    .get("text")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        "browser type: missing string field 'text'. \
                         Example: {\"action\": \"type\", \"selector\": \"input#email\", \"text\": \"user@example.com\"}."
                            .to_string()
                    })?;
                let el = tab.find_element(selector).map_err(|e| {
                    format!(
                        "browser type: no element matched selector '{selector}': {e}. \
                         What to do next: fix the selector or use wait_for before typing."
                    )
                })?;
                el.type_into(text).map_err(|e| {
                    format!(
                        "browser type: could not type into selector '{selector}': {e}. \
                         What to do next: click focus manually with evaluate, clear the field, or try press_key to submit after filling."
                    )
                })?;
                Ok(serde_json::json!({ "ok": true, "selector": selector, "text": text }).to_string())
            }
            "hover" => {
                let tab = Self::get_or_create_tab(state)?;
                let selector = Self::require_selector(args, "hover")?;
                let el = tab.find_element(selector).map_err(|e| {
                    format!(
                        "browser hover: no element matched selector '{selector}': {e}. \
                         What to do next: fix the CSS selector and ensure the page finished rendering."
                    )
                })?;
                el.call_js_fn(
                    "function() { this.dispatchEvent(new MouseEvent('mouseover', {bubbles: true})); }",
                    vec![],
                    false,
                )
                .map_err(|e| {
                    format!(
                        "browser hover: could not emit mouseover on selector '{selector}': {e}. \
                         What to do next: use evaluate to trigger a custom event or call an exposed JS hook if the element is virtualized."
                    )
                })?;
                Ok(serde_json::json!({ "ok": true, "selector": selector }).to_string())
            }
            "select" => {
                let tab = Self::get_or_create_tab(state)?;
                let selector = Self::require_selector(args, "select")?;
                let value = args
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        "browser select: missing string field 'value' (the option value). \
                         Example: {\"action\": \"select\", \"selector\": \"select#country\", \"value\": \"US\"}."
                            .to_string()
                    })?;
                let sel_lit = serde_json::to_string(selector).map_err(|e| {
                    format!(
                        "browser select: could not encode selector: {e}. \
                         What to do next: use a short ASCII selector, then retry."
                    )
                })?;
                let val_lit = serde_json::to_string(value).map_err(|e| {
                    format!(
                        "browser select: could not encode value: {e}. \
                         What to do next: pass a string option value that matches a real <option> in the list."
                    )
                })?;
                let script = format!(
                    "(() => {{ const el = document.querySelector({sel_lit}); if (!el) throw new Error('not found'); el.value = {val_lit}; el.dispatchEvent(new Event('input', {{bubbles: true}})); el.dispatchEvent(new Event('change', {{bubbles: true}})); }})()",
                );
                tab.evaluate(&script, false).map_err(|e| {
                    format!(
                        "browser select: could not set value on selector '{selector}': {e}. \
                         What to do next: ensure the target is a <select> and the value matches an <option> value, then retry."
                    )
                })?;
                Ok(serde_json::json!({ "ok": true, "selector": selector, "value": value }).to_string())
            }
            "wait_for" => {
                let tab = Self::get_or_create_tab(state)?;
                let selector = Self::require_selector(args, "wait_for")?;
                let timeout = Self::parse_timeout(args);
                tab.wait_for_element_with_custom_timeout(selector, timeout)
                    .map_err(|e| {
                        format!(
                            "browser wait_for: element '{selector}' did not appear before timeout: {e}. \
                             What to do next: increase timeout_ms, check navigation or SPA delays, or relax the CSS selector so it matches a stable node."
                        )
                    })?;
                Ok(serde_json::json!({ "ok": true, "selector": selector, "found": true }).to_string())
            }
            "press_key" => {
                let tab = Self::get_or_create_tab(state)?;
                let key = args
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        "browser press_key: missing string field 'key'. \
                         Example: {\"action\": \"press_key\", \"key\": \"Enter\"}."
                            .to_string()
                    })?;
                tab.press_key(key).map_err(|e| {
                    format!(
                        "browser press_key: could not send key '{key}': {e}. \
                         What to do next: use a name from the puppeteer key map (e.g. Enter, Tab, Escape) or use evaluate to call keyboard handlers directly."
                    )
                })?;
                Ok(serde_json::json!({ "ok": true, "key": key }).to_string())
            }
            "scroll" => {
                let tab = Self::get_or_create_tab(state)?;
                let direction = args
                    .get("direction")
                    .and_then(|v| v.as_str())
                    .unwrap_or("down");
                if direction != "up" && direction != "down" {
                    return Err(format!(
                        "browser scroll: 'direction' must be \"up\" or \"down\", got '{direction}'. \
                         What to do next: pass direction explicitly or omit it to scroll down, then retry."
                    ));
                }
                let amount = args.get("amount").and_then(|v| v.as_i64()).unwrap_or(300);
                let delta = if direction == "up" {
                    -amount.abs()
                } else {
                    amount.abs()
                };
                let script = format!("window.scrollBy(0, {delta})");
                tab.evaluate(&script, false).map_err(|e| {
                    format!(
                        "browser scroll: could not run window.scrollBy: {e}. \
                         What to do next: navigate to a page with a real document, then retry."
                    )
                })?;
                Ok(serde_json::json!({ "ok": true, "direction": direction, "amount": amount.abs() as u64 }).to_string())
            }
            "go_back" => {
                let tab = Self::get_or_create_tab(state)?;
                tab.evaluate("history.back()", false).map_err(|e| {
                    format!(
                        "browser go_back: history.back() failed: {e}. \
                         What to do next: wait after navigation, ensure there is history to return to, or use navigate with a known URL."
                    )
                })?;
                Ok(serde_json::json!({ "ok": true }).to_string())
            }
            "go_forward" => {
                let tab = Self::get_or_create_tab(state)?;
                tab.evaluate("history.forward()", false).map_err(|e| {
                    format!(
                        "browser go_forward: history.forward() failed: {e}. \
                         What to do next: go_back first, wait for the load, then try forward again."
                    )
                })?;
                Ok(serde_json::json!({ "ok": true }).to_string())
            }
            "reload" => {
                let tab = Self::get_or_create_tab(state)?;
                tab.reload(false, None).map_err(|e| {
                    format!(
                        "browser reload: Page.reload failed: {e}. \
                         What to do next: wait until navigation settles, or navigate explicitly to the same URL to refresh."
                    )
                })?;
                Ok(serde_json::json!({ "ok": true }).to_string())
            }
            "cookies" => {
                let tab = Self::get_or_create_tab(state)?;
                let op = args
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("get");
                match op {
                    "get" => {
                        let cookies = tab.get_cookies().map_err(|e| {
                            format!(
                                "browser cookies get: Network.getCookies failed: {e}. \
                                 What to do next: open an http(s) page first, then reread—about: or empty tabs may not expose cookies the same way."
                            )
                        })?;
                        let list = serde_json::to_value(&cookies).map_err(|e| {
                            format!(
                                "browser cookies get: could not encode cookies: {e}. \
                                 What to do next: retry; if the error repeats, report a bug with a minimal repro."
                            )
                        })?;
                        Ok(serde_json::json!({ "ok": true, "cookies": list }).to_string())
                    }
                    "set" => {
                        let name = args
                            .get("cookie_name")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                "browser cookies set: missing string field 'cookie_name'. \
                                 Example: {\"action\": \"cookies\", \"operation\": \"set\", \"cookie_name\": \"token\", \"cookie_value\": \"abc\"}."
                                    .to_string()
                            })?;
                        let value = args
                            .get("cookie_value")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                "browser cookies set: missing string field 'cookie_value'. \
                                 Example: {\"action\": \"cookies\", \"operation\": \"set\", \"cookie_name\": \"token\", \"cookie_value\": \"abc\"}."
                                    .to_string()
                            })?;
                        tab.set_cookies(vec![Network::CookieParam {
                            name: name.to_string(),
                            value: value.to_string(),
                            url: None,
                            domain: None,
                            path: None,
                            secure: None,
                            http_only: None,
                            same_site: None,
                            expires: None,
                            priority: None,
                            same_party: None,
                            source_scheme: None,
                            source_port: None,
                            partition_key: None,
                        }])
                        .map_err(|e| {
                            format!(
                                "browser cookies set: could not set cookie '{name}': {e}. \
                                 What to do next: load a normal http(s) page on the right origin first, then set the value again."
                            )
                        })?;
                        Ok(serde_json::json!({
                            "ok": true,
                            "operation": "set",
                            "cookie_name": name,
                            "cookie_value": value
                        })
                        .to_string())
                    }
                    "delete" => {
                        let name = args
                            .get("cookie_name")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                "browser cookies delete: missing string field 'cookie_name'. \
                                 Example: {\"action\": \"cookies\", \"operation\": \"delete\", \"cookie_name\": \"token\"}."
                                    .to_string()
                            })?;
                        tab.delete_cookies(vec![Network::DeleteCookies {
                            name: name.to_string(),
                            url: None,
                            domain: None,
                            path: None,
                            partition_key: None,
                        }])
                        .map_err(|e| {
                            format!(
                                "browser cookies delete: could not delete cookie '{name}': {e}. \
                                 What to do next: open the site that set the cookie, match name/domain, then retry; some HttpOnly flags require host cookies."
                            )
                        })?;
                        Ok(serde_json::json!({
                            "ok": true,
                            "operation": "delete",
                            "cookie_name": name
                        })
                        .to_string())
                    }
                    "clear" => {
                        let cookies = tab.get_cookies().map_err(|e| {
                            format!(
                                "browser cookies clear: could not list cookies: {e}. \
                                 What to do next: open an http(s) page, then run clear on that tab so the right jar is targeted."
                            )
                        })?;
                        let n = cookies.len();
                        if n > 0 {
                            let dels: Vec<Network::DeleteCookies> = cookies
                                .into_iter()
                                .map(|c| Network::DeleteCookies {
                                    name: c.name,
                                    url: None,
                                    domain: Some(c.domain),
                                    path: Some(c.path),
                                    partition_key: c.partition_key,
                                })
                                .collect();
                            tab.delete_cookies(dels).map_err(|e| {
                                format!(
                                    "browser cookies clear: delete_cookies failed: {e}. \
                                     What to do next: try deleting a single cookie by name, or clear from Chrome manually if a protected cookie blocks removal."
                                )
                            })?;
                        }
                        Ok(serde_json::json!({ "ok": true, "operation": "clear", "deleted": n }).to_string())
                    }
                    other => Err(format!(
                        "browser cookies: unknown operation '{other}'. \
                         What to do next: use one of get, set, delete, or clear, then retry."
                    )),
                }
            }
            "pdf" => {
                let tab = Self::get_or_create_tab(state)?;
                let path = args
                    .get("output_path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        "browser pdf: missing string field 'output_path'. \
                         Example: {\"action\": \"pdf\", \"output_path\": \"page.pdf\"}."
                            .to_string()
                    })?;
                let bytes = tab.print_to_pdf(None).map_err(|e| {
                    format!(
                        "browser pdf: print_to_pdf failed: {e}. \
                         What to do next: wait for the page to finish painting, or pass print settings through the operator if the site is huge."
                    )
                })?;
                let n = bytes.len();
                std::fs::write(path, &bytes).map_err(|e| {
                    format!(
                        "browser pdf: could not write PDF to '{path}': {e}. \
                         What to do next: pick a writable path with enough free disk, then retry."
                    )
                })?;
                Ok(serde_json::json!({ "ok": true, "path": path, "bytes": n }).to_string())
            }
            other => Err(format!(
                "browser: unknown action '{other}'. \
                 Use exactly 'navigate', 'screenshot', 'evaluate', 'click', 'type', 'press_key', 'hover', 'select', 'wait_for', 'scroll', 'go_back', 'go_forward', 'reload', 'cookies', 'pdf', 'interact', or 'get_content' (see tool schema), then retry with the required fields for that action."
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
        "Control a visible Chrome window via CDP—login, CAPTCHAs, and JS-heavy pages. \
         Non-headless by default (FASTCLAW_BROWSER_HEADLESS=true for CI). \
         One persistent tab keeps session/cookies across calls. \
         navigate / go_back / go_forward / reload — history and full loads. \
         screenshot, pdf — image/PDF output (paths optional for screenshot). \
         evaluate — run JS. \
         click, type, hover, select, wait_for, press_key, scroll — DOM automation. \
         cookies (get, set, delete, clear) — cookie jar. \
         interact — user-driven login/CAPTCHA (optional url, wait_seconds). \
         get_content — current URL, title, text without navigating. \
         Prefer web_fetch for static HTML/APIs. Example: {\"action\": \"interact\", \"url\": \"https://accounts.google.com\"}."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "action".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": [
                    "navigate", "screenshot", "evaluate", "click", "type", "press_key", "hover", "select",
                    "wait_for", "scroll", "go_back", "go_forward", "reload", "cookies", "pdf", "interact", "get_content"
                ],
                "description": "What to do: navigate, screenshot, evaluate, DOM actions (click/type/hover/select/wait_for/press_key/scroll), history (go_back/forward/reload), cookies, pdf, interact (user login), or get_content (read current page text)."
            }),
        );
        props.insert(
            "url".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "HTTP(S) page to load. Required for navigate. Optional for screenshot, evaluate, and interact (uses current page if omitted)."
            }),
        );
        props.insert(
            "script".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "JavaScript for evaluate. Return value is Debug-formatted; use JSON.stringify in script for clean text."
            }),
        );
        props.insert(
            "output_path".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Filesystem path: screenshot PNG or pdf (pdf action) output."
            }),
        );
        props.insert(
            "wait_seconds".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "For interact: max seconds to wait (default 60). Returns early if the page URL changes."
            }),
        );
        props.insert(
            "selector".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "CSS selector for click, type, hover, select, and wait_for."
            }),
        );
        props.insert(
            "text".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Text to type (type action)."
            }),
        );
        props.insert(
            "key".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Key name for press_key (e.g. Enter, Tab, Escape) per Chromium key table."
            }),
        );
        props.insert(
            "value".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Option value to select (select action)."
            }),
        );
        props.insert(
            "direction".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "scroll: \"up\" or \"down\" (default down)."
            }),
        );
        props.insert(
            "amount".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "scroll: vertical pixels to scroll (default 300)."
            }),
        );
        props.insert(
            "timeout_ms".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "wait_for: max time to wait for the selector in milliseconds (default 10000)."
            }),
        );
        props.insert(
            "operation".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "cookies: get (list), set (needs cookie_name + cookie_value), delete (cookie_name), or clear (all in tab)."
            }),
        );
        props.insert(
            "cookie_name".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Cookie name for cookies set and delete."
            }),
        );
        props.insert(
            "cookie_value".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Cookie value for cookies set."
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

        if let Err(e) = Self::validate_args(&action, &args) {
            return ToolResult::err(e);
        }

        let inner = self.inner.clone();

        let result = tokio::time::timeout(
            ACTION_TIMEOUT,
            tokio::task::spawn_blocking(move || {
                Self::ensure_browser(&inner)?;
                Self::run_action(&inner, &action, &args)
            }),
        )
        .await;

        match result {
            Ok(Ok(Ok(v))) => ToolResult::ok(v),
            Ok(Ok(Err(e))) => ToolResult::err(e),
            Ok(Err(e)) => ToolResult::err(format!(
                "browser: the blocking worker task panicked or failed to join: {e}. \
                 What went wrong: spawn_blocking did not return a normal tool result (worker crash or runtime shutdown). \
                 What to do next: retry once with a smaller action; if it repeats, restart the gateway browser worker and report the panic to the operator."
            )),
            Err(_) => ToolResult::err(
                "browser: action timed out after 60 seconds. \
                 What to do next: the page or Chrome may be unresponsive; retry with a simpler page, or check that Chrome is installed and responsive."
                    .to_string(),
            ),
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
        assert!(schema.properties.contains_key("selector"));
        assert!(schema.properties.contains_key("timeout_ms"));
        assert!(schema.required.contains(&"action".to_string()));
    }

    #[test]
    fn parse_timeout_defaults() {
        let args = serde_json::json!({});
        assert_eq!(BrowserTool::parse_timeout(&args), DEFAULT_ELEMENT_TIMEOUT);
    }

    #[test]
    fn parse_timeout_custom() {
        let args = serde_json::json!({"timeout_ms": 5000});
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
        for a in [
            "navigate",
            "screenshot",
            "evaluate",
            "click",
            "type",
            "press_key",
            "hover",
            "select",
            "wait_for",
            "scroll",
            "go_back",
            "go_forward",
            "reload",
            "cookies",
            "pdf",
            "interact",
            "get_content",
        ] {
            assert!(actions.contains(&a), "enum missing action: {a}");
        }
    }

    #[test]
    fn is_headless_env_var() {
        std::env::remove_var("FASTCLAW_BROWSER_HEADLESS");
        assert!(!BrowserTool::is_headless());
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

    #[tokio::test]
    async fn browser_click_missing_selector() {
        let tool = BrowserTool::new();
        let result = tool.execute(r#"{"action":"click"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("selector"));
    }

    #[tokio::test]
    async fn browser_type_missing_selector() {
        let tool = BrowserTool::new();
        let result = tool
            .execute(r#"{"action":"type","text":"hello"}"#)
            .await;
        assert!(!result.success);
        assert!(result.output.contains("selector"));
    }

    #[tokio::test]
    async fn browser_type_missing_text() {
        let tool = BrowserTool::new();
        let result = tool
            .execute(r#"{"action":"type","selector":"input"}"#)
            .await;
        assert!(!result.success);
        assert!(result.output.contains("text"));
    }

    #[tokio::test]
    async fn browser_press_key_missing_key() {
        let tool = BrowserTool::new();
        let result = tool.execute(r#"{"action":"press_key"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("key"));
    }

    #[tokio::test]
    async fn browser_hover_missing_selector() {
        let tool = BrowserTool::new();
        let result = tool.execute(r#"{"action":"hover"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("selector"));
    }

    #[tokio::test]
    async fn browser_select_missing_selector() {
        let tool = BrowserTool::new();
        let result = tool.execute(r#"{"action":"select"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("selector"));
    }

    #[tokio::test]
    async fn browser_wait_for_missing_selector() {
        let tool = BrowserTool::new();
        let result = tool.execute(r#"{"action":"wait_for"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("selector"));
    }

    #[tokio::test]
    async fn browser_cookies_set_missing_name() {
        let tool = BrowserTool::new();
        let result = tool
            .execute(r#"{"action":"cookies","operation":"set"}"#)
            .await;
        assert!(!result.success);
        assert!(result.output.contains("cookie_name"));
    }

    #[tokio::test]
    async fn browser_cookies_delete_missing_name() {
        let tool = BrowserTool::new();
        let result = tool
            .execute(r#"{"action":"cookies","operation":"delete"}"#)
            .await;
        assert!(!result.success);
        assert!(result.output.contains("cookie_name"));
    }

    #[tokio::test]
    async fn browser_pdf_missing_output_path() {
        let tool = BrowserTool::new();
        let result = tool.execute(r#"{"action":"pdf"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("output_path"));
    }

    #[tokio::test]
    async fn browser_validate_rejects_unknown_action() {
        let args = serde_json::json!({"action": "explode"});
        let err = BrowserTool::validate_args("explode", &args).unwrap_err();
        assert!(err.contains("unknown action"));
    }

    #[test]
    fn browser_validate_passes_simple_actions() {
        let args = serde_json::json!({});
        assert!(BrowserTool::validate_args("go_back", &args).is_ok());
        assert!(BrowserTool::validate_args("go_forward", &args).is_ok());
        assert!(BrowserTool::validate_args("reload", &args).is_ok());
        assert!(BrowserTool::validate_args("scroll", &args).is_ok());
        assert!(BrowserTool::validate_args("interact", &args).is_ok());
        assert!(BrowserTool::validate_args("get_content", &args).is_ok());
        assert!(BrowserTool::validate_args("screenshot", &args).is_ok());
    }

    #[test]
    fn browser_validate_cookies_get_needs_no_name() {
        let args = serde_json::json!({"operation": "get"});
        assert!(BrowserTool::validate_args("cookies", &args).is_ok());
        let args = serde_json::json!({"operation": "clear"});
        assert!(BrowserTool::validate_args("cookies", &args).is_ok());
    }

    #[test]
    fn register_browser_tool_adds_to_registry() {
        let registry = ToolRegistry::new();
        register_browser_tool(&registry);
        assert!(registry.get("browser").is_some());
    }

    #[tokio::test]
    #[ignore]
    async fn browser_smoke_navigate() {
        let tool = BrowserTool::new();
        let result = tool
            .execute(r#"{"action":"navigate","url":"https://example.com"}"#)
            .await;
        eprintln!("result.success = {}", result.success);
        eprintln!("result.output (first 500 chars) = {}", &result.output[..result.output.len().min(500)]);
        assert!(result.success, "navigate failed: {}", result.output);
        assert!(result.output.contains("example.com") || result.output.contains("Example Domain"));
    }
}
