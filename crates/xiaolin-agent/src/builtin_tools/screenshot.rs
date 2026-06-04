use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use xiaolin_core::tool::{Tool, ToolImage, ToolKind, ToolParameterSchema, ToolResult};

/// Desktop screenshot tool for capturing the system screen, a specific window,
/// or a rectangular region. Returns the image to the LLM as a multimodal
/// content part for visual understanding — the foundation of "computer use".
///
/// Platform backends (in priority order):
///   Linux  — gnome-screenshot, scrot, maim, import (ImageMagick)
///   macOS  — screencapture (built-in)
///   Windows — PowerShell [System.Windows.Forms.Screen] + System.Drawing
pub struct ScreenshotTool;

impl Default for ScreenshotTool {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenshotTool {
    pub fn new() -> Self {
        Self
    }
}

// ── Capture mode parsed from tool arguments ─────────────────────────────────

enum CaptureMode {
    FullScreen,
    ActiveWindow,
    Region {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
}

struct CaptureRequest {
    mode: CaptureMode,
    delay_secs: u32,
    #[allow(dead_code)]
    display: Option<u32>,
    ocr: bool,
}

fn parse_request(args: &serde_json::Value) -> Result<CaptureRequest, String> {
    let mode_str = args
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("screen");

    let mode = match mode_str {
        "screen" | "full" | "fullscreen" => CaptureMode::FullScreen,
        "window" | "active_window" => CaptureMode::ActiveWindow,
        "region" => {
            let x = args.get("x").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let y = args.get("y").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let width = args
                .get("width")
                .and_then(|v| v.as_u64())
                .ok_or("screenshot region: missing 'width'")? as u32;
            let height = args
                .get("height")
                .and_then(|v| v.as_u64())
                .ok_or("screenshot region: missing 'height'")? as u32;
            if width == 0 || height == 0 {
                return Err("screenshot region: width and height must be > 0".into());
            }
            CaptureMode::Region {
                x,
                y,
                width,
                height,
            }
        }
        other => {
            return Err(format!(
                "screenshot: unknown mode '{other}'. Use 'screen', 'window', or 'region'."
            ))
        }
    };

    let delay_secs = args.get("delay").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

    let display = args
        .get("display")
        .and_then(|v| v.as_u64())
        .map(|d| d as u32);

    let ocr = args.get("ocr").and_then(|v| v.as_bool()).unwrap_or(false);

    Ok(CaptureRequest {
        mode,
        delay_secs,
        display,
        ocr,
    })
}

// ── Platform-specific capture implementations ───────────────────────────────

fn tmp_path() -> PathBuf {
    let name = format!("xiaolin-screenshot-{}.png", std::process::id());
    std::env::temp_dir().join(name)
}

fn which(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn run_capture(cmd: &str, args: &[&str]) -> Result<(), String> {
    let output = std::process::Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("screenshot: failed to execute '{cmd}': {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "screenshot: '{cmd}' exited with {}: {stderr}",
            output.status
        ));
    }
    Ok(())
}

fn apply_delay(secs: u32) {
    if secs > 0 {
        std::thread::sleep(std::time::Duration::from_secs(secs as u64));
    }
}

#[cfg(target_os = "linux")]
fn capture_linux(req: &CaptureRequest, out: &Path) -> Result<(), String> {
    let out_str = out.to_string_lossy();
    apply_delay(req.delay_secs);

    // Try gnome-screenshot first (works on both X11 and Wayland/GNOME)
    if which("gnome-screenshot") {
        let mut args: Vec<&str> = vec!["-f", &out_str];
        match &req.mode {
            CaptureMode::FullScreen => {}
            CaptureMode::ActiveWindow => args.push("-w"),
            CaptureMode::Region { .. } => {
                // gnome-screenshot doesn't support arbitrary regions well;
                // fall through to other tools
                if which("maim") || which("scrot") || which("import") {
                    // let fallback handle it
                } else {
                    // no fallback, use gnome-screenshot interactive area mode
                    args.push("-a");
                }
            }
        }
        if (!matches!(&req.mode, CaptureMode::Region { .. })
            || (!which("maim") && !which("scrot") && !which("import")))
            && run_capture("gnome-screenshot", &args).is_ok()
            && out.exists()
        {
            return Ok(());
        }
    }

    // Try grim (Wayland-native, sway/wlroots)
    if which("grim") {
        match &req.mode {
            CaptureMode::FullScreen => {
                if run_capture("grim", &[out_str.as_ref()]).is_ok() && out.exists() {
                    return Ok(());
                }
            }
            CaptureMode::Region {
                x,
                y,
                width,
                height,
            } => {
                let geom = format!("{x},{y} {width}x{height}");
                if run_capture("grim", &["-g", &geom, &out_str]).is_ok() && out.exists() {
                    return Ok(());
                }
            }
            CaptureMode::ActiveWindow => {
                // grim + swaymsg/jq for focused window geometry
                if which("swaymsg") {
                    let jq_filter =
                        r#".. | select(.focused?) | .rect | "\(.x),\(.y) \(.width)x\(.height)"#;
                    let geom_output = std::process::Command::new("sh")
                        .args(["-c", &format!("swaymsg -t get_tree | jq -r '{jq_filter}'")])
                        .output();
                    if let Ok(o) = geom_output {
                        let geom = String::from_utf8_lossy(&o.stdout).trim().to_string();
                        if !geom.is_empty()
                            && run_capture("grim", &["-g", &geom, &out_str]).is_ok()
                            && out.exists()
                        {
                            return Ok(());
                        }
                    }
                }
                // fallback: capture full screen
                if run_capture("grim", &[out_str.as_ref()]).is_ok() && out.exists() {
                    return Ok(());
                }
            }
        }
    }

    // Try scrot (X11, widely available)
    if which("scrot") {
        let mut args: Vec<String> = vec![out_str.to_string()];
        match &req.mode {
            CaptureMode::FullScreen => {}
            CaptureMode::ActiveWindow => {
                args.insert(0, "-u".to_string());
            }
            CaptureMode::Region {
                x,
                y,
                width,
                height,
            } => {
                args.insert(0, "-a".to_string());
                args.insert(1, format!("{x},{y},{width},{height}"));
            }
        }
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        if run_capture("scrot", &arg_refs).is_ok() && out.exists() {
            return Ok(());
        }
    }

    // Try maim (X11, modern replacement for scrot)
    if which("maim") {
        match &req.mode {
            CaptureMode::FullScreen => {
                if run_capture("maim", &[&out_str]).is_ok() && out.exists() {
                    return Ok(());
                }
            }
            CaptureMode::ActiveWindow => {
                // maim -i $(xdotool getactivewindow)
                if which("xdotool") {
                    let wid = std::process::Command::new("xdotool")
                        .arg("getactivewindow")
                        .output()
                        .ok()
                        .and_then(|o| {
                            String::from_utf8_lossy(&o.stdout)
                                .trim()
                                .parse::<u64>()
                                .ok()
                        });
                    if let Some(wid) = wid {
                        let wid_s = wid.to_string();
                        if run_capture("maim", &["-i", &wid_s, &out_str]).is_ok() && out.exists() {
                            return Ok(());
                        }
                    }
                }
                // fallback to full screen
                if run_capture("maim", &[&out_str]).is_ok() && out.exists() {
                    return Ok(());
                }
            }
            CaptureMode::Region {
                x,
                y,
                width,
                height,
            } => {
                let geom = format!("{width},{height},{x},{y}");
                if run_capture("maim", &["-g", &geom, &out_str]).is_ok() && out.exists() {
                    return Ok(());
                }
            }
        }
    }

    // Try import from ImageMagick (X11)
    if which("import") {
        match &req.mode {
            CaptureMode::FullScreen => {
                if run_capture("import", &["-window", "root", &out_str]).is_ok() && out.exists() {
                    return Ok(());
                }
            }
            CaptureMode::ActiveWindow => {
                if run_capture("import", &[&out_str]).is_ok() && out.exists() {
                    return Ok(());
                }
            }
            CaptureMode::Region {
                x,
                y,
                width,
                height,
            } => {
                let geom = format!("{width}x{height}+{x}+{y}");
                if run_capture("import", &["-window", "root", "-crop", &geom, &out_str]).is_ok()
                    && out.exists()
                {
                    return Ok(());
                }
            }
        }
    }

    Err(
        "screenshot: no supported screenshot tool found on this Linux system. \
         Install one of: gnome-screenshot, grim (Wayland), scrot, maim, or ImageMagick (import)."
            .to_string(),
    )
}

#[cfg(target_os = "macos")]
fn capture_macos(req: &CaptureRequest, out: &Path) -> Result<(), String> {
    let out_str = out.to_string_lossy();
    let mut args: Vec<String> = Vec::new();

    match &req.mode {
        CaptureMode::FullScreen => {}
        CaptureMode::ActiveWindow => {
            args.push("-w".to_string());
            args.push("-l".to_string());
            // capture front window — use AppleScript to find it
            let wid = std::process::Command::new("osascript")
                .args([
                    "-e",
                    r#"tell application "System Events" to set wid to id of first window of (first process whose frontmost is true)"#,
                ])
                .output()
                .ok()
                .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u64>().ok());
            if let Some(wid) = wid {
                args.push(wid.to_string());
            } else {
                args.clear();
                args.push("-w".to_string());
            }
        }
        CaptureMode::Region {
            x,
            y,
            width,
            height,
        } => {
            args.push("-R".to_string());
            args.push(format!("{x},{y},{width},{height}"));
        }
    }

    if req.delay_secs > 0 {
        args.push("-T".to_string());
        args.push(req.delay_secs.to_string());
    }

    args.push(out_str.to_string());

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run_capture("screencapture", &arg_refs)?;

    if !out.exists() {
        return Err("screenshot: screencapture produced no output file".to_string());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn capture_windows(req: &CaptureRequest, out: &Path) -> Result<(), String> {
    let out_str = out.to_string_lossy().replace('/', "\\");
    apply_delay(req.delay_secs);

    let ps_script = match &req.mode {
        CaptureMode::FullScreen => format!(
            r#"
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
$bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$bmp = New-Object System.Drawing.Bitmap($bounds.Width, $bounds.Height)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
$bmp.Save('{out_str}', [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
"#
        ),
        CaptureMode::ActiveWindow => format!(
            r#"
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Win32 {{
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
    [StructLayout(LayoutKind.Sequential)] public struct RECT {{ public int Left, Top, Right, Bottom; }}
}}
"@
$hwnd = [Win32]::GetForegroundWindow()
$rect = New-Object Win32+RECT
[Win32]::GetWindowRect($hwnd, [ref]$rect) | Out-Null
$w = $rect.Right - $rect.Left; $h = $rect.Bottom - $rect.Top
$bmp = New-Object System.Drawing.Bitmap($w, $h)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($rect.Left, $rect.Top, 0, 0, (New-Object System.Drawing.Size($w,$h)))
$bmp.Save('{out_str}', [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
"#
        ),
        CaptureMode::Region {
            x,
            y,
            width,
            height,
        } => format!(
            r#"
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
$bmp = New-Object System.Drawing.Bitmap({width}, {height})
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen({x}, {y}, 0, 0, (New-Object System.Drawing.Size({width},{height})))
$bmp.Save('{out_str}', [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
"#
        ),
    };

    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps_script])
        .output()
        .map_err(|e| format!("screenshot: PowerShell exec failed: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("screenshot: PowerShell error: {stderr}"));
    }
    if !out.exists() {
        return Err("screenshot: PowerShell produced no output file".to_string());
    }
    Ok(())
}

fn capture(req: &CaptureRequest, out: &Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        capture_linux(req, out)
    }
    #[cfg(target_os = "macos")]
    {
        return capture_macos(req, out);
    }
    #[cfg(target_os = "windows")]
    {
        return capture_windows(req, out);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Err("screenshot: unsupported platform".to_string())
    }
}

fn detect_available_backends() -> Vec<&'static str> {
    let mut found = Vec::new();
    #[cfg(target_os = "linux")]
    {
        for cmd in &["gnome-screenshot", "grim", "scrot", "maim", "import"] {
            if which(cmd) {
                found.push(*cmd);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        found.push("screencapture");
    }
    #[cfg(target_os = "windows")]
    {
        found.push("powershell");
    }
    found
}

// ── Tool trait implementation ───────────────────────────────────────────────

#[async_trait]
impl Tool for ScreenshotTool {
    fn name(&self) -> &str {
        "screenshot"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }

    fn supports_parallel(&self) -> bool {
        true
    }

    fn search_hint(&self) -> &str {
        "screenshot screen capture desktop window region visual computer use"
    }

    fn description(&self) -> &str {
        "Capture a screenshot of the desktop, the active window, or a specific screen region. \
         Returns the image directly to you for visual analysis — use this to see what's on screen, \
         verify UI state, read error dialogs, or understand the current desktop context.\n\n\
         ## Modes\n\
         - **screen** (default): Capture the entire screen\n\
         - **window**: Capture the currently active/focused window\n\
         - **region**: Capture a specific rectangle (requires x, y, width, height)\n\n\
         ## When to Use\n\
         - Before performing computer-use actions to understand current state\n\
         - After interactions to verify results\n\
         - When you need visual context about what the user sees\n\
         - To read error messages, UI elements, or application state\n\n\
         ## Parameters\n\
         | param | type | description |\n\
         |-------|------|-------------|\n\
         | mode | string | \"screen\" (default), \"window\", or \"region\" |\n\
         | x | number | Left edge for region mode |\n\
         | y | number | Top edge for region mode |\n\
         | width | number | Width for region mode |\n\
         | height | number | Height for region mode |\n\
         | delay | number | Seconds to wait before capture (0 default) |\n\
         | display | number | Monitor index for multi-display (0-based) |\n\
         | ocr | boolean | If true, run OCR on the image and include text |\n\n\
         Does NOT require a browser. Works with any application or desktop.\n\
         OCR requires tesseract to be installed (optional)."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "mode".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["screen", "window", "region"],
                "description": "Capture mode. 'screen' = full desktop, 'window' = active window, 'region' = specific rectangle."
            }),
        );
        props.insert(
            "x".to_string(),
            serde_json::json!({
                "type": "number",
                "description": "Left edge of region (pixels). Required for mode='region'."
            }),
        );
        props.insert(
            "y".to_string(),
            serde_json::json!({
                "type": "number",
                "description": "Top edge of region (pixels). Required for mode='region'."
            }),
        );
        props.insert(
            "width".to_string(),
            serde_json::json!({
                "type": "number",
                "description": "Width of region (pixels). Required for mode='region'."
            }),
        );
        props.insert(
            "height".to_string(),
            serde_json::json!({
                "type": "number",
                "description": "Height of region (pixels). Required for mode='region'."
            }),
        );
        props.insert(
            "delay".to_string(),
            serde_json::json!({
                "type": "number",
                "description": "Delay in seconds before capturing. Useful for capturing menus or tooltips."
            }),
        );
        props.insert(
            "display".to_string(),
            serde_json::json!({
                "type": "number",
                "description": "Monitor index for multi-display setups (0-based). Omit for primary display."
            }),
        );
        props.insert(
            "ocr".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "If true, run OCR on the captured image and include extracted text. Requires tesseract installed."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec![],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::err(format!(
                    "screenshot: invalid JSON: {e}. Example: {{\"mode\": \"screen\"}}"
                ))
            }
        };

        let req = match parse_request(&args) {
            Ok(r) => r,
            Err(e) => return ToolResult::err(e),
        };
        let want_ocr = req.ocr;

        let out_path = tmp_path();
        let _ = std::fs::remove_file(&out_path);

        let out_for_blocking = out_path.clone();
        let result = tokio::task::spawn_blocking(move || capture(&req, &out_for_blocking)).await;

        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return ToolResult::err(e),
            Err(e) => return ToolResult::err(format!("screenshot: capture task panicked: {e}")),
        }

        let png_data = match std::fs::read(&out_path) {
            Ok(data) => {
                let _ = std::fs::remove_file(&out_path);
                data
            }
            Err(e) => {
                return ToolResult::err(format!("screenshot: could not read captured image: {e}"))
            }
        };

        if let Some(save_path) = args.get("save_path").and_then(|v| v.as_str()) {
            if let Err(e) = std::fs::write(save_path, &png_data) {
                tracing::warn!("screenshot: save to '{save_path}' failed: {e}");
            }
        }

        let mode_label = match args
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("screen")
        {
            "window" | "active_window" => "active window",
            "region" => "region",
            _ => "full screen",
        };
        let backends = detect_available_backends();
        let size_kb = png_data.len() / 1024;

        let ocr_text = if want_ocr {
            run_ocr(&out_path, &png_data)
        } else {
            None
        };

        let mut summary = format!(
            "Screenshot captured ({mode_label}, {size_kb} KB). \
             Available backends: [{}].",
            backends.join(", ")
        );
        if let Some(ref text) = ocr_text {
            summary.push_str(&format!("\n\nOCR extracted text:\n{text}"));
        }

        ToolResult::ok_with_images(
            summary,
            vec![ToolImage {
                mime_type: "image/png".into(),
                data: png_data,
            }],
        )
    }
}

fn run_ocr(img_path: &std::path::Path, png_data: &[u8]) -> Option<String> {
    let ocr_path = std::env::temp_dir().join(format!("xiaolin-ocr-{}.png", std::process::id()));
    if std::fs::write(&ocr_path, png_data).is_err() && !img_path.exists() {
        return None;
    }
    let target = if ocr_path.exists() {
        &ocr_path
    } else {
        img_path
    };

    let output = std::process::Command::new("tesseract")
        .arg(target)
        .arg("stdout")
        .arg("-l")
        .arg("eng+chi_sim")
        .output();

    let _ = std::fs::remove_file(&ocr_path);

    match output {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
        Ok(o) => {
            tracing::debug!(
                "tesseract exited with {}: {}",
                o.status,
                String::from_utf8_lossy(&o.stderr)
            );
            None
        }
        Err(_) => {
            tracing::debug!("tesseract not found; OCR skipped");
            None
        }
    }
}

pub fn register_screenshot_tool(registry: &xiaolin_core::tool::ToolRegistry) {
    registry.register(Arc::new(ScreenshotTool::new()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaolin_core::tool::Tool;

    #[test]
    fn screenshot_tool_metadata() {
        let tool = ScreenshotTool::new();
        assert_eq!(tool.name(), "screenshot");
        assert!(!tool.description().is_empty());
        assert!(tool.description().contains("desktop"));
        assert!(tool.description().contains("window"));
        assert!(tool.description().contains("region"));
        let schema = tool.parameters_schema();
        assert_eq!(schema.schema_type, "object");
        assert!(schema.properties.contains_key("mode"));
        assert!(schema.properties.contains_key("x"));
        assert!(schema.properties.contains_key("width"));
        assert!(schema.properties.contains_key("delay"));
    }

    #[test]
    fn screenshot_tool_kind() {
        let tool = ScreenshotTool::new();
        assert_eq!(tool.kind(), ToolKind::Read);
    }

    #[test]
    fn parse_request_defaults() {
        let args = serde_json::json!({});
        let req = parse_request(&args).unwrap();
        assert!(matches!(req.mode, CaptureMode::FullScreen));
        assert_eq!(req.delay_secs, 0);
        assert!(req.display.is_none());
        assert!(!req.ocr);
    }

    #[test]
    fn parse_request_ocr_flag() {
        let args = serde_json::json!({"ocr": true});
        let req = parse_request(&args).unwrap();
        assert!(req.ocr);

        let args = serde_json::json!({"ocr": false});
        let req = parse_request(&args).unwrap();
        assert!(!req.ocr);
    }

    #[test]
    fn parse_request_window_mode() {
        let args = serde_json::json!({"mode": "window"});
        let req = parse_request(&args).unwrap();
        assert!(matches!(req.mode, CaptureMode::ActiveWindow));
    }

    #[test]
    fn parse_request_region_mode() {
        let args =
            serde_json::json!({"mode": "region", "x": 10, "y": 20, "width": 800, "height": 600});
        let req = parse_request(&args).unwrap();
        match req.mode {
            CaptureMode::Region {
                x,
                y,
                width,
                height,
            } => {
                assert_eq!((x, y, width, height), (10, 20, 800, 600));
            }
            _ => panic!("expected Region mode"),
        }
    }

    #[test]
    fn parse_request_region_missing_width() {
        let args = serde_json::json!({"mode": "region", "x": 0, "y": 0, "height": 100});
        assert!(parse_request(&args).is_err());
    }

    #[test]
    fn parse_request_region_zero_dimensions() {
        let args = serde_json::json!({"mode": "region", "x": 0, "y": 0, "width": 0, "height": 100});
        assert!(parse_request(&args).is_err());
    }

    #[test]
    fn parse_request_unknown_mode() {
        let args = serde_json::json!({"mode": "panorama"});
        assert!(parse_request(&args).is_err());
    }

    #[test]
    fn parse_request_with_delay_and_display() {
        let args = serde_json::json!({"mode": "screen", "delay": 3, "display": 1});
        let req = parse_request(&args).unwrap();
        assert_eq!(req.delay_secs, 3);
        assert_eq!(req.display, Some(1));
    }

    #[tokio::test]
    async fn screenshot_rejects_bad_json() {
        let tool = ScreenshotTool::new();
        let result = tool.execute("not json").await;
        assert!(!result.success);
        assert!(result.output.contains("invalid JSON"));
    }

    #[tokio::test]
    async fn screenshot_rejects_unknown_mode() {
        let tool = ScreenshotTool::new();
        let result = tool.execute(r#"{"mode":"panorama"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("unknown mode"));
    }

    #[test]
    fn detect_backends_returns_list() {
        let backends = detect_available_backends();
        // On CI or dev machines, at least something should be available
        // (but we don't assert non-empty since it depends on system)
        for b in &backends {
            assert!(!b.is_empty());
        }
    }

    #[test]
    fn register_adds_to_registry() {
        let registry = xiaolin_core::tool::ToolRegistry::new();
        register_screenshot_tool(&registry);
        assert!(registry.get("screenshot").is_some());
    }
}
