use serde_json::json;
use std::path::{Path, PathBuf};

const MAX_SKILL_ZIP_ENTRIES: usize = 1000;
const MAX_SKILL_ZIP_TOTAL_BYTES: u64 = 100 * 1024 * 1024;
const MAX_SKILL_UPLOAD_BYTES: u64 = MAX_SKILL_ZIP_TOTAL_BYTES;

fn is_valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn ensure_within_skills_dir(skills_dir: &Path, target: &Path) -> Result<PathBuf, String> {
    let skills_canon = skills_dir.canonicalize().map_err(|e| {
        tracing::warn!(path = %skills_dir.display(), error = %e, "skills directory unavailable");
        String::from("skills directory unavailable")
    })?;
    let target_canon = target.canonicalize().map_err(|e| {
        tracing::warn!(path = %target.display(), error = %e, "invalid destination path");
        String::from("invalid path")
    })?;
    if !target_canon.starts_with(&skills_canon) {
        return Err("path escapes skills directory".into());
    }
    Ok(target_canon)
}

fn allowed_upload_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = dirs::home_dir() {
        if let Ok(canon) = home.canonicalize() {
            roots.push(canon.clone());
            for sub in ["Downloads", "Documents", "Desktop"] {
                let dir = canon.join(sub);
                if dir.exists() {
                    if let Ok(c) = dir.canonicalize() {
                        roots.push(c);
                    }
                }
            }
        }
    }
    roots.push(std::env::temp_dir());
    roots
}

fn ensure_allowed_source_path(path: &Path) -> Result<PathBuf, String> {
    let canonical = path.canonicalize().map_err(|e| {
        tracing::warn!(path = %path.display(), error = %e, "upload_skill: invalid source path");
        String::from("invalid source path")
    })?;
    let allowed = allowed_upload_roots();
    if !allowed.iter().any(|root| canonical.starts_with(root)) {
        tracing::warn!(
            path = %canonical.display(),
            "upload_skill: source path outside allowed upload directories"
        );
        return Err(
            "source path must be under home, Downloads, Documents, Desktop, or temp directory"
                .into(),
        );
    }
    Ok(canonical)
}

fn dir_size(path: &Path) -> Result<u64, std::io::Error> {
    let mut total = 0u64;
    if path.is_file() {
        return Ok(path.metadata()?.len());
    }
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_dir() {
            total = total.saturating_add(dir_size(&entry.path())?);
        } else {
            total = total.saturating_add(meta.len());
        }
    }
    Ok(total)
}

fn ensure_source_size_within_limit(path: &Path) -> Result<(), String> {
    let size = dir_size(path).map_err(|e| {
        tracing::warn!(path = %path.display(), error = %e, "upload_skill: failed to measure source size");
        String::from("failed to read source")
    })?;
    if size > MAX_SKILL_UPLOAD_BYTES {
        return Err(format!(
            "source exceeds {} MB limit",
            MAX_SKILL_UPLOAD_BYTES / (1024 * 1024)
        ));
    }
    Ok(())
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
        tracing::warn!(path = %source_path, "upload_skill: path does not exist");
        return Err("file not found".into());
    }

    let src = ensure_allowed_source_path(src)?;
    ensure_source_size_within_limit(&src)?;

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
            std::fs::remove_dir_all(&dest).map_err(|e| {
                tracing::warn!(error = %e, "upload_skill: failed to clean existing skill dir");
                String::from("operation failed")
            })?;
        }
        copy_dir_recursive(&src, &dest).map_err(|e| {
            tracing::warn!(error = %e, "upload_skill: failed to copy skill directory");
            String::from("operation failed")
        })?;
        ensure_within_skills_dir(&skills_dir, &dest)?;
        return Ok(json!({ "installed": dir_name }));
    }

    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext == "zip" {
        let file = std::fs::File::open(&src).map_err(|e| {
            tracing::warn!(path = %source_path, error = %e, "failed to open zip");
            String::from("failed to open file")
        })?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| {
            tracing::warn!(path = %source_path, error = %e, "invalid zip archive");
            String::from("invalid zip archive")
        })?;
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
            let f = archive.by_index(i).map_err(|e| {
                tracing::warn!(index = i, error = %e, "failed to read zip entry");
                String::from("failed to read zip entry")
            })?;
            total_uncompressed = total_uncompressed.saturating_add(f.size());
            if total_uncompressed > MAX_SKILL_ZIP_TOTAL_BYTES {
                return Err(format!(
                    "zip archive uncompressed size exceeds {} MB limit",
                    MAX_SKILL_ZIP_TOTAL_BYTES / (1024 * 1024)
                ));
            }
            let Some(enclosed) = f.enclosed_name() else {
                tracing::warn!(entry_name = f.name(), "zip contains unsafe path traversal entry");
                return Err("zip contains unsafe path".into());
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
        std::fs::create_dir_all(&extract_to).map_err(|e| {
            tracing::warn!(error = %e, "failed to create extraction directory");
            String::from("failed to prepare extraction directory")
        })?;
        let mut extracted_bytes: u64 = 0;
        for i in 0..archive.len() {
            let mut f = archive.by_index(i).map_err(|e| {
                tracing::warn!(index = i, error = %e, "failed to read zip entry during extraction");
                String::from("failed to read zip entry")
            })?;
            let Some(enclosed) = f.enclosed_name().map(|p| p.to_path_buf()) else {
                tracing::warn!(entry_name = f.name(), "zip contains unsafe path traversal entry");
                return Err("zip contains unsafe path".into());
            };
            let out_path = extract_to.join(enclosed);
            if f.is_dir() {
                std::fs::create_dir_all(&out_path).map_err(|e| {
                    tracing::warn!(error = %e, "failed to create directory during extraction");
                    String::from("extraction failed")
                })?;
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
                        tracing::warn!(error = %e, "failed to create parent dir during extraction");
                        String::from("extraction failed")
                    })?;
                }
                let mut out_file = std::fs::File::create(&out_path).map_err(|e| {
                    tracing::warn!(error = %e, "failed to create extracted file");
                    String::from("extraction failed")
                })?;
                std::io::copy(&mut f, &mut out_file).map_err(|e| {
                    tracing::warn!(error = %e, "failed to write extracted file");
                    String::from("extraction failed")
                })?;
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