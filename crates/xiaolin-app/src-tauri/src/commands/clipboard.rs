use arboard::Clipboard;
use base64::Engine;
use std::path::Path;

/// Read text from the system clipboard.
#[tauri::command]
pub fn clipboard_read_text() -> Result<Option<String>, String> {
    let mut cb = Clipboard::new().map_err(|e| format!("Failed to access clipboard: {e}"))?;
    match cb.get_text() {
        Ok(text) if !text.is_empty() => Ok(Some(text)),
        Ok(_) => Ok(None),
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(e) => Err(format!("Failed to read clipboard text: {e}")),
    }
}

/// Write text to the system clipboard.
#[tauri::command]
pub fn clipboard_write_text(text: String) -> Result<(), String> {
    let mut cb = Clipboard::new().map_err(|e| format!("Failed to access clipboard: {e}"))?;
    cb.set_text(&text)
        .map_err(|e| format!("Failed to write clipboard text: {e}"))
}

/// Read an image from the system clipboard.
/// Returns base64-encoded PNG data, or null if no image found.
#[tauri::command]
pub fn clipboard_read_image() -> Result<Option<String>, String> {
    let mut cb = Clipboard::new().map_err(|e| format!("Failed to access clipboard: {e}"))?;
    match cb.get_image() {
        Ok(img) => {
            let png_data =
                encode_rgba_to_png(&img.bytes, img.width as u32, img.height as u32)?;
            let b64 = base64::engine::general_purpose::STANDARD.encode(&png_data);
            Ok(Some(b64))
        }
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(e) => Err(format!("Failed to read clipboard image: {e}")),
    }
}

fn encode_rgba_to_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut buf, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| format!("PNG encode error: {e}"))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| format!("PNG write error: {e}"))?;
    }
    Ok(buf)
}

/// Write a base64-encoded PNG image to the system clipboard.
#[tauri::command]
pub fn clipboard_write_image(base64_png: String) -> Result<(), String> {
    let png_data = base64::engine::general_purpose::STANDARD
        .decode(&base64_png)
        .map_err(|e| format!("Invalid base64: {e}"))?;

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

    let mut cb = Clipboard::new().map_err(|e| format!("Failed to access clipboard: {e}"))?;
    cb.set_image(img)
        .map_err(|e| format!("Failed to write clipboard image: {e}"))
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
