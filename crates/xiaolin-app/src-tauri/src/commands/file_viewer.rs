use base64::Engine;
use serde::Serialize;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
const MAX_TEXT_FILE_BYTES: u64 = 5 * 1024 * 1024;
const MAX_BINARY_FILE_BYTES: u64 = 10 * 1024 * 1024;
const BINARY_PROBE_BYTES: usize = 8192;

const BLOCKED_DIR_NAMES: &[&str] = &[
    "node_modules", "target", ".git", "dist", "build", ".next",
];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadFileResult {
    pub content: String,
    pub size: u64,
    pub is_readonly: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinaryFileResult {
    pub base64: String,
    pub mime: String,
    pub size: u64,
}

/// Canonicalize `path` and ensure it stays within `work_dir` (also canonicalized).
/// Security: rejects NUL bytes, root work_dir, and paths outside the work directory.
fn validate_path(path: &str, work_dir: &str) -> Result<PathBuf, String> {
    if path.contains('\0') || work_dir.contains('\0') {
        return Err("invalid path".into());
    }

    let work_root = Path::new(work_dir);
    if !work_root.is_dir() {
        return Err("work directory unavailable".into());
    }

    let canonical_work = work_root
        .canonicalize()
        .map_err(|_| "work directory unavailable".to_string())?;

    if canonical_work == PathBuf::from("/") || canonical_work.parent().is_none() {
        return Err("work directory unavailable".into());
    }

    let target = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        canonical_work.join(path)
    };

    if !target.exists() {
        return Err("path not found".into());
    }

    let canonical_path = target
        .canonicalize()
        .map_err(|_| "invalid path".to_string())?;

    if !canonical_path.starts_with(&canonical_work) {
        return Err("path outside work directory".into());
    }

    Ok(canonical_path)
}

fn file_size(path: &Path) -> Result<u64, String> {
    fs::metadata(path)
        .map(|m| m.len())
        .map_err(|_| "failed to read file metadata".to_string())
}

fn is_readonly(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(meta) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                meta.permissions().mode() & 0o222 == 0
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                meta.file_attributes() & 0x1 != 0
            }
            #[cfg(not(any(unix, windows)))]
            {
                let _ = meta;
                false
            }
        }
        Err(_) => true,
    }
}

fn has_nul_bytes(path: &Path) -> Result<bool, String> {
    let mut file = fs::File::open(path).map_err(|_| "failed to open file".to_string())?;
    let mut buf = vec![0u8; BINARY_PROBE_BYTES];
    let n = file
        .read(&mut buf)
        .map_err(|_| "failed to read file".to_string())?;
    Ok(buf[..n].contains(&0))
}

fn is_blocked_dir_name(name: &str) -> bool {
    BLOCKED_DIR_NAMES.contains(&name)
}

fn mime_from_extension(path: &Path) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
    .to_string()
}

#[tauri::command]
pub fn read_file_for_viewer(path: String, work_dir: String) -> Result<ReadFileResult, String> {
    let canonical = validate_path(&path, &work_dir)?;

    if canonical.is_dir() {
        return Err("path is a directory".into());
    }

    let size = file_size(&canonical)?;
    if size > MAX_TEXT_FILE_BYTES {
        return Err("file too large".into());
    }

    if has_nul_bytes(&canonical)? {
        return Err("binary file cannot be previewed as text".into());
    }

    let content = fs::read_to_string(&canonical).map_err(|_| "failed to read file".to_string())?;

    Ok(ReadFileResult {
        content,
        size,
        is_readonly: is_readonly(&canonical),
    })
}

#[tauri::command]
pub fn list_directory(path: String, work_dir: String) -> Result<Vec<DirEntry>, String> {
    let canonical = validate_path(&path, &work_dir)?;

    if !canonical.is_dir() {
        return Err("path is not a directory".into());
    }

    const MAX_DIR_ENTRIES: usize = 2000;

    let mut entries = Vec::new();
    let read_dir = fs::read_dir(&canonical).map_err(|_| "failed to read directory".to_string())?;

    for entry in read_dir {
        if entries.len() >= MAX_DIR_ENTRIES {
            break;
        }
        let entry = entry.map_err(|_| "failed to read directory entry".to_string())?;
        let file_type = entry
            .file_type()
            .map_err(|_| "failed to read entry type".to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();

        if name.starts_with('.') {
            continue;
        }
        if file_type.is_dir() && is_blocked_dir_name(&name) {
            continue;
        }

        let size = if file_type.is_dir() {
            0
        } else {
            entry
                .metadata()
                .map(|m| m.len())
                .unwrap_or(0)
        };

        entries.push(DirEntry {
            name,
            is_dir: file_type.is_dir(),
            size,
        });
    }

    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    Ok(entries)
}

#[tauri::command]
pub fn read_binary_for_viewer(path: String, work_dir: String) -> Result<BinaryFileResult, String> {
    let canonical = validate_path(&path, &work_dir)?;

    if canonical.is_dir() {
        return Err("path is a directory".into());
    }

    let size = file_size(&canonical)?;
    if size > MAX_BINARY_FILE_BYTES {
        return Err("file too large".into());
    }

    let bytes = fs::read(&canonical).map_err(|_| "failed to read file".to_string())?;
    let base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let mime = mime_from_extension(&canonical);

    Ok(BinaryFileResult {
        base64,
        mime,
        size,
    })
}
