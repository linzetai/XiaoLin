use super::helpers::{copy_dir_recursive, get_state};
use crate::AppData;
use serde_json::json;

// ─── Skills & Tools ───

#[tauri::command]
pub async fn list_skills(
    state: tauri::State<'_, AppData>,
    agent_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let aid = agent_id.clone().unwrap_or_else(|| "main".to_string());
    tracing::info!(agent_id = %aid, "IPC list_skills called");
    let registry = app.skill_registry_for(&aid);
    let skills: Vec<serde_json::Value> = registry
        .list()
        .into_iter()
        .filter(|s| s.frontmatter.enabled.unwrap_or(true))
        .map(|s| {
            json!({
                "id": s.id,
                "name": s.name,
                "description": s.description,
                "tags": s.frontmatter.tags,
            })
        })
        .collect();
    tracing::info!(count = skills.len(), "IPC list_skills returning");
    Ok(json!({ "skills": skills, "count": skills.len() }))
}

#[tauri::command]
pub async fn refresh_skills(state: tauri::State<'_, AppData>) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let count = app.reload_skills().map_err(|e| e.to_string())?;
    Ok(json!({ "refreshed": true, "count": count }))
}

#[tauri::command]
pub async fn upload_skill(
    state: tauri::State<'_, AppData>,
    source_path: String,
) -> Result<serde_json::Value, String> {
    let src = std::path::Path::new(&source_path);
    if !src.exists() {
        return Err(format!("path does not exist: {source_path}"));
    }

    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let skills_dir = fastclaw_core::paths::resolve_skills_dir_from(Some(&app.cfg.config.paths));

    if src.is_dir() {
        let skill_md = src.join("SKILL.md");
        if !skill_md.exists() {
            return Err("selected folder does not contain a SKILL.md file".into());
        }
        let dir_name = src
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("invalid directory name")?;
        let dest = skills_dir.join(dir_name);
        if dest.exists() {
            std::fs::remove_dir_all(&dest)
                .map_err(|e| format!("failed to clean existing skill dir: {e}"))?;
        }
        copy_dir_recursive(src, &dest).map_err(|e| format!("failed to copy skill dir: {e}"))?;
        let count = app.reload_skills().map_err(|e| e.to_string())?;
        return Ok(json!({ "installed": dir_name, "count": count }));
    }

    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext == "zip" {
        let file = std::fs::File::open(src).map_err(|e| format!("failed to open zip: {e}"))?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("invalid zip: {e}"))?;
        let mut top_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut has_skill_md = false;
        for i in 0..archive.len() {
            let f = archive
                .by_index(i)
                .map_err(|e| format!("failed to read zip entry at index {i}: {e}"))?;
            let Some(enclosed) = f.enclosed_name() else {
                return Err(format!(
                    "zip contains unsafe path traversal entry: {}",
                    f.name()
                ));
            };
            if enclosed
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n == "SKILL.md")
            {
                has_skill_md = true;
            }
            if let Some(component) = enclosed.components().next() {
                top_dirs.insert(component.as_os_str().to_string_lossy().to_string());
            }
        }
        if !has_skill_md {
            return Err("zip archive does not contain a SKILL.md file".into());
        }

        let is_flat = top_dirs.len() == 1;
        let extract_to = if is_flat {
            skills_dir.clone()
        } else {
            let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("skill");
            skills_dir.join(stem)
        };
        std::fs::create_dir_all(&extract_to)
            .map_err(|e| format!("failed to create extraction dir: {e}"))?;
        for i in 0..archive.len() {
            let mut f = archive
                .by_index(i)
                .map_err(|e| format!("failed to read zip entry at index {i}: {e}"))?;
            let Some(enclosed) = f.enclosed_name().map(|p| p.to_path_buf()) else {
                return Err(format!(
                    "zip contains unsafe path traversal entry: {}",
                    f.name()
                ));
            };
            let out_path = extract_to.join(enclosed);
            if f.is_dir() {
                std::fs::create_dir_all(&out_path)
                    .map_err(|e| format!("failed to create dir during extraction: {e}"))?;
            } else {
                if let Some(parent) = out_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        format!("failed to create parent dir during extraction: {e}")
                    })?;
                }
                let mut out_file = std::fs::File::create(&out_path)
                    .map_err(|e| format!("failed to create extracted file: {e}"))?;
                std::io::copy(&mut f, &mut out_file)
                    .map_err(|e| format!("failed to write extracted file: {e}"))?;
            }
        }

        let skill_name = if is_flat {
            top_dirs.into_iter().next().unwrap_or_default()
        } else {
            src.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("skill")
                .to_string()
        };
        let count = app.reload_skills().map_err(|e| e.to_string())?;
        return Ok(json!({ "installed": skill_name, "count": count }));
    }

    Err(format!(
        "unsupported file type: .{ext} (expected a folder or .zip)"
    ))
}
