use arboard::Clipboard;
use base64::Engine;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const ALLOWED_IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"];
const MAX_IMAGE_FILE_BYTES: u64 = 20 * 1024 * 1024;

/// Maximum decoded PNG payload size for clipboard writes.
const MAX_CLIPBOARD_PNG_BYTES: usize = 50 * 1024 * 1024;
/// Base64 expands ~4/3; reject oversize input before decoding.
const MAX_CLIPBOARD_B64_LEN: usize = MAX_CLIPBOARD_PNG_BYTES * 4 / 3 + 4;

fn allowed_image_roots() -> Vec<PathBuf> {
    let mut roots = vec![std::env::temp_dir()];
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Pictures"));
        roots.push(home.join("Downloads"));
    }
    roots
}

fn is_path_under_allowed_roots(path: &Path) -> bool {
    allowed_image_roots().iter().any(|root| {
        root.canonicalize()
            .ok()
            .is_some_and(|canonical_root| path.starts_with(&canonical_root))
    })
}

pub struct ClipboardState(pub Mutex<Option<Clipboard>>);

fn with_clipboard<F, R>(state: &tauri::State<'_, ClipboardState>, f: F) -> Result<R, String>
where
    F: FnOnce(&mut Clipboard) -> Result<R, String>,
{
    let mut guard = state
        .0
        .lock()
        .map_err(|_| "clipboard lock poisoned".to_string())?;
    if guard.is_none() {
        *guard = Some(Clipboard::new().map_err(|e| {
            tracing::warn!(error = %e, "failed to access clipboard");
            String::from("clipboard unavailable")
        })?);
    }
    let cb = guard.as_mut().expect("clipboard initialized above");
    f(cb)
}

#[tauri::command]
pub fn clipboard_read_text(
    state: tauri::State<'_, ClipboardState>,
) -> Result<Option<String>, String> {
    with_clipboard(&state, |cb| match cb.get_text() {
        Ok(text) if !text.is_empty() => Ok(Some(text)),
        Ok(_) => Ok(None),
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(e) => {
            tracing::warn!(error = %e, "failed to read clipboard text");
            Err("failed to read clipboard".into())
        }
    })
}

#[tauri::command]
pub fn clipboard_write_text(
    text: String,
    state: tauri::State<'_, ClipboardState>,
) -> Result<(), String> {
    with_clipboard(&state, |cb| {
        cb.set_text(&text).map_err(|e| {
            tracing::warn!(error = %e, "failed to write clipboard text");
            String::from("failed to write clipboard")
        })
    })
}

#[tauri::command]
pub fn clipboard_read_image(
    state: tauri::State<'_, ClipboardState>,
) -> Result<Option<String>, String> {
    with_clipboard(&state, |cb| match cb.get_image() {
        Ok(img) => {
            if img.bytes.len() > MAX_CLIPBOARD_PNG_BYTES {
                tracing::warn!(
                    bytes = img.bytes.len(),
                    width = img.width,
                    height = img.height,
                    "clipboard image exceeds size limit"
                );
                return Err("clipboard image too large".into());
            }
            let png_data = encode_rgba_to_png(&img.bytes, img.width as u32, img.height as u32)?;
            let b64 = base64::engine::general_purpose::STANDARD.encode(&png_data);
            Ok(Some(b64))
        }
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(e) => {
            tracing::warn!(error = %e, "failed to read clipboard image");
            Err("failed to read clipboard".into())
        }
    })
}

fn encode_rgba_to_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut buf, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| {
            tracing::warn!(error = %e, "clipboard PNG encode header failed");
            String::from("failed to encode image")
        })?;
        writer.write_image_data(rgba).map_err(|e| {
            tracing::warn!(error = %e, "clipboard PNG encode write failed");
            String::from("failed to encode image")
        })?;
    }
    Ok(buf)
}

#[tauri::command]
pub fn clipboard_write_image(
    base64_png: String,
    state: tauri::State<'_, ClipboardState>,
) -> Result<(), String> {
    if base64_png.len() > MAX_CLIPBOARD_B64_LEN {
        return Err(format!(
            "PNG too large: base64 payload exceeds {} MB limit",
            MAX_CLIPBOARD_PNG_BYTES / (1024 * 1024)
        ));
    }

    let png_data = base64::engine::general_purpose::STANDARD
        .decode(&base64_png)
        .map_err(|e| format!("Invalid base64: {e}"))?;

    if png_data.len() > MAX_CLIPBOARD_PNG_BYTES {
        return Err(format!(
            "PNG too large: decoded size {} bytes exceeds {} MB limit",
            png_data.len(),
            MAX_CLIPBOARD_PNG_BYTES / (1024 * 1024)
        ));
    }

    let decoder = png::Decoder::new(std::io::Cursor::new(&png_data));
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("Invalid PNG: {e}"))?;
    let mut img_buf = vec![0u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut img_buf)
        .map_err(|e| format!("PNG decode error: {e}"))?;
    img_buf.truncate(info.buffer_size());

    let rgba_data = match info.color_type {
        png::ColorType::Rgba => img_buf,
        png::ColorType::Rgb => {
            let mut rgba = Vec::with_capacity((info.width * info.height * 4) as usize);
            for chunk in img_buf.chunks(3) {
                rgba.extend_from_slice(chunk);
                rgba.push(255);
            }
            rgba
        }
        _ => return Err(format!("Unsupported PNG color type: {:?}", info.color_type)),
    };

    let img = arboard::ImageData {
        width: info.width as usize,
        height: info.height as usize,
        bytes: std::borrow::Cow::Owned(rgba_data),
    };

    with_clipboard(&state, |cb| {
        cb.set_image(img).map_err(|e| {
            tracing::warn!(error = %e, "failed to write clipboard image");
            String::from("failed to write clipboard")
        })
    })
}

#[tauri::command]
pub async fn read_image_file(path: String) -> Result<(String, String), String> {
    let p = Path::new(&path);
    if !p.exists() {
        tracing::warn!(path = %path, "read_image_file: file not found");
        return Err("file not found".into());
    }

    let canonical = std::fs::canonicalize(p).map_err(|e| {
        tracing::warn!(path = %path, error = %e, "read_image_file: invalid path");
        String::from("invalid path")
    })?;

    let ext = canonical
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !ALLOWED_IMAGE_EXTS.contains(&ext.as_str()) {
        tracing::warn!(extension = %ext, "read_image_file: unsupported image extension");
        return Err("unsupported file type".into());
    }
    if !is_path_under_allowed_roots(&canonical) {
        tracing::warn!(path = %canonical.display(), "read_image_file: path not under allowed roots");
        return Err("invalid path".into());
    }

    let meta = tokio::fs::metadata(&canonical).await.map_err(|e| {
        tracing::warn!(error = %e, "read_image_file: failed to read metadata");
        String::from("operation failed")
    })?;
    if meta.len() > MAX_IMAGE_FILE_BYTES {
        tracing::warn!(size = meta.len(), "read_image_file: file too large");
        return Err("file too large".into());
    }

    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        _ => unreachable!(),
    };

    let bytes = tokio::fs::read(&canonical).await.map_err(|e| {
        tracing::warn!(error = %e, "read_image_file: failed to read file");
        String::from("operation failed")
    })?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok((b64, mime.to_string()))
}
