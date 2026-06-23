use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use async_trait::async_trait;
use xiaolin_core::tool::ToolImage;
use xiaolin_tools_network::{strip_html_tags, truncate_text};

use crate::actions;
use super::{BrowserEngine, EngineActionResult};

const BROWSER_LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);

struct CdpActionResult {
    text: String,
    images: Vec<ToolImage>,
}

impl CdpActionResult {
    fn text(s: String) -> Self {
        Self { text: s, images: vec![] }
    }
}

struct CdpState {
    browser: headless_chrome::Browser,
    persistent_tab: Option<Arc<headless_chrome::Tab>>,
}

/// Chrome DevTools Protocol engine (headless_chrome).
pub struct CdpEngine {
    inner: Arc<Mutex<Option<CdpState>>>,
}

impl Default for CdpEngine {
    fn default() -> Self { Self::new() }
}

impl CdpEngine {
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(None)) }
    }

    pub fn shutdown_sync(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            *guard = None;
        }
        CdpEngine::kill_orphan_chrome(&CdpEngine::profile_dir());
    }

    pub fn is_headless() -> bool {
        std::env::var("XIAOLIN_BROWSER_HEADLESS")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    }

    fn profile_dir() -> std::path::PathBuf {
        if let Ok(dir) = std::env::var("XIAOLIN_BROWSER_PROFILE") {
            return std::path::PathBuf::from(dir);
        }
        let base = dirs::data_local_dir().unwrap_or_else(|| std::env::temp_dir().join("xiaolin"));
        base.join("xiaolin").join("browser-profile")
    }

    fn find_chrome() -> Option<std::path::PathBuf> {
        if let Ok(p) = std::env::var("CHROME") {
            let path = std::path::PathBuf::from(&p);
            if path.exists() {
                return Some(path);
            }
        }

        #[cfg(windows)]
        {
            let candidates = [
                r"C:\Program Files\Google\Chrome\Application\chrome.exe",
                r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
                r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
                r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
                r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
            ];
            if let Some(local) = dirs::data_local_dir() {
                let local_chrome = local.join(r"Google\Chrome\Application\chrome.exe");
                if local_chrome.exists() {
                    return Some(local_chrome);
                }
            }
            for c in &candidates {
                let p = std::path::PathBuf::from(c);
                if p.exists() {
                    return Some(p);
                }
            }
        }

        #[cfg(target_os = "macos")]
        {
            let candidates = [
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                "/Applications/Chromium.app/Contents/MacOS/Chromium",
                "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            ];
            for c in &candidates {
                let p = std::path::PathBuf::from(c);
                if p.exists() {
                    return Some(p);
                }
            }
        }

        None
    }

    fn try_reuse_existing(profile: &std::path::Path) -> Option<headless_chrome::Browser> {
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        let profile_str = profile.to_string_lossy();

        #[cfg(target_os = "windows")]
        let pids_and_ports = {
            let wql_pattern = profile_str.replace('\\', "\\\\");
            let ps_result = {
                let mut cmd = std::process::Command::new("powershell");
                cmd.args([
                    "-NoProfile", "-NonInteractive", "-Command",
                    &format!(
                        "Get-CimInstance Win32_Process -Filter \"name='chrome.exe' AND commandline LIKE '%{}%'\" | Select-Object -ExpandProperty CommandLine",
                        wql_pattern
                    ),
                ]);
                cmd.creation_flags(0x08000000);
                cmd.output().ok()
            };
            let text = match ps_result {
                Some(ref out) if out.status.success() => {
                    String::from_utf8_lossy(&out.stdout).to_string()
                }
                _ => {
                    let mut cmd = std::process::Command::new("wmic");
                    cmd.args([
                        "process",
                        "where",
                        &format!("commandline like '%{wql_pattern}%' and name='chrome.exe'"),
                        "get",
                        "commandline",
                        "/format:list",
                    ]);
                    cmd.creation_flags(0x08000000);
                    match cmd.output() {
                        Ok(out) => String::from_utf8_lossy(&out.stdout).to_string(),
                        Err(_) => return None,
                    }
                }
            };
            CdpEngine::extract_debug_ports(&text)
        };

        #[cfg(not(target_os = "windows"))]
        let pids_and_ports = {
            let output = std::process::Command::new("pgrep")
                .args(["-af", &format!("chrome.*{profile_str}")])
                .output()
                .ok()?;
            let text = String::from_utf8_lossy(&output.stdout);
            CdpEngine::extract_debug_ports(&text)
        };

        for port in pids_and_ports {
            if std::time::Instant::now() > deadline {
                tracing::debug!("browser: try_reuse_existing timed out scanning ports");
                break;
            }
            if let Some(body) = CdpEngine::http_get(&format!("127.0.0.1:{port}"), "/json/version") {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&body) {
                    if let Some(ws_url) = val["webSocketDebuggerUrl"].as_str() {
                        if let Ok(browser) = headless_chrome::Browser::connect_with_timeout(
                            ws_url.to_string(),
                            Duration::from_secs(600),
                        ) {
                            return Some(browser);
                        }
                    }
                }
            }
        }
        None
    }

    fn http_get(host_port: &str, path: &str) -> Option<String> {
        use std::io::{Read, Write};
        let mut stream = std::net::TcpStream::connect(host_port).ok()?;
        stream.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n\r\n"
        )
        .ok()?;
        let mut buf = vec![0u8; 8192];
        let mut total = 0;
        loop {
            match stream.read(&mut buf[total..]) {
                Ok(0) => break,
                Ok(n) => {
                    total += n;
                    if total >= buf.len() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let text = String::from_utf8_lossy(&buf[..total]);
        text.split_once("\r\n\r\n")
            .map(|(_, body)| body.to_string())
    }

    fn extract_debug_ports(text: &str) -> Vec<u16> {
        static RE_PORT: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let re_port = RE_PORT.get_or_init(|| {
            regex::Regex::new(r"--remote-debugging-port=(\d+)").expect("static regex must compile")
        });
        let mut ports: Vec<u16> = re_port
            .captures_iter(text)
            .filter_map(|c| c[1].parse().ok())
            .collect();
        ports.sort_unstable();
        ports.dedup();
        ports
    }

    fn ensure_browser(inner: &Mutex<Option<CdpState>>) -> Result<(), String> {
        let mut guard = inner.lock().map_err(|e| {
            format!("browser: mutex lock failed: {e}. Retry or restart the gateway.")
        })?;

        if let Some(state) = guard.as_ref() {
            if CdpEngine::is_browser_dead(state) {
                tracing::warn!("browser: existing Chrome connection is dead, will rebuild");
                *guard = None;
            } else {
                return Ok(());
            }
        }

        let profile = CdpEngine::profile_dir();
        std::fs::create_dir_all(&profile).ok();

        if let Some(browser) = CdpEngine::try_reuse_existing(&profile) {
            // Reconnected to an existing Chrome — adopt an existing tab to
            // preserve login state, cookies, etc.
            let existing_tab = browser.get_tabs().try_lock().ok().and_then(|tabs| {
                tabs.iter()
                    .rev()
                    .find(|t| {
                        let url = t.get_url();
                        !url.is_empty() && url != "about:blank"
                    })
                    .or_else(|| tabs.first())
                    .cloned()
            });
            *guard = Some(CdpState {
                browser,
                persistent_tab: existing_tab,
            });
            return Ok(());
        }

        CdpEngine::cleanup_profile(&profile);
        CdpEngine::launch_fresh_browser(&mut guard, &profile)
    }

    fn launch_fresh_browser(
        guard: &mut std::sync::MutexGuard<'_, Option<CdpState>>,
        profile: &std::path::Path,
    ) -> Result<(), String> {
        let headless = CdpEngine::is_headless();
        let chrome_path = CdpEngine::find_chrome();
        let mut builder = headless_chrome::LaunchOptions::default_builder();
        let sandbox_enabled = match std::env::var("XIAOLIN_CDP_SANDBOX") {
            Ok(v) if v == "1" || v.eq_ignore_ascii_case("true") => true,
            Ok(v) if v == "0" || v.eq_ignore_ascii_case("false") => false,
            _ => !headless,
        };
        if !sandbox_enabled {
            tracing::warn!(
                "browser: launching Chrome with sandbox disabled (set XIAOLIN_CDP_SANDBOX=true to enable)"
            );
        }
        builder
            .headless(headless)
            .sandbox(sandbox_enabled)
            .window_size(Some((1280, 900)))
            .idle_browser_timeout(Duration::from_secs(600))
            .user_data_dir(Some(profile.to_path_buf()));
        if let Some(ref p) = chrome_path {
            builder.path(Some(p.clone()));
        }
        let launch_options = builder
            .build()
            .map_err(|e| format!("browser: invalid Chrome launch options: {e}."))?;

        let (tx, rx) = std::sync::mpsc::channel();
        let opts = launch_options;
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_flag = cancel.clone();
        std::thread::spawn(move || {
            if cancel_flag.load(Ordering::Relaxed) {
                return;
            }
            match headless_chrome::Browser::new(opts) {
                Ok(browser) => {
                    if cancel_flag.load(Ordering::Relaxed) {
                        drop(browser);
                        return;
                    }
                    let _ = tx.send(Ok(browser));
                }
                Err(e) => {
                    if !cancel_flag.load(Ordering::Relaxed) {
                        let _ = tx.send(Err(e));
                    }
                }
            }
        });
        let browser = rx
            .recv_timeout(BROWSER_LAUNCH_TIMEOUT)
            .map_err(|_| {
                cancel.store(true, Ordering::Relaxed);
                CdpEngine::kill_orphan_chrome(&profile);
                "browser: Chrome launch timed out (30s). Ensure Chrome/Chromium is installed."
                    .to_string()
            })?
            .map_err(|e| format!("browser: could not start Chrome/Chromium: {e}."))?;
        **guard = Some(CdpState {
            browser,
            persistent_tab: None,
        });
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
        CdpEngine::kill_orphan_chrome(profile);
    }

    #[cfg(target_os = "windows")]
    fn kill_orphan_chrome(profile: &std::path::Path) {
        let profile_str = profile.to_string_lossy().replace('/', "\\");
        let wql_pattern = profile_str.replace('\\', "\\\\");
        let mut cmd = std::process::Command::new("wmic");
        cmd.args([
            "process",
            "where",
            &format!("commandline like '%{wql_pattern}%' and name='chrome.exe'"),
            "get",
            "processid",
        ]);
        cmd.creation_flags(0x08000000);
        let output = cmd.output();
        if let Ok(out) = output {
            let text = String::from_utf8_lossy(&out.stdout);
            let mut pids: Vec<u32> = text
                .lines()
                .filter_map(|line| line.trim().parse().ok())
                .collect();
            pids.sort_unstable();
            pids.dedup();
            for pid in &pids {
                let mut kill_cmd = std::process::Command::new("taskkill");
                kill_cmd.args(["/PID", &pid.to_string()]);
                kill_cmd.creation_flags(0x08000000);
                let _ = kill_cmd.output();
            }
            if !pids.is_empty() {
                std::thread::sleep(Duration::from_secs(2));
            }
            for pid in pids {
                let mut kill_cmd = std::process::Command::new("taskkill");
                kill_cmd.args(["/F", "/PID", &pid.to_string()]);
                kill_cmd.creation_flags(0x08000000);
                let _ = kill_cmd.output();
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
            let mut pids: Vec<i32> = text
                .lines()
                .filter_map(|line| line.trim().parse().ok())
                .collect();
            pids.sort_unstable();
            pids.dedup();
            for pid in &pids {
                let _ = std::process::Command::new("kill")
                    .args(["-15", &pid.to_string()])
                    .output();
            }
            if !pids.is_empty() {
                std::thread::sleep(Duration::from_secs(2));
            }
            for pid in pids {
                let still_alive = std::process::Command::new("kill")
                    .args(["-0", &pid.to_string()])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                if still_alive {
                    let _ = std::process::Command::new("kill")
                        .args(["-9", &pid.to_string()])
                        .output();
                }
            }
        }
    }

    fn get_or_create_tab(state: &mut CdpState) -> Result<Arc<headless_chrome::Tab>, String> {
        if let Some(ref tab) = state.persistent_tab {
            if !tab.get_url().is_empty() || tab.get_title().is_ok() {
                return Ok(tab.clone());
            }
        }

        // Persistent tab is dead or absent — try to adopt an existing tab
        // (preserves login state from prior sessions using the same profile).
        if let Ok(tabs) = state.browser.get_tabs().try_lock() {
            for existing in tabs.iter().rev() {
                let url = existing.get_url();
                if !url.is_empty() && url != "about:blank" {
                    existing.set_default_timeout(Duration::from_secs(30));
                    state.persistent_tab = Some(existing.clone());
                    return Ok(existing.clone());
                }
            }
            // All tabs are blank — pick the first one instead of creating new
            if let Some(first) = tabs.first() {
                first.set_default_timeout(Duration::from_secs(30));
                state.persistent_tab = Some(first.clone());
                return Ok(first.clone());
            }
        }

        // No existing tabs available — create one as last resort
        let tab = state
            .browser
            .new_tab()
            .map_err(|e| format!("browser: could not open a new tab: {e}."))?;
        tab.set_default_timeout(Duration::from_secs(30));
        state.persistent_tab = Some(tab.clone());
        Ok(tab)
    }

    /// Build a condensed a11y-tree snapshot (modeled after chrome-devtools-mcp take_snapshot).
    fn build_a11y_snapshot(tab: &headless_chrome::Tab, verbose: bool) -> Result<String, String> {
        let script = if verbose {
            r#"(() => {
  let uid = 0;
  function walk(el) {
    const id = 'e' + (uid++);
    el.setAttribute('data-fc-uid', id);
    const tag = el.tagName?.toLowerCase() || '';
    const role = el.getAttribute('role') || '';
    const ariaLabel = el.getAttribute('aria-label') || '';
    const text = el.childNodes.length === 1 && el.childNodes[0].nodeType === 3
      ? el.childNodes[0].textContent.trim().substring(0, 120) : '';
    const type = el.getAttribute('type') || '';
    const value = el.value !== undefined ? String(el.value).substring(0, 80) : '';
    const href = el.getAttribute('href') || '';
    const cls = el.className && typeof el.className === 'string' ? el.className.substring(0, 60) : '';
    const elId = el.id || '';
    const checked = el.checked;
    const disabled = el.disabled;
    const placeholder = el.getAttribute('placeholder') || '';
    const rect = el.getBoundingClientRect();
    const visible = rect.width > 0 && rect.height > 0;
    const children = [];
    for (const child of el.children) { children.push(walk(child)); }
    return { uid: id, tag, role, ariaLabel, text, type, value, href, cls, elId,
             checked, disabled, placeholder, visible, children };
  }
  return JSON.stringify(walk(document.body));
})()"#
        } else {
            r#"(() => {
  let uid = 0;
  const interactiveTags = new Set(['a','button','input','select','textarea','details','summary','label']);
  const interactiveRoles = new Set(['button','link','textbox','checkbox','radio','combobox','menuitem','tab','switch','slider','option','listbox','menu','dialog','alertdialog','tree','treeitem','grid','gridcell','row','columnheader','rowheader','searchbox','spinbutton']);
  function isInteractive(el) {
    const tag = el.tagName?.toLowerCase() || '';
    if (interactiveTags.has(tag)) return true;
    const role = el.getAttribute('role') || '';
    if (interactiveRoles.has(role)) return true;
    if (el.getAttribute('onclick') || el.getAttribute('tabindex')) return true;
    if (el.contentEditable === 'true') return true;
    return false;
  }
  function walk(el) {
    const rect = el.getBoundingClientRect();
    const visible = rect.width > 0 && rect.height > 0;
    if (!visible) return null;
    const tag = el.tagName?.toLowerCase() || '';
    const interactive = isInteractive(el);
    const children = [];
    for (const child of el.children) {
      const c = walk(child);
      if (c) children.push(c);
    }
    const hasInteractiveDescendant = children.length > 0;
    if (!interactive && !hasInteractiveDescendant) {
      const text = el.innerText?.trim().substring(0, 200) || '';
      if (!text && tag !== 'img') return null;
    }
    const id = 'e' + (uid++);
    el.setAttribute('data-fc-uid', id);
    const role = el.getAttribute('role') || '';
    const ariaLabel = el.getAttribute('aria-label') || '';
    const text = el.childNodes.length === 1 && el.childNodes[0].nodeType === 3
      ? el.childNodes[0].textContent.trim().substring(0, 120) : '';
    const value = el.value !== undefined && interactive ? String(el.value).substring(0, 80) : '';
    const type = interactive ? (el.getAttribute('type') || '') : '';
    const placeholder = interactive ? (el.getAttribute('placeholder') || '') : '';
    return { uid: id, tag, role, ariaLabel, text, type, value, placeholder, interactive, children };
  }
  return JSON.stringify(walk(document.body));
})()"#
        };

        let result = tab
            .evaluate(script, false)
            .map_err(|e| format!("browser take_snapshot: JS evaluation failed: {e}."))?;

        let raw = result
            .value
            .as_ref()
            .and_then(|v| v.as_str())
            .ok_or("browser take_snapshot: empty a11y tree result")?;

        let tree: serde_json::Value = serde_json::from_str(raw)
            .map_err(|e| format!("browser take_snapshot: parse failed: {e}"))?;

        fn render(node: &serde_json::Value, indent: usize, out: &mut String) {
            let uid = node["uid"].as_str().unwrap_or("");
            let tag = node["tag"].as_str().unwrap_or("");
            let role = node["role"].as_str().unwrap_or("");
            let text = node["text"].as_str().unwrap_or("");
            let aria = node["ariaLabel"].as_str().unwrap_or("");
            let val = node["value"].as_str().unwrap_or("");
            let typ = node["type"].as_str().unwrap_or("");
            let ph = node["placeholder"].as_str().unwrap_or("");
            let interactive = node["interactive"].as_bool().unwrap_or(false);

            let prefix = "  ".repeat(indent);
            let mut line = format!("{prefix}[{uid}] {tag}");
            if !role.is_empty() {
                line.push_str(&format!(" role={role}"));
            }
            if !typ.is_empty() {
                line.push_str(&format!(" type={typ}"));
            }
            if interactive {
                line.push_str(" *");
            }
            if !aria.is_empty() {
                line.push_str(&format!(" \"{aria}\""));
            } else if !text.is_empty() {
                line.push_str(&format!(" \"{text}\""));
            }
            if !val.is_empty() {
                line.push_str(&format!(" value=\"{val}\""));
            }
            if !ph.is_empty() {
                line.push_str(&format!(" placeholder=\"{ph}\""));
            }
            out.push_str(&line);
            out.push('\n');

            if let Some(children) = node["children"].as_array() {
                for child in children {
                    render(child, indent + 1, out);
                }
            }
        }

        let mut snapshot = String::new();
        render(&tree, 0, &mut snapshot);

        if snapshot.len() > 32_000 {
            let end = snapshot.floor_char_boundary(31_900);
            snapshot.truncate(end);
            snapshot.push_str("\n... [snapshot truncated at 32KB]");
        }

        Ok(snapshot)
    }

    /// Capture a screenshot, returning (text_summary, png_bytes).
    fn do_screenshot(
        state: &mut CdpState,
        args: &serde_json::Value,
    ) -> Result<(String, Vec<u8>), String> {
        use headless_chrome::protocol::cdp::Page;

        let tab = CdpEngine::get_or_create_tab(state)?;

        if let Some(u) = args.get("url").and_then(|v| v.as_str()) {
            actions::validate_url_scheme(u)?;
            tab.navigate_to(u)
                .map_err(|e| format!("browser screenshot navigate: {e}"))?;
            tab.wait_until_navigated().ok();
        }

        let format = match args.get("format").and_then(|v| v.as_str()).unwrap_or("png") {
            "jpeg" => Page::CaptureScreenshotFormatOption::Jpeg,
            "webp" => Page::CaptureScreenshotFormatOption::Webp,
            _ => Page::CaptureScreenshotFormatOption::Png,
        };
        let quality = args
            .get("quality")
            .and_then(|v| v.as_u64())
            .map(|q| q as u32);
        let full_page = args
            .get("fullPage")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Element screenshot via uid
        let clip = if let Some(uid) = args.get("uid").and_then(|v| v.as_str()) {
            crate::actions::validate_uid(uid)?;
            let selector = format!("[data-fc-uid=\"{uid}\"]");
            let rect_js = format!(
                "(() => {{ const el = document.querySelector('{}'); \
                 if (!el) return null; const r = el.getBoundingClientRect(); \
                 return JSON.stringify({{x: r.x, y: r.y, width: r.width, height: r.height}}); \
                 }})()",
                selector.replace('\'', "\\'")
            );
            let result = tab
                .evaluate(&rect_js, false)
                .map_err(|e| format!("browser screenshot: element rect eval failed: {e}"))?;
            let raw = result
                .value
                .as_ref()
                .and_then(|v| v.as_str())
                .ok_or_else(|| format!("browser screenshot: element uid '{uid}' not found"))?;
            let rect: serde_json::Value = serde_json::from_str(raw)
                .map_err(|e| format!("browser screenshot: parse rect: {e}"))?;
            Some(Page::Viewport {
                x: rect["x"].as_f64().unwrap_or(0.0),
                y: rect["y"].as_f64().unwrap_or(0.0),
                width: rect["width"].as_f64().unwrap_or(100.0),
                height: rect["height"].as_f64().unwrap_or(100.0),
                scale: 1.0,
            })
        } else {
            None
        };

        let png = tab
            .capture_screenshot(format, quality, clip, full_page)
            .map_err(|e| format!("browser screenshot: capture failed: {e}."))?;

        if let Some(path) = args
            .get("filePath")
            .or(args.get("output_path"))
            .and_then(|v| v.as_str())
        {
            let validated = actions::validate_output_path(path)?;
            std::fs::write(&validated, &png).map_err(|e| {
                format!(
                    "browser screenshot: could not write to '{}': {e}.",
                    validated.display()
                )
            })?;
        }

        let summary = format!(
            "Screenshot captured ({}x{} viewport, {} bytes). URL: {} Title: {}",
            1280,
            900,
            png.len(),
            tab.get_url(),
            tab.get_title().unwrap_or_default(),
        );
        Ok((summary, png))
    }

    fn run_action(
        inner: &Mutex<Option<CdpState>>,
        action: &str,
        args: &serde_json::Value,
    ) -> Result<CdpActionResult, String> {
        let mut guard = inner
            .lock()
            .map_err(|e| format!("browser: mutex lock failed for action '{action}': {e}."))?;
        let state = guard
            .as_mut()
            .ok_or("browser: no Chrome instance after ensure_browser.".to_string())?;

        if CdpEngine::is_browser_dead(state) {
            drop(guard);
            CdpEngine::reset_and_relaunch(inner)?;
            let mut guard = inner
                .lock()
                .map_err(|e| format!("browser: re-lock failed: {e}"))?;
            let state = guard
                .as_mut()
                .ok_or("browser: state empty after relaunch")?;
            if action == "screenshot" {
                let (text, png) = CdpEngine::do_screenshot(state, args)?;
                return Ok(CdpActionResult {
                    text,
                    images: vec![ToolImage {
                        mime_type: "image/png".into(),
                        data: png,
                    }],
                });
            }
            return CdpEngine::dispatch_action(state, action, args).map(CdpActionResult::text);
        }

        // Screenshot gets special handling to return image data
        if action == "screenshot" {
            let result = CdpEngine::do_screenshot(state, args);
            if let Err(ref e) = result {
                let lower = e.to_lowercase();
                if lower.contains("websocket")
                    || lower.contains("channel closed")
                    || lower.contains("broken pipe")
                {
                    drop(guard);
                    if CdpEngine::reset_and_relaunch(inner).is_ok() {
                        let mut guard =
                            inner.lock().map_err(|e| format!("browser: re-lock: {e}"))?;
                        if let Some(state) = guard.as_mut() {
                            let (text, png) = CdpEngine::do_screenshot(state, args)?;
                            return Ok(CdpActionResult {
                                text,
                                images: vec![ToolImage {
                                    mime_type: "image/png".into(),
                                    data: png,
                                }],
                            });
                        }
                    }
                }
            }
            let (text, png) = result?;
            return Ok(CdpActionResult {
                text,
                images: vec![ToolImage {
                    mime_type: "image/png".into(),
                    data: png,
                }],
            });
        }

        let result = CdpEngine::dispatch_action(state, action, args);

        if let Err(ref e) = result {
            let lower = e.to_lowercase();
            let is_cdp_dead = lower.contains("could not open a new tab")
                || lower.contains("websocket")
                || lower.contains("channel closed")
                || lower.contains("not connected")
                || lower.contains("browser process")
                || lower.contains("pipe error")
                || lower.contains("broken pipe");
            if is_cdp_dead {
                tracing::warn!("browser: CDP connection error, attempting reconnect: {e}");
                drop(guard);
                if CdpEngine::reset_and_relaunch(inner).is_ok() {
                    let mut guard = inner
                        .lock()
                        .map_err(|e| format!("browser: re-lock after reconnect failed: {e}"))?;
                    if let Some(state) = guard.as_mut() {
                        return CdpEngine::dispatch_action(state, action, args).map(CdpActionResult::text);
                    }
                }
            }
        }

        result.map(CdpActionResult::text)
    }

    fn is_browser_dead(state: &CdpState) -> bool {
        match state.browser.get_tabs().try_lock() {
            Ok(tabs) => {
                if tabs.is_empty() {
                    return true;
                }
                // Only dead if ALL tabs are unreachable (not just the first one
                // or the persistent tab — a single healthy tab means browser is alive).
                let any_alive = tabs
                    .iter()
                    .any(|t| !t.get_url().is_empty() || t.get_title().is_ok());
                if !any_alive {
                    return true;
                }
            }
            Err(std::sync::TryLockError::Poisoned(_)) => return true,
            Err(std::sync::TryLockError::WouldBlock) => {
                // Another thread holds the lock — browser is alive
            }
        }
        false
    }

    fn reset_and_relaunch(inner: &Mutex<Option<CdpState>>) -> Result<(), String> {
        {
            let mut guard = inner
                .lock()
                .map_err(|e| format!("browser: lock failed during reset: {e}"))?;
            *guard = None;
        }
        CdpEngine::ensure_browser(inner)
    }

    /// Resolve an element from either uid (data-fc-uid) or CSS selector.
    fn find_element<'a>(
        tab: &'a headless_chrome::Tab,
        args: &serde_json::Value,
    ) -> Result<headless_chrome::Element<'a>, String> {
        if let Some(uid) = args.get("uid").and_then(|v| v.as_str()) {
            crate::actions::validate_uid(uid)?;
            let selector = format!("[data-fc-uid=\"{uid}\"]");
            tab.find_element(&selector).map_err(|e| {
                format!("browser: element with uid '{uid}' not found: {e}. Run take_snapshot for fresh UIDs.")
            })
        } else if let Some(selector) = args.get("selector").and_then(|v| v.as_str()) {
            tab.find_element(selector)
                .map_err(|e| format!("browser: no element matched selector '{selector}': {e}."))
        } else {
            Err("browser: provide 'uid' (from take_snapshot) or 'selector' (CSS).".to_string())
        }
    }

    fn dispatch_action(
        state: &mut CdpState,
        action: &str,
        args: &serde_json::Value,
    ) -> Result<String, String> {
        use headless_chrome::protocol::cdp::Network;

        match action {
            // ── Navigation (unified, chrome-devtools-mcp style) ─────────
            "navigate" | "go_back" | "go_forward" | "reload" => {
                let nav_type = if action == "navigate" {
                    args.get("type").and_then(|v| v.as_str()).unwrap_or("url")
                } else {
                    // Legacy aliases
                    match action {
                        "go_back" => "back",
                        "go_forward" => "forward",
                        "reload" => "reload",
                        _ => "url",
                    }
                };

                let tab = CdpEngine::get_or_create_tab(state)?;

                match nav_type {
                    "back" => {
                        tab.evaluate("history.back()", false)
                            .map_err(|e| format!("browser navigate back: {e}."))?;
                        std::thread::sleep(Duration::from_millis(500));
                    }
                    "forward" => {
                        tab.evaluate("history.forward()", false)
                            .map_err(|e| format!("browser navigate forward: {e}."))?;
                        std::thread::sleep(Duration::from_millis(500));
                    }
                    "reload" => {
                        let ignore_cache = args
                            .get("ignoreCache")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        tab.reload(ignore_cache, None)
                            .map_err(|e| format!("browser navigate reload: {e}."))?;
                        tab.wait_until_navigated().ok();
                    }
                    _ => {
                        let url = args.get("url").and_then(|v| v.as_str()).ok_or(
                            "browser navigate: 'type' is 'url' but missing 'url' field."
                                .to_string(),
                        )?;
                        actions::validate_url_scheme(url)?;
                        tab.navigate_to(url).map_err(|e| {
                            format!("browser navigate: failed to load '{url}': {e}.")
                        })?;
                        tab.wait_until_navigated().map_err(|e| {
                            format!("browser navigate: wait for '{url}' failed: {e}.")
                        })?;
                    }
                }

                let title = tab.get_title().unwrap_or_default();
                let current_url = tab.get_url();
                Ok(serde_json::json!({
                    "url": current_url,
                    "title": title,
                })
                .to_string())
            }

            // ── A11y tree snapshot (core chrome-devtools-mcp feature) ───
            "take_snapshot" => {
                let tab = CdpEngine::get_or_create_tab(state)?;
                let verbose = args
                    .get("verbose")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let snapshot = CdpEngine::build_a11y_snapshot(&tab, verbose)?;
                let url = tab.get_url();
                let title = tab.get_title().unwrap_or_default();
                let output = serde_json::json!({
                    "source": crate::js::UNTRUSTED_SOURCE,
                    "warning": crate::js::UNTRUSTED_WARNING,
                    "title": title,
                    "url": url,
                    "snapshot": snapshot,
                }).to_string();
                if let Some(path) = args.get("filePath").and_then(|v| v.as_str()) {
                    let validated = actions::validate_output_path(path)?;
                    std::fs::write(&validated, &output).map_err(|e| {
                        format!(
                            "browser take_snapshot: write to '{}': {e}",
                            validated.display()
                        )
                    })?;
                    Ok(serde_json::json!({
                        "saved": validated.to_string_lossy(),
                        "elements": snapshot.lines().count()
                    })
                    .to_string())
                } else {
                    Ok(output)
                }
            }

            // Screenshot is handled by run_action -> do_screenshot for multimodal support
            "screenshot" => {
                let (summary, _png) = CdpEngine::do_screenshot(state, args)?;
                Ok(summary)
            }

            // ── Get content (text) ─────────────────────────────────────
            "get_content" => {
                let tab = CdpEngine::get_or_create_tab(state)?;
                let html = tab.get_content().unwrap_or_default();
                let cleaned = strip_html_tags(&html);
                let truncated = truncate_text(&cleaned, 16_384);
                Ok(serde_json::json!({
                    "source": crate::js::UNTRUSTED_SOURCE,
                    "warning": crate::js::UNTRUSTED_WARNING,
                    "url": tab.get_url(),
                    "title": tab.get_title().unwrap_or_default(),
                    "content": truncated,
                })
                .to_string())
            }

            // ── PDF ────────────────────────────────────────────────────
            "pdf" => {
                let tab = CdpEngine::get_or_create_tab(state)?;
                let path = args
                    .get("output_path")
                    .and_then(|v| v.as_str())
                    .ok_or("browser pdf: missing 'output_path'.")?;
                let validated = actions::validate_output_path(path)?;
                let bytes = tab
                    .print_to_pdf(None)
                    .map_err(|e| format!("browser pdf: {e}."))?;
                std::fs::write(&validated, &bytes).map_err(|e| {
                    format!(
                        "browser pdf: write to '{}' failed: {e}.",
                        validated.display()
                    )
                })?;
                Ok(serde_json::json!({
                    "ok": true,
                    "path": validated.to_string_lossy(),
                    "bytes": bytes.len()
                })
                .to_string())
            }

            // ── Evaluate (supports chrome-devtools-mcp "function" syntax) ──
            "evaluate" => {
                let tab = CdpEngine::get_or_create_tab(state)?;

                if let Some(u) = args.get("url").and_then(|v| v.as_str()) {
                    actions::validate_url_scheme(u)?;
                    tab.navigate_to(u)
                        .map_err(|e| format!("browser evaluate navigate: {e}"))?;
                    tab.wait_until_navigated().ok();
                }

                let script = args
                    .get("function")
                    .or(args.get("script"))
                    .and_then(|v| v.as_str())
                    .ok_or("browser evaluate: missing 'script' or 'function'.")?;

                let result = tab
                    .evaluate(script, false)
                    .map_err(|e| format!("browser evaluate: JS failed: {e}."))?;

                Ok(serde_json::json!({
                    "result": format!("{:?}", result.value),
                })
                .to_string())
            }

            // ── Click (uid or selector, supports dblClick) ─────────────
            "click" => {
                let tab = CdpEngine::get_or_create_tab(state)?;
                let el = CdpEngine::find_element(&tab, args)?;
                let dbl_click = args
                    .get("dblClick")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if dbl_click {
                    el.click().map_err(|e| format!("browser click: {e}."))?;
                    std::thread::sleep(Duration::from_millis(50));
                    el.click()
                        .map_err(|e| format!("browser dblClick second click: {e}."))?;
                } else {
                    el.click().map_err(|e| format!("browser click: {e}."))?;
                }
                let include_snapshot = args
                    .get("includeSnapshot")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if include_snapshot {
                    let snap = CdpEngine::build_a11y_snapshot(&tab, false)?;
                    Ok(serde_json::json!({ "ok": true, "snapshot": snap }).to_string())
                } else {
                    Ok(serde_json::json!({ "ok": true }).to_string())
                }
            }

            // ── Fill (chrome-devtools-mcp: input/select by uid) ────────
            "fill" => {
                let tab = CdpEngine::get_or_create_tab(state)?;
                let el = CdpEngine::find_element(&tab, args)?;
                let value = args
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or("browser fill: missing 'value'.")?;

                el.call_js_fn(
                    "function(v) { \
                       if (this.tagName === 'SELECT') { this.value = v; } \
                       else { this.focus(); this.value = ''; this.value = v; } \
                       this.dispatchEvent(new Event('input', {bubbles: true})); \
                       this.dispatchEvent(new Event('change', {bubbles: true})); \
                     }",
                    vec![serde_json::json!(value)],
                    false,
                )
                .map_err(|e| format!("browser fill: {e}."))?;

                let include_snapshot = args
                    .get("includeSnapshot")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if include_snapshot {
                    let snap = CdpEngine::build_a11y_snapshot(&tab, false)?;
                    Ok(
                        serde_json::json!({ "ok": true, "value": value, "snapshot": snap })
                            .to_string(),
                    )
                } else {
                    Ok(serde_json::json!({ "ok": true, "value": value }).to_string())
                }
            }

            // ── Fill form (batch, chrome-devtools-mcp style) ───────────
            "fill_form" => {
                let tab = CdpEngine::get_or_create_tab(state)?;
                let elements = args
                    .get("elements")
                    .and_then(|v| v.as_array())
                    .ok_or("browser fill_form: missing 'elements' array.")?;

                let mut filled = 0u32;
                for item in elements {
                    let uid = item.get("uid").and_then(|v| v.as_str()).unwrap_or("");
                    let value = item.get("value").and_then(|v| v.as_str()).unwrap_or("");
                    if uid.is_empty() || value.is_empty() {
                        continue;
                    }

                    let selector = format!("[data-fc-uid=\"{uid}\"]");
                    if let Ok(el) = tab.find_element(&selector) {
                        let _ = el.call_js_fn(
                            "function(v) { \
                               if (this.tagName === 'SELECT') { this.value = v; } \
                               else { this.focus(); this.value = ''; this.value = v; } \
                               this.dispatchEvent(new Event('input', {bubbles: true})); \
                               this.dispatchEvent(new Event('change', {bubbles: true})); \
                             }",
                            vec![serde_json::json!(value)],
                            false,
                        );
                        filled += 1;
                    }
                }
                Ok(serde_json::json!({ "ok": true, "filled": filled }).to_string())
            }

            // ── Type text (into currently focused element) ─────────────
            "type_text" => {
                let tab = CdpEngine::get_or_create_tab(state)?;
                let text = args
                    .get("text")
                    .and_then(|v| v.as_str())
                    .ok_or("browser type_text: missing 'text'.")?;

                for ch in text.chars() {
                    tab.press_key(&ch.to_string())
                        .map_err(|e| format!("browser type_text: key press failed: {e}."))?;
                }

                if let Some(submit) = args.get("submitKey").and_then(|v| v.as_str()) {
                    tab.press_key(submit)
                        .map_err(|e| format!("browser type_text submitKey '{submit}': {e}."))?;
                }
                Ok(serde_json::json!({ "ok": true, "typed": text }).to_string())
            }

            // ── Legacy type (selector + text) ──────────────────────────
            "type" => {
                let tab = CdpEngine::get_or_create_tab(state)?;
                let selector = actions::require_selector(args, "type")?;
                let text = args
                    .get("text")
                    .and_then(|v| v.as_str())
                    .ok_or("browser type: missing 'text'.")?;
                let el = tab
                    .find_element(selector)
                    .map_err(|e| format!("browser type: '{selector}' not found: {e}."))?;
                el.type_into(text)
                    .map_err(|e| format!("browser type: could not type into '{selector}': {e}."))?;
                Ok(
                    serde_json::json!({ "ok": true, "selector": selector, "text": text })
                        .to_string(),
                )
            }

            // ── Press key (supports modifiers like "Control+A") ────────
            "press_key" => {
                let tab = CdpEngine::get_or_create_tab(state)?;
                let key = args
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or("browser press_key: missing 'key'.")?;
                tab.press_key(key)
                    .map_err(|e| format!("browser press_key '{key}': {e}."))?;
                Ok(serde_json::json!({ "ok": true, "key": key }).to_string())
            }

            // ── Hover (uid or selector) ────────────────────────────────
            "hover" => {
                let tab = CdpEngine::get_or_create_tab(state)?;
                let el = CdpEngine::find_element(&tab, args)?;
                el.call_js_fn(
                    "function() { this.dispatchEvent(new MouseEvent('mouseover', {bubbles: true})); }",
                    vec![],
                    false,
                ).map_err(|e| format!("browser hover: {e}."))?;
                Ok(serde_json::json!({ "ok": true }).to_string())
            }

            // ── Select (legacy, selector + value) ──────────────────────
            "select" => {
                let tab = CdpEngine::get_or_create_tab(state)?;
                let selector = actions::require_selector(args, "select")?;
                let value = args
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or("browser select: missing 'value'.")?;
                let sel_lit = serde_json::to_string(selector).unwrap();
                let val_lit = serde_json::to_string(value).unwrap();
                let script = format!(
                    "(() => {{ const el = document.querySelector({sel_lit}); if (!el) throw new Error('not found'); el.value = {val_lit}; el.dispatchEvent(new Event('input', {{bubbles: true}})); el.dispatchEvent(new Event('change', {{bubbles: true}})); }})()",
                );
                tab.evaluate(&script, false)
                    .map_err(|e| format!("browser select on '{selector}': {e}."))?;
                Ok(
                    serde_json::json!({ "ok": true, "selector": selector, "value": value })
                        .to_string(),
                )
            }

            // ── Wait for (text or selector) ────────────────────────────
            "wait_for" => {
                let tab = CdpEngine::get_or_create_tab(state)?;
                let timeout = actions::parse_timeout(args);

                if let Some(texts) = args.get("text").and_then(|v| v.as_array()) {
                    let text_list: Vec<&str> = texts.iter().filter_map(|t| t.as_str()).collect();
                    if text_list.is_empty() {
                        return Err("browser wait_for: 'text' array is empty.".to_string());
                    }
                    let text_json = serde_json::to_string(&text_list).unwrap();
                    let script = format!(
                        "new Promise((resolve, reject) => {{ \
                           const texts = {text_json}; \
                           const deadline = Date.now() + {timeout_ms}; \
                           const check = () => {{ \
                             const body = document.body?.innerText || ''; \
                             for (const t of texts) {{ if (body.includes(t)) return resolve(t); }} \
                             if (Date.now() > deadline) return reject('timeout'); \
                             setTimeout(check, 200); \
                           }}; check(); \
                         }})",
                        timeout_ms = timeout.as_millis(),
                    );
                    let result = tab
                        .evaluate(&script, true)
                        .map_err(|e| format!("browser wait_for text: {e}."))?;
                    Ok(
                        serde_json::json!({ "ok": true, "matched": format!("{:?}", result.value) })
                            .to_string(),
                    )
                } else if let Some(selector) = args.get("selector").and_then(|v| v.as_str()) {
                    tab.wait_for_element_with_custom_timeout(selector, timeout)
                        .map_err(|e| format!("browser wait_for selector '{selector}': {e}."))?;
                    Ok(
                        serde_json::json!({ "ok": true, "selector": selector, "found": true })
                            .to_string(),
                    )
                } else {
                    Err("browser wait_for: provide 'text' or 'selector'.".to_string())
                }
            }

            // ── Scroll ─────────────────────────────────────────────────
            "scroll" => {
                let tab = CdpEngine::get_or_create_tab(state)?;
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
                tab.evaluate(&format!("window.scrollBy(0, {delta})"), false)
                    .map_err(|e| format!("browser scroll: {e}."))?;
                Ok(serde_json::json!({ "ok": true, "direction": direction, "amount": amount.unsigned_abs() }).to_string())
            }

            // ── Drag (from uid to uid, chrome-devtools-mcp style) ──────
            "drag" => {
                let tab = CdpEngine::get_or_create_tab(state)?;
                let from_uid = args
                    .get("from_uid")
                    .or(args.get("uid"))
                    .and_then(|v| v.as_str())
                    .ok_or("browser drag: missing 'from_uid'.")?;
                let to_uid = args
                    .get("to_uid")
                    .and_then(|v| v.as_str())
                    .ok_or("browser drag: missing 'to_uid'.")?;
                actions::validate_uid(from_uid)?;
                actions::validate_uid(to_uid)?;

                let script = format!(
                    r#"(() => {{
  const from = document.querySelector('[data-fc-uid="{from_uid}"]');
  const to = document.querySelector('[data-fc-uid="{to_uid}"]');
  if (!from || !to) throw new Error('elements not found');
  const fr = from.getBoundingClientRect();
  const tr = to.getBoundingClientRect();
  const dt = new DataTransfer();
  from.dispatchEvent(new DragEvent('dragstart', {{bubbles: true, dataTransfer: dt, clientX: fr.x+fr.width/2, clientY: fr.y+fr.height/2}}));
  to.dispatchEvent(new DragEvent('dragover', {{bubbles: true, dataTransfer: dt, clientX: tr.x+tr.width/2, clientY: tr.y+tr.height/2}}));
  to.dispatchEvent(new DragEvent('drop', {{bubbles: true, dataTransfer: dt, clientX: tr.x+tr.width/2, clientY: tr.y+tr.height/2}}));
  from.dispatchEvent(new DragEvent('dragend', {{bubbles: true, dataTransfer: dt}}));
  return 'ok';
}})()"#
                );
                tab.evaluate(&script, false)
                    .map_err(|e| format!("browser drag: {e}."))?;
                Ok(serde_json::json!({ "ok": true, "from": from_uid, "to": to_uid }).to_string())
            }

            // ── Handle dialog ──────────────────────────────────────────
            "handle_dialog" => {
                let tab = CdpEngine::get_or_create_tab(state)?;
                let dialog_action = args
                    .get("dialog_action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("accept");
                let prompt_text = args.get("promptText").and_then(|v| v.as_str());

                let script = if dialog_action == "dismiss" {
                    "window.__fc_dialog_dismiss = true; undefined".to_string()
                } else if let Some(pt) = prompt_text {
                    let escaped = serde_json::to_string(pt).unwrap();
                    format!("window.__fc_dialog_response = {escaped}; undefined")
                } else {
                    "undefined".to_string()
                };

                tab.evaluate(&script, false)
                    .map_err(|e| format!("browser handle_dialog: {e}"))?;
                Ok(serde_json::json!({ "ok": true, "action": dialog_action }).to_string())
            }

            // ── List pages (tabs) ──────────────────────────────────────
            "list_pages" => {
                let tabs = state
                    .browser
                    .get_tabs()
                    .try_lock()
                    .map_err(|_| "browser list_pages: tabs lock failed.".to_string())?;
                let pages: Vec<serde_json::Value> = tabs
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        serde_json::json!({
                            "pageId": i,
                            "url": t.get_url(),
                            "title": t.get_title().unwrap_or_default(),
                        })
                    })
                    .collect();
                Ok(serde_json::json!({ "pages": pages }).to_string())
            }

            // ── Select page ────────────────────────────────────────────
            "select_page" => {
                let page_id = args.get("pageId").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let tabs = state
                    .browser
                    .get_tabs()
                    .try_lock()
                    .map_err(|_| "browser select_page: tabs lock failed.".to_string())?;
                let tab = tabs
                    .get(page_id)
                    .ok_or(format!(
                        "browser select_page: pageId {page_id} out of range (have {} tabs).",
                        tabs.len()
                    ))?
                    .clone();
                drop(tabs);
                tab.bring_to_front().ok();
                state.persistent_tab = Some(tab.clone());
                let url = tab.get_url();
                let title = tab.get_title().unwrap_or_default();
                Ok(serde_json::json!({ "ok": true, "pageId": page_id, "url": url, "title": title }).to_string())
            }

            // ── New page ───────────────────────────────────────────────
            "new_page" => {
                let url = args
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or("browser new_page: missing 'url'.")?;
                actions::validate_url_scheme(url)?;
                let tab = state
                    .browser
                    .new_tab()
                    .map_err(|e| format!("browser new_page: {e}."))?;
                tab.navigate_to(url)
                    .map_err(|e| format!("browser new_page navigate to '{url}': {e}."))?;
                tab.wait_until_navigated().ok();
                state.persistent_tab = Some(tab.clone());
                let title = tab.get_title().unwrap_or_default();
                Ok(serde_json::json!({
                    "ok": true, "url": tab.get_url(), "title": title,
                })
                .to_string())
            }

            // ── Close page ─────────────────────────────────────────────
            "close_page" => {
                let page_id = args.get("pageId").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let tabs = state
                    .browser
                    .get_tabs()
                    .try_lock()
                    .map_err(|_| "browser close_page: tabs lock failed.".to_string())?;
                if tabs.len() <= 1 {
                    return Err("browser close_page: cannot close the last tab.".to_string());
                }
                let tab = tabs
                    .get(page_id)
                    .ok_or(format!(
                        "browser close_page: pageId {page_id} out of range."
                    ))?
                    .clone();
                drop(tabs);

                tab.evaluate("window.close()", false).ok();

                if state.persistent_tab.as_ref().map(|t| t.get_url()) == Some(tab.get_url()) {
                    state.persistent_tab = None;
                }
                Ok(serde_json::json!({ "ok": true, "closed": page_id }).to_string())
            }

            // ── List network requests ──────────────────────────────────
            "list_network_requests" => {
                let tab = CdpEngine::get_or_create_tab(state)?;
                let script = r#"(() => {
  const entries = performance.getEntriesByType('resource').concat(performance.getEntriesByType('navigation'));
  return JSON.stringify(entries.slice(-100).map((e, i) => ({
    reqid: i,
    name: e.name,
    type: e.initiatorType || e.entryType,
    duration: Math.round(e.duration),
    transferSize: e.transferSize || 0,
  })));
})()"#;
                let result = tab
                    .evaluate(script, false)
                    .map_err(|e| format!("browser list_network_requests: {e}."))?;
                let raw = result
                    .value
                    .as_ref()
                    .and_then(|v| v.as_str())
                    .unwrap_or("[]");
                Ok(format!("{{\"requests\":{raw}}}"))
            }

            // ── List console messages ──────────────────────────────────
            "list_console_messages" => {
                let tab = CdpEngine::get_or_create_tab(state)?;
                let script = r#"(() => {
  if (!window.__fc_console_log) return '[]';
  return JSON.stringify(window.__fc_console_log.slice(-50));
})()"#;
                let result = tab
                    .evaluate(script, false)
                    .map_err(|e| format!("browser list_console_messages: {e}."))?;
                let raw = result
                    .value
                    .as_ref()
                    .and_then(|v| v.as_str())
                    .unwrap_or("[]");
                Ok(format!("{{\"messages\":{raw}}}"))
            }

            // ── Emulate ────────────────────────────────────────────────
            "emulate" => {
                let tab = CdpEngine::get_or_create_tab(state)?;
                let mut parts = Vec::new();

                if let Some(ua) = args.get("userAgent").and_then(|v| v.as_str()) {
                    let ua_escaped = serde_json::to_string(ua).unwrap();
                    tab.evaluate(&format!("Object.defineProperty(navigator, 'userAgent', {{get: () => {ua_escaped}}})"), false).ok();
                    parts.push(format!("userAgent: {ua}"));
                }

                if let Some(color) = args.get("colorScheme").and_then(|v| v.as_str()) {
                    tab.evaluate(
                        &format!("document.documentElement.style.colorScheme = '{color}'"),
                        false,
                    )
                    .ok();
                    parts.push(format!("colorScheme: {color}"));
                }

                if parts.is_empty() {
                    parts.push("no emulation changes applied".to_string());
                }
                Ok(serde_json::json!({ "ok": true, "applied": parts }).to_string())
            }

            // ── Resize page ────────────────────────────────────────────
            "resize_page" => {
                let tab = CdpEngine::get_or_create_tab(state)?;
                let w = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1280);
                let h = args.get("height").and_then(|v| v.as_u64()).unwrap_or(900);
                tab.evaluate(&format!("window.resizeTo({w}, {h})"), false)
                    .map_err(|e| format!("browser resize_page: {e}."))?;
                Ok(serde_json::json!({ "ok": true, "width": w, "height": h }).to_string())
            }

            // ── Interact (visible browser, user-driven) ────────────────
            "interact" => {
                if CdpEngine::is_headless() {
                    return Err("browser interact: requires a visible browser window. \
                         Set XIAOLIN_BROWSER_HEADLESS to false."
                        .to_string());
                }

                let tab = CdpEngine::get_or_create_tab(state)?;
                if let Some(u) = args.get("url").and_then(|v| v.as_str()) {
                    actions::validate_url_scheme(u)?;
                    tab.navigate_to(u)
                        .map_err(|e| format!("browser interact: {e}"))?;
                    tab.wait_until_navigated().ok();
                }

                let started_url = tab.get_url();
                let wait_seconds = args
                    .get("wait_seconds")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(60);
                let deadline = std::time::Instant::now() + Duration::from_secs(wait_seconds);

                while std::time::Instant::now() < deadline {
                    std::thread::sleep(Duration::from_secs(2));
                    if tab.get_url() != started_url {
                        break;
                    }
                }

                let final_url = tab.get_url();
                let title = tab.get_title().unwrap_or_default();
                Ok(serde_json::json!({
                    "started_url": started_url,
                    "final_url": final_url,
                    "title": title,
                    "url_changed": started_url != final_url,
                })
                .to_string())
            }

            // ── Upload file (chrome-devtools-mcp style) ───────────────
            "upload_file" => {
                let tab = CdpEngine::get_or_create_tab(state)?;
                let file_path = args
                    .get("filePath")
                    .and_then(|v| v.as_str())
                    .ok_or("browser upload_file: missing 'filePath'.")?;
                let validated_path = actions::validate_upload_path(file_path)?;
                let validated_str = validated_path.to_string_lossy();
                let el = CdpEngine::find_element(&tab, args)?;
                el.set_input_files(&[validated_str.as_ref()])
                    .map_err(|e| format!("browser upload_file: {e}."))?;
                let include_snapshot = args
                    .get("includeSnapshot")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if include_snapshot {
                    let snap = CdpEngine::build_a11y_snapshot(&tab, false)?;
                    Ok(
                        serde_json::json!({ "ok": true, "filePath": file_path, "snapshot": snap })
                            .to_string(),
                    )
                } else {
                    Ok(serde_json::json!({ "ok": true, "filePath": file_path }).to_string())
                }
            }

            // ── Get single console message by index ──────────────────
            "get_console_message" => {
                let tab = CdpEngine::get_or_create_tab(state)?;
                let msgid = args
                    .get("msgid")
                    .and_then(|v| v.as_u64())
                    .ok_or("browser get_console_message: missing 'msgid'.")?;
                let script = format!(
                    "(() => {{ const msgs = window.__fc_console_msgs || []; \
                     const m = msgs[{msgid}]; \
                     if (!m) return JSON.stringify({{error: 'message not found'}}); \
                     return JSON.stringify(m); }})()"
                );
                let result = tab
                    .evaluate(&script, false)
                    .map_err(|e| format!("browser get_console_message: {e}"))?;
                let val = result
                    .value
                    .as_ref()
                    .and_then(|v| v.as_str())
                    .unwrap_or("{}");
                Ok(val.to_string())
            }

            // ── Get single network request by index ──────────────────
            "get_network_request" => {
                let tab = CdpEngine::get_or_create_tab(state)?;
                let reqid = args
                    .get("reqid")
                    .and_then(|v| v.as_u64())
                    .ok_or("browser get_network_request: missing 'reqid'.")?;
                let script = format!(
                    "(() => {{ const reqs = window.__fc_network_reqs || []; \
                     const r = reqs[{reqid}]; \
                     if (!r) return JSON.stringify({{error: 'request not found'}}); \
                     return JSON.stringify(r); }})()"
                );
                let result = tab
                    .evaluate(&script, false)
                    .map_err(|e| format!("browser get_network_request: {e}"))?;
                let val = result
                    .value
                    .as_ref()
                    .and_then(|v| v.as_str())
                    .unwrap_or("{}");
                Ok(val.to_string())
            }

            // ── Cookies ────────────────────────────────────────────────
            "cookies" => {
                let tab = CdpEngine::get_or_create_tab(state)?;
                let op = args
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("get");
                match op {
                    "get" => {
                        let cookies = tab
                            .get_cookies()
                            .map_err(|e| format!("browser cookies get: {e}."))?;
                        let list = serde_json::to_value(&cookies).unwrap_or_default();
                        Ok(serde_json::json!({ "ok": true, "cookies": list }).to_string())
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
                        .map_err(|e| format!("browser cookies set: {e}."))?;
                        Ok(serde_json::json!({ "ok": true, "operation": "set", "cookie_name": name }).to_string())
                    }
                    "delete" => {
                        let name = args
                            .get("cookie_name")
                            .and_then(|v| v.as_str())
                            .ok_or("missing cookie_name")?;
                        tab.delete_cookies(vec![Network::DeleteCookies {
                            name: name.to_string(),
                            url: None,
                            domain: None,
                            path: None,
                            partition_key: None,
                        }])
                        .map_err(|e| format!("browser cookies delete: {e}."))?;
                        Ok(serde_json::json!({ "ok": true, "operation": "delete", "cookie_name": name }).to_string())
                    }
                    "clear" => {
                        let cookies = tab
                            .get_cookies()
                            .map_err(|e| format!("browser cookies clear: {e}."))?;
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
                            tab.delete_cookies(dels)
                                .map_err(|e| format!("browser cookies clear: {e}."))?;
                        }
                        Ok(
                            serde_json::json!({ "ok": true, "operation": "clear", "deleted": n })
                                .to_string(),
                        )
                    }
                    other => Err(format!("browser cookies: unknown operation '{other}'.")),
                }
            }

            other => Err(format!("browser: unhandled action '{other}'.")),
        }
    }
}

#[async_trait]
impl BrowserEngine for CdpEngine {
    fn engine_type(&self) -> &str { "cdp" }

    fn shutdown_sync(&self) {
        CdpEngine::shutdown_sync(self);
    }

    async fn execute_action(
        &self,
        action: &str,
        args: &serde_json::Value,
    ) -> Result<EngineActionResult, String> {
        let inner = self.inner.clone();
        let action = action.to_string();
        let args = args.clone();
        tokio::task::spawn_blocking(move || {
            CdpEngine::ensure_browser(&inner)?;
            CdpEngine::run_action(&inner, &action, &args).map(|r| EngineActionResult {
                text: r.text,
                images: r.images,
            })
        })
        .await
        .map_err(|e| format!("browser cdp: worker task panicked: {e}"))?
    }
}
