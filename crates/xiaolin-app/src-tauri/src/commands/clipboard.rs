use base64::Engine;
use std::path::Path;
use tokio::process::Command;

/// Read an image from the system clipboard via shell tools (wl-paste / xclip).
/// Returns base64-encoded PNG data, or null if no image found.
#[tauri::command]
pub async fn clipboard_read_image() -> Result<Option<String>, String> {
    // Try wl-paste first (Wayland)
    if let Ok(output) = Command::new("wl-paste")
        .args(["--type", "image/png"])
        .output()
        .await
    {
        if output.status.success() && !output.stdout.is_empty() {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&output.stdout);
            return Ok(Some(b64));
        }
    }

    // Fallback to xclip (X11)
    if let Ok(output) = Command::new("xclip")
        .args(["-selection", "clipboard", "-t", "image/png", "-o"])
        .output()
        .await
    {
        if output.status.success() && !output.stdout.is_empty() {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&output.stdout);
            return Ok(Some(b64));
        }
    }

    Ok(None)
}

/// Read an image file from a local path. Returns base64-encoded data with its MIME type.
#[tauri::command]
pub async fn read_image_file(path: String) -> Result<(String, String), String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(format!("File not found: {path}"));
    }

    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    };

    let bytes = tokio::fs::read(p)
        .await
        .map_err(|e| format!("Failed to read {path}: {e}"))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok((b64, mime.to_string()))
}
