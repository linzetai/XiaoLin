use serde_json::json;
use std::path::{Path, PathBuf};

const MAX_SKILL_ZIP_ENTRIES: usize = 1000;
const MAX_SKILL_ZIP_TOTAL_BYTES: u64 = 100 * 1024 * 1024;

fn is_valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn ensure_within_skills_dir(skills_dir: &Path, target: &Path) -> Result<PathBuf, String> {
    let skills_canon = skills_dir
        .canonicalize()
        .map_err(|e| format!("skills directory unavailable: {e}"))?;
    let target_canon = target
        .canonicalize()
        .map_err(|e| format!("invalid destination path: {e}"))?;
    if !target_canon.starts_with(&skills_canon) {
        return Err("path escapes skills directory".into());
    }
    Ok(target_canon)
}

fn config_mode() -> xiaolin_core::config::ConfigMode {
    crate::resolve_config_mode()
}

/// Get the state directory for the current config mode.
fn state_dir() -> std::path::PathBuf {
    xiaolin_core::config::state_dir(&config_mode())
}

/// Upload/install a skill from a local folder or .zip file.
///
/// This is a local file operation - extracts the skill to the skills directory.
/// The skill registry refresh is handled by the Gateway via WebSocket.
#[tauri::command]
pub async fn upload_skill(source_path: String) -> Result<serde_json::Value, String> {
    let src = Path::new(&source_path);
    if !src.exists() {
        return Err(format!("path does not exist: {source_path}"));
    }

    let sd = state_dir();
    let skills_dir = sd.join("skills");

    if src.is_dir() {
        let skill_md = src.join("SKILL.md");
        if !skill_md.exists() {
            return Err("selected folder does not contain a SKILL.md file".into());
        }
        let dir_name = src
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("invalid directory name")?;
        if !is_valid_skill_name(dir_name) {
            return Err(format!("invalid skill directory name: {dir_name}"));
        }
        let dest = skills_dir.join(dir_name);
        if dest.exists() {
            std::fs::remove_dir_all(&dest)
                .map_err(|e| format!("failed to clean existing skill dir: {e}"))?;
        }
        copy_dir_recursive(src, &dest)
            .map_err(|e| format!("failed to copy skill dir: {e}"))?;
        ensure_within_skills_dir(&skills_dir, &dest)?;
        return Ok(json!({ "installed": dir_name }));
    }

    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext == "zip" {
        let file =
            std::fs::File::open(src).map_err(|e| format!("failed to open zip: {e}"))?;
        let mut archive =
            zip::ZipArchive::new(file).map_err(|e| format!("invalid zip: {e}"))?;
        if archive.len() > MAX_SKILL_ZIP_ENTRIES {
            return Err(format!(
                "zip archive has too many entries ({}); maximum is {MAX_SKILL_ZIP_ENTRIES}",
                archive.len()
            ));
        }
        let mut top_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut has_skill_md = false;
        let mut total_uncompressed: u64 = 0;
        for i in 0..archive.len() {
            let f = archive
                .by_index(i)
                .map_err(|e| format!("failed to read zip entry at index {i}: {e}"))?;
            total_uncompressed = total_uncompressed.saturating_add(f.size());
            if total_uncompressed > MAX_SKILL_ZIP_TOTAL_BYTES {
                return Err(format!(
                    "zip archive uncompressed size exceeds {} MB limit",
                    MAX_SKILL_ZIP_TOTAL_BYTES / (1024 * 1024)
                ));
            }
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
        for name in &top_dirs {
            if !is_valid_skill_name(name) {
                return Err(format!("invalid skill name in zip: {name}"));
            }
        }

        let is_flat = top_dirs.len() == 1;
        let extract_to = if is_flat {
            skills_dir.clone()
        } else {
            let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("skill");
            if !is_valid_skill_name(stem) {
                return Err(format!("invalid skill archive name: {stem}"));
            }
            skills_dir.join(stem)
        };
        std::fs::create_dir_all(&extract_to)
            .map_err(|e| format!("failed to create extraction dir: {e}"))?;
        let mut extracted_bytes: u64 = 0;
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
                extracted_bytes = extracted_bytes.saturating_add(f.size());
                if extracted_bytes > MAX_SKILL_ZIP_TOTAL_BYTES {
                    let _ = std::fs::remove_dir_all(&extract_to);
                    return Err(format!(
                        "zip extraction exceeded {} MB limit",
                        MAX_SKILL_ZIP_TOTAL_BYTES / (1024 * 1024)
                    ));
                }
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

        ensure_within_skills_dir(&skills_dir, &extract_to)?;

        let skill_name = if is_flat {
            top_dirs.into_iter().next().unwrap_or_default()
        } else {
            src.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("skill")
                .to_string()
        };
        return Ok(json!({ "installed": skill_name }));
    }

    Err(format!(
        "unsupported file type: .{ext} (expected a folder or .zip)"
    ))
}

/// Recursively copy a directory.
fn copy_dir_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&src_path, &dest_path)?;
        } else {
            std::fs::copy(&src_path, &dest_path)?;
        }
    }
    Ok(())
}