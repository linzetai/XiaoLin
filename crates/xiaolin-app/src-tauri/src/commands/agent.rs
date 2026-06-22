use serde_json::json;
use std::path::PathBuf;

const MAX_AVATAR_BYTES: u64 = 10 * 1024 * 1024;
const ALLOWED_AVATAR_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];

fn config_mode() -> xiaolin_core::config::ConfigMode {
    crate::resolve_config_mode()
}

/// Get the state directory for the current config mode.
fn state_dir() -> PathBuf {
    xiaolin_core::config::state_dir(&config_mode())
}

/// Upload an agent avatar image.
///
/// This is a local file operation - copies the selected image to the
/// avatars directory and updates the agent config.
#[tauri::command]
pub async fn upload_agent_avatar(
    agent_id: String,
    source_path: String,
) -> Result<serde_json::Value, String> {
    // Validate agent ID to prevent path traversal
    if agent_id.contains(|c: char| !c.is_alphanumeric() && c != '-' && c != '_') {
        return Err("invalid agent ID".into());
    }

    let src = std::path::Path::new(&source_path);
    if !src.exists() {
        return Err(format!("source file not found: {source_path}"));
    }

    let meta = tokio::fs::metadata(src)
        .await
        .map_err(|e| format!("read source metadata: {e}"))?;
    if meta.len() > MAX_AVATAR_BYTES {
        return Err(format!(
            "avatar file too large ({} bytes); maximum is {} MB",
            meta.len(),
            MAX_AVATAR_BYTES / (1024 * 1024)
        ));
    }

    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !ALLOWED_AVATAR_EXTS.contains(&ext.as_str()) {
        return Err(format!(
            "unsupported avatar extension '.{ext}'; allowed: {}",
            ALLOWED_AVATAR_EXTS.join(", ")
        ));
    }

    let sd = state_dir();
    let avatars_dir = sd.join("avatars");
    tokio::fs::create_dir_all(&avatars_dir)
        .await
        .map_err(|e| format!("create avatars dir: {e}"))?;

    let dest = avatars_dir.join(format!("{agent_id}.{ext}"));
    tokio::fs::copy(src, &dest)
        .await
        .map_err(|e| format!("copy avatar: {e}"))?;

    let dest_str = dest.to_string_lossy().to_string();

    // Update agent config with avatar path
    let agents_dir = sd.join("config/agents");
    let cfg_path = agents_dir.join(format!("{agent_id}.json"));
    if cfg_path.exists() {
        let bytes = tokio::fs::read(&cfg_path)
            .await
            .map_err(|e| format!("read agent config: {e}"))?;
        let mut val = serde_json::from_slice::<serde_json::Value>(&bytes)
            .map_err(|e| format!("parse agent config: {e}"))?;
        val["avatar"] = json!(dest_str);
        let out =
            serde_json::to_vec_pretty(&val).map_err(|e| format!("serialize agent config: {e}"))?;
        tokio::fs::write(&cfg_path, out)
            .await
            .map_err(|e| format!("write agent config: {e}"))?;
    }

    Ok(json!({ "ok": true, "path": dest_str }))
}

/// Read identity files (SOUL.md, USER.md, etc.) for an agent.
///
/// These are local files in the agent's workspace directory.
#[tauri::command]
pub async fn read_identity_files(agent_id: String) -> Result<serde_json::Value, String> {
    // Validate agent ID
    if agent_id.contains(|c: char| !c.is_alphanumeric() && c != '-' && c != '_') {
        return Err("invalid agent ID".into());
    }

    let sd = state_dir();
    let ws_root = xiaolin_core::workspace::resolve_workspace_root(&sd, &agent_id, None);
    let ws = xiaolin_core::workspace::AgentWorkspace::new(&ws_root, &agent_id);
    let _ = ws.ensure_workspace();

    let read = |name: &str| -> serde_json::Value {
        let p = ws_root.join(name);
        match std::fs::read_to_string(&p) {
            Ok(s) if !s.trim().is_empty() => json!(s),
            _ => serde_json::Value::Null,
        }
    };

    Ok(json!({
        "soul": read("SOUL.md"),
        "identity": read("IDENTITY.md"),
        "user": read("USER.md"),
        "agents": read("AGENTS.md"),
        "tools": read("TOOLS.md"),
    }))
}