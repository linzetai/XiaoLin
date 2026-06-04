use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use xiaolin_core::tool::{Tool, ToolImage, ToolKind, ToolParameterSchema, ToolResult};

/// Read current clipboard contents (text or image).
pub struct ClipboardReadTool;

/// Write text or image data to the system clipboard.
pub struct ClipboardWriteTool;

#[async_trait]
impl Tool for ClipboardReadTool {
    fn name(&self) -> &str {
        "clipboard_read"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }

    fn supports_parallel(&self) -> bool {
        true
    }

    fn search_hint(&self) -> &str {
        "clipboard read paste text image copy system"
    }

    fn description(&self) -> &str {
        "Read the current system clipboard contents. Returns text content if available, \
         or an image (as PNG) if the clipboard contains an image.\n\n\
         ## Parameters\n\
         | param | type | description |\n\
         |-------|------|-------------|\n\
         | format | string | \"text\" (default), \"image\", or \"auto\" (try text then image) |"
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "format".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["text", "image", "auto"],
                "description": "Content type to read. 'auto' tries text first, then image."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec![],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = serde_json::from_str(arguments).unwrap_or_default();
        let format = args
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("auto")
            .to_string();

        let result = tokio::task::spawn_blocking(move || read_clipboard(&format)).await;

        match result {
            Ok(Ok(content)) => content,
            Ok(Err(e)) => ToolResult::err(e),
            Err(e) => ToolResult::err(format!("clipboard_read: task panicked: {e}")),
        }
    }
}

fn read_clipboard(format: &str) -> Result<ToolResult, String> {
    let mut cb =
        arboard::Clipboard::new().map_err(|e| format!("Failed to access clipboard: {e}"))?;

    match format {
        "text" => read_text(&mut cb),
        "image" => read_image(&mut cb),
        _ => {
            if let Ok(result) = read_text(&mut cb) {
                if result.success {
                    return Ok(result);
                }
            }
            read_image(&mut cb)
        }
    }
}

fn read_text(cb: &mut arboard::Clipboard) -> Result<ToolResult, String> {
    match cb.get_text() {
        Ok(text) if !text.is_empty() => Ok(ToolResult::ok(format!(
            "Clipboard text ({} chars):\n{text}",
            text.len()
        ))),
        Ok(_) => Ok(ToolResult::ok("Clipboard is empty (no text content).")),
        Err(arboard::Error::ContentNotAvailable) => {
            Ok(ToolResult::ok("Clipboard has no text content."))
        }
        Err(e) => Err(format!("Failed to read clipboard text: {e}")),
    }
}

fn read_image(cb: &mut arboard::Clipboard) -> Result<ToolResult, String> {
    match cb.get_image() {
        Ok(img) => {
            let png_data = encode_rgba_to_png(&img.bytes, img.width as u32, img.height as u32)?;
            let size_kb = png_data.len() / 1024;
            Ok(ToolResult::ok_with_images(
                format!(
                    "Clipboard image: {}x{} ({size_kb} KB PNG)",
                    img.width, img.height
                ),
                vec![ToolImage {
                    mime_type: "image/png".into(),
                    data: png_data,
                }],
            ))
        }
        Err(arboard::Error::ContentNotAvailable) => {
            Ok(ToolResult::ok("Clipboard has no image content."))
        }
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

#[async_trait]
impl Tool for ClipboardWriteTool {
    fn name(&self) -> &str {
        "clipboard_write"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }

    fn supports_parallel(&self) -> bool {
        false
    }

    fn search_hint(&self) -> &str {
        "clipboard write copy text system paste"
    }

    fn description(&self) -> &str {
        "Write text to the system clipboard, making it available for pasting in other applications.\n\n\
         ## Parameters\n\
         | param | type | required | description |\n\
         |-------|------|----------|-------------|\n\
         | text | string | yes | The text content to write to the clipboard |"
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "text".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Text content to write to the system clipboard."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["text".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("clipboard_write: invalid JSON: {e}")),
        };

        let text = match args.get("text").and_then(|v| v.as_str()) {
            Some(t) => t.to_string(),
            None => return ToolResult::err("clipboard_write: 'text' parameter is required"),
        };

        let text_len = text.len();
        let result = tokio::task::spawn_blocking(move || {
            let mut cb = arboard::Clipboard::new()
                .map_err(|e| format!("Failed to access clipboard: {e}"))?;
            cb.set_text(&text)
                .map_err(|e| format!("Failed to write to clipboard: {e}"))
        })
        .await;

        match result {
            Ok(Ok(())) => ToolResult::ok(format!(
                "Successfully wrote {text_len} chars to the system clipboard."
            )),
            Ok(Err(e)) => ToolResult::err(e),
            Err(e) => ToolResult::err(format!("clipboard_write: task panicked: {e}")),
        }
    }
}

pub fn register_clipboard_tools(registry: &xiaolin_core::tool::ToolRegistry) {
    registry.register_deferred(Arc::new(ClipboardReadTool));
    registry.register_deferred(Arc::new(ClipboardWriteTool));
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaolin_core::tool::Tool;

    #[test]
    fn clipboard_read_metadata() {
        let tool = ClipboardReadTool;
        assert_eq!(tool.name(), "clipboard_read");
        assert_eq!(tool.kind(), ToolKind::Read);
        let schema = tool.parameters_schema();
        assert!(schema.properties.contains_key("format"));
    }

    #[test]
    fn clipboard_write_metadata() {
        let tool = ClipboardWriteTool;
        assert_eq!(tool.name(), "clipboard_write");
        assert_eq!(tool.kind(), ToolKind::Edit);
        let schema = tool.parameters_schema();
        assert!(schema.properties.contains_key("text"));
        assert!(schema.required.contains(&"text".to_string()));
    }

    #[tokio::test]
    async fn clipboard_write_rejects_missing_text() {
        let tool = ClipboardWriteTool;
        let result = tool.execute(r#"{}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("text"));
    }
}
