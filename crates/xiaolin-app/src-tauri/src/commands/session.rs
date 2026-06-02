use serde_json::json;
use tauri_plugin_dialog::DialogExt;

/// Export session content to a local file.
///
/// In the unified WebSocket architecture, the frontend fetches session data
/// via WebSocket and passes it to this IPC command for local file export.
/// This separates concerns: Gateway handles business logic, Tauri handles
/// local file operations.
#[tauri::command]
pub async fn export_session_content(
    app: tauri::AppHandle,
    content: String,
    filename: String,
    mime_type: String,
) -> Result<serde_json::Value, String> {
    // Open file save dialog
    let Some(file_path) = app
        .dialog()
        .file()
        .add_filter("Export", &[mime_type.split('/').nth(1).unwrap_or("*")])
        .set_file_name(&filename)
        .blocking_save_file()
    else {
        return Err("cancelled".into());
    };

    // Convert FilePath to PathBuf
    let path_str = file_path.to_string();
    let path = std::path::Path::new(&path_str);

    // Write content to file using std::fs
    std::fs::write(path, &content)
        .map_err(|e| format!("failed to write file: {e}"))?;

    Ok(json!({
        "success": true,
        "path": path_str
    }))
}