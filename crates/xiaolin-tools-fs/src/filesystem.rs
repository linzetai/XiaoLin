use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use base64::Engine as _;
use chardetng::EncodingDetector;
use encoding_rs::Encoding;
use xiaolin_core::agent_config::FileAccessMode;
use xiaolin_core::tool::{Tool, ToolErrorType, ToolKind, ToolParameterSchema, ToolResult};
use regex::RegexBuilder;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;

use crate::file_state_cache::{FileStateCache, StaleCheckResult};

type CodeGraphHookFn = Box<dyn Fn(PathBuf, String, String) + Send + Sync>;
static CODE_GRAPH_HOOK: OnceLock<CodeGraphHookFn> = OnceLock::new();

/// Register a callback invoked when a file read extracts code structure.
/// Called by the agent runtime to wire in `CodeGraphCache` without creating
/// a circular dependency.
pub fn set_code_graph_hook(hook: impl Fn(PathBuf, String, String) + Send + Sync + 'static) {
    CODE_GRAPH_HOOK.set(Box::new(hook)).ok();
}

tokio::task_local! {
    static FILE_ACCESS_MODE: FileAccessMode;
    static EFFECTIVE_WORK_DIR: Option<PathBuf>;
    static ADDITIONAL_ALLOWED_PATHS: Vec<PathBuf>;
    static FILE_STATE_CACHE: Arc<FileStateCache>;
}

pub async fn with_file_access_mode<F, T>(mode: FileAccessMode, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    FILE_ACCESS_MODE.scope(mode, fut).await
}

pub async fn with_work_dir<F, T>(work_dir: Option<PathBuf>, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    EFFECTIVE_WORK_DIR.scope(work_dir, fut).await
}

pub async fn with_additional_allowed_paths<F, T>(paths: Vec<PathBuf>, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    ADDITIONAL_ALLOWED_PATHS.scope(paths, fut).await
}

pub async fn with_file_state_cache<F, T>(cache: Arc<FileStateCache>, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    FILE_STATE_CACHE.scope(cache, fut).await
}

pub fn get_file_state_cache() -> Option<Arc<FileStateCache>> {
    FILE_STATE_CACHE.try_with(|c| c.clone()).ok()
}

/// Stub message returned when the file hasn't changed since last read.
/// The LLM should refer to the earlier Read result in context.
const FILE_UNCHANGED_STUB: &str =
    "File unchanged since last read. The content from the earlier read_file \
     result in this conversation is still current — refer to that instead of re-reading.";

fn workspace_root() -> std::io::Result<PathBuf> {
    if let Ok(Some(dir)) = EFFECTIVE_WORK_DIR.try_with(|d| d.clone()) {
        if dir.is_dir() {
            return Ok(dir);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        return Ok(cwd);
    }
    if let Some(home) = dirs::home_dir() {
        if home.is_dir() {
            return Ok(home);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "unable to determine workspace root: no work_dir, cwd, or home directory available",
    ))
}

/// Get the effective work directory from the task-local context.
/// Returns `None` if not set or if the task-local is not available.
pub fn get_effective_work_dir() -> Option<PathBuf> {
    EFFECTIVE_WORK_DIR.try_with(|d| d.clone()).ok().flatten()
}

fn state_dir_root() -> Option<PathBuf> {
    xiaolin_core::paths::resolve_state_dir_from(None)
        .canonicalize()
        .ok()
}

pub fn current_file_access_mode() -> FileAccessMode {
    FILE_ACCESS_MODE
        .try_with(|m| *m)
        .unwrap_or(FileAccessMode::Workspace)
}

pub fn current_effective_work_dir() -> Option<PathBuf> {
    EFFECTIVE_WORK_DIR.try_with(|d| d.clone()).unwrap_or(None)
}

pub fn current_additional_allowed_paths() -> Vec<PathBuf> {
    ADDITIONAL_ALLOWED_PATHS
        .try_with(|p| p.clone())
        .unwrap_or_default()
}

fn well_known_allowed_prefixes() -> Vec<PathBuf> {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return Vec::new(),
    };
    vec![
        home.join(".cursor").join("skills"),
        home.join(".cursor").join("rules"),
        home.join(".cursor").join("projects"),
        home.join(".agents").join("skills"),
        home.join(".codex").join("skills"),
    ]
}

fn additional_allowed_paths_from_task_local() -> Vec<PathBuf> {
    ADDITIONAL_ALLOWED_PATHS
        .try_with(|p| p.clone())
        .unwrap_or_default()
}

fn collect_all_allowed_prefixes() -> Vec<PathBuf> {
    let mut prefixes = well_known_allowed_prefixes();
    prefixes.extend(additional_allowed_paths_from_task_local());
    prefixes
}

/// Normalize a path by resolving `.` and `..` components lexically
/// (without hitting the filesystem). This is safe for `starts_with`
/// checks on paths whose parents may not yet exist.
fn lexical_clean(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                if !out.pop() {
                    out.push(c);
                }
            }
            Component::CurDir => {}
            _ => out.push(c),
        }
    }
    out
}

/// Check if `resolved` falls under any of the allowed prefixes.
/// Uses canonicalize when the prefix directory exists, falls back to
/// raw `starts_with` for directories that haven't been created yet.
fn is_path_under_allowed_prefixes(resolved: &Path, raw_absolute: &Path) -> bool {
    for prefix in collect_all_allowed_prefixes() {
        if let Ok(canon_prefix) = prefix.canonicalize() {
            if resolved.starts_with(&canon_prefix) {
                return true;
            }
        }
        if raw_absolute.starts_with(&prefix) {
            return true;
        }
    }
    false
}

fn format_allowed_locations(root: &Path) -> String {
    let mut locs = vec![format!("  • Workspace: {}", root.display())];
    if let Some(state_root) = state_dir_root() {
        locs.push(format!("  • XiaoLin data: {}", state_root.display()));
    }
    for prefix in well_known_allowed_prefixes() {
        locs.push(format!("  • {}", prefix.display()));
    }
    let extra = additional_allowed_paths_from_task_local();
    for p in &extra {
        locs.push(format!("  • {}", p.display()));
    }
    locs.join("\n")
}

pub fn ensure_within_workspace(path: &Path, must_exist: bool) -> std::io::Result<PathBuf> {
    let mode = current_file_access_mode();
    match mode {
        FileAccessMode::None => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "file access is disabled (execution mode = Plan). \
                 The user can change the execution mode in Settings → Security.",
            ))
        }
        FileAccessMode::Full => {
            let cwd = workspace_root()?;
            let absolute = if path.is_absolute() {
                path.to_path_buf()
            } else {
                cwd.join(path)
            };
            if must_exist {
                return absolute.canonicalize().map_err(|e| {
                    std::io::Error::new(
                        e.kind(),
                        format!(
                            "cannot access '{}' (full access mode): {}",
                            absolute.display(),
                            e
                        ),
                    )
                });
            }
            return if absolute.exists() {
                absolute.canonicalize()
            } else {
                if let Some(parent) = absolute.parent() {
                    if !parent.exists() {
                        std::fs::create_dir_all(parent)?;
                    }
                }
                Ok(absolute)
            };
        }
        FileAccessMode::Workspace => {}
    }

    let root = workspace_root()?.canonicalize()?;
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };

    let resolve_result: Result<PathBuf, std::io::Error> = if must_exist || absolute.exists() {
        absolute.canonicalize()
    } else {
        let parent = absolute
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.clone());
        match parent.canonicalize() {
            Ok(canon_parent) => {
                Ok(canon_parent.join(absolute.file_name().map(PathBuf::from).unwrap_or_default()))
            }
            Err(_) => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("parent directory '{}' does not exist", parent.display()),
            )),
        }
    };

    match resolve_result {
        Ok(resolved) => {
            if resolved.starts_with(&root) {
                return Ok(resolved);
            }
            if let Some(state_root) = state_dir_root() {
                if resolved.starts_with(&state_root) {
                    return Ok(resolved);
                }
            }
            if is_path_under_allowed_prefixes(&resolved, &absolute) {
                if !must_exist {
                    if let Some(parent) = resolved.parent() {
                        if !parent.exists() {
                            std::fs::create_dir_all(parent)?;
                        }
                    }
                }
                return Ok(resolved);
            }
        }
        Err(e) if must_exist => {
            // Before returning NotFound, verify the path would be within an
            // allowed location.  If not, the real error is "outside workspace"
            // — returning NotFound would mislead the LLM into thinking it just
            // needs to fix the filename.
            let cleaned = lexical_clean(&absolute);
            let allowed = cleaned.starts_with(&root)
                || state_dir_root().is_some_and(|sr| cleaned.starts_with(&sr))
                || is_path_under_allowed_prefixes(&cleaned, &absolute);
            if allowed {
                return Err(e);
            }
            // Fall through to the "outside all allowed locations" error below.
        }
        Err(_) => {
            // Parent doesn't exist yet — normalize the path lexically
            // (resolve `..` without touching the filesystem) before
            // checking workspace root, state dir, and whitelist.
            let cleaned = lexical_clean(&absolute);
            let under_workspace = cleaned.starts_with(&root);
            let under_state = state_dir_root().is_some_and(|sr| cleaned.starts_with(&sr));
            if under_workspace || under_state || is_path_under_allowed_prefixes(&cleaned, &absolute)
            {
                if let Some(parent) = cleaned.parent() {
                    if !parent.exists() {
                        std::fs::create_dir_all(parent)?;
                    }
                }
                return Ok(cleaned);
            }
        }
    }

    let allowed_list = format_allowed_locations(&root);
    Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        format!(
            "path '{}' is outside all allowed locations.\n\
             Allowed locations:\n{}\n\
             To access other paths, switch to Full (YOLO) mode in Settings → Security, \
             or add custom paths via 'additional_allowed_paths' in your agent config.",
            path.display(),
            allowed_list,
        ),
    ))
}

/// Maximum file size for full `read_file` loads (50 MiB). Larger files require offset/limit.
const MAX_READ_FILE_BYTES: u64 = 50 * 1024 * 1024;

const DEFAULT_READ_FILE_MAX_CHARS: usize = 32_768;
const ABSOLUTE_READ_FILE_MAX_CHARS: usize = 256_000;
/// Default maximum lines to return when no offset/limit is specified.
/// Prevents accidentally dumping 100k lines into the LLM context.
const DEFAULT_READ_FILE_MAX_LINES: usize = 2000;

#[derive(Debug, Deserialize)]
struct ReadFileArgs {
    #[serde(alias = "path")]
    file_path: String,
    offset: Option<i64>,
    limit: Option<usize>,
    /// Shorthand line range, e.g. "10-30" or "50-". Overrides offset/limit when present.
    lines: Option<String>,
    #[serde(default)]
    number_lines: bool,
    max_chars: Option<usize>,
    pages: Option<String>,
}

/// Parse a line range string like "10-30", "50-", or "100" into (offset, limit).
/// Returns (1-indexed start line, Option<number of lines>).
fn parse_line_range(range: &str) -> Result<(i64, Option<usize>), String> {
    let range = range.trim();
    if range.is_empty() {
        return Err("empty line range".to_string());
    }

    if let Some((start_s, end_s)) = range.split_once('-') {
        let start: i64 = start_s
            .trim()
            .parse()
            .map_err(|_| format!("invalid start line number in range '{range}'"))?;
        if start < 1 {
            return Err(format!("start line must be >= 1, got {start}"));
        }
        let end_s = end_s.trim();
        if end_s.is_empty() {
            return Ok((start, None));
        }
        let end: i64 = end_s
            .parse()
            .map_err(|_| format!("invalid end line number in range '{range}'"))?;
        if end < start {
            return Err(format!("end line ({end}) must be >= start line ({start})"));
        }
        let count = (end - start + 1) as usize;
        Ok((start, Some(count)))
    } else {
        let line: i64 = range.parse().map_err(|_| {
            format!("invalid line number '{range}', expected number or 'start-end' range")
        })?;
        if line < 1 {
            return Err(format!("line number must be >= 1, got {line}"));
        }
        Ok((line, Some(1)))
    }
}

#[derive(Debug, PartialEq)]
enum DetectedFileType {
    Text,
    Image { mime: &'static str },
    Pdf,
    JupyterNotebook,
    Binary,
}

fn detect_file_type(path: &Path, bytes: &[u8]) -> DetectedFileType {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match ext.as_str() {
        "ipynb" => return DetectedFileType::JupyterNotebook,
        "png" => return DetectedFileType::Image { mime: "image/png" },
        "jpg" | "jpeg" => return DetectedFileType::Image { mime: "image/jpeg" },
        "gif" => return DetectedFileType::Image { mime: "image/gif" },
        "webp" => return DetectedFileType::Image { mime: "image/webp" },
        "svg" => {
            return DetectedFileType::Image {
                mime: "image/svg+xml",
            }
        }
        "bmp" => return DetectedFileType::Image { mime: "image/bmp" },
        "ico" => {
            return DetectedFileType::Image {
                mime: "image/x-icon",
            }
        }
        "pdf" => return DetectedFileType::Pdf,
        _ => {}
    }

    if bytes.len() >= 4 {
        if bytes.starts_with(b"%PDF") {
            return DetectedFileType::Pdf;
        }
        if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
            return DetectedFileType::Image { mime: "image/png" };
        }
        if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return DetectedFileType::Image { mime: "image/jpeg" };
        }
        if bytes.starts_with(b"GIF8") {
            return DetectedFileType::Image { mime: "image/gif" };
        }
        if bytes.starts_with(b"RIFF") && bytes.len() >= 12 && &bytes[8..12] == b"WEBP" {
            return DetectedFileType::Image { mime: "image/webp" };
        }
        if bytes.starts_with(b"BM") {
            return DetectedFileType::Image { mime: "image/bmp" };
        }
    }

    if looks_like_binary(bytes, 8192) {
        DetectedFileType::Binary
    } else {
        DetectedFileType::Text
    }
}

fn decode_bytes_to_string(raw: &[u8]) -> Result<(String, &'static str), String> {
    if raw.is_empty() {
        return Ok((String::new(), "utf-8"));
    }

    if let Some((content, bom_enc)) = try_decode_with_bom(raw) {
        return Ok((content, bom_enc));
    }

    if let Ok(s) = std::str::from_utf8(raw) {
        return Ok((s.to_string(), "utf-8"));
    }

    let mut detector = EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    detector.feed(raw, true);
    let encoding = detector.guess(None, chardetng::Utf8Detection::Allow);

    let (decoded, _enc_used, had_errors) = encoding.decode(raw);
    if had_errors {
        let name = encoding.name();
        return Err(format!(
            "detected encoding '{name}' but decoding produced errors. \
             The file may use a mixed or unsupported encoding."
        ));
    }

    Ok((decoded.into_owned(), encoding.name()))
}

fn try_decode_with_bom(raw: &[u8]) -> Option<(String, &'static str)> {
    if raw.len() >= 3 && raw[..3] == [0xEF, 0xBB, 0xBF] {
        return std::str::from_utf8(&raw[3..])
            .ok()
            .map(|s| (s.to_string(), "utf-8-bom"));
    }

    if raw.len() >= 4 {
        if raw[..4] == [0x00, 0x00, 0xFE, 0xFF] {
            let encoding = Encoding::for_label(b"utf-32be")?;
            let (decoded, _, _) = encoding.decode(&raw[4..]);
            return Some((decoded.into_owned(), "utf-32be"));
        }
        if raw[..4] == [0xFF, 0xFE, 0x00, 0x00] {
            let encoding = Encoding::for_label(b"utf-32le")?;
            let (decoded, _, _) = encoding.decode(&raw[4..]);
            return Some((decoded.into_owned(), "utf-32le"));
        }
    }

    if raw.len() >= 2 {
        if raw[..2] == [0xFE, 0xFF] {
            let (decoded, _, _) = encoding_rs::UTF_16BE.decode(&raw[2..]);
            return Some((decoded.into_owned(), "utf-16be"));
        }
        if raw[..2] == [0xFF, 0xFE] {
            let (decoded, _, _) = encoding_rs::UTF_16LE.decode(&raw[2..]);
            return Some((decoded.into_owned(), "utf-16le"));
        }
    }

    None
}

fn handle_image_file(path: &str, raw_bytes: &[u8], mime: &str) -> ToolResult {
    let encoded = base64::engine::general_purpose::STANDARD.encode(raw_bytes);
    let size_kb = raw_bytes.len() as f64 / 1024.0;

    let dims = guess_image_dimensions(raw_bytes, mime);

    let info = if let Some((w, h)) = dims {
        format!(
            "Image file: {path}\nType: {mime}\nSize: {size_kb:.1} KB\nDimensions: {w}x{h}\n\nbase64:{mime};{encoded}"
        )
    } else {
        format!(
            "Image file: {path}\nType: {mime}\nSize: {size_kb:.1} KB\n\nbase64:{mime};{encoded}"
        )
    };

    let mut result = ToolResult::ok(info);
    result.metadata = Some(serde_json::json!({
        "fileType": "image",
        "mimeType": mime,
        "fileSize": raw_bytes.len(),
        "dimensions": dims.map(|(w,h)| serde_json::json!({"width": w, "height": h})),
    }));
    result
}

fn guess_image_dimensions(bytes: &[u8], mime: &str) -> Option<(u32, u32)> {
    match mime {
        "image/png" if bytes.len() >= 24 => {
            let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
            let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
            Some((w, h))
        }
        "image/jpeg" => parse_jpeg_dimensions(bytes),
        "image/gif" if bytes.len() >= 10 => {
            let w = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
            let h = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
            Some((w, h))
        }
        "image/bmp" if bytes.len() >= 26 => {
            let w = u32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]);
            let h = u32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
            Some((w, h))
        }
        _ => None,
    }
}

fn parse_jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 4 < bytes.len() {
        if bytes[i] != 0xFF {
            return None;
        }
        let marker = bytes[i + 1];
        if marker == 0xC0 || marker == 0xC2 {
            if i + 9 < bytes.len() {
                let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
                let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
                return Some((w, h));
            }
            return None;
        }
        let seg_len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        i += 2 + seg_len;
    }
    None
}

fn handle_pdf_file(path: &str, raw_bytes: &[u8], pages_arg: Option<&str>) -> ToolResult {
    let text = match pdf_extract::extract_text_from_mem(raw_bytes) {
        Ok(t) => t,
        Err(e) => {
            return ToolResult::typed_err(
                ToolErrorType::ReadContentFailure,
                format!(
                    "Failed to extract text from PDF '{path}': {e}. \
                     The PDF may be image-based (scanned) or encrypted. \
                     Try shell_exec with `pdftotext {path} -` for alternative extraction."
                ),
            );
        }
    };

    let all_pages: Vec<&str> = text.split('\u{000C}').collect();
    let total_pages = all_pages.len();

    let (selected_text, page_range_desc) = if let Some(range_str) = pages_arg {
        match parse_page_range(range_str, total_pages) {
            Ok(indices) => {
                let selected: Vec<&str> = indices.iter().map(|&i| all_pages[i]).collect();
                let desc = format!("pages {range_str} of {total_pages}");
                (selected.join("\n--- Page Break ---\n"), desc)
            }
            Err(e) => {
                return ToolResult::typed_err(
                    ToolErrorType::InvalidToolParams,
                    format!("Invalid pages parameter '{range_str}': {e}. Use formats like '1-5', '3', '10-20'. Pages are 1-indexed, max 20 per request."),
                );
            }
        }
    } else {
        let capped = all_pages.len().min(20);
        let text = all_pages[..capped].join("\n--- Page Break ---\n");
        let desc = if total_pages > 20 {
            format!("pages 1-20 of {total_pages} (use 'pages' parameter for more)")
        } else {
            format!("all {total_pages} pages")
        };
        (text, desc)
    };

    let total_chars = selected_text.chars().count();
    let truncated = total_chars > ABSOLUTE_READ_FILE_MAX_CHARS;
    let output = if truncated {
        let head: String = selected_text
            .chars()
            .take(ABSOLUTE_READ_FILE_MAX_CHARS)
            .collect();
        format!(
            "PDF: {path} ({page_range_desc})\n\n{head}\n\n\
             [Content truncated: showing {ABSOLUTE_READ_FILE_MAX_CHARS} of {total_chars} chars. \
             Use 'pages' parameter to read specific page ranges.]"
        )
    } else {
        format!("PDF: {path} ({page_range_desc})\n\n{selected_text}")
    };

    let mut result = ToolResult::ok(output);
    result.metadata = Some(serde_json::json!({
        "fileType": "pdf",
        "totalPages": total_pages,
        "fileSize": raw_bytes.len(),
        "truncated": truncated,
    }));
    result
}

fn parse_page_range(range_str: &str, total: usize) -> Result<Vec<usize>, String> {
    let mut indices = Vec::new();
    for part in range_str.split(',') {
        let part = part.trim();
        if let Some(dash_pos) = part.find('-') {
            let start: usize = part[..dash_pos]
                .trim()
                .parse()
                .map_err(|_| format!("invalid page number in '{part}'"))?;
            let end: usize = part[dash_pos + 1..]
                .trim()
                .parse()
                .map_err(|_| format!("invalid page number in '{part}'"))?;
            if start == 0 || end == 0 {
                return Err("page numbers are 1-indexed".to_string());
            }
            if start > end {
                return Err(format!("start ({start}) > end ({end})"));
            }
            for p in start..=end {
                if p > total {
                    return Err(format!("page {p} exceeds total {total} pages"));
                }
                indices.push(p - 1);
            }
        } else {
            let p: usize = part
                .parse()
                .map_err(|_| format!("invalid page number '{part}'"))?;
            if p == 0 {
                return Err("page numbers are 1-indexed".to_string());
            }
            if p > total {
                return Err(format!("page {p} exceeds total {total} pages"));
            }
            indices.push(p - 1);
        }
    }
    if indices.len() > 20 {
        return Err(format!(
            "requested {} pages, max 20 per request",
            indices.len()
        ));
    }
    Ok(indices)
}

#[derive(Debug, Deserialize)]
struct NotebookCell {
    cell_type: String,
    source: serde_json::Value,
    #[serde(default)]
    outputs: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct JupyterNotebook {
    cells: Vec<NotebookCell>,
    #[serde(default)]
    metadata: serde_json::Value,
}

fn handle_jupyter_notebook(path: &str, raw_bytes: &[u8]) -> ToolResult {
    let nb: JupyterNotebook = match serde_json::from_slice(raw_bytes) {
        Ok(n) => n,
        Err(e) => {
            return ToolResult::typed_err(
                ToolErrorType::ReadContentFailure,
                format!("Failed to parse Jupyter notebook '{path}': {e}"),
            );
        }
    };

    let kernel = nb
        .metadata
        .get("kernelspec")
        .and_then(|k| k.get("language"))
        .and_then(|l| l.as_str())
        .unwrap_or("python");

    let mut output = format!(
        "Jupyter Notebook: {path} ({} cells, kernel: {kernel})\n",
        nb.cells.len()
    );
    output.push_str("═".repeat(60).as_str());
    output.push('\n');

    for (i, cell) in nb.cells.iter().enumerate() {
        let source = extract_notebook_source(&cell.source);
        let cell_label = match cell.cell_type.as_str() {
            "code" => format!("In [{i}]"),
            "markdown" => format!("Markdown [{i}]"),
            "raw" => format!("Raw [{i}]"),
            other => format!("{other} [{i}]"),
        };
        output.push_str(&format!("\n── {cell_label} "));
        output.push_str(
            "─"
                .repeat(40usize.saturating_sub(cell_label.len()))
                .as_str(),
        );
        output.push('\n');
        output.push_str(&source);
        if !source.ends_with('\n') {
            output.push('\n');
        }

        if cell.cell_type == "code" && !cell.outputs.is_empty() {
            output.push_str("── Output ──\n");
            for out_val in &cell.outputs {
                let rendered = render_notebook_output(out_val);
                if !rendered.is_empty() {
                    output.push_str(&rendered);
                    if !rendered.ends_with('\n') {
                        output.push('\n');
                    }
                }
            }
        }
    }

    let total_chars = output.chars().count();
    let truncated = total_chars > ABSOLUTE_READ_FILE_MAX_CHARS;
    let output = if truncated {
        let head: String = output.chars().take(ABSOLUTE_READ_FILE_MAX_CHARS).collect();
        format!(
            "{head}\n\n[Notebook content truncated: showing {ABSOLUTE_READ_FILE_MAX_CHARS} of {total_chars} chars]"
        )
    } else {
        output
    };

    let mut result = ToolResult::ok(output);
    result.metadata = Some(serde_json::json!({
        "fileType": "jupyter",
        "totalCells": nb.cells.len(),
        "kernel": kernel,
        "fileSize": raw_bytes.len(),
        "truncated": truncated,
    }));
    result
}

fn extract_notebook_source(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn render_notebook_output(out: &serde_json::Value) -> String {
    if let Some(text) = out.get("text") {
        return extract_notebook_source(text);
    }
    if let Some(data) = out.get("data") {
        if let Some(text) = data.get("text/plain") {
            return extract_notebook_source(text);
        }
        if data.get("image/png").is_some() {
            return "[image/png output omitted]".to_string();
        }
        if let Some(html) = data.get("text/html") {
            let html_str = extract_notebook_source(html);
            if html_str.len() > 500 {
                return format!("[HTML output: {} chars, truncated]", html_str.len());
            }
            return html_str;
        }
    }
    if let Some(ename) = out.get("ename") {
        let evalue = out.get("evalue").and_then(|v| v.as_str()).unwrap_or("");
        return format!("{}: {}", ename.as_str().unwrap_or("Error"), evalue);
    }
    String::new()
}

#[derive(Debug, Deserialize)]
struct WriteFileArgs {
    #[serde(alias = "path")]
    file_path: String,
    content: String,
    mode: Option<String>,
    expected_content: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum MatchMode {
    #[default]
    Exact,
    Fuzzy,
    Contains,
}

#[derive(Debug, Deserialize)]
struct EditFileArgs {
    #[serde(alias = "path")]
    file_path: String,
    #[serde(default)]
    old_string: String,
    #[serde(default)]
    new_string: String,
    #[serde(default)]
    replace_all: bool,
    expected_replacements: Option<usize>,
    #[serde(default)]
    match_mode: MatchMode,
    /// Batch mode: apply multiple changes to the same file atomically.
    /// When provided, old_string/new_string at the top level are ignored.
    edits: Option<Vec<EditChangeItem>>,
    #[serde(default)]
    dry_run: bool,
    /// Line-range hint for fallback: 1-indexed start line of the region to edit.
    start_line: Option<usize>,
    /// Line-range hint for fallback: 1-indexed end line (inclusive) of the region to edit.
    end_line: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct EditChangeItem {
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
    expected_replacements: Option<usize>,
    #[serde(default)]
    match_mode: MatchMode,
    start_line: Option<usize>,
    end_line: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct SearchInFilesArgs {
    pattern: String,
    path: Option<String>,
    glob: Option<String>,
    case_sensitive: Option<bool>,
    max_results: Option<usize>,
    context_lines: Option<usize>,
    /// When true, enrich each match with structural context: what function,
    /// struct, or class the match lives in. Helps the model understand results
    /// semantically without extra read_file calls.
    semantic_context: Option<bool>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ApplyPatchArgs {
    #[serde(alias = "path")]
    file_path: String,
    edits: Vec<ApplyPatchEdit>,
    expected_content: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ApplyPatchEdit {
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
    expected_replacements: Option<usize>,
}

fn compute_slice_bounds(
    total_lines: usize,
    offset: Option<i64>,
    limit: Option<usize>,
) -> (usize, usize) {
    let start = match offset {
        None => 0usize,
        Some(v) if v > 0 => (v - 1) as usize,
        Some(v) if v < 0 => {
            let from_end = v.unsigned_abs() as usize;
            total_lines.saturating_sub(from_end)
        }
        Some(_) => 0usize,
    };
    let start = start.min(total_lines);
    let end = match limit {
        Some(l) => start.saturating_add(l).min(total_lines),
        None => total_lines,
    };
    (start, end)
}

fn render_line_slice(
    content: &str,
    offset: Option<i64>,
    limit: Option<usize>,
    number_lines: bool,
) -> String {
    if content.is_empty() {
        return "File is empty.".to_string();
    }
    let total = content.lines().count();
    let (start, end) = compute_slice_bounds(total, offset, limit);
    let slice_len = end.saturating_sub(start);
    let mut rendered = String::new();
    for (idx, line) in content.lines().skip(start).take(slice_len).enumerate() {
        if number_lines {
            rendered.push_str(&format!("{}|", start + idx + 1));
        }
        rendered.push_str(line);
        if idx + 1 < slice_len {
            rendered.push('\n');
        }
    }
    if offset.is_some() || limit.is_some() {
        let next_start = end + 1;
        if end < total {
            rendered.push_str(&format!(
                "\n[Showing lines {}-{} of {} total. \
                 To continue: lines=\"{}-{}\"]",
                start + 1,
                end,
                total,
                next_start,
                (next_start + 99).min(total),
            ));
        } else {
            rendered.push_str(&format!(
                "\n[Showing lines {}-{} of {} total (end of file)]",
                start + 1,
                end,
                total,
            ));
        }
    }
    rendered
}

fn looks_like_binary(bytes: &[u8], sample_size: usize) -> bool {
    let check = &bytes[..bytes.len().min(sample_size)];
    let null_count = check.iter().filter(|&&b| b == 0).count();
    null_count > 0
}

fn detect_line_ending(content: &str) -> &'static str {
    if content.contains("\r\n") {
        "crlf"
    } else {
        "lf"
    }
}

fn build_edit_snippet(content: &str, needle: &str, context: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if let Some(pos) = content.find(needle) {
        let line_idx = content[..pos].matches('\n').count();
        let needle_lines = needle.matches('\n').count() + 1;
        let start = line_idx.saturating_sub(context);
        let end = (line_idx + needle_lines + context).min(lines.len());
        lines[start..end]
            .iter()
            .enumerate()
            .map(|(i, l)| format!("{}|{}", start + i + 1, l))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        String::new()
    }
}

#[cfg(test)]
fn normalize_whitespace(s: &str) -> String {
    s.lines().map(normalize_line).collect::<Vec<_>>().join("\n")
}

fn maybe_augment_old_string_for_deletion<'a>(
    file_content: &str,
    old_string: &'a str,
    new_string: &str,
) -> std::borrow::Cow<'a, str> {
    if old_string.is_empty() || !new_string.is_empty() || old_string.ends_with('\n') {
        return std::borrow::Cow::Borrowed(old_string);
    }
    let candidate = format!("{old_string}\n");
    if file_content.contains(&candidate) {
        std::borrow::Cow::Owned(candidate)
    } else {
        std::borrow::Cow::Borrowed(old_string)
    }
}

/// Try fuzzy matching when exact match fails.
/// Returns (normalized_file_content, start_byte, end_byte) if a unique fuzzy match is found.
enum FuzzyMatchResult {
    /// Exact match (no fuzzy needed) — or a unique fuzzy match found.
    UniqueMatch {
        /// Byte offset in the *original* (LF-normalized) file content.
        start: usize,
        end: usize,
    },
    NoMatch,
    MultipleMatches(usize),
}

fn normalize_unicode_char(ch: char) -> char {
    match ch {
        '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
        | '\u{2212}' => '-',
        '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => '\'',
        '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => '"',
        '\u{00A0}' | '\u{2002}' | '\u{2003}' | '\u{2004}' | '\u{2005}' | '\u{2006}'
        | '\u{2007}' | '\u{2008}' | '\u{2009}' | '\u{200A}' | '\u{202F}' | '\u{205F}'
        | '\u{3000}' => ' ',
        other => other,
    }
}

fn normalize_line(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let mut prev_ws = false;
    for ch in line.chars() {
        let ch = normalize_unicode_char(ch);
        if ch == ' ' || ch == '\t' {
            if !prev_ws {
                result.push(' ');
            }
            prev_ws = true;
        } else {
            result.push(ch);
            prev_ws = false;
        }
    }
    result.trim().to_string()
}

fn normalize_unicode_text(text: &str) -> String {
    text.chars().map(normalize_unicode_char).collect()
}

#[allow(dead_code)]
fn seek_line_sequence<F>(file_lines: &[&str], pattern_lines: &[&str], transform: F) -> Option<usize>
where
    F: Fn(&str) -> String,
{
    if pattern_lines.is_empty() {
        return Some(0);
    }
    if pattern_lines.len() > file_lines.len() {
        return None;
    }
    'outer: for i in 0..=file_lines.len() - pattern_lines.len() {
        for p in 0..pattern_lines.len() {
            if transform(file_lines[i + p]) != transform(pattern_lines[p]) {
                continue 'outer;
            }
        }
        return Some(i);
    }
    None
}

fn count_line_sequence<F>(
    file_lines: &[&str],
    pattern_lines: &[&str],
    transform: F,
) -> (usize, Option<usize>)
where
    F: Fn(&str) -> String,
{
    if pattern_lines.is_empty() || pattern_lines.len() > file_lines.len() {
        return (0, None);
    }
    let mut count = 0usize;
    let mut first = None;
    'outer: for i in 0..=file_lines.len() - pattern_lines.len() {
        for p in 0..pattern_lines.len() {
            if transform(file_lines[i + p]) != transform(pattern_lines[p]) {
                continue 'outer;
            }
        }
        count += 1;
        if first.is_none() {
            first = Some(i);
        }
    }
    (count, first)
}

fn compute_byte_range(
    file_lines: &[&str],
    start_line: usize,
    line_count: usize,
    old_ends_with_newline: bool,
    file_len: usize,
) -> (usize, usize) {
    let mut byte_offset = 0usize;
    for line in file_lines.iter().take(start_line) {
        byte_offset += line.len() + 1;
    }
    let start_byte = byte_offset;
    for line in file_lines.iter().skip(start_line).take(line_count) {
        byte_offset += line.len() + 1;
    }
    let end_byte = if old_ends_with_newline {
        byte_offset.min(file_len)
    } else {
        byte_offset.saturating_sub(1).min(file_len)
    };
    (start_byte, end_byte)
}

fn try_fuzzy_match(file_content: &str, old_string: &str) -> FuzzyMatchResult {
    let file_lines: Vec<&str> = file_content.lines().collect();
    let old_lines: Vec<&str> = old_string.lines().collect();

    if old_lines.is_empty() || old_lines.iter().all(|l| l.trim().is_empty()) {
        return FuzzyMatchResult::NoMatch;
    }

    let build_result = |start_line: usize, line_count: usize| -> FuzzyMatchResult {
        let (start, end) = compute_byte_range(
            &file_lines,
            start_line,
            line_count,
            old_string.ends_with('\n'),
            file_content.len(),
        );
        FuzzyMatchResult::UniqueMatch { start, end }
    };

    // Pass 1: trimEnd per line (trailing whitespace tolerance)
    let (count, first) = count_line_sequence(&file_lines, &old_lines, |l| l.trim_end().to_string());
    if count == 1 {
        return build_result(first.unwrap(), old_lines.len());
    }
    if count > 1 {
        // Continue to more aggressive normalization — if it finds unique, use it
    }

    // Pass 2: Unicode normalization + trimEnd
    let (count2, first2) = count_line_sequence(&file_lines, &old_lines, |l| {
        normalize_unicode_text(l).trim_end().to_string()
    });
    if count2 == 1 {
        return build_result(first2.unwrap(), old_lines.len());
    }
    if count2 > 1 {
        return FuzzyMatchResult::MultipleMatches(count2);
    }

    // Pass 3: Full whitespace normalization (collapse internal whitespace + trim)
    let (count3, first3) = count_line_sequence(&file_lines, &old_lines, normalize_line);
    if count3 == 1 {
        return build_result(first3.unwrap(), old_lines.len());
    }
    if count3 > 1 {
        return FuzzyMatchResult::MultipleMatches(count3);
    }

    // Pass 4: Handle trailing empty line in old_string
    if old_lines.last().is_some_and(|l| l.is_empty()) && old_lines.len() > 1 {
        let trimmed_old: Vec<&str> = old_lines[..old_lines.len() - 1].to_vec();
        let (count4, first4) =
            count_line_sequence(&file_lines, &trimmed_old, |l| l.trim_end().to_string());
        if count4 == 1 {
            return build_result(first4.unwrap(), trimmed_old.len());
        }
        let (count5, first5) = count_line_sequence(&file_lines, &trimmed_old, normalize_line);
        if count5 == 1 {
            return build_result(first5.unwrap(), trimmed_old.len());
        }
    }

    // Pass 5: Literal escape sequence mismatch recovery.
    // When LLM sends real newlines but the file has literal `\n`, or vice versa.
    // This handles a common failure mode: files with literal escape sequences
    // (e.g., JSON-like content stored as single-line with `\n` literals).
    let escaped_old = old_string.replace('\n', "\\n").replace('\t', "\\t");
    if escaped_old != old_string {
        if let Some(pos) = file_content.find(&escaped_old) {
            let end = pos + escaped_old.len();
            if file_content[end..].find(&escaped_old).is_none() {
                return FuzzyMatchResult::UniqueMatch { start: pos, end };
            }
        }
    }

    if count > 1 {
        return FuzzyMatchResult::MultipleMatches(count);
    }

    FuzzyMatchResult::NoMatch
}

/// Contains-mode matching: find `needle` as a contiguous substring of `haystack`.
/// Uses whitespace-normalized matching line by line like fuzzy, but allows partial
/// coverage (the needle doesn't have to span from the start of a line to the end).
fn try_contains_match(haystack: &str, needle: &str) -> FuzzyMatchResult {
    if needle.is_empty() {
        return FuzzyMatchResult::NoMatch;
    }

    // Simple substring search; count occurrences for uniqueness.
    let mut positions = Vec::new();
    let mut search_from = 0;
    while let Some(pos) = haystack[search_from..].find(needle) {
        let abs_pos = search_from + pos;
        positions.push(abs_pos);
        search_from = abs_pos + 1;
    }

    if positions.len() == 1 {
        let start = positions[0];
        return FuzzyMatchResult::UniqueMatch {
            start,
            end: start + needle.len(),
        };
    }
    if positions.len() > 1 {
        return FuzzyMatchResult::MultipleMatches(positions.len());
    }

    // Exact substring not found — try whitespace-normalized line-by-line contains.
    let hay_lines: Vec<&str> = haystack.lines().collect();
    let needle_lines: Vec<&str> = needle.lines().collect();

    if needle_lines.is_empty() || needle_lines.iter().all(|l| l.trim().is_empty()) {
        return FuzzyMatchResult::NoMatch;
    }

    let (count, first) = count_line_sequence(&hay_lines, &needle_lines, normalize_line);
    match count {
        0 => FuzzyMatchResult::NoMatch,
        1 => {
            let start_line = first.unwrap();
            let (start, end) = compute_byte_range(
                &hay_lines,
                start_line,
                needle_lines.len(),
                needle.ends_with('\n'),
                haystack.len(),
            );
            FuzzyMatchResult::UniqueMatch { start, end }
        }
        n => FuzzyMatchResult::MultipleMatches(n),
    }
}

/// Line-range-based matching result.
#[allow(dead_code)]
enum LineRangeMatchResult {
    /// old_string matched a subset of the line range — use byte offsets from the match.
    ContextMatch { start: usize, end: usize },
    /// old_string didn't match at all — overwrite the entire line range.
    Overwrite {
        start: usize,
        end: usize,
        extracted: String,
    },
    /// Line range is out of bounds.
    OutOfBounds { total_lines: usize },
}

/// Try to locate `old_string` within the given line range, or fall back to
/// direct line-range overwrite.
///
/// Strategy:
/// 1. Extract the text at `[start_line, end_line]` (1-indexed, inclusive).
/// 2. Try exact match of `old_string` within the extracted region.
/// 3. Try fuzzy match of `old_string` within the extracted region (whitespace-
///    tolerant), scoped to a ±3 line buffer around the range for robustness.
/// 4. If neither succeeds, return `Overwrite` with the byte range of the full
///    line span so the caller can splice `new_string` directly.
fn try_line_range_match(
    file_content: &str,
    old_string: &str,
    start_line_1idx: usize,
    end_line_1idx: usize,
) -> LineRangeMatchResult {
    let lines: Vec<&str> = file_content.lines().collect();
    let total = lines.len();

    if start_line_1idx == 0 || start_line_1idx > total || end_line_1idx < start_line_1idx {
        return LineRangeMatchResult::OutOfBounds { total_lines: total };
    }

    let clamped_end = end_line_1idx.min(total);
    let start_0 = start_line_1idx - 1;

    let buffer = 3usize;
    let region_start = start_0.saturating_sub(buffer);
    let region_end = (clamped_end + buffer).min(total);

    let (region_byte_start, _) =
        compute_byte_range(&lines, region_start, 0, false, file_content.len());
    let (_, region_byte_end) = compute_byte_range(
        &lines,
        region_start,
        region_end - region_start,
        true,
        file_content.len(),
    );
    let region_text = &file_content[region_byte_start..region_byte_end];

    if !old_string.is_empty() {
        if let Some(pos) = region_text.find(old_string) {
            let abs_start = region_byte_start + pos;
            let abs_end = abs_start + old_string.len();
            if region_text[pos + old_string.len()..]
                .find(old_string)
                .is_none()
            {
                return LineRangeMatchResult::ContextMatch {
                    start: abs_start,
                    end: abs_end,
                };
            }
        }

        let region_lines: Vec<&str> = region_text.lines().collect();
        let old_lines: Vec<&str> = old_string.lines().collect();
        if !old_lines.is_empty() {
            let (count, first) =
                count_line_sequence(&region_lines, &old_lines, |l| l.trim_end().to_string());
            if count == 1 {
                let region_line_start = first.unwrap();
                let (local_start, local_end) = compute_byte_range(
                    &region_lines,
                    region_line_start,
                    old_lines.len(),
                    old_string.ends_with('\n'),
                    region_text.len(),
                );
                return LineRangeMatchResult::ContextMatch {
                    start: region_byte_start + local_start,
                    end: region_byte_start + local_end,
                };
            }

            let (count2, first2) = count_line_sequence(&region_lines, &old_lines, normalize_line);
            if count2 == 1 {
                let region_line_start = first2.unwrap();
                let (local_start, local_end) = compute_byte_range(
                    &region_lines,
                    region_line_start,
                    old_lines.len(),
                    old_string.ends_with('\n'),
                    region_text.len(),
                );
                return LineRangeMatchResult::ContextMatch {
                    start: region_byte_start + local_start,
                    end: region_byte_start + local_end,
                };
            }
        }
    }

    let (overwrite_start, overwrite_end) = compute_byte_range(
        &lines,
        start_0,
        clamped_end - start_0,
        true,
        file_content.len(),
    );
    let extracted = file_content[overwrite_start..overwrite_end].to_string();
    LineRangeMatchResult::Overwrite {
        start: overwrite_start,
        end: overwrite_end,
        extracted,
    }
}

/// Apply a line-range replacement: splice `new_string` into `content` at byte range.
fn apply_line_range_splice(content: &str, start: usize, end: usize, new_string: &str) -> String {
    let mut result = String::with_capacity(content.len() + new_string.len());
    result.push_str(&content[..start]);
    result.push_str(new_string);
    if !new_string.ends_with('\n')
        && end < content.len()
        && content.as_bytes()[end..].first() != Some(&b'\n')
    {
        result.push('\n');
    }
    result.push_str(&content[end..]);
    result
}

/// Result of attempting to apply a single edit change with the full fallback chain.
enum ApplyChangeResult {
    Ok {
        new_content: String,
        log_entry: serde_json::Value,
    },
    Err(String),
}

/// Apply one edit change with the full fallback chain: exact → fuzzy → line-range.
/// Shared by `execute_batch` and `multi_edit`.
#[allow(clippy::too_many_arguments)]
fn apply_single_change(
    current: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
    expected_replacements: Option<usize>,
    match_mode: MatchMode,
    start_line: Option<usize>,
    end_line: Option<usize>,
    label: &str,
) -> ApplyChangeResult {
    if old_string.is_empty() {
        return ApplyChangeResult::Err(format!("{label}: empty old_string."));
    }

    let old_norm = old_string.replace("\r\n", "\n");
    let new_norm = new_string.replace("\r\n", "\n");

    match match_mode {
        MatchMode::Contains => match try_contains_match(current, &old_norm) {
            FuzzyMatchResult::UniqueMatch { start, end } => {
                let result = apply_line_range_splice(current, start, end, &new_norm);
                ApplyChangeResult::Ok {
                    new_content: result,
                    log_entry: serde_json::json!({"replacements": 1, "mode": "contains"}),
                }
            }
            FuzzyMatchResult::NoMatch => {
                ApplyChangeResult::Err(format!("{label}: old_string not found (contains mode)."))
            }
            FuzzyMatchResult::MultipleMatches(n) => ApplyChangeResult::Err(format!(
                "{label}: found {n} matches (contains mode). Provide more context."
            )),
        },
        MatchMode::Fuzzy => match try_fuzzy_match(current, &old_norm) {
            FuzzyMatchResult::UniqueMatch { start, end } => {
                let result = apply_line_range_splice(current, start, end, &new_norm);
                ApplyChangeResult::Ok {
                    new_content: result,
                    log_entry: serde_json::json!({"replacements": 1, "mode": "fuzzy"}),
                }
            }
            FuzzyMatchResult::NoMatch => {
                if let (Some(sl), Some(el)) = (start_line, end_line) {
                    apply_line_range_fallback(current, &old_norm, &new_norm, sl, el, label)
                } else {
                    ApplyChangeResult::Err(format!("{label}: old_string not found (fuzzy mode)."))
                }
            }
            FuzzyMatchResult::MultipleMatches(n) => {
                if let (Some(sl), Some(el)) = (start_line, end_line) {
                    apply_line_range_fallback(current, &old_norm, &new_norm, sl, el, label)
                } else {
                    ApplyChangeResult::Err(format!(
                        "{label}: found {n} fuzzy matches. Provide more context."
                    ))
                }
            }
        },
        MatchMode::Exact => {
            let match_count = current.matches(&old_norm).count();

            if let Some(expected) = expected_replacements {
                if match_count != expected {
                    return ApplyChangeResult::Err(format!(
                        "{label}: expected {expected} matches but found {match_count}."
                    ));
                }
            }

            if match_count == 0 {
                match try_fuzzy_match(current, &old_norm) {
                    FuzzyMatchResult::UniqueMatch { start, end } => {
                        let result = apply_line_range_splice(current, start, end, &new_norm);
                        ApplyChangeResult::Ok {
                            new_content: result,
                            log_entry: serde_json::json!({"replacements": 1, "fuzzy": true}),
                        }
                    }
                    _ => {
                        if let (Some(sl), Some(el)) = (start_line, end_line) {
                            return apply_line_range_fallback(
                                current, &old_norm, &new_norm, sl, el, label,
                            );
                        }
                        ApplyChangeResult::Err(format!(
                            "{label}: old_string not found. Re-read the file to get current content."
                        ))
                    }
                }
            } else if !replace_all && expected_replacements.is_none() && match_count > 1 {
                if let (Some(sl), Some(el)) = (start_line, end_line) {
                    return apply_line_range_fallback(current, &old_norm, &new_norm, sl, el, label);
                }
                ApplyChangeResult::Err(format!(
                    "{label}: found {match_count} matches. Set replace_all=true or provide start_line/end_line."
                ))
            } else {
                let replaced = if replace_all { match_count } else { 1 };
                let new_content = if replace_all {
                    current.replace(&old_norm, &new_norm)
                } else {
                    current.replacen(&old_norm, &new_norm, 1)
                };
                ApplyChangeResult::Ok {
                    new_content,
                    log_entry: serde_json::json!({"replacements": replaced}),
                }
            }
        }
    }
}

fn apply_line_range_fallback(
    content: &str,
    old_norm: &str,
    new_norm: &str,
    start_line: usize,
    end_line: usize,
    label: &str,
) -> ApplyChangeResult {
    match try_line_range_match(content, old_norm, start_line, end_line) {
        LineRangeMatchResult::ContextMatch { start, end } => {
            let result = apply_line_range_splice(content, start, end, new_norm);
            ApplyChangeResult::Ok {
                new_content: result,
                log_entry: serde_json::json!({"replacements": 1, "line_range_context": true}),
            }
        }
        LineRangeMatchResult::Overwrite { start, end, .. } => {
            let result = apply_line_range_splice(content, start, end, new_norm);
            ApplyChangeResult::Ok {
                new_content: result,
                log_entry: serde_json::json!({"replacements": 1, "line_range_overwrite": true, "lines": format!("{start_line}-{end_line}")}),
            }
        }
        LineRangeMatchResult::OutOfBounds { total_lines } => {
            ApplyChangeResult::Err(format!(
                "{label}: start_line={start_line}/end_line={end_line} out of bounds (file has {total_lines} lines)."
            ))
        }
    }
}

/// Count actual added/removed lines using Myers diff, not just total line count difference.
fn count_diff_lines(old_text: &str, new_text: &str) -> (usize, usize) {
    use similar::{ChangeTag, TextDiff};
    let text_diff = TextDiff::from_lines(old_text, new_text);
    let mut added = 0usize;
    let mut removed = 0usize;
    for change in text_diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => added += 1,
            ChangeTag::Delete => removed += 1,
            ChangeTag::Equal => {}
        }
    }
    (added, removed)
}

/// Build a unified-diff style snippet using a proper Myers diff algorithm.
fn build_diff_snippet(old_text: &str, new_text: &str, file_path: &str) -> String {
    use similar::{ChangeTag, TextDiff};
    let text_diff = TextDiff::from_lines(old_text, new_text);
    let mut diff = format!("--- a/{file_path}\n+++ b/{file_path}\n");
    for hunk in text_diff.unified_diff().context_radius(3).iter_hunks() {
        diff.push_str(&hunk.header().to_string());
        for change in hunk.iter_changes() {
            let sign = match change.tag() {
                ChangeTag::Delete => "-",
                ChangeTag::Insert => "+",
                ChangeTag::Equal => " ",
            };
            diff.push_str(sign);
            diff.push_str(change.as_str().unwrap_or(""));
            if change.missing_newline() {
                diff.push('\n');
            }
        }
    }
    diff
}

/// Map an `io::Error` from `ensure_within_workspace` to the correct
/// `ToolErrorType`, so callers don't blindly report `PathNotInWorkspace`
/// for files that simply don't exist.
fn classify_workspace_error(e: &std::io::Error) -> ToolErrorType {
    match e.kind() {
        std::io::ErrorKind::NotFound => ToolErrorType::FileNotFound,
        std::io::ErrorKind::PermissionDenied => {
            let msg = e.to_string();
            if msg.contains("outside all allowed locations")
                || msg.contains("file access is disabled")
            {
                ToolErrorType::PathNotInWorkspace
            } else {
                ToolErrorType::PermissionDenied
            }
        }
        _ => ToolErrorType::PathNotInWorkspace,
    }
}

/// Structured error codes for `edit_file`, enabling LLM to parse failure type and recovery action.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditErrorCode {
    NoChange = 1,
    FileExists = 3,
    NotFound = 4,
    Stale = 7,
    NotMatched = 8,
    Ambiguous = 9,
}

impl EditErrorCode {
    fn error_type(self) -> &'static str {
        match self {
            Self::NoChange => "no_change",
            Self::FileExists => "file_exists",
            Self::NotFound => "not_found",
            Self::Stale => "stale",
            Self::NotMatched => "not_matched",
            Self::Ambiguous => "ambiguous",
        }
    }

    fn recovery_hint(self) -> &'static str {
        match self {
            Self::NoChange => "Modify new_string so it differs from old_string.",
            Self::FileExists => "The file already exists. Use old_string + new_string to edit, not create.",
            Self::NotFound => "Check the file path. See suggestions below, or use list_directory / glob.",
            Self::Stale => "File was modified externally. Re-read with read_file before editing.",
            Self::NotMatched => "old_string not found in file. Re-read the target section with read_file, copy exact text, and retry.",
            Self::Ambiguous => "Multiple matches found. Add more surrounding context to old_string, or set replace_all=true.",
        }
    }

    fn format_error(self, file_path: &str, detail: &str) -> String {
        serde_json::json!({
            "errorCode": self as u8,
            "errorType": self.error_type(),
            "file": file_path,
            "recovery_hint": self.recovery_hint(),
            "message": detail,
        })
        .to_string()
    }
}

/// When a requested path resolves under the CWD's parent but not under CWD itself,
/// try re-rooting it under CWD. This fixes the common LLM mistake of omitting the
/// repository directory from the path (e.g. `/tmp/src/lib.rs` instead of
/// `/tmp/myrepo/src/lib.rs` when CWD is `/tmp/myrepo`).
fn suggest_path_under_cwd(requested_path: &Path) -> Option<PathBuf> {
    let cwd = get_effective_work_dir()?;
    let cwd_parent = cwd.parent()?;

    let resolved = if requested_path.is_absolute() {
        requested_path.to_path_buf()
    } else {
        cwd.join(requested_path)
    };

    if resolved.starts_with(&cwd) {
        return None;
    }

    if !resolved.starts_with(cwd_parent) {
        return None;
    }

    let rel_from_parent = resolved.strip_prefix(cwd_parent).ok()?;
    let corrected = cwd.join(rel_from_parent);
    if corrected.exists() {
        Some(corrected)
    } else {
        None
    }
}

/// Search for files with the same basename under `root`, up to `max_depth` levels deep.
/// Returns at most `max_results` absolute paths, sorted by depth (shallowest first).
fn find_similar_files(basename: &str, root: &Path, max_depth: usize, max_results: usize) -> Vec<PathBuf> {
    use std::collections::BinaryHeap;
    use std::cmp::Reverse;

    if basename.is_empty() {
        return Vec::new();
    }

    let mut heap: BinaryHeap<Reverse<(usize, PathBuf)>> = BinaryHeap::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];

    while let Some((dir, depth)) = stack.pop() {
        if depth > max_depth {
            continue;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name();
                let skip = matches!(
                    name.to_str(),
                    Some(".git" | "node_modules" | "target" | ".next" | "__pycache__" | ".venv")
                );
                if !skip && depth < max_depth {
                    stack.push((path, depth + 1));
                }
            } else if entry.file_name().to_str() == Some(basename) {
                heap.push(Reverse((depth, path)));
            }
        }
    }

    let mut results = Vec::with_capacity(max_results);
    while results.len() < max_results {
        match heap.pop() {
            Some(Reverse((_, p))) => results.push(p),
            None => break,
        }
    }
    results
}

fn format_not_found_with_suggestions(path: &str, suggestions: &[PathBuf]) -> String {
    let cwd_hint = get_effective_work_dir()
        .map(|d| format!(" Current working directory: {}.", d.display()))
        .unwrap_or_default();
    let mut msg = format!("The file '{}' does not exist.{}", path, cwd_hint);
    if suggestions.is_empty() {
        msg.push_str(
            " Recovery: use glob (e.g. \"*keyword*\") to discover the correct path, \
             or list_directory on the parent directory to see available files.",
        );
    } else {
        msg.push_str(" Did you mean:\n");
        for s in suggestions {
            msg.push_str(&format!("  - {}\n", s.display()));
        }
        msg.push_str("Use the correct absolute path and try again.");
    }
    msg
}

/// Creates user-friendly error messages based on the error type.
/// This helps prevent exposing internal system details to the LLM.
fn create_user_friendly_error(error_type: ToolErrorType, path: &str) -> String {
    match error_type {
        ToolErrorType::FileNotFound => {
            format!(
                "The file '{}' does not exist. \
                 Recovery: run list_directory on the parent directory to see available files, \
                 or use glob (e.g. glob pattern \"*快问*\" or \"*keyword*\") to search by partial name.",
                path
            )
        }
        ToolErrorType::FileWriteFailure => {
            format!(
                "Could not write to file '{}'. Check file permissions or disk space.",
                path
            )
        }
        ToolErrorType::ReadContentFailure => {
            format!(
                "Could not read file '{}'. The file may be locked, corrupted, or inaccessible.",
                path
            )
        }
        ToolErrorType::AttemptToCreateExistingFile => {
            format!("File '{}' already exists. To modify an existing file, provide non-empty old_string and new_string parameters.", path)
        }
        ToolErrorType::PermissionDenied => {
            format!(
                "Permission denied accessing '{path}'. \
                 Possible causes: \
                 (1) The path is outside all allowed locations (workspace, XiaoLin data, skill directories). \
                 (2) The execution mode is set to Plan (read-only) — the user can change this in Settings → Security → Execution Mode. \
                 (3) OS-level file permissions prevent access. \
                 Suggestion: use a path within the workspace, or ask the user to switch to Full (YOLO) mode in Settings → Security.",
            )
        }
        ToolErrorType::NoSpaceLeft => {
            format!(
                "No space left on device while writing to '{}'. Free up some disk space.",
                path
            )
        }
        ToolErrorType::TargetIsDirectory => {
            format!(
                "Expected a file but '{}' is a directory. Please specify a file path.",
                path
            )
        }
        ToolErrorType::PathNotInWorkspace => {
            format!(
                "Cannot access path '{path}': it is outside all allowed locations. \
                 Allowed locations include: workspace root, XiaoLin data directory (~/.xiaolin/), \
                 skill directories (~/.cursor/skills/, ~/.agents/skills/, ~/.codex/skills/), \
                 and any user-configured additional_allowed_paths. \
                 Solutions: \
                 (1) Use a path within an allowed location. \
                 (2) Ask the user to change the working directory via the folder icon at the bottom of the chat input. \
                 (3) If full filesystem access is needed, the user can switch to Full (YOLO) mode in Settings → Security.",
            )
        }
        ToolErrorType::SearchPathNotFound => {
            format!(
                "Search path '{}' does not exist or is not accessible.",
                path
            )
        }
        ToolErrorType::SearchPathNotADirectory => {
            format!(
                "Search path '{}' is not a directory. Please specify a directory path.",
                path
            )
        }
        ToolErrorType::EditPreparationFailure => {
            format!("Could not prepare to edit '{}'. Check file permissions or if the file is accessible.", path)
        }
        ToolErrorType::EditNoOccurrenceFound => {
            format!("In file '{}': Could not find the specified text to replace. The file may have changed or the text to find is incorrect.", path)
        }
        ToolErrorType::EditMultipleOccurrences => {
            format!("In file '{}': Found multiple occurrences of the text. Provide more context to uniquely identify the location to edit.", path)
        }
        ToolErrorType::EditNoChange => {
            format!(
                "In file '{}': Old and new content are identical. No changes were made.",
                path
            )
        }
        _ => {
            format!("An error occurred while processing '{}'.", path)
        }
    }
}

fn is_skippable_dir_name(name: &str) -> bool {
    matches!(
        name,
        ".git" | "target" | "node_modules" | ".idea" | ".cursor"
    )
}

fn simple_glob_match(glob: &str, text: &str) -> bool {
    if glob == "*" {
        return true;
    }
    let mut pattern = String::from("^");
    for ch in glob.chars() {
        match ch {
            '*' => pattern.push_str(".*"),
            '?' => pattern.push('.'),
            '.' => pattern.push_str("\\."),
            '+' | '(' | ')' | '|' | '^' | '$' | '{' | '}' | '[' | ']' | '\\' => {
                pattern.push('\\');
                pattern.push(ch);
            }
            _ => pattern.push(ch),
        }
    }
    pattern.push('$');
    regex::Regex::new(&pattern)
        .map(|re| re.is_match(text))
        .unwrap_or(false)
}

struct GitignorePatterns {
    patterns: Vec<(String, bool)>,
}

impl GitignorePatterns {
    fn is_ignored(&self, rel_path: &str) -> bool {
        let mut ignored = false;
        for (pattern, negated) in &self.patterns {
            if gitignore_pattern_matches(pattern, rel_path) {
                ignored = !negated;
            }
        }
        ignored
    }
}

fn load_gitignore_patterns(root: &Path) -> GitignorePatterns {
    let gitignore_path = root.join(".gitignore");
    let mut patterns = Vec::new();
    if let Ok(content) = fs::read_to_string(gitignore_path) {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let (pat, negated) = if let Some(stripped) = trimmed.strip_prefix('!') {
                (stripped.to_string(), true)
            } else {
                (trimmed.to_string(), false)
            };
            patterns.push((pat, negated));
        }
    }
    GitignorePatterns { patterns }
}

fn gitignore_pattern_matches(pattern: &str, rel_path: &str) -> bool {
    let pat = pattern.trim_end_matches('/');

    // Handle absolute patterns (starting with /) - only match relative to root
    if let Some(clean_pat) = pat.strip_prefix('/') {
        // Remove leading slash
        return simple_glob_match(clean_pat, rel_path);
    }

    // Handle directory-only patterns (ending with /)
    if pattern.ends_with('/') {
        let dir_pat = &pat[..pat.len() - 1]; // Remove trailing slash
                                             // Match if path starts with the directory pattern
        return rel_path.starts_with(&format!("{}/", dir_pat))
            || simple_glob_match(dir_pat, rel_path);
    }

    // Split the path into segments
    let segments: Vec<&str> = rel_path.split('/').collect();

    // Check if pattern matches any segment (for patterns like "node_modules")
    for seg in &segments {
        if simple_glob_match(pat, seg) {
            return true;
        }
    }

    // Check if pattern matches the full path (for specific file patterns like "build.gradle")
    if simple_glob_match(pat, rel_path) {
        return true;
    }

    // Handle globstar patterns (though simplified)
    if pat.contains("**") {
        // For "**" patterns, match anywhere in the path
        let parts: Vec<&str> = pat.split("**").collect();
        if parts.len() == 2 {
            let (prefix, suffix) = (parts[0], parts[1]);
            if (prefix.is_empty() && rel_path.contains(suffix))
                || (suffix.is_empty() && rel_path.starts_with(prefix))
            {
                return true;
            } else if !prefix.is_empty() && !suffix.is_empty() {
                if let Some(pos) = rel_path.find(prefix) {
                    if rel_path[pos + prefix.len()..].ends_with(suffix) {
                        return true;
                    }
                }
            }
        }
    }

    false
}

fn collect_text_files_filtered(
    base: &Path,
    max_files: usize,
    gitignore: &GitignorePatterns,
    root: &Path,
) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![base.to_path_buf()];

    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };

            let rel = path
                .strip_prefix(root)
                .unwrap_or(path.as_path())
                .to_string_lossy()
                .to_string();

            if gitignore.is_ignored(&rel) {
                continue;
            }

            if file_type.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !is_skippable_dir_name(name) {
                    stack.push(path);
                }
                continue;
            }
            if file_type.is_file() {
                files.push(path);
                if files.len() >= max_files {
                    return Ok(files);
                }
            }
        }
    }
    Ok(files)
}

async fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let tmp_name = format!(".xiaolin-write-{}-{now}.tmp", std::process::id());
    let tmp_path = parent.join(tmp_name);

    let mut tmp = tokio::fs::File::create(&tmp_path).await?;
    tmp.write_all(bytes).await?;
    tmp.flush().await?;
    drop(tmp);
    tokio::fs::rename(&tmp_path, path).await
}

#[derive(Debug, Clone)]
struct FileEncodingMeta {
    encoding: &'static str,
    has_bom: bool,
    line_ending: &'static str,
}

impl Default for FileEncodingMeta {
    fn default() -> Self {
        Self {
            encoding: "utf-8",
            has_bom: false,
            line_ending: "lf",
        }
    }
}

fn detect_file_encoding_meta(raw_bytes: &[u8]) -> (String, FileEncodingMeta) {
    if raw_bytes.is_empty() {
        return (String::new(), FileEncodingMeta::default());
    }

    let (content, enc_name, has_bom) =
        if let Some((decoded, bom_enc)) = try_decode_with_bom(raw_bytes) {
            let is_bom = bom_enc.contains("bom")
                || bom_enc.starts_with("utf-16")
                || bom_enc.starts_with("utf-32");
            (decoded, bom_enc, is_bom)
        } else if let Ok(s) = std::str::from_utf8(raw_bytes) {
            (s.to_string(), "utf-8", false)
        } else {
            let mut detector = EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
            detector.feed(raw_bytes, true);
            let encoding = detector.guess(None, chardetng::Utf8Detection::Allow);
            let (decoded, _, _) = encoding.decode(raw_bytes);
            (decoded.into_owned(), encoding.name(), false)
        };

    let line_ending = detect_line_ending(&content);
    let meta = FileEncodingMeta {
        encoding: match enc_name {
            "utf-8" => "utf-8",
            "utf-8-bom" => "utf-8",
            "utf-16be" => "utf-16be",
            "utf-16le" => "utf-16le",
            "utf-32be" => "utf-32be",
            "utf-32le" => "utf-32le",
            _ => "utf-8",
        },
        has_bom,
        line_ending,
    };

    (content, meta)
}

fn encode_with_meta(content: &str, meta: &FileEncodingMeta) -> Vec<u8> {
    let content_with_endings = if meta.line_ending == "crlf" {
        let lf_content = content.replace("\r\n", "\n");
        lf_content.replace('\n', "\r\n")
    } else {
        content.to_string()
    };

    let mut bytes = Vec::new();

    match meta.encoding {
        "utf-16be" => {
            if meta.has_bom {
                bytes.extend_from_slice(&[0xFE, 0xFF]);
            }
            for ch in content_with_endings.encode_utf16() {
                bytes.extend_from_slice(&ch.to_be_bytes());
            }
        }
        "utf-16le" => {
            if meta.has_bom {
                bytes.extend_from_slice(&[0xFF, 0xFE]);
            }
            for ch in content_with_endings.encode_utf16() {
                bytes.extend_from_slice(&ch.to_le_bytes());
            }
        }
        _ => {
            if meta.has_bom {
                bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
            }
            bytes.extend_from_slice(content_with_endings.as_bytes());
        }
    }

    bytes
}

const UTF8_BOM_EXTENSIONS: &[&str] = &[".ps1", ".psm1", ".psd1", ".ps1xml", ".csv"];

fn needs_utf8_bom(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let dotted = format!(".{}", ext.to_ascii_lowercase());
    UTF8_BOM_EXTENSIONS.contains(&dotted.as_str())
}

/// Read a file and return its contents.
pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn supports_parallel(&self) -> bool {
        true
    }
    fn name(&self) -> &str {
        "read_file"
    }

    // Opt out of per-message budget persistence. read_file already self-limits
    // via DEFAULT_READ_FILE_MAX_CHARS / DEFAULT_READ_FILE_MAX_LINES. Persisting
    // its output to a file for the model to re-read is circular.
    fn max_result_size_chars(&self) -> usize {
        usize::MAX
    }

    fn description(&self) -> &str {
        "Reads and returns the content of a specified file. If the file is large, the content \
         will be truncated with details on how to read more using 'offset' and 'limit' parameters. \
         Handles text files (with automatic encoding detection for non-UTF-8), images (PNG, JPG, GIF, \
         WEBP, SVG, BMP — returned as base64), PDF files (text extraction, use 'pages' for specific \
         page ranges, max 20 per request), and Jupyter notebooks (.ipynb — structured cell content \
         with outputs). For text files, use offset/limit to read specific line ranges."
    }

    fn prompt(&self) -> String {
        "Reads a file from the local filesystem. You can access any file directly by using this tool.\n\
Assume this tool is able to read all files on the machine. If the User provides a path to a file \
assume that path is valid. It is okay to read a file that does not exist; an error will be returned.\n\n\
Usage:\n\
- The file_path parameter must be an absolute path, not a relative path\n\
- By default, it reads up to 2000 lines starting from the beginning of the file\n\
- Use the 'lines' parameter for easy range reading: '10-30' (lines 10-30), '50-' (from 50 to EOF), '100' (single line)\n\
- Alternatively, use offset+limit for pagination (offset is 1-indexed, negative = from end)\n\
- When you already know which part of the file you need, only read that part. This is important for larger files\n\
- Results are returned with line numbers starting at 1, using the format: LINE_NUMBER|LINE_CONTENT\n\
- This tool can read images (PNG, JPG, GIF, WEBP, SVG, BMP). When reading an image file \
the contents are presented visually as the model is multimodal\n\
- This tool can read PDF files (.pdf). For large PDFs (more than 10 pages), you MUST provide \
the pages parameter to read specific page ranges (e.g., pages: \"1-5\"). Maximum 20 pages per request\n\
- This tool can read Jupyter notebooks (.ipynb) and returns all cells with their outputs\n\
- This tool can only read files, not directories. To list a directory, use `list_directory` or `shell_exec`\n\
- If you read a file that exists but has empty contents you will receive a system reminder warning\n\n\
Large file strategy:\n\
- For files with 200+ lines, the output automatically starts with a **file outline** showing \
all symbols (functions, structs, classes, etc.) with their line ranges. Use this to decide \
which section to read next — no need to call file_outline separately\n\
- If the file is truncated, the output will show exactly where truncation happened and \
suggest the offset/limit values for continuation\n\
- For targeted edits on large files: first read the section you need (offset + limit), \
then use edit_file with the EXACT text from the output\n\
- Use offset=-N to read the last N lines (e.g., offset=-100 for last 100 lines)\n\
- NEVER fall back to shell scripts when the file is large. Instead, use offset/limit to read \
the precise section, then edit_file to modify it\n\n\
IMPORTANT: Do NOT use shell commands (cat, head, tail, less) to read files. Always use this tool instead — \
it provides structured output with line numbers, handles encoding detection, and works with binary/image/PDF files".to_string()
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "file_path".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The absolute path to the file to read (e.g., '/home/user/project/file.txt'). \
                                Relative paths are resolved against the workspace root but absolute paths are preferred."
            }),
        );
        props.insert(
            "lines".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional: line range shorthand (e.g., '10-30', '50-', '100'). \
                                Preferred over offset+limit for readability. '10-30' reads lines 10 through 30, \
                                '50-' reads from line 50 to EOF, '100' reads only line 100."
            }),
        );
        props.insert(
            "offset".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Optional: starting line number (1-indexed). Negative values count from end (-1 = last line). \
                                Ignored if 'lines' is provided."
            }),
        );
        props.insert(
            "limit".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Optional: maximum number of lines to return. Use with 'offset' to paginate. \
                                Ignored if 'lines' is provided."
            }),
        );
        props.insert(
            "number_lines".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "Optional. When true, prefixes each line as '<line_number>|<content>' for precise referencing."
            }),
        );
        props.insert(
            "max_chars".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Optional: output cap in characters. Defaults to 32768, hard max 256000."
            }),
        );
        props.insert(
            "pages".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional: for PDF files, the page range to extract (e.g., '1-5', '3', '10-20'). \
                                Pages are 1-indexed. Max 20 pages per request. Comma-separated for multiple ranges."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["file_path".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let mut args: ReadFileArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::err(format!(
                    "read_file arguments are not valid JSON: {e}. \
                 Pass a JSON object like {{\"file_path\": \"/absolute/path/to/file.rs\"}}; \
                 optional fields: lines, offset, limit, number_lines, max_chars, pages."
                ))
            }
        };

        if let Some(ref range_str) = args.lines {
            match parse_line_range(range_str) {
                Ok((start, count)) => {
                    args.offset = Some(start);
                    args.limit = count;
                }
                Err(e) => {
                    return ToolResult::err(format!(
                        "read_file: invalid 'lines' parameter: {e}. \
                         Expected format: '10-30' (range), '50-' (from line to EOF), or '100' (single line)."
                    ));
                }
            }
        }

        let path = args.file_path.as_str();

        let validated = match ensure_within_workspace(Path::new(path), true) {
            Ok(p) => p,
            Err(e) => {
                let err_type = classify_workspace_error(&e);
                return ToolResult::typed_err(err_type, create_user_friendly_error(err_type, path));
            }
        };

        // Dedup: if we've already read this exact range and the file hasn't
        // changed on disk, return a stub instead of re-sending the full content.
        // The earlier read_file result is still in context — two full copies
        // waste tokens on every subsequent turn.
        if let Some(cache) = get_file_state_cache() {
            if cache.is_unchanged_for_range(&validated, args.offset, args.limit) {
                let mut result = ToolResult::ok(FILE_UNCHANGED_STUB);
                result.metadata = Some(serde_json::json!({
                    "fileType": "unchanged",
                    "filePath": path,
                    "dedup": true,
                }));
                return result;
            }
        }

        let meta = match tokio::fs::metadata(&validated).await {
            Ok(m) => m,
            Err(e) => {
                let err_type = if e.kind() == std::io::ErrorKind::NotFound {
                    ToolErrorType::FileNotFound
                } else if e.kind() == std::io::ErrorKind::PermissionDenied {
                    ToolErrorType::PermissionDenied
                } else {
                    ToolErrorType::ReadContentFailure
                };
                return ToolResult::typed_err(
                    err_type,
                    create_user_friendly_error(err_type, path),
                );
            }
        };
        if meta.len() > MAX_READ_FILE_BYTES {
            return ToolResult::err(format!(
                "read_file: file '{path}' is too large ({} bytes, max {} bytes). \
                 Use offset and limit parameters to read a portion of the file.",
                meta.len(),
                MAX_READ_FILE_BYTES
            ));
        }

        let raw_bytes = match tokio::fs::read(&validated).await {
            Ok(b) => b,
            Err(e) => {
                let err_type = if e.kind() == std::io::ErrorKind::NotFound {
                    ToolErrorType::FileNotFound
                } else if e.kind() == std::io::ErrorKind::PermissionDenied {
                    ToolErrorType::PermissionDenied
                } else {
                    ToolErrorType::ReadContentFailure
                };
                let msg = if err_type == ToolErrorType::FileNotFound {
                    let cwd_suggestion = suggest_path_under_cwd(Path::new(path));
                    let mut suggestions: Vec<PathBuf> = Vec::new();
                    if let Some(s) = cwd_suggestion {
                        suggestions.push(s);
                    }
                    let basename = Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if let Some(root) = get_effective_work_dir() {
                        for s in find_similar_files(basename, &root, 3, 3) {
                            if !suggestions.contains(&s) {
                                suggestions.push(s);
                            }
                        }
                    }
                    suggestions.truncate(5);
                    format_not_found_with_suggestions(path, &suggestions)
                } else {
                    format!(
                        "read_file failed for path '{path}': {e}. Recovery: {recovery}",
                        recovery = match err_type {
                            ToolErrorType::PermissionDenied => "The user may need to switch execution mode in Settings → Security, or set a different working directory via the folder icon at the bottom of the chat.",
                            _ => "Check file permissions or retry. For binary files, use shell_exec to inspect."
                        }
                    )
                };
                return ToolResult::typed_err(err_type, msg);
            }
        };

        let file_size = raw_bytes.len();
        let file_type = detect_file_type(&validated, &raw_bytes);

        match file_type {
            DetectedFileType::Image { mime } => {
                return handle_image_file(path, &raw_bytes, mime);
            }
            DetectedFileType::Pdf => {
                return handle_pdf_file(path, &raw_bytes, args.pages.as_deref());
            }
            DetectedFileType::JupyterNotebook => {
                return handle_jupyter_notebook(path, &raw_bytes);
            }
            DetectedFileType::Binary => {
                return ToolResult::typed_err(
                    ToolErrorType::ReadContentFailure,
                    format!(
                        "read_file: '{path}' appears to be a binary file ({file_size} bytes). \
                         Binary files are not supported. Use shell_exec to inspect binary content \
                         (e.g. `file {path}`, `hexdump -C {path} | head`) or request a text export."
                    ),
                );
            }
            DetectedFileType::Text => {}
        }

        let (content, encoding) = match decode_bytes_to_string(&raw_bytes) {
            Ok(pair) => pair,
            Err(msg) => {
                return ToolResult::typed_err(
                    ToolErrorType::ReadContentFailure,
                    format!("read_file: '{path}' encoding error ({file_size} bytes): {msg}"),
                );
            }
        };

        let total_lines = content.lines().count();
        let line_ending = detect_line_ending(&content);

        if args.offset.is_some() || args.limit.is_some() || args.number_lines {
            let (slice_start, slice_end) =
                compute_slice_bounds(total_lines, args.offset, args.limit);

            let nav = if total_lines >= SMART_READ_OUTLINE_THRESHOLD {
                generate_navigation_context(&validated, slice_start + 1, slice_end, total_lines)
            } else {
                None
            };

            let rendered = render_line_slice(&content, args.offset, args.limit, args.number_lines);

            let output = match nav {
                Some(ref ctx) => {
                    let header =
                        format_navigation_header(ctx, slice_start + 1, slice_end, total_lines);
                    format!("{header}{rendered}")
                }
                None => rendered,
            };

            let mut meta = serde_json::json!({
                "fileType": "text",
                "totalLines": total_lines,
                "fileSize": file_size,
                "lineEnding": line_ending,
                "encoding": encoding,
                "readRange": {
                    "start": slice_start + 1,
                    "end": slice_end,
                },
            });

            if let Some(ref ctx) = nav {
                if let Some(ref enc) = ctx.enclosing_symbol {
                    meta["enclosingSymbol"] = enc.clone();
                }
                if !ctx.nearby_symbols.is_empty() {
                    meta["nearbySymbols"] = serde_json::json!(ctx.nearby_symbols);
                }
            }

            let mut result = ToolResult::ok(output);
            result.metadata = Some(meta);
            return result;
        }

        let max_chars = args
            .max_chars
            .unwrap_or(DEFAULT_READ_FILE_MAX_CHARS)
            .clamp(1, ABSOLUTE_READ_FILE_MAX_CHARS);

        let lines_shown;
        let line_truncated = total_lines > DEFAULT_READ_FILE_MAX_LINES;
        let content = if line_truncated {
            lines_shown = DEFAULT_READ_FILE_MAX_LINES;
            let truncated_text: String = content
                .lines()
                .take(DEFAULT_READ_FILE_MAX_LINES)
                .collect::<Vec<_>>()
                .join("\n");
            let next_offset = DEFAULT_READ_FILE_MAX_LINES + 1;
            format!(
                "{truncated_text}\n\
                 [File truncated: showing lines 1-{DEFAULT_READ_FILE_MAX_LINES} of {total_lines}. \
                 To continue reading: use offset={next_offset}. \
                 To read a specific section: use offset=<line> limit=<count>. \
                 To read the end: use offset=-100.]"
            )
        } else {
            lines_shown = total_lines;
            content
        };

        let char_count = content.chars().count();
        let char_truncated = char_count > max_chars;
        let text = if char_truncated {
            let head: String = content.chars().take(max_chars).collect();
            let approx_line = head.lines().count();
            format!(
                "{head}\n[Content truncated at ~line {approx_line}: showing {max_chars} of {char_count} chars, \
                 {total_lines} total lines. Use offset={} limit=500 to continue reading.]",
                approx_line + 1
            )
        } else {
            content
        };

        let is_truncated = line_truncated || char_truncated;

        // Record file state for dedup (next read) and stale detection (next edit/write).
        if let Some(cache) = get_file_state_cache() {
            cache.update_with_range(&validated, &text, args.offset, args.limit);
        }

        // Fire-and-forget: extract code context for the auto code graph.
        if let Some(hook) = CODE_GRAPH_HOOK.get() {
            if let Some(lang) = xiaolin_treesitter::CodeParser::detect_language(&validated) {
                if xiaolin_treesitter::CodeParser::is_language_available(&lang) {
                    let content_for_graph = text.clone();
                    let path_for_graph = validated.clone();
                    let lang_for_graph = lang;
                    (hook)(path_for_graph, content_for_graph, lang_for_graph);
                }
            }
        }

        // Smart read: prepend a compact outline for large files so the LLM
        // immediately understands the file's structure before seeing content.
        let (text, has_outline) = if total_lines >= SMART_READ_OUTLINE_THRESHOLD {
            match generate_compact_outline(&validated, total_lines) {
                Some(outline) => (format!("{outline}{text}"), true),
                None => (text, false),
            }
        } else {
            (text, false)
        };

        // For large truncated files, append a routing tip so the LLM uses
        // structural tools on subsequent interactions instead of blind reads.
        let text = if is_truncated && total_lines >= SMART_READ_OUTLINE_THRESHOLD {
            format!(
                "{text}\n[Tip: use file_outline or code_sections to see this file's structure, \
                 then read_file with lines=\"start-end\" for the section you need.]"
            )
        } else {
            text
        };

        let mut result = ToolResult::ok(text);
        result.metadata = Some(serde_json::json!({
            "fileType": "text",
            "totalLines": total_lines,
            "linesShown": lines_shown,
            "fileSize": file_size,
            "lineEnding": line_ending,
            "encoding": encoding,
            "isTruncated": is_truncated,
            "hasOutline": has_outline,
        }));
        result
    }
}

/// Write content to a file, creating it if needed.
pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Writes content to a specified file in the local filesystem. \
         Modes: overwrite (default), append, create_new. \
         Parent directories are created automatically. \
         Preserves existing file encoding (BOM, UTF-16, line endings) when overwriting. \
         Use expected_content for optimistic locking (write only if current content matches)."
    }

    fn prompt(&self) -> String {
        "Writes a file to the local filesystem.\n\n\
Usage:\n\
- This tool will overwrite the existing file if there is one at the provided path\n\
- If this is an existing file, you MUST use the `read_file` tool first to read the file's contents. \
This tool will fail if you did not read the file first\n\
- Prefer the `edit_file` tool for modifying existing files — it only sends the diff. \
Only use write_file to create NEW files or for complete file rewrites where edit_file would be impractical\n\
- NEVER create documentation files (*.md) or README files unless explicitly requested by the User\n\
- Only use emojis if the user explicitly requests it. Avoid writing emojis to files unless asked\n\
- Parent directories are created automatically if they don't exist\n\
- Modes: overwrite (default — replaces entire file), append (add to end), create_new (fail if exists)\n\
- Preserves existing file encoding (BOM, UTF-16, line endings) when overwriting\n\
- IMPORTANT: Do NOT use shell commands (echo/cat with redirection, tee) to write files. \
Always use this tool or edit_file — they provide atomic writes, encoding preservation, and stale-file detection".to_string()
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "file_path".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The absolute path to the file to write to (e.g., '/home/user/project/file.txt'). \
                                Relative paths are resolved against the workspace root. Parent dirs auto-created."
            }),
        );
        props.insert(
            "content".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "File content to write. In overwrite/create_new modes this is the complete file body; in append mode this text is appended at EOF."
            }),
        );
        props.insert(
            "mode".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional write mode: 'overwrite' (default), 'append', or 'create_new'."
            }),
        );
        props.insert(
            "expected_content".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional optimistic-lock baseline. If provided, the current file content must match exactly, otherwise write is rejected."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["file_path".to_string(), "content".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: WriteFileArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "write_file arguments are not valid JSON: {e}. \
                 Pass {{\"file_path\": \"/absolute/path\", \"content\": \"...\"}}; optional fields: mode, expected_content."
            )),
        };
        let path = args.file_path.as_str();
        let content = args.content.as_str();
        let mode = args.mode.as_deref().unwrap_or("overwrite");
        if !matches!(mode, "overwrite" | "append" | "create_new") {
            return ToolResult::err(format!(
                "write_file received unsupported mode '{mode}'. Valid values: overwrite, append, create_new."
            ));
        }

        let validated = match ensure_within_workspace(Path::new(path), false) {
            Ok(p) => p,
            Err(e) => {
                let err_type = classify_workspace_error(&e);
                return ToolResult::typed_err(err_type, create_user_friendly_error(err_type, path));
            }
        };
        let file_path = validated.as_path();
        if let Some(parent) = file_path.parent() {
            if tokio::fs::create_dir_all(parent).await.is_err() {
                return ToolResult::typed_err(
                    ToolErrorType::FileWriteFailure,
                    format!("Could not create parent directories for '{path}'. Check directory permissions."),
                );
            }
        }

        // Stale detection: reject overwrites if file changed since we last read it.
        // Only applies when overwriting existing files (not append or create_new).
        if mode == "overwrite" && validated.exists() {
            if let Some(cache) = get_file_state_cache() {
                match cache.check_stale(&validated).await {
                    StaleCheckResult::Stale => {
                        return ToolResult::typed_err(
                            ToolErrorType::FileWriteFailure,
                            format!(
                                "File '{path}' has been modified since you last read it (by the user, \
                                 a linter, or another tool). Read the file again with read_file before \
                                 attempting to write. This prevents accidental data loss."
                            ),
                        );
                    }
                    StaleCheckResult::NeverRead => {
                        // CC requires read-before-write for existing files.
                        // We warn in the prompt but don't hard-block here.
                    }
                    StaleCheckResult::Fresh => {}
                }
            }
        }

        if let Some(expected) = args.expected_content.as_deref() {
            match tokio::fs::read_to_string(&validated).await {
                Ok(current) => {
                    if current != expected {
                        return ToolResult::err(format!(
                            "write_file optimistic lock failed for '{path}': current content differs from expected_content. \
                             Re-read the file and retry with fresh baseline."
                        ));
                    }
                }
                Err(e) => {
                    return ToolResult::err(format!(
                        "write_file could not read current content for optimistic lock on '{path}': {e}"
                    ));
                }
            }
        }

        let (existing_text, existing_meta) = if validated.exists() && mode == "overwrite" {
            match tokio::fs::read(&validated).await {
                Ok(raw) => {
                    let (text, meta) = detect_file_encoding_meta(&raw);
                    (Some(text), Some(meta))
                }
                Err(_) => (None, None),
            }
        } else {
            (None, None)
        };

        let write_result = match mode {
            "overwrite" => {
                if let Some(ref meta) = existing_meta {
                    let bytes = encode_with_meta(content, meta);
                    atomic_write_bytes(&validated, &bytes).await
                } else {
                    let meta = if needs_utf8_bom(&validated) {
                        FileEncodingMeta {
                            has_bom: true,
                            ..FileEncodingMeta::default()
                        }
                    } else {
                        FileEncodingMeta::default()
                    };
                    let bytes = encode_with_meta(content, &meta);
                    atomic_write_bytes(&validated, &bytes).await
                }
            }
            "append" => {
                async {
                    let mut f = tokio::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&validated)
                        .await?;
                    f.write_all(content.as_bytes()).await?;
                    f.flush().await
                }
                .await
            }
            "create_new" => {
                if validated.exists() {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        "target file already exists",
                    ))
                } else {
                    let meta = if needs_utf8_bom(&validated) {
                        FileEncodingMeta {
                            has_bom: true,
                            ..FileEncodingMeta::default()
                        }
                    } else {
                        FileEncodingMeta::default()
                    };
                    let bytes = encode_with_meta(content, &meta);
                    atomic_write_bytes(&validated, &bytes).await
                }
            }
            _ => unreachable!(),
        };

        match write_result {
            Ok(()) => {
                // Update file state cache after successful write.
                if let Some(cache) = get_file_state_cache() {
                    cache.update(&validated, content);
                }

                let final_bytes = match tokio::fs::metadata(&validated).await {
                    Ok(meta) => meta.len() as usize,
                    Err(_) => content.len(),
                };
                let enc_info = existing_meta.as_ref().map_or("utf-8", |m| m.encoding);
                let (lines_added, lines_removed) = match mode {
                    "overwrite" => {
                        let old = existing_text.as_deref().unwrap_or("");
                        count_diff_lines(old, content)
                    }
                    "append" => (content.lines().count(), 0),
                    "create_new" => count_diff_lines("", content),
                    _ => (0, 0),
                };
                let diff_stat = format!("+{lines_added} -{lines_removed} lines");
                ToolResult::ok(
                    serde_json::json!({
                        "written": true,
                        "file_path": path,
                        "mode": mode,
                        "bytes": final_bytes,
                        "encoding": enc_info,
                        "diffStat": diff_stat,
                        "linesAdded": lines_added,
                        "linesRemoved": lines_removed,
                    })
                    .to_string(),
                )
            }
            Err(e) => {
                let err_type = if e.kind() == std::io::ErrorKind::AlreadyExists {
                    ToolErrorType::AttemptToCreateExistingFile
                } else if e.kind() == std::io::ErrorKind::PermissionDenied {
                    ToolErrorType::PermissionDenied
                } else {
                    ToolErrorType::FileWriteFailure
                };
                ToolResult::typed_err(
                    err_type,
                    format!(
                        "Could not write to file '{path}'. Check file permissions or disk space."
                    ),
                )
            }
        }
    }
}

/// Edit text in a file by replacing an exact snippet.
pub struct EditFileTool;

impl EditFileTool {
    async fn execute_batch(
        &self,
        file_path: &str,
        edits: &[EditChangeItem],
        dry_run: bool,
    ) -> ToolResult {
        if edits.is_empty() {
            return ToolResult::err(
                "edit_file: 'edits' array is empty. Provide at least one change.".to_string(),
            );
        }

        let validated = match ensure_within_workspace(Path::new(file_path), true) {
            Ok(p) => p,
            Err(e) => {
                let err_type = classify_workspace_error(&e);
                return ToolResult::typed_err(
                    err_type,
                    create_user_friendly_error(err_type, file_path),
                );
            }
        };

        if let Some(cache) = get_file_state_cache() {
            if let StaleCheckResult::Stale = cache.check_stale(&validated).await {
                return ToolResult::typed_err(
                    ToolErrorType::EditPreparationFailure,
                    format!(
                        "File '{}' has been modified since you last read it. \
                         Read the file again before editing.",
                        file_path
                    ),
                );
            }
        }

        let raw_bytes = match tokio::fs::read(&validated).await {
            Ok(b) => b,
            Err(e) => {
                let err_type = if e.kind() == std::io::ErrorKind::NotFound {
                    ToolErrorType::FileNotFound
                } else {
                    ToolErrorType::ReadContentFailure
                };
                return ToolResult::typed_err(
                    err_type,
                    create_user_friendly_error(err_type, file_path),
                );
            }
        };

        let (original, enc_meta) = detect_file_encoding_meta(&raw_bytes);
        if original.is_empty() && !raw_bytes.is_empty() {
            return ToolResult::typed_err(
                ToolErrorType::ReadContentFailure,
                format!(
                    "File '{}' contains binary data which cannot be edited.",
                    file_path
                ),
            );
        }

        let mut current = original.replace("\r\n", "\n");
        let mut change_log: Vec<serde_json::Value> = Vec::new();

        for (idx, change) in edits.iter().enumerate() {
            let label = format!("edit_file batch: edit #{idx}");
            match apply_single_change(
                &current,
                &change.old_string,
                &change.new_string,
                change.replace_all,
                change.expected_replacements,
                change.match_mode,
                change.start_line,
                change.end_line,
                &label,
            ) {
                ApplyChangeResult::Ok {
                    new_content,
                    mut log_entry,
                } => {
                    current = new_content;
                    log_entry
                        .as_object_mut()
                        .unwrap()
                        .insert("index".to_string(), serde_json::json!(idx));
                    change_log.push(log_entry);
                }
                ApplyChangeResult::Err(msg) => {
                    return ToolResult::err(format!("{msg} All edits aborted, file unchanged."));
                }
            }
        }

        if dry_run {
            return ToolResult::ok(
                serde_json::json!({
                    "dry_run": true,
                    "file_path": file_path,
                    "edits_valid": change_log.len(),
                    "status": "all edits valid, file not written",
                })
                .to_string(),
            );
        }

        let final_bytes = encode_with_meta(&current, &enc_meta);
        match atomic_write_bytes(&validated, &final_bytes).await {
            Ok(()) => {
                if let Some(cache) = get_file_state_cache() {
                    cache.update(&validated, &current);
                }

                let (added, removed) = count_diff_lines(&original, &current);

                ToolResult::ok(
                    serde_json::json!({
                        "edited": true,
                        "file_path": file_path,
                        "edits_applied": change_log,
                        "bytes": final_bytes.len(),
                        "diffStat": format!("+{added} -{removed} lines"),
                        "linesAdded": added,
                        "linesRemoved": removed,
                    })
                    .to_string(),
                )
            }
            Err(e) => ToolResult::typed_err(
                ToolErrorType::FileWriteFailure,
                format!("edit_file batch: failed to write '{}': {e}", file_path),
            ),
        }
    }
}

#[async_trait]
impl Tool for EditFileTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Replaces text within a file. Supports single edit (old_string + new_string) or batch mode \
         (edits array for multiple changes in one file, applied atomically). \
         Set replace_all=true to replace every instance. Use empty old_string to create a new file. \
         old_string MUST be the exact literal text from the file including all whitespace and indentation. \
         Include at least 3 lines of context BEFORE and AFTER for unique identification. \
         Preserves file encoding (BOM, line endings) automatically. \
         Falls back to Unicode-normalized and whitespace-normalized fuzzy matching if exact match fails. \
         Use match_mode='fuzzy' to skip exact match and directly use whitespace-tolerant matching. \
         Use match_mode='contains' when old_string is a partial substring of the actual file text. \
         Use start_line + end_line as a safety net: when exact+fuzzy both fail, the tool \
         locates old_string within ±3 lines of the range, or directly overwrites the range as last resort. \
         Use dry_run=true to validate edits without writing."
    }

    fn prompt(&self) -> String {
        "Performs exact string replacements in files.\n\n\
Usage:\n\
- You MUST use `read_file` at least once before editing. This tool will error if you attempt \
an edit without reading the file first. The file may have been modified since you last saw it\n\
- When editing text from read_file output, ensure you preserve the exact indentation (tabs/spaces) \
as it appears AFTER the line number prefix. The line number prefix format is: LINE_NUMBER|LINE_CONTENT. \
Everything after the pipe is the actual file content to match. Never include any part of the line number prefix \
in the old_string or new_string\n\
- ALWAYS prefer editing existing files in the codebase. NEVER write new files unless explicitly required\n\
- Only use emojis if the user explicitly requests it. Avoid adding emojis to files unless asked\n\
- The edit will FAIL if `old_string` is not unique in the file. Either provide a larger string \
with more surrounding context to make it unique or use `replace_all` to change every instance of `old_string`\n\
- Use the smallest old_string that's clearly unique — usually 2-4 adjacent lines is sufficient. \
Avoid including 10+ lines of context when less uniquely identifies the target\n\
- Use `replace_all` for replacing and renaming strings across the file. This parameter is useful \
if you want to rename a variable for instance\n\n\
When edit_file fails with 'not found':\n\
1. Use read_file with offset/limit to re-read the EXACT target section\n\
2. Copy the precise text from read_file output (after line number prefix)\n\
3. Retry edit_file with the corrected old_string\n\
NEVER fall back to shell scripts (sed, awk, Python) — always retry with the correct text from read_file.\n\n\
Handling files with literal escape sequences:\n\
- Some files contain literal \\n, \\t characters (not actual newlines/tabs)\n\
- The fuzzy matcher handles this automatically by trying escape-aware matching\n\
- If you see single-line content with \\n literals, that IS the file format — do not try to \"fix\" it\n\n\
IMPORTANT: Do NOT use shell commands (sed, awk, perl -i) to edit files. Always use this tool — \
it provides atomic writes, encoding preservation, fuzzy matching, and stale-file detection".to_string()
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "file_path".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The absolute path to the file to modify. Must start with '/'."
            }),
        );
        props.insert(
            "old_string".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The exact literal text to replace. Include at least 3 lines of context BEFORE and AFTER \
                                the target text, matching whitespace and indentation precisely. Empty string = create new file."
            }),
        );
        props.insert(
            "new_string".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The exact literal text to replace old_string with. Ensure the resulting code is correct."
            }),
        );
        props.insert(
            "replace_all".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "Optional. True to replace every match. Default false (single replacement)."
            }),
        );
        props.insert(
            "expected_replacements".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Optional strict check for number of matches before editing."
            }),
        );
        props.insert(
            "match_mode".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["exact", "fuzzy", "contains"],
                "description": "Matching mode. 'exact'=verbatim match (default, falls back to fuzzy automatically). \
                                'fuzzy'=skip exact, ignores whitespace/indentation differences directly. \
                                'contains'=old_string is a substring of the file text; matches if unique."
            }),
        );
        props.insert(
            "edits".to_string(),
            serde_json::json!({
                "type": "array",
                "description": "Optional: batch mode. Array of {old_string, new_string, replace_all?, match_mode?, start_line?, end_line?} \
                                for multiple edits on the same file, applied atomically in order. \
                                When provided, top-level old_string/new_string are ignored."
            }),
        );
        props.insert(
            "dry_run".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "Optional: if true, validate all edits but do not write to disk. Default false."
            }),
        );
        props.insert(
            "start_line".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Optional line-range hint (1-indexed). When exact+fuzzy matching both fail, \
                                the tool will use start_line..end_line to locate the edit region. \
                                First tries to find old_string within ±3 lines of the range; \
                                if that fails, overwrites the entire range with new_string."
            }),
        );
        props.insert(
            "end_line".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Optional end of the line-range hint (1-indexed, inclusive). Used with start_line."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["file_path".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: EditFileArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::err(format!(
                    "edit_file arguments are not valid JSON: {e}. \
                     Pass {{\"file_path\":\"/absolute/path\", \"old_string\":\"...\", \"new_string\":\"...\"}} \
                     or {{\"file_path\":\"/path\", \"edits\": [{{\"old_string\":\"...\", \"new_string\":\"...\"}}]}}."
                ))
            }
        };

        if let Some(ref edits) = args.edits {
            return self
                .execute_batch(&args.file_path, edits, args.dry_run)
                .await;
        }

        if args.old_string.is_empty() {
            if args.new_string.is_empty() {
                return ToolResult::typed_err(
                    ToolErrorType::InvalidToolParams,
                    "edit_file: both old_string and new_string are empty. Nothing to do.",
                );
            }
            let validated = match ensure_within_workspace(Path::new(&args.file_path), false) {
                Ok(p) => p,
                Err(e) => {
                    let err_type = classify_workspace_error(&e);
                    return ToolResult::typed_err(
                        err_type,
                        create_user_friendly_error(err_type, &args.file_path),
                    );
                }
            };
            if validated.exists() {
                return ToolResult::typed_err(
                    ToolErrorType::AttemptToCreateExistingFile,
                    EditErrorCode::FileExists.format_error(
                        &args.file_path,
                        &format!("File '{}' already exists.", args.file_path),
                    ),
                );
            }
            if let Some(parent) = validated.parent() {
                if tokio::fs::create_dir_all(parent).await.is_err() {
                    return ToolResult::typed_err(
                        ToolErrorType::FileWriteFailure,
                        format!("Could not create parent directories for '{}'. Check directory permissions.", args.file_path),
                    );
                }
            }
            let new_lines = args.new_string.lines().count();
            let meta = if needs_utf8_bom(&validated) {
                FileEncodingMeta {
                    has_bom: true,
                    ..FileEncodingMeta::default()
                }
            } else {
                FileEncodingMeta::default()
            };
            let bytes = encode_with_meta(&args.new_string, &meta);
            return match atomic_write_bytes(&validated, &bytes).await {
                Ok(()) => ToolResult::ok(
                    serde_json::json!({
                        "created": true,
                        "file_path": args.file_path,
                        "bytes": bytes.len(),
                        "linesAdded": new_lines,
                        "linesRemoved": 0,
                        "diffStat": format!("+{new_lines} -0 lines"),
                    })
                    .to_string(),
                ),
                Err(e) => ToolResult::typed_err(
                    ToolErrorType::FileWriteFailure,
                    format!("edit_file failed to create '{}': {e}", args.file_path),
                ),
            };
        }

        if args.old_string == args.new_string {
            return ToolResult::typed_err(
                ToolErrorType::EditNoChange,
                EditErrorCode::NoChange.format_error(
                    &args.file_path,
                    &format!("old_string and new_string are identical in '{}'.", args.file_path),
                ),
            );
        }

        let validated = match ensure_within_workspace(Path::new(&args.file_path), true) {
            Ok(p) => p,
            Err(e) => {
                let err_type = classify_workspace_error(&e);
                return ToolResult::typed_err(
                    err_type,
                    create_user_friendly_error(err_type, &args.file_path),
                );
            }
        };

        // File size guard: prevent OOM on multi-GB files (matches CC's 1 GiB limit).
        const MAX_EDIT_FILE_SIZE: u64 = 1024 * 1024 * 1024;
        if let Ok(meta) = tokio::fs::metadata(&validated).await {
            if meta.len() > MAX_EDIT_FILE_SIZE {
                return ToolResult::typed_err(
                    ToolErrorType::FileTooLarge,
                    format!(
                        "File '{}' is too large to edit ({:.1} MB). Maximum editable file size is 1 GB. \
                         Use shell_exec with sed/awk for targeted edits on very large files.",
                        args.file_path,
                        meta.len() as f64 / (1024.0 * 1024.0)
                    ),
                );
            }
        }

        // Stale detection: reject if the file was modified externally since we last read it.
        if let Some(cache) = get_file_state_cache() {
            match cache.check_stale(&validated).await {
                StaleCheckResult::Stale => {
                    return ToolResult::typed_err(
                        ToolErrorType::EditPreparationFailure,
                        EditErrorCode::Stale.format_error(
                            &args.file_path,
                            &format!(
                                "File '{}' has been modified since you last read it.",
                                args.file_path
                            ),
                        ),
                    );
                }
                StaleCheckResult::NeverRead => {}
                StaleCheckResult::Fresh => {}
            }
        }

        let raw_bytes = match tokio::fs::read(&validated).await {
            Ok(b) => b,
            Err(e) => {
                let err_type = if e.kind() == std::io::ErrorKind::NotFound {
                    ToolErrorType::FileNotFound
                } else if e.kind() == std::io::ErrorKind::PermissionDenied {
                    ToolErrorType::PermissionDenied
                } else {
                    ToolErrorType::ReadContentFailure
                };
                let msg = if err_type == ToolErrorType::FileNotFound {
                    let cwd_suggestion = suggest_path_under_cwd(Path::new(&args.file_path));
                    let mut suggestions: Vec<PathBuf> = Vec::new();
                    if let Some(s) = cwd_suggestion {
                        suggestions.push(s);
                    }
                    let basename = Path::new(&args.file_path).file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if let Some(root) = get_effective_work_dir() {
                        for s in find_similar_files(basename, &root, 3, 3) {
                            if !suggestions.contains(&s) {
                                suggestions.push(s);
                            }
                        }
                    }
                    suggestions.truncate(5);
                    let detail = format_not_found_with_suggestions(&args.file_path, &suggestions);
                    EditErrorCode::NotFound.format_error(&args.file_path, &detail)
                } else {
                    create_user_friendly_error(err_type, &args.file_path)
                };
                return ToolResult::typed_err(err_type, msg);
            }
        };

        let (current, enc_meta) = detect_file_encoding_meta(&raw_bytes);

        if current.is_empty() && !raw_bytes.is_empty() {
            return ToolResult::typed_err(
                ToolErrorType::ReadContentFailure,
                format!("File '{}' contains binary data which cannot be edited. Use appropriate tools for binary files.", args.file_path),
            );
        }

        let normalized = current.replace("\r\n", "\n");
        let old_normalized = args.old_string.replace("\r\n", "\n");
        let new_normalized = args.new_string.replace("\r\n", "\n");

        let old_augmented =
            maybe_augment_old_string_for_deletion(&normalized, &old_normalized, &new_normalized);

        let (updated_normalized, replaced, fuzzy_used) = match args.match_mode {
            MatchMode::Contains => match try_contains_match(&normalized, &old_normalized) {
                FuzzyMatchResult::UniqueMatch { start, end } => {
                    let mut result = String::with_capacity(normalized.len());
                    result.push_str(&normalized[..start]);
                    result.push_str(&new_normalized);
                    result.push_str(&normalized[end..]);
                    (result, 1usize, true)
                }
                FuzzyMatchResult::NoMatch => {
                    return ToolResult::typed_err(
                        ToolErrorType::EditNoOccurrenceFound,
                        EditErrorCode::NotMatched.format_error(
                            &args.file_path,
                            &format!("Could not find old_string as a substring (contains mode) in '{}'.", args.file_path),
                        ),
                    );
                }
                FuzzyMatchResult::MultipleMatches(n) => {
                    return ToolResult::typed_err(
                        ToolErrorType::EditMultipleOccurrences,
                        EditErrorCode::Ambiguous.format_error(
                            &args.file_path,
                            &format!("Found {n} substring matches (contains mode) in '{}'.", args.file_path),
                        ),
                    );
                }
            },
            MatchMode::Fuzzy => match try_fuzzy_match(&normalized, &old_normalized) {
                FuzzyMatchResult::UniqueMatch { start, end } => {
                    let mut result = String::with_capacity(normalized.len());
                    result.push_str(&normalized[..start]);
                    result.push_str(&new_normalized);
                    result.push_str(&normalized[end..]);
                    (result, 1usize, true)
                }
                FuzzyMatchResult::NoMatch => {
                    return ToolResult::typed_err(
                        ToolErrorType::EditNoOccurrenceFound,
                        EditErrorCode::NotMatched.format_error(
                            &args.file_path,
                            &format!("Could not find old_string with fuzzy matching in '{}'.", args.file_path),
                        ),
                    );
                }
                FuzzyMatchResult::MultipleMatches(n) => {
                    return ToolResult::typed_err(
                        ToolErrorType::EditMultipleOccurrences,
                        EditErrorCode::Ambiguous.format_error(
                            &args.file_path,
                            &format!("Found {n} fuzzy matches in '{}'.", args.file_path),
                        ),
                    );
                }
            },
            MatchMode::Exact => {
                let mut match_count = normalized.matches(old_augmented.as_ref()).count();

                let unicode_match = if match_count == 0 {
                    let norm_hay = normalize_unicode_text(&normalized);
                    let norm_needle = normalize_unicode_text(&old_augmented);
                    let c = norm_hay.matches(&norm_needle).count();
                    if c > 0 {
                        match_count = c;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };

                if let Some(expected) = args.expected_replacements {
                    if match_count != expected {
                        return ToolResult::typed_err(
                            ToolErrorType::EditMultipleOccurrences,
                            EditErrorCode::Ambiguous.format_error(
                                &args.file_path,
                                &format!("Expected {expected} matches but found {match_count} in '{}'.", args.file_path),
                            ),
                        );
                    }
                }

                if match_count == 0 {
                    match try_fuzzy_match(&normalized, &old_normalized) {
                        FuzzyMatchResult::UniqueMatch { start, end } => {
                            let mut result = String::with_capacity(normalized.len());
                            result.push_str(&normalized[..start]);
                            result.push_str(&new_normalized);
                            result.push_str(&normalized[end..]);
                            (result, 1usize, true)
                        }
                        FuzzyMatchResult::NoMatch => {
                            if let (Some(sl), Some(el)) = (args.start_line, args.end_line) {
                                match try_line_range_match(&normalized, &old_normalized, sl, el) {
                                    LineRangeMatchResult::ContextMatch { start, end } => {
                                        let result = apply_line_range_splice(
                                            &normalized,
                                            start,
                                            end,
                                            &new_normalized,
                                        );
                                        (result, 1usize, true)
                                    }
                                    LineRangeMatchResult::Overwrite { start, end, .. } => {
                                        let result = apply_line_range_splice(
                                            &normalized,
                                            start,
                                            end,
                                            &new_normalized,
                                        );
                                        (result, 1usize, true)
                                    }
                                    LineRangeMatchResult::OutOfBounds { total_lines } => {
                                        return ToolResult::typed_err(
                                            ToolErrorType::EditNoOccurrenceFound,
                                            EditErrorCode::NotMatched.format_error(
                                                &args.file_path,
                                                &format!(
                                                    "start_line={sl}/end_line={el} out of bounds (file has {total_lines} lines) in '{}'.",
                                                    args.file_path
                                                ),
                                            ),
                                        );
                                    }
                                }
                            } else {
                                let file_preview: String = normalized
                                    .lines()
                                    .take(20)
                                    .enumerate()
                                    .map(|(i, l)| format!("{}|{}", i + 1, l))
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                return ToolResult::typed_err(
                                    ToolErrorType::EditNoOccurrenceFound,
                                    format!(
                                        "{}\n\nFile preview (first 20 lines):\n{file_preview}",
                                        EditErrorCode::NotMatched.format_error(
                                            &args.file_path,
                                            &format!(
                                                "Could not find old_string in '{}' (neither exact nor fuzzy match).",
                                                args.file_path
                                            ),
                                        ),
                                    ),
                                );
                            }
                        }
                        FuzzyMatchResult::MultipleMatches(n) => {
                            if let (Some(sl), Some(el)) = (args.start_line, args.end_line) {
                                match try_line_range_match(&normalized, &old_normalized, sl, el) {
                                    LineRangeMatchResult::ContextMatch { start, end } => {
                                        let result = apply_line_range_splice(
                                            &normalized,
                                            start,
                                            end,
                                            &new_normalized,
                                        );
                                        (result, 1usize, true)
                                    }
                                    LineRangeMatchResult::Overwrite { start, end, .. } => {
                                        let result = apply_line_range_splice(
                                            &normalized,
                                            start,
                                            end,
                                            &new_normalized,
                                        );
                                        (result, 1usize, true)
                                    }
                                    LineRangeMatchResult::OutOfBounds { total_lines } => {
                                        return ToolResult::typed_err(
                                            ToolErrorType::EditNoOccurrenceFound,
                                            EditErrorCode::NotMatched.format_error(
                                                &args.file_path,
                                                &format!(
                                                    "start_line={sl}/end_line={el} out of bounds (file has {total_lines} lines) in '{}'.",
                                                    args.file_path
                                                ),
                                            ),
                                        );
                                    }
                                }
                            } else {
                                return ToolResult::typed_err(
                                    ToolErrorType::EditMultipleOccurrences,
                                    EditErrorCode::Ambiguous.format_error(
                                        &args.file_path,
                                        &format!(
                                            "Found {n} fuzzy matches (exact found 0) in '{}'. Add more context or use replace_all=true.",
                                            args.file_path
                                        ),
                                    ),
                                );
                            }
                        }
                    }
                } else if !args.replace_all
                    && args.expected_replacements.is_none()
                    && match_count > 1
                {
                    let kind = if unicode_match { "Unicode-normalized" } else { "exact" };
                    return ToolResult::typed_err(
                        ToolErrorType::EditMultipleOccurrences,
                        EditErrorCode::Ambiguous.format_error(
                            &args.file_path,
                            &format!(
                                "Found {match_count} {kind} matches in '{}'. Add more context or use replace_all=true.",
                                args.file_path
                            ),
                        ),
                    );
                } else if unicode_match {
                    match try_fuzzy_match(&normalized, &old_augmented) {
                        FuzzyMatchResult::UniqueMatch { start, end } => {
                            let mut result = String::with_capacity(normalized.len());
                            result.push_str(&normalized[..start]);
                            result.push_str(&new_normalized);
                            result.push_str(&normalized[end..]);
                            (result, 1usize, true)
                        }
                        FuzzyMatchResult::NoMatch => {
                            return ToolResult::typed_err(
                                ToolErrorType::EditNoOccurrenceFound,
                                format!(
                                    "In file '{}': Unicode-normalized count found matches but fuzzy replace failed.",
                                    args.file_path
                                ),
                            );
                        }
                        FuzzyMatchResult::MultipleMatches(n) => {
                            return ToolResult::typed_err(
                                ToolErrorType::EditMultipleOccurrences,
                                format!(
                                    "In file '{}': Found {n} Unicode-normalized matches. Provide more context.",
                                    args.file_path
                                ),
                            );
                        }
                    }
                } else {
                    let result = if args.replace_all {
                        normalized.replace(old_augmented.as_ref(), &new_normalized)
                    } else {
                        normalized.replacen(old_augmented.as_ref(), &new_normalized, 1)
                    };
                    let count = if args.replace_all { match_count } else { 1 };
                    (result, count, false)
                }
            }
        };

        let (added, removed) = count_diff_lines(&normalized, &updated_normalized);

        let snippet = build_edit_snippet(&updated_normalized, &new_normalized, 4);
        let diff = build_diff_snippet(&old_normalized, &new_normalized, &args.file_path);

        let write_bytes = encode_with_meta(&updated_normalized, &enc_meta);

        match atomic_write_bytes(&validated, &write_bytes).await {
            Ok(()) => {
                // Update file state cache so subsequent edits detect external changes.
                if let Some(cache) = get_file_state_cache() {
                    cache.update(&validated, &updated_normalized);
                }

                let diff_stat = format!("+{} -{} lines", added, removed);
                let mut result = ToolResult::ok(
                    serde_json::json!({
                        "edited": true,
                        "file_path": args.file_path,
                        "replacements": replaced,
                        "bytes": write_bytes.len(),
                        "diffStat": diff_stat,
                        "linesAdded": added,
                        "linesRemoved": removed,
                        "snippet": snippet,
                        "fuzzyMatch": fuzzy_used,
                    })
                    .to_string(),
                );
                result.display_output = Some(
                    serde_json::json!({
                        "edited": true,
                        "file_path": args.file_path,
                        "replacements": replaced,
                        "diffStat": diff_stat,
                        "linesAdded": added,
                        "linesRemoved": removed,
                        "fuzzyMatch": fuzzy_used,
                        "diff": diff,
                        "snippet": snippet,
                    })
                    .to_string(),
                );
                result.metadata = Some(serde_json::json!({
                    "lineEnding": enc_meta.line_ending,
                    "encoding": enc_meta.encoding,
                    "totalLines": updated_normalized.lines().count(),
                    "fuzzyMatch": fuzzy_used,
                }));
                result
            }
            Err(_) => ToolResult::typed_err(
                ToolErrorType::FileWriteFailure,
                format!(
                    "Could not write to file '{}'. Check file permissions or disk space.",
                    args.file_path
                ),
            ),
        }
    }
}

/// Enrich search results with structural context from tree-sitter.
///
/// For each match line like `src/foo.rs:42:  some_code`, resolve the
/// enclosing symbol (function, struct, impl, class) and annotate the line
/// with `[in fn bar()]` or `[in struct Foo]`.
fn enrich_search_with_symbols(text_output: &str, root: &Path) -> String {
    use std::collections::HashMap;

    // Cache: file path → Vec<(name, kind, start_line, end_line)>
    let mut symbol_cache: HashMap<String, Vec<(String, String, usize, usize)>> = HashMap::new();

    let mut enriched_lines = Vec::new();

    for line in text_output.lines() {
        // Parse ripgrep-style output: "file:line:content" or "file:line-content"
        let annotated = annotate_match_line(line, root, &mut symbol_cache);
        enriched_lines.push(annotated);
    }

    enriched_lines.join("\n")
}

/// Annotate a single search result line with the enclosing symbol.
fn annotate_match_line(
    line: &str,
    root: &Path,
    symbol_cache: &mut std::collections::HashMap<String, Vec<(String, String, usize, usize)>>,
) -> String {
    // Match lines look like: "path:linenum:content" or "path:linenum-content" (context)
    let (file_part, line_num, _rest) = match parse_rg_line(line) {
        Some(v) => v,
        None => return line.to_string(),
    };

    let file_path = if Path::new(&file_part).is_absolute() {
        file_part.clone()
    } else {
        root.join(&file_part).to_string_lossy().to_string()
    };

    let symbols = symbol_cache
        .entry(file_path.clone())
        .or_insert_with(|| extract_file_symbols(&file_path));

    // Find the innermost (narrowest) symbol that contains this line.
    let enclosing = symbols
        .iter()
        .filter(|(_, _, start, end)| line_num >= *start && line_num <= *end)
        .min_by_key(|(_, _, start, end)| end - start);

    match enclosing {
        Some((name, kind, start, _)) => {
            format!("{line}  // [in {kind} {name}, line {start}]")
        }
        None => line.to_string(),
    }
}

/// Parse a ripgrep-style match line into (file_path, line_number, rest).
fn parse_rg_line(line: &str) -> Option<(String, usize, &str)> {
    // Formats: "path:123:content" or "path:123-content" (context line)
    // Also handle Windows paths with drive letters like "C:\path:123:content"
    let bytes = line.as_bytes();
    let mut colon_positions = Vec::new();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b':' {
            colon_positions.push(i);
        }
    }

    // Need at least 2 colons (or 1 colon + 1 dash) for path:linenum:content
    for &pos in &colon_positions {
        if pos == 0 {
            continue;
        }
        let after_colon = &line[pos + 1..];
        // Try to parse a line number starting at pos+1
        let num_end = after_colon.find([':', '-']).unwrap_or(after_colon.len());
        if num_end == 0 {
            continue;
        }
        let num_str = &after_colon[..num_end];
        if let Ok(line_num) = num_str.parse::<usize>() {
            let file_part = &line[..pos];
            let rest = if num_end < after_colon.len() {
                &after_colon[num_end + 1..]
            } else {
                ""
            };
            return Some((file_part.to_string(), line_num, rest));
        }
    }
    None
}

/// Extract symbols from a file using tree-sitter, returning (name, kind, start_line, end_line).
fn extract_file_symbols(file_path: &str) -> Vec<(String, String, usize, usize)> {
    let path = Path::new(file_path);
    let lang = match xiaolin_treesitter::CodeParser::detect_language(path) {
        Some(l) if xiaolin_treesitter::CodeParser::is_language_available(&l) => l,
        _ => return Vec::new(),
    };

    let parsed = match xiaolin_treesitter::CodeParser::parse_file(path) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };

    xiaolin_treesitter::extract_symbols(&parsed.tree, &parsed.source, &lang)
        .into_iter()
        .filter(|s| {
            matches!(
                s.kind,
                xiaolin_treesitter::SymbolKind::Function
                    | xiaolin_treesitter::SymbolKind::Method
                    | xiaolin_treesitter::SymbolKind::Class
                    | xiaolin_treesitter::SymbolKind::Struct
                    | xiaolin_treesitter::SymbolKind::Enum
                    | xiaolin_treesitter::SymbolKind::Trait
                    | xiaolin_treesitter::SymbolKind::Interface
                    | xiaolin_treesitter::SymbolKind::Module
            )
        })
        .map(|s| (s.name, s.kind.to_string(), s.start_line, s.end_line))
        .collect()
}

/// Threshold (lines) above which read_file auto-prepends a file outline.
const SMART_READ_OUTLINE_THRESHOLD: usize = 200;

fn symbol_kind_label(kind: &xiaolin_treesitter::SymbolKind) -> &'static str {
    match kind {
        xiaolin_treesitter::SymbolKind::Function => "fn",
        xiaolin_treesitter::SymbolKind::Method => "method",
        xiaolin_treesitter::SymbolKind::Class => "class",
        xiaolin_treesitter::SymbolKind::Struct => "struct",
        xiaolin_treesitter::SymbolKind::Enum => "enum",
        xiaolin_treesitter::SymbolKind::Trait => "trait",
        xiaolin_treesitter::SymbolKind::Interface => "interface",
        xiaolin_treesitter::SymbolKind::Module => "mod",
        xiaolin_treesitter::SymbolKind::Constant => "const",
        xiaolin_treesitter::SymbolKind::Variable => "var",
        _ => "other",
    }
}

fn truncate_signature(sig: &str, max_len: usize) -> String {
    if sig.len() > max_len {
        format!("{}…", &sig[..sig.floor_char_boundary(max_len)])
    } else {
        sig.to_string()
    }
}

/// Generate a compact file outline string for prepending to large file reads.
fn generate_compact_outline(file_path: &Path, total_lines: usize) -> Option<String> {
    let lang = xiaolin_treesitter::CodeParser::detect_language(file_path)?;
    if !xiaolin_treesitter::CodeParser::is_language_available(&lang) {
        return None;
    }
    let parsed = xiaolin_treesitter::CodeParser::parse_file(file_path).ok()?;
    let symbols = xiaolin_treesitter::extract_symbols(&parsed.tree, &parsed.source, &lang);

    if symbols.is_empty() {
        return None;
    }

    let mut outline = format!(
        "── File outline ({total_lines} lines, {} symbols) ──\n",
        symbols.len()
    );

    for s in &symbols {
        if matches!(s.kind, xiaolin_treesitter::SymbolKind::Import) {
            continue;
        }
        let kind_label = symbol_kind_label(&s.kind);
        let sig = if s.signature.is_empty() {
            &s.name
        } else {
            &s.signature
        };
        let sig_short = truncate_signature(sig, 80);
        outline.push_str(&format!(
            "  L{}-{}: {kind_label} {sig_short}\n",
            s.start_line, s.end_line
        ));
    }
    outline.push_str("──────────────────────\n\n");
    Some(outline)
}

/// Navigation context returned for partial reads on large files.
/// Tells the LLM where the read range sits relative to surrounding symbols.
struct NavigationContext {
    enclosing: Option<String>,
    nearby_before: Vec<String>,
    nearby_after: Vec<String>,
    /// Structured data for metadata enrichment.
    enclosing_symbol: Option<serde_json::Value>,
    nearby_symbols: Vec<serde_json::Value>,
}

/// Generate a compact navigation header for partial reads on large files.
///
/// Shows the enclosing symbol (the one whose line range contains the read range)
/// plus 1-2 symbols immediately before and after.
fn generate_navigation_context(
    file_path: &Path,
    read_start: usize,
    read_end: usize,
    _total_lines: usize,
) -> Option<NavigationContext> {
    let lang = xiaolin_treesitter::CodeParser::detect_language(file_path)?;
    if !xiaolin_treesitter::CodeParser::is_language_available(&lang) {
        return None;
    }
    let parsed = xiaolin_treesitter::CodeParser::parse_file(file_path).ok()?;
    let symbols = xiaolin_treesitter::extract_symbols(&parsed.tree, &parsed.source, &lang);

    let symbols: Vec<_> = symbols
        .into_iter()
        .filter(|s| !matches!(s.kind, xiaolin_treesitter::SymbolKind::Import))
        .collect();

    if symbols.is_empty() {
        return None;
    }

    let mut enclosing: Option<&xiaolin_treesitter::Symbol> = None;
    let mut before: Vec<&xiaolin_treesitter::Symbol> = Vec::new();
    let mut after: Vec<&xiaolin_treesitter::Symbol> = Vec::new();

    for s in &symbols {
        if s.start_line <= read_start && s.end_line >= read_end {
            if enclosing.is_none_or(|e| (s.end_line - s.start_line) < (e.end_line - e.start_line)) {
                enclosing = Some(s);
            }
        } else if s.end_line < read_start {
            before.push(s);
        } else if s.start_line > read_end {
            after.push(s);
        }
    }

    let nearby_before: Vec<_> = before.iter().rev().take(2).rev().cloned().collect();
    let nearby_after: Vec<_> = after.iter().take(2).cloned().collect();

    fn format_sym(s: &xiaolin_treesitter::Symbol) -> String {
        let kind = symbol_kind_label(&s.kind);
        let sig = if s.signature.is_empty() {
            &s.name
        } else {
            &s.signature
        };
        let sig_short = truncate_signature(sig, 60);
        format!("{kind} {sig_short} (L{}-{})", s.start_line, s.end_line)
    }

    fn sym_to_json(s: &xiaolin_treesitter::Symbol) -> serde_json::Value {
        let kind = symbol_kind_label(&s.kind);
        let sig = if s.signature.is_empty() {
            &s.name
        } else {
            &s.signature
        };
        serde_json::json!({
            "name": format!("{kind} {}", s.name),
            "startLine": s.start_line,
            "endLine": s.end_line,
            "signature": truncate_signature(sig, 80),
        })
    }

    let enclosing_str = enclosing.map(format_sym);
    let enclosing_json = enclosing.map(sym_to_json);
    let before_strs: Vec<String> = nearby_before.iter().map(|s| format_sym(s)).collect();
    let after_strs: Vec<String> = nearby_after.iter().map(|s| format_sym(s)).collect();
    let all_nearby_json: Vec<serde_json::Value> = nearby_before
        .iter()
        .chain(nearby_after.iter())
        .map(|s| sym_to_json(s))
        .collect();

    if enclosing_str.is_none() && before_strs.is_empty() && after_strs.is_empty() {
        return None;
    }

    Some(NavigationContext {
        enclosing: enclosing_str,
        nearby_before: before_strs,
        nearby_after: after_strs,
        enclosing_symbol: enclosing_json,
        nearby_symbols: all_nearby_json,
    })
}

/// Format a NavigationContext into a compact text header for prepending to read output.
fn format_navigation_header(
    nav: &NavigationContext,
    _read_start: usize,
    _read_end: usize,
    _total_lines: usize,
) -> String {
    let mut header = String::from("── Navigation ──\n");
    if let Some(ref enc) = nav.enclosing {
        header.push_str(&format!("  Enclosing: {enc}\n"));
    }
    if !nav.nearby_before.is_empty() {
        let items: Vec<&str> = nav.nearby_before.iter().map(|s| s.as_str()).collect();
        header.push_str(&format!("  Before: {}\n", items.join(", ")));
    }
    if !nav.nearby_after.is_empty() {
        let items: Vec<&str> = nav.nearby_after.iter().map(|s| s.as_str()).collect();
        header.push_str(&format!("  After: {}\n", items.join(", ")));
    }
    header.push_str("────────────────\n");
    header
}

/// Search text across files under a directory.
/// Uses ripgrep (rg) when available for blazing fast search; otherwise falls back to built-in Rust implementation.
pub struct SearchInFilesTool;

/// Check if ripgrep is available on the system.
fn is_ripgrep_available() -> bool {
    std::process::Command::new("rg")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Execute search via ripgrep for much faster results on large codebases.
async fn search_via_ripgrep(
    pattern: &str,
    scope: &Path,
    glob: Option<&str>,
    case_sensitive: bool,
    max_results: usize,
    context_lines: usize,
    root: &Path,
) -> Result<(String, usize, usize, bool), String> {
    let mut cmd = tokio::process::Command::new("rg");
    cmd.arg("--json"); // JSON output for structured parsing
    cmd.arg("--max-count").arg(max_results.to_string());

    if !case_sensitive {
        cmd.arg("--ignore-case");
    }
    if context_lines > 0 {
        cmd.arg("-C").arg(context_lines.to_string());
    }
    if let Some(g) = glob {
        cmd.arg("--glob").arg(g);
    }

    cmd.arg("--").arg(pattern).arg(scope);
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let output = tokio::time::timeout(tokio::time::Duration::from_secs(30), cmd.output())
        .await
        .map_err(|_| "ripgrep search timed out after 30s".to_string())?
        .map_err(|e| format!("ripgrep execution failed: {e}"))?;

    // rg exits with 1 when no matches found, 2+ on error
    if output.status.code().unwrap_or(-1) > 1 {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ripgrep error: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut text_output = String::new();
    let mut match_count = 0usize;
    let mut matched_files = std::collections::HashSet::new();
    let mut current_file = String::new();
    let mut truncated = false;

    for line in stdout.lines() {
        let json: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match json.get("type").and_then(|t| t.as_str()) {
            Some("match") => {
                if match_count >= max_results {
                    truncated = true;
                    break;
                }
                let data = &json["data"];
                let path_text = data["path"]["text"].as_str().unwrap_or("");
                let rel_path = Path::new(path_text)
                    .strip_prefix(root)
                    .unwrap_or(Path::new(path_text))
                    .to_string_lossy();
                let line_number = data["line_number"].as_u64().unwrap_or(0);
                let line_text = data["lines"]["text"].as_str().unwrap_or("").trim_end();

                if current_file != rel_path.as_ref() {
                    if !current_file.is_empty() {
                        text_output.push_str("---\n");
                    }
                    current_file = rel_path.to_string();
                }
                text_output.push_str(&format!("{rel_path}:{line_number}:{line_text}\n"));
                matched_files.insert(rel_path.to_string());
                match_count += 1;
            }
            Some("context") => {
                if context_lines > 0 {
                    let data = &json["data"];
                    let path_text = data["path"]["text"].as_str().unwrap_or("");
                    let rel_path = Path::new(path_text)
                        .strip_prefix(root)
                        .unwrap_or(Path::new(path_text))
                        .to_string_lossy();
                    let line_number = data["line_number"].as_u64().unwrap_or(0);
                    let line_text = data["lines"]["text"].as_str().unwrap_or("").trim_end();
                    text_output.push_str(&format!("{rel_path}-{line_number}-{line_text}\n"));
                }
            }
            _ => {}
        }
    }

    Ok((text_output, match_count, matched_files.len(), truncated))
}

/// Fallback: built-in Rust search when ripgrep is not available.
fn search_builtin(
    pattern: &str,
    files: &[PathBuf],
    glob_filter: Option<&str>,
    case_sensitive: bool,
    max_results: usize,
    context_lines: usize,
    root: &Path,
) -> Result<(String, usize, usize, bool), String> {
    let regex = RegexBuilder::new(pattern)
        .case_insensitive(!case_sensitive)
        .build()
        .map_err(|_| format!("Invalid regex pattern '{pattern}'. Please check the syntax."))?;

    let mut text_output = String::new();
    let mut match_count = 0usize;
    let mut matched_files = 0usize;
    let mut truncated = false;

    for file in files {
        if match_count >= max_results {
            truncated = true;
            break;
        }
        let rel = file
            .strip_prefix(root)
            .unwrap_or(file.as_path())
            .to_string_lossy()
            .to_string();
        if let Some(glob) = glob_filter {
            if !simple_glob_match(glob, &rel) {
                continue;
            }
        }

        let content = match fs::read_to_string(file) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let all_lines: Vec<&str> = content.lines().collect();
        let mut file_hit = false;
        for (line_no, line) in all_lines.iter().enumerate() {
            if match_count >= max_results {
                truncated = true;
                break;
            }
            if regex.is_match(line) {
                if !file_hit && !text_output.is_empty() {
                    text_output.push_str("---\n");
                }

                if context_lines > 0 {
                    let ctx_start = line_no.saturating_sub(context_lines);
                    for (i, ctx_line) in all_lines[ctx_start..line_no].iter().enumerate() {
                        text_output.push_str(&format!("{rel}-{}-{ctx_line}\n", ctx_start + i + 1));
                    }
                }

                text_output.push_str(&format!("{rel}:{}:{line}\n", line_no + 1));

                if context_lines > 0 {
                    let ctx_end = (line_no + context_lines + 1).min(all_lines.len());
                    for (i, ctx_line) in all_lines[(line_no + 1)..ctx_end].iter().enumerate() {
                        text_output.push_str(&format!("{rel}-{}-{ctx_line}\n", line_no + 2 + i));
                    }
                }

                file_hit = true;
                match_count += 1;
            }
        }
        if file_hit {
            matched_files += 1;
        }
    }

    Ok((text_output, match_count, matched_files, truncated))
}

#[async_trait]
impl Tool for SearchInFilesTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }
    fn supports_parallel(&self) -> bool {
        true
    }
    fn name(&self) -> &str {
        "search_in_files"
    }

    fn max_result_size_chars(&self) -> usize {
        20_000
    }

    fn description(&self) -> &str {
        "Search files using regex. Returns matches with file paths and line numbers in ripgrep-style format. \
         Uses ripgrep (rg) for fast search when available; falls back to built-in implementation. \
         Case-insensitive by default. Supports glob filter, context lines (0-5). Respects .gitignore. \
         Set semantic_context=true to annotate each match with the enclosing function/class/struct."
    }

    fn prompt(&self) -> String {
        "A powerful search tool built on ripgrep.\n\n\
Usage:\n\
- ALWAYS use search_in_files for search tasks. NEVER invoke `grep` or `rg` as a shell_exec command. \
This tool has been optimized for correct permissions and access\n\
- Supports full regex syntax (e.g., \"log.*Error\", \"function\\\\s+\\\\w+\")\n\
- Filter files with glob parameter (e.g., \"*.js\", \"**/*.tsx\")\n\
- Output modes: \"content\" shows matching lines, \"files_with_matches\" shows only file paths, \
\"count\" shows match counts\n\
- Pattern syntax: Uses ripgrep (not grep) — literal braces need escaping \
(use `interface\\\\{\\\\}` to find `interface{}` in Go code)\n\
- Multiline matching: By default patterns match within single lines only. For cross-line patterns \
like `struct \\\\{[\\\\s\\\\S]*?field`, use `multiline: true`\n\
- Case-insensitive by default\n\
- Respects .gitignore automatically\n\
- semantic_context: when true, each match is annotated with the enclosing symbol \
(function, struct, class) so you can understand WHERE in the code the match lives \
without extra read_file calls. Use for exploratory queries like \
\"find all places that handle user login\" or \"where is this error processed\"".to_string()
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "pattern".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Regex pattern to search for. Example: \"fn\\s+main\", \"TODO|FIXME\"."
            }),
        );
        props.insert(
            "path".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional scope path (default '.'). Can be a directory or a single file."
            }),
        );
        props.insert(
            "glob".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional glob filter for file names, e.g. '*.rs', '*.{ts,tsx}', 'src/*.ts'."
            }),
        );
        props.insert(
            "case_sensitive".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "Optional. Defaults to false (case-insensitive). Set true for case-sensitive search."
            }),
        );
        props.insert(
            "max_results".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Optional cap on returned matches. Default 200, max 2000."
            }),
        );
        props.insert(
            "context_lines".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Optional number of lines to show before and after each match (0-15). Default 0. Useful for understanding match context without a separate read_file call."
            }),
        );
        props.insert(
            "semantic_context".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "Optional. When true, enriches each match with structural context: which function, class, or struct the match is inside. Helps understand search results without extra read_file calls. Default false."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["pattern".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: SearchInFilesArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::err(format!(
                    "search_in_files arguments are not valid JSON: {e}. \
                     Pass {{\"pattern\":\"...\"}} with optional path/glob/case_sensitive/max_results/context_lines."
                ))
            }
        };

        if args.pattern.trim().is_empty() {
            return ToolResult::err("search_in_files requires non-empty 'pattern'.".to_string());
        }

        let scope = args.path.as_deref().unwrap_or(".");
        let validated = match ensure_within_workspace(Path::new(scope), true) {
            Ok(p) => p,
            Err(e) => {
                let err_type = classify_workspace_error(&e);
                return ToolResult::typed_err(
                    err_type,
                    create_user_friendly_error(err_type, scope),
                );
            }
        };
        let root = match workspace_root().and_then(|p| p.canonicalize()) {
            Ok(p) => p,
            Err(e) => {
                return ToolResult::err(format!(
                    "search_in_files failed to resolve workspace root: {e}"
                ))
            }
        };

        let case_sensitive = args.case_sensitive.unwrap_or(false);
        let max_results = args.max_results.unwrap_or(200).clamp(1, 2000);
        let ctx = args.context_lines.unwrap_or(0).min(15);

        // Try ripgrep first for speed (respects .gitignore natively)
        let use_rg = is_ripgrep_available();
        let (text_output, match_count, matched_files, truncated) = if use_rg {
            match search_via_ripgrep(
                &args.pattern,
                &validated,
                args.glob.as_deref(),
                case_sensitive,
                max_results,
                ctx,
                &root,
            )
            .await
            {
                Ok(result) => result,
                Err(rg_err) => {
                    // Fallback to built-in on rg failure
                    tracing::warn!("ripgrep failed, falling back to built-in: {rg_err}");
                    let gitignore = load_gitignore_patterns(&root);
                    let mut files = if validated.is_file() {
                        vec![validated.clone()]
                    } else {
                        collect_text_files_filtered(&validated, 50_000, &gitignore, &root)
                            .unwrap_or_default()
                    };
                    files.sort();
                    match search_builtin(
                        &args.pattern,
                        &files,
                        args.glob.as_deref(),
                        case_sensitive,
                        max_results,
                        ctx,
                        &root,
                    ) {
                        Ok(r) => r,
                        Err(e) => {
                            return ToolResult::typed_err(ToolErrorType::GrepExecutionError, e)
                        }
                    }
                }
            }
        } else {
            let gitignore = load_gitignore_patterns(&root);
            let mut files = if validated.is_file() {
                vec![validated.clone()]
            } else {
                match collect_text_files_filtered(&validated, 50_000, &gitignore, &root) {
                    Ok(v) => v,
                    Err(_) => {
                        return ToolResult::typed_err(
                            ToolErrorType::GrepExecutionError,
                            format!("Could not search in '{}'. Check directory permissions or if the path exists.", scope),
                        )
                    }
                }
            };
            files.sort();
            match search_builtin(
                &args.pattern,
                &files,
                args.glob.as_deref(),
                case_sensitive,
                max_results,
                ctx,
                &root,
            ) {
                Ok(r) => r,
                Err(e) => return ToolResult::typed_err(ToolErrorType::GrepExecutionError, e),
            }
        };

        // Semantic context enrichment: annotate each match with the enclosing
        // symbol (function/struct/class/method) so the LLM can understand
        // results structurally without extra read_file calls.
        let enriched_output = if args.semantic_context.unwrap_or(false) && match_count > 0 {
            enrich_search_with_symbols(&text_output, &root)
        } else {
            text_output.clone()
        };

        // LLM-friendly text output (like ripgrep format)
        let header = format!(
            "Found {} matches in {} files for pattern \"{}\" in \"{}\"{}:\n",
            match_count,
            matched_files,
            args.pattern,
            scope,
            if let Some(ref g) = args.glob {
                format!(" (filter: \"{}\")", g)
            } else {
                String::new()
            },
        );
        let truncation_note = if truncated {
            format!("\n[Results truncated at {} matches]", max_results)
        } else {
            String::new()
        };
        let llm_output = format!("{header}{enriched_output}{truncation_note}");

        // JSON metadata for the UI
        let mut result = ToolResult::ok_split(
            &llm_output,
            serde_json::json!({
                "pattern": args.pattern,
                "scope": scope,
                "glob": args.glob,
                "case_sensitive": case_sensitive,
                "count": match_count,
                "matched_files": matched_files,
                "truncated": truncated,
                "engine": if use_rg { "ripgrep" } else { "builtin" },
                "text": text_output,
            })
            .to_string(),
        );
        result.metadata = Some(serde_json::json!({
            "engine": if use_rg { "ripgrep" } else { "builtin" },
            "matchCount": match_count,
            "matchedFiles": matched_files,
        }));
        result
    }
}

/// Apply multiple string replacement edits to one file atomically.
/// Coexists with `edit_file` / `multi_edit`: use `apply_patch` for large batch
/// changes across many locations in a single file, `edit_file` for precision edits.
pub struct ApplyPatchTool;

#[async_trait]
impl Tool for ApplyPatchTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn name(&self) -> &str {
        "apply_patch"
    }

    fn is_deferred(&self) -> bool {
        true
    }

    fn search_hint(&self) -> &str {
        "patch batch edit replace multiple edits atomic file"
    }

    fn description(&self) -> &str {
        "Apply multiple exact-string edits to a single UTF-8 file and write once atomically. \
         This is a patch-style safer alternative to full file rewrite. \
         Each edit includes old_string and new_string; supports replace_all and expected_replacements."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "path".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Target file path."
            }),
        );
        props.insert(
            "edits".to_string(),
            serde_json::json!({
                "type": "array",
                "description": "List of edit objects: { old_string, new_string, replace_all?, expected_replacements? }"
            }),
        );
        props.insert(
            "expected_content".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional optimistic lock baseline. File content must match exactly before patching."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["file_path".to_string(), "edits".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: ApplyPatchArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::err(format!(
                    "apply_patch arguments are not valid JSON: {e}. \
                     Pass {{\"file_path\":\"/absolute/path\", \"edits\":[{{\"old_string\":\"...\", \"new_string\":\"...\"}}]}}."
                ))
            }
        };

        if args.edits.is_empty() {
            return ToolResult::err("apply_patch requires at least one edit.".to_string());
        }

        let validated = match ensure_within_workspace(Path::new(&args.file_path), true) {
            Ok(p) => p,
            Err(e) => {
                let err_type = classify_workspace_error(&e);
                return ToolResult::typed_err(
                    err_type,
                    create_user_friendly_error(err_type, &args.file_path),
                );
            }
        };

        let raw_bytes = match tokio::fs::read(&validated).await {
            Ok(b) => b,
            Err(e) => {
                let err_type = if e.kind() == std::io::ErrorKind::NotFound {
                    ToolErrorType::FileNotFound
                } else if e.kind() == std::io::ErrorKind::PermissionDenied {
                    ToolErrorType::PermissionDenied
                } else {
                    ToolErrorType::ReadContentFailure
                };
                return ToolResult::typed_err(
                    err_type,
                    create_user_friendly_error(err_type, &args.file_path),
                );
            }
        };

        let (current, enc_meta) = detect_file_encoding_meta(&raw_bytes);

        if let Some(expected) = args.expected_content.as_deref() {
            if current != expected {
                return ToolResult::err(format!(
                    "apply_patch optimistic lock failed for '{}': content differs from expected_content.",
                    args.file_path
                ));
            }
        }

        let mut working = current.replace("\r\n", "\n");

        let mut applied = Vec::new();
        for (idx, edit) in args.edits.iter().enumerate() {
            if edit.old_string.is_empty() {
                return ToolResult::err(format!(
                    "apply_patch edit #{idx} has empty old_string, which is not allowed."
                ));
            }
            let old_norm = edit.old_string.replace("\r\n", "\n");
            let new_norm = edit.new_string.replace("\r\n", "\n");

            let match_count = working.matches(&old_norm).count();
            if let Some(expected) = edit.expected_replacements {
                if match_count != expected {
                    return ToolResult::err(format!(
                        "apply_patch edit #{idx} expected {expected} matches but found {match_count}."
                    ));
                }
            }

            if match_count == 0 {
                match try_fuzzy_match(&working, &old_norm) {
                    FuzzyMatchResult::UniqueMatch { start, end } => {
                        let mut result = String::with_capacity(working.len());
                        result.push_str(&working[..start]);
                        result.push_str(&new_norm);
                        result.push_str(&working[end..]);
                        working = result;
                        applied.push(serde_json::json!({
                            "edit_index": idx,
                            "replacements": 1,
                            "fuzzy": true,
                        }));
                        continue;
                    }
                    FuzzyMatchResult::NoMatch => {
                        let file_preview: String = working
                            .lines()
                            .take(15)
                            .enumerate()
                            .map(|(i, l)| format!("{}|{}", i + 1, l))
                            .collect::<Vec<_>>()
                            .join("\n");
                        return ToolResult::err(format!(
                            "apply_patch edit #{idx} found no matches for old_string \
                             (neither exact nor whitespace-normalized). \
                             Re-read the file to get current content.\n\
                             File preview:\n{file_preview}"
                        ));
                    }
                    FuzzyMatchResult::MultipleMatches(n) => {
                        return ToolResult::err(format!(
                            "apply_patch edit #{idx} found {n} fuzzy matches (0 exact). \
                             Provide more context or set replace_all=true."
                        ));
                    }
                }
            }

            if !edit.replace_all && edit.expected_replacements.is_none() && match_count > 1 {
                return ToolResult::err(format!(
                    "apply_patch edit #{idx} found {match_count} matches; set replace_all=true or expected_replacements to disambiguate."
                ));
            }
            let replaced = if edit.replace_all { match_count } else { 1 };
            working = if edit.replace_all {
                working.replace(&old_norm, &new_norm)
            } else {
                working.replacen(&old_norm, &new_norm, 1)
            };
            applied.push(serde_json::json!({
                "edit_index": idx,
                "replacements": replaced,
            }));
        }

        let original_normalized = current.replace("\r\n", "\n");
        let (lines_added, lines_removed) = count_diff_lines(&original_normalized, &working);
        let diff_stat = format!("+{lines_added} -{lines_removed} lines");

        let write_bytes = encode_with_meta(&working, &enc_meta);

        match atomic_write_bytes(&validated, &write_bytes).await {
            Ok(()) => ToolResult::ok(
                serde_json::json!({
                    "patched": true,
                    "file_path": args.file_path,
                    "edits_applied": applied,
                    "bytes": write_bytes.len(),
                    "encoding": enc_meta.encoding,
                    "diffStat": diff_stat,
                    "linesAdded": lines_added,
                    "linesRemoved": lines_removed,
                })
                .to_string(),
            ),
            Err(_) => ToolResult::typed_err(
                ToolErrorType::FileWriteFailure,
                format!(
                    "Could not write to file '{}'. Check file permissions or disk space.",
                    args.file_path
                ),
            ),
        }
    }
}

/// List files and directories at a given path.
pub struct ListDirectoryTool;

#[async_trait]
impl Tool for ListDirectoryTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn supports_parallel(&self) -> bool {
        true
    }
    fn name(&self) -> &str {
        "list_directory"
    }

    fn description(&self) -> &str {
        "List immediate children of a directory. Returns name, type (file/directory/symlink), and size. \
         Non-recursive — one level per call. Use glob for recursive file search."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "path".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Directory path (absolute or relative to cwd)."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["path".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::err(format!(
                    "list_directory arguments are not valid JSON: {e}. \
                 Pass exactly {{\"path\": \"your/dir\"}} with a string path, then retry."
                ))
            }
        };

        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return ToolResult::err(
                    "list_directory is missing string field 'path'. \
                 Example: {\"path\": \".\"} or {\"path\": \"src/components\"}. \
                 Use read_file if you need the contents of a single file."
                        .to_string(),
                )
            }
        };

        let mut entries = Vec::new();
        let validated = match ensure_within_workspace(Path::new(path), true) {
            Ok(p) => p,
            Err(e) => {
                let err_type = classify_workspace_error(&e);
                return ToolResult::typed_err(err_type, create_user_friendly_error(err_type, path));
            }
        };

        let mut dir = match tokio::fs::read_dir(&validated).await {
            Ok(d) => d,
            Err(e) => return ToolResult::err(format!(
                "list_directory could not open '{path}': {e}. \
                 Confirm the path exists, is a directory (not a file), and is readable. \
                 If ENOENT, list the parent with list_directory or fix the path; if permission denied, choose a readable directory; if ENOTDIR, you passed a file—use read_file instead."
            )),
        };

        while let Ok(Some(entry)) = dir.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            let file_type = match entry.file_type().await {
                Ok(ft) => {
                    if ft.is_dir() {
                        "directory"
                    } else if ft.is_symlink() {
                        "symlink"
                    } else {
                        "file"
                    }
                }
                Err(_) => "unknown",
            };
            let size = entry.metadata().await.map(|m| m.len()).unwrap_or(0);
            entries.push(serde_json::json!({
                "name": name,
                "type": file_type,
                "size": size,
            }));
        }

        entries.sort_by(|a, b| {
            let a_name = a.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let b_name = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
            a_name.cmp(b_name)
        });

        ToolResult::ok(
            serde_json::json!({
                "path": path,
                "entries": entries,
                "count": entries.len(),
            })
            .to_string(),
        )
    }
}

// ─── Glob (file pattern search) ──────────────────────────────────────

pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn supports_parallel(&self) -> bool {
        true
    }
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Find files by glob pattern. Recursive search, results sorted by modification time (newest first). \
         Respects .gitignore patterns. Examples: '*.rs', 'src/**/*.tsx', '**/test_*.py'. Returns up to 100 results."
    }

    fn prompt(&self) -> String {
        "Fast file pattern matching tool that works with any codebase size.\n\n\
- Supports glob patterns like \"**/*.js\" or \"src/**/*.ts\"\n\
- Returns matching file paths sorted by modification time\n\
- Use this tool when you need to find files by name patterns\n\
- Respects .gitignore patterns automatically\n\
- Returns up to 100 results\n\
- Patterns not starting with \"**/\" are automatically prepended with \"**/\" for recursive searching\n\n\
Examples:\n\
- \"*.rs\" → find all Rust source files\n\
- \"src/**/*.tsx\" → find all TSX files under src/\n\
- \"**/Cargo.toml\" → find all Cargo.toml files\n\
- \"**/test_*.py\" → find all Python test files".to_string()
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert("pattern".to_string(), serde_json::json!({
            "type": "string",
            "description": "Glob pattern to match files against. Examples: '*.rs', 'src/**/*.tsx', '**/Cargo.toml'."
        }));
        props.insert(
            "path".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Directory to search in (default '.'). Relative to workspace root."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["pattern".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("glob: invalid JSON: {e}")),
        };

        let pattern = match args.get("pattern").and_then(|v| v.as_str()) {
            Some(p) if !p.trim().is_empty() => p.trim(),
            _ => {
                return ToolResult::err(
                    "glob requires a non-empty 'pattern'. Example: {\"pattern\": \"*.rs\"}"
                        .to_string(),
                )
            }
        };
        let base = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        let base_dir = match ensure_within_workspace(Path::new(base), true) {
            Ok(p) => p,
            Err(e) => {
                let err_type = classify_workspace_error(&e);
                return ToolResult::typed_err(err_type, create_user_friendly_error(err_type, base));
            }
        };

        let root = workspace_root()
            .and_then(|p| p.canonicalize())
            .unwrap_or_else(|_| base_dir.clone());
        let gitignore = load_gitignore_patterns(&root);

        let full_pattern = if pattern.starts_with("**/") || pattern.contains('/') {
            base_dir.join(pattern).to_string_lossy().to_string()
        } else {
            base_dir
                .join("**")
                .join(pattern)
                .to_string_lossy()
                .to_string()
        };

        let options = glob::MatchOptions {
            case_sensitive: true,
            require_literal_separator: false,
            require_literal_leading_dot: true,
        };

        let mut entries: Vec<(PathBuf, u64)> = Vec::new();
        match glob::glob_with(&full_pattern, options) {
            Ok(paths) => {
                for entry in paths.flatten() {
                    if !entry.is_file() {
                        continue;
                    }
                    // Filter out .gitignore'd files
                    let rel = entry
                        .strip_prefix(&root)
                        .unwrap_or(entry.as_path())
                        .to_string_lossy()
                        .to_string();
                    if gitignore.is_ignored(&rel) {
                        continue;
                    }
                    // Skip common noise directories
                    let rel_lower = rel.to_lowercase();
                    if rel_lower.contains("node_modules/")
                        || rel_lower.contains(".git/")
                        || rel_lower.contains("/target/")
                    {
                        continue;
                    }
                    let mtime = entry
                        .metadata()
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    entries.push((entry, mtime));
                    if entries.len() >= 200 {
                        break;
                    }
                }
            }
            Err(e) => return ToolResult::err(format!("glob: invalid pattern: {e}")),
        }

        entries.sort_by(|a, b| b.1.cmp(&a.1));
        let max_results = 100;
        let truncated = entries.len() > max_results;
        entries.truncate(max_results);

        let effective_root = workspace_root().unwrap_or_else(|_| std::env::current_dir().unwrap_or_default());
        let file_list: Vec<String> = entries
            .iter()
            .map(|(p, _)| {
                p.strip_prefix(&effective_root)
                    .unwrap_or(p)
                    .to_string_lossy()
                    .to_string()
            })
            .collect();

        let total = file_list.len();

        ToolResult::ok(
            serde_json::json!({
                "pattern": pattern,
                "path": base,
                "files": file_list,
                "count": total,
                "truncated": truncated,
            })
            .to_string(),
        )
    }
}

// ─── Multi-file Atomic Edit ─────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct MultiEditArgs {
    edits: Vec<MultiEditFileEntry>,
    #[serde(default)]
    dry_run: bool,
    /// If true, run `git stash push` before the transaction and `git stash pop`
    /// if all writes succeed. On failure, the stash remains for manual recovery.
    #[serde(default)]
    auto_snapshot: bool,
}

#[derive(Debug, Deserialize)]
struct MultiEditFileEntry {
    #[serde(alias = "path")]
    file_path: String,
    changes: Vec<MultiEditChange>,
}

#[derive(Debug, Deserialize)]
struct MultiEditChange {
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
    expected_replacements: Option<usize>,
    #[serde(default)]
    match_mode: MatchMode,
    start_line: Option<usize>,
    end_line: Option<usize>,
}

/// Atomically apply edits across multiple files.
///
/// All files are validated and patched in memory first.
/// Only if every file's edits succeed are the results written to disk.
/// If any edit in any file fails, no files are modified (all-or-nothing).
pub struct MultiEditTool;

#[async_trait]
impl Tool for MultiEditTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn name(&self) -> &str {
        "multi_edit"
    }

    fn description(&self) -> &str {
        "Atomically apply edits across MULTIPLE files with rollback safety. All edits are validated \
         in memory first; only if every edit succeeds are results written to disk. If any write fails \
         after partial commit, previously written files are automatically restored from backup. \
         Each change supports start_line/end_line for line-range fallback when text matching fails. \
         Set auto_snapshot=true to create a git stash before the transaction. \
         Use dry_run=true to validate without writing. \
         For single-file multi-edit, prefer edit_file with the 'edits' array parameter instead."
    }

    fn search_hint(&self) -> &str {
        "multi file cross atomic batch refactor rename"
    }

    fn is_deferred(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert("edits".to_string(), serde_json::json!({
            "type": "array",
            "description": "List of file edit entries. Each entry: { path: string, changes: [{ old_string, new_string, replace_all?, expected_replacements?, match_mode?, start_line?, end_line? }] }. \
                            match_mode per change: 'exact' (default), 'fuzzy' (whitespace-tolerant), 'contains' (substring). \
                            start_line/end_line: optional 1-indexed line-range hint for fallback when text matching fails."
        }));
        props.insert(
            "dry_run".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "If true, validate all edits but do not write. Default false."
            }),
        );
        props.insert("auto_snapshot".to_string(), serde_json::json!({
            "type": "boolean",
            "description": "If true, create a git stash snapshot before writing. On success the stash is popped; \
                            on failure the stash remains for manual recovery via `git stash pop`. Default false."
        }));
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["edits".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: MultiEditArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "multi_edit invalid JSON: {e}. Expected {{\"edits\": [{{\"path\": \"...\", \"changes\": [...]}}]}}."
            )),
        };

        if args.edits.is_empty() {
            return ToolResult::err("multi_edit requires at least one file entry.".to_string());
        }

        let mut staged: Vec<(PathBuf, String, Vec<u8>, Vec<serde_json::Value>, usize, usize)> = Vec::new();

        for (file_idx, entry) in args.edits.iter().enumerate() {
            let validated = match ensure_within_workspace(Path::new(&entry.file_path), true) {
                Ok(p) => p,
                Err(e) => {
                    let err_type = classify_workspace_error(&e);
                    let reason = if err_type == ToolErrorType::FileNotFound {
                        format!("multi_edit: file #{file_idx} '{}' does not exist. Transaction aborted, no files modified.", entry.file_path)
                    } else {
                        format!("multi_edit: file #{file_idx} '{}' is outside the workspace. Transaction aborted, no files modified.", entry.file_path)
                    };
                    return ToolResult::typed_err(err_type, reason);
                }
            };

            let raw_bytes = match tokio::fs::read(&validated).await {
                Ok(b) => b,
                Err(e) => return ToolResult::err(format!(
                    "multi_edit: could not read file #{file_idx} '{}': {e}. Transaction aborted, no files modified.",
                    entry.file_path
                )),
            };

            let (original, enc_meta) = detect_file_encoding_meta(&raw_bytes);
            if original.is_empty() && !raw_bytes.is_empty() {
                return ToolResult::err(format!(
                    "multi_edit: file #{file_idx} '{}' contains binary data. Transaction aborted.",
                    entry.file_path
                ));
            }

            let mut current = original.replace("\r\n", "\n");
            let mut change_log = Vec::new();

            for (change_idx, change) in entry.changes.iter().enumerate() {
                let label = format!(
                    "multi_edit: file #{file_idx} '{}', change #{change_idx}",
                    entry.file_path
                );
                match apply_single_change(
                    &current,
                    &change.old_string,
                    &change.new_string,
                    change.replace_all,
                    change.expected_replacements,
                    change.match_mode,
                    change.start_line,
                    change.end_line,
                    &label,
                ) {
                    ApplyChangeResult::Ok {
                        new_content,
                        mut log_entry,
                    } => {
                        current = new_content;
                        log_entry
                            .as_object_mut()
                            .unwrap()
                            .insert("change_index".to_string(), serde_json::json!(change_idx));
                        change_log.push(log_entry);
                    }
                    ApplyChangeResult::Err(msg) => {
                        return ToolResult::err(format!(
                            "{msg} Transaction aborted, no files modified."
                        ));
                    }
                }
            }

            let original_normalized = original.replace("\r\n", "\n");
            let (lines_added, lines_removed) = count_diff_lines(&original_normalized, &current);
            let final_bytes = encode_with_meta(&current, &enc_meta);

            staged.push((validated, entry.file_path.clone(), final_bytes, change_log, lines_added, lines_removed));
        }

        if args.dry_run {
            let results: Vec<serde_json::Value> = staged
                .iter()
                .map(|(_, fp, bytes, log, added, removed)| {
                    serde_json::json!({
                        "file_path": fp,
                        "changes_applied": log,
                        "result_bytes": bytes.len(),
                        "linesAdded": added,
                        "linesRemoved": removed,
                    })
                })
                .collect();

            return ToolResult::ok(
                serde_json::json!({
                    "dry_run": true,
                    "files": results,
                    "count": results.len(),
                    "status": "all edits valid, no files written",
                })
                .to_string(),
            );
        }

        // Git stash snapshot (optional pre-transaction safety net).
        let stash_created = if args.auto_snapshot {
            match create_git_stash_snapshot().await {
                Ok(created) => created,
                Err(e) => {
                    return ToolResult::err(format!(
                        "multi_edit: auto_snapshot failed to create git stash: {e}. Transaction aborted."
                    ));
                }
            }
        } else {
            false
        };

        // Back up original bytes for each file before writing, for rollback.
        let mut backups: Vec<(PathBuf, Vec<u8>)> = Vec::new();
        for (validated_path, _, _, _, _, _) in &staged {
            match tokio::fs::read(validated_path).await {
                Ok(original_bytes) => backups.push((validated_path.clone(), original_bytes)),
                Err(e) => {
                    return ToolResult::err(format!(
                        "multi_edit: could not back up '{}' before writing: {e}. Transaction aborted.",
                        validated_path.display()
                    ));
                }
            }
        }

        let mut written = Vec::new();
        for (validated_path, display_path, bytes, change_log, lines_added, lines_removed) in &staged {
            match atomic_write_bytes(validated_path, bytes).await {
                Ok(()) => {
                    if let Some(cache) = get_file_state_cache() {
                        if let Ok(text) = String::from_utf8(bytes.clone()) {
                            cache.update(validated_path, &text);
                        }
                    }
                    written.push(serde_json::json!({
                        "file_path": display_path,
                        "changes_applied": change_log,
                        "bytes": bytes.len(),
                        "linesAdded": lines_added,
                        "linesRemoved": lines_removed,
                    }));
                }
                Err(e) => {
                    let mut restored = Vec::new();
                    let mut restore_failed = Vec::new();
                    for (backup_path, backup_bytes) in &backups {
                        if written.iter().any(|w| {
                            w.get("file_path")
                                .and_then(|p| p.as_str())
                                .is_some_and(|fp| backup_path.ends_with(fp))
                        }) {
                            match atomic_write_bytes(backup_path, backup_bytes).await {
                                Ok(()) => restored.push(backup_path.display().to_string()),
                                Err(re) => {
                                    restore_failed.push(format!("{}: {re}", backup_path.display()))
                                }
                            }
                        }
                    }
                    let mut msg = format!("multi_edit: failed to write '{display_path}': {e}.");
                    if !restored.is_empty() {
                        msg.push_str(&format!(
                            " Rolled back {} file(s): {}.",
                            restored.len(),
                            restored.join(", ")
                        ));
                    }
                    if !restore_failed.is_empty() {
                        msg.push_str(&format!(
                            " WARNING: rollback failed for: {}.",
                            restore_failed.join("; ")
                        ));
                    }
                    if stash_created {
                        msg.push_str(" A git stash was created before the transaction — use `git stash pop` to restore.");
                    }
                    return ToolResult::err(msg);
                }
            }
        }

        // If auto_snapshot was used and everything succeeded, pop the stash.
        if stash_created {
            let _ = pop_git_stash_snapshot().await;
        }

        ToolResult::ok(
            serde_json::json!({
                "success": true,
                "files": written,
                "count": written.len(),
                "auto_snapshot": stash_created,
            })
            .to_string(),
        )
    }
}

/// Create a git stash as a pre-transaction snapshot.
/// Returns `true` if a new stash entry was created, `false` if working tree was clean.
async fn create_git_stash_snapshot() -> Result<bool, String> {
    let root = workspace_root().map_err(|e| format!("could not determine workspace root: {e}"))?;

    let output = tokio::process::Command::new("git")
        .args([
            "stash",
            "push",
            "-m",
            "xiaolin-multi-edit-snapshot",
            "--include-untracked",
        ])
        .current_dir(&root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("git stash push failed: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("No local changes") || stdout.contains("No local changes") {
            return Ok(false);
        }
        return Err(format!(
            "git stash push returned {}: {stderr}",
            output.status
        ));
    }
    Ok(!stdout.contains("No local changes"))
}

/// Pop the most recent git stash (the snapshot we created).
async fn pop_git_stash_snapshot() -> Result<(), String> {
    let root = workspace_root().map_err(|e| format!("could not determine workspace root: {e}"))?;

    let output = tokio::process::Command::new("git")
        .args(["stash", "pop"])
        .current_dir(&root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("git stash pop failed: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "git stash pop returned {}: {stderr}",
            output.status
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaolin_core::tool::Tool;
    use std::io::Write;
    use tempfile::tempdir_in;

    #[test]
    fn workspace_boundary_allows_repo_relative_paths() {
        let result = ensure_within_workspace(Path::new("."), true);
        assert!(result.is_ok());
    }

    #[test]
    fn workspace_boundary_rejects_outside_paths() {
        let result = ensure_within_workspace(Path::new("/tmp"), true);
        assert!(result.is_err());
    }

    #[test]
    fn well_known_prefixes_include_skill_dirs() {
        let prefixes = well_known_allowed_prefixes();
        assert!(!prefixes.is_empty(), "should have well-known prefixes");
        let names: Vec<String> = prefixes.iter().map(|p| p.display().to_string()).collect();
        let joined = names.join(", ");
        assert!(
            names.iter().any(|n| n.contains(".cursor/skills")),
            "should include .cursor/skills: {joined}"
        );
        assert!(
            names.iter().any(|n| n.contains(".agents/skills")),
            "should include .agents/skills: {joined}"
        );
        assert!(
            names.iter().any(|n| n.contains(".codex/skills")),
            "should include .codex/skills: {joined}"
        );
    }

    #[test]
    fn whitelist_allows_cursor_skills_path() {
        let home = dirs::home_dir().expect("home dir");
        let skill_dir = home.join(".cursor").join("skills");
        std::fs::create_dir_all(&skill_dir).ok();
        let skill_path = skill_dir.join("some-skill").join("SKILL.md");
        // Even if the file doesn't exist, the prefix should match
        assert!(
            is_path_under_allowed_prefixes(&skill_path, &skill_path),
            "~/.cursor/skills/some-skill/SKILL.md should be allowed"
        );
    }

    #[test]
    fn whitelist_rejects_random_home_path() {
        let home = dirs::home_dir().expect("home dir");
        let random_path = home.join("Desktop").join("random.txt");
        assert!(
            !is_path_under_allowed_prefixes(&random_path, &random_path),
            "~/Desktop/random.txt should NOT be allowed"
        );
    }

    #[test]
    fn permission_error_includes_allowed_locations() {
        let result = ensure_within_workspace(Path::new("/var/log/syslog"), true);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Allowed locations"),
            "error should list allowed locations: {err_msg}"
        );
        assert!(
            err_msg.contains("Full (YOLO)"),
            "error should suggest Full mode: {err_msg}"
        );
    }

    #[test]
    fn classify_workspace_error_returns_not_found_for_missing_files() {
        let e = std::io::Error::new(std::io::ErrorKind::NotFound, "No such file or directory");
        assert_eq!(classify_workspace_error(&e), ToolErrorType::FileNotFound);
    }

    #[test]
    fn classify_workspace_error_returns_path_not_in_workspace_for_outside_path() {
        let e = std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "path '/tmp/evil.txt' is outside all allowed locations.",
        );
        assert_eq!(
            classify_workspace_error(&e),
            ToolErrorType::PathNotInWorkspace
        );
    }

    #[test]
    fn classify_workspace_error_returns_permission_denied_for_os_perm() {
        let e = std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Operation not permitted",
        );
        assert_eq!(
            classify_workspace_error(&e),
            ToolErrorType::PermissionDenied
        );
    }

    #[test]
    fn classify_workspace_error_returns_path_not_in_workspace_for_plan_mode() {
        let e = std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "file access is disabled (execution mode = Plan).",
        );
        assert_eq!(
            classify_workspace_error(&e),
            ToolErrorType::PathNotInWorkspace
        );
    }

    #[tokio::test]
    async fn read_file_nonexistent_in_workspace_returns_not_found() {
        let cwd = std::env::current_dir().expect("current dir");
        let missing = cwd.join("_definitely_missing_file_xyz.txt");
        assert!(!missing.exists());

        let tool = ReadFileTool;
        let args = serde_json::json!({ "file_path": missing.to_string_lossy() }).to_string();
        let out =
            with_file_access_mode(FileAccessMode::Workspace, Tool::execute(&tool, &args)).await;
        assert!(!out.success);
        assert!(
            out.output.contains("does not exist"),
            "should say 'does not exist', not 'outside workspace': {}",
            out.output
        );
        assert_eq!(
            out.error_type,
            Some(ToolErrorType::FileNotFound),
            "error_type should be FileNotFound, not PathNotInWorkspace"
        );
    }

    #[tokio::test]
    async fn list_dir_nonexistent_in_workspace_returns_not_found() {
        let cwd = std::env::current_dir().expect("current dir");
        let missing = cwd.join("_definitely_missing_dir_xyz");
        assert!(!missing.exists());

        let tool = ListDirectoryTool;
        let args = serde_json::json!({ "path": missing.to_string_lossy() }).to_string();
        let out =
            with_file_access_mode(FileAccessMode::Workspace, Tool::execute(&tool, &args)).await;
        assert!(!out.success);
        assert_eq!(
            out.error_type,
            Some(ToolErrorType::FileNotFound),
            "error_type should be FileNotFound for missing dir, not PathNotInWorkspace"
        );
    }

    #[tokio::test]
    async fn read_file_allowed_in_cursor_skills_dir() {
        let home = dirs::home_dir().expect("home dir");
        let skill_dir = home
            .join(".cursor")
            .join("skills")
            .join("_test_whitelist_skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let skill_file = skill_dir.join("SKILL.md");
        tokio::fs::write(&skill_file, "# Test Skill\nContent here.")
            .await
            .unwrap();

        let tool = ReadFileTool;
        let args = serde_json::json!({ "file_path": skill_file.to_string_lossy() }).to_string();
        let out =
            with_file_access_mode(FileAccessMode::Workspace, Tool::execute(&tool, &args)).await;
        assert!(
            out.success,
            "workspace mode should allow reading ~/.cursor/skills/: {}",
            out.output
        );
        assert!(out.output.contains("Test Skill"));

        // Cleanup
        let _ = std::fs::remove_dir_all(&skill_dir);
    }

    #[tokio::test]
    async fn write_file_allowed_in_cursor_skills_dir() {
        let home = dirs::home_dir().expect("home dir");
        let skill_dir = home
            .join(".cursor")
            .join("skills")
            .join("_test_whitelist_write");
        let skill_file = skill_dir.join("SKILL.md");

        let tool = WriteFileTool;
        let args = serde_json::json!({
            "file_path": skill_file.to_string_lossy(),
            "content": "# New Skill\nCreated by test."
        })
        .to_string();
        let out =
            with_file_access_mode(FileAccessMode::Workspace, Tool::execute(&tool, &args)).await;
        assert!(
            out.success,
            "workspace mode should allow writing ~/.cursor/skills/: {}",
            out.output
        );

        let content = tokio::fs::read_to_string(&skill_file).await.unwrap();
        assert!(content.contains("New Skill"));

        // Cleanup
        let _ = std::fs::remove_dir_all(&skill_dir);
    }

    #[tokio::test]
    async fn desktop_path_still_rejected_in_workspace_mode() {
        let home = dirs::home_dir().expect("home dir");
        let desktop_file = home.join("Desktop").join("_test_whitelist_reject.txt");

        let tool = ReadFileTool;
        let args = serde_json::json!({ "file_path": desktop_file.to_string_lossy() }).to_string();
        let out =
            with_file_access_mode(FileAccessMode::Workspace, Tool::execute(&tool, &args)).await;
        assert!(
            !out.success,
            "workspace mode should reject ~/Desktop/ access"
        );
        assert!(
            out.output.contains("outside") || out.output.contains("Allowed locations"),
            "should mention path restriction: {}",
            out.output
        );
    }

    #[tokio::test]
    async fn search_in_files_finds_match_in_scoped_directory() {
        let cwd = std::env::current_dir().expect("current dir");
        let tmp = tempdir_in(&cwd).expect("temp dir in workspace");
        let file_path = tmp.path().join("sample.rs");
        tokio::fs::write(&file_path, "fn hello_world() {}\n")
            .await
            .expect("write sample");

        let tool = SearchInFilesTool;
        let args = serde_json::json!({
            "pattern": "hello_world",
            "path": tmp.path().to_string_lossy(),
            "glob": "*.rs"
        })
        .to_string();
        let out = Tool::execute(&tool, &args).await;
        assert!(out.success, "tool should succeed: {}", out.output);

        // LLM output is now text-based (ripgrep-style); verify it contains the match
        assert!(
            out.output.contains("hello_world"),
            "output should contain the match text: {}",
            out.output
        );
        assert!(
            out.output.contains("Found") && out.output.contains("match"),
            "output should have the header line: {}",
            out.output
        );

        // display_output contains JSON metadata
        if let Some(ref display) = out.display_output {
            let payload: serde_json::Value =
                serde_json::from_str(display).expect("display json payload");
            let count = payload.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
            assert!(count >= 1, "should have at least one match in metadata");
        }
    }

    #[tokio::test]
    async fn apply_patch_applies_multiple_edits() {
        let cwd = std::env::current_dir().expect("current dir");
        let tmp = tempdir_in(&cwd).expect("temp dir in workspace");
        let file_path = tmp.path().join("patch.txt");
        tokio::fs::write(&file_path, "alpha\nbeta\ngamma\n")
            .await
            .expect("write sample");

        let tool = ApplyPatchTool;
        let args = serde_json::json!({
            "path": file_path.to_string_lossy(),
            "edits": [
                {"old_string": "alpha", "new_string": "ALPHA"},
                {"old_string": "beta", "new_string": "BETA"}
            ]
        })
        .to_string();
        let out = Tool::execute(&tool, &args).await;
        assert!(out.success, "apply_patch should succeed: {}", out.output);

        let updated = tokio::fs::read_to_string(&file_path)
            .await
            .expect("read updated");
        assert!(updated.contains("ALPHA"));
        assert!(updated.contains("BETA"));
        assert!(updated.contains("gamma"));
    }

    #[tokio::test]
    async fn read_file_denied_when_file_access_none() {
        let cwd = std::env::current_dir().expect("current dir");
        let tmp = tempdir_in(&cwd).expect("temp dir in workspace");
        let file_path = tmp.path().join("deny.txt");
        tokio::fs::write(&file_path, "blocked\n")
            .await
            .expect("write sample");

        let tool = ReadFileTool;
        let args = serde_json::json!({ "file_path": file_path.to_string_lossy() }).to_string();
        let out = with_file_access_mode(FileAccessMode::None, Tool::execute(&tool, &args)).await;
        assert!(!out.success, "read_file should be blocked in none mode");
        assert!(
            out.output.contains("outside") || out.output.contains("file access is disabled"),
            "unexpected error output: {}",
            out.output
        );
    }

    #[tokio::test]
    async fn read_file_backward_compat_path_alias() {
        let cwd = std::env::current_dir().expect("current dir");
        let tmp = tempdir_in(&cwd).expect("temp dir in workspace");
        let file_path = tmp.path().join("compat.txt");
        tokio::fs::write(&file_path, "backward-compat\n")
            .await
            .expect("write sample");

        let tool = ReadFileTool;
        let args = serde_json::json!({ "path": file_path.to_string_lossy() }).to_string();
        let out = Tool::execute(&tool, &args).await;
        assert!(
            out.success,
            "old 'path' param should still work: {}",
            out.output
        );
        assert!(out.output.contains("backward-compat"));
    }

    #[tokio::test]
    async fn read_file_workspace_blocks_outside_path() {
        let mut temp = tempfile::NamedTempFile::new().expect("tmp file");
        writeln!(temp, "outside").expect("write outside file");
        let outside_path = temp.path().to_string_lossy().to_string();

        let tool = ReadFileTool;
        let args = serde_json::json!({ "file_path": outside_path }).to_string();
        let out =
            with_file_access_mode(FileAccessMode::Workspace, Tool::execute(&tool, &args)).await;
        assert!(!out.success, "workspace mode should block outside path");
        assert!(
            out.output.contains("outside") || out.output.contains("Allowed locations"),
            "unexpected error output: {}",
            out.output
        );
    }

    #[tokio::test]
    async fn read_file_full_allows_outside_workspace_path() {
        let mut temp = tempfile::NamedTempFile::new().expect("tmp file");
        writeln!(temp, "outside-ok").expect("write outside file");
        let outside_path = temp.path().to_string_lossy().to_string();

        let tool = ReadFileTool;
        let args = serde_json::json!({ "file_path": outside_path }).to_string();
        let out = with_file_access_mode(FileAccessMode::Full, Tool::execute(&tool, &args)).await;
        assert!(
            out.success,
            "full mode should allow outside path: {}",
            out.output
        );
        assert!(out.output.contains("outside-ok"));
    }

    #[tokio::test]
    async fn write_file_denied_when_file_access_none() {
        let cwd = std::env::current_dir().expect("current dir");
        let tmp = tempdir_in(&cwd).expect("temp dir in workspace");
        let file_path = tmp.path().join("deny-write.txt");
        let tool = WriteFileTool;
        let args = serde_json::json!({
            "path": file_path.to_string_lossy(),
            "content": "hello"
        })
        .to_string();

        let out = with_file_access_mode(FileAccessMode::None, Tool::execute(&tool, &args)).await;
        assert!(!out.success, "write_file should be blocked in none mode");
        assert!(out.output.contains("outside") || out.output.contains("file access is disabled"));
    }

    #[tokio::test]
    async fn edit_file_denied_when_workspace_mode_on_outside_path() {
        let mut temp = tempfile::NamedTempFile::new().expect("tmp file");
        writeln!(temp, "outside-edit").expect("write outside file");
        let outside_path = temp.path().to_string_lossy().to_string();

        let tool = EditFileTool;
        let args = serde_json::json!({
            "path": outside_path,
            "old_string": "outside-edit",
            "new_string": "changed"
        })
        .to_string();
        let out =
            with_file_access_mode(FileAccessMode::Workspace, Tool::execute(&tool, &args)).await;
        assert!(!out.success, "workspace mode should block outside edit");
        assert!(
            out.output.contains("outside") || out.output.contains("Allowed locations"),
            "unexpected: {}",
            out.output
        );
    }

    #[tokio::test]
    async fn search_in_files_denied_when_file_access_none() {
        let cwd = std::env::current_dir().expect("current dir");
        let tmp = tempdir_in(&cwd).expect("temp dir in workspace");
        let file_path = tmp.path().join("search.txt");
        tokio::fs::write(&file_path, "needle\n")
            .await
            .expect("write sample");

        let tool = SearchInFilesTool;
        let args = serde_json::json!({
            "pattern": "needle",
            "path": tmp.path().to_string_lossy()
        })
        .to_string();
        let out = with_file_access_mode(FileAccessMode::None, Tool::execute(&tool, &args)).await;
        assert!(
            !out.success,
            "search_in_files should be blocked in none mode"
        );
        assert!(
            out.output.contains("outside") || out.output.contains("file access is disabled"),
            "unexpected: {}",
            out.output
        );
    }

    #[tokio::test]
    async fn write_file_workspace_allows_new_subdir_in_workspace() {
        let cwd = std::env::current_dir().expect("current dir");
        let tmp = tempdir_in(&cwd).expect("temp dir in workspace");
        let new_subdir_file = tmp.path().join("brand-new-subdir").join("nested.txt");
        assert!(!new_subdir_file.parent().unwrap().exists());

        let tool = WriteFileTool;
        let args = serde_json::json!({
            "file_path": new_subdir_file.to_string_lossy(),
            "content": "created-in-new-subdir"
        })
        .to_string();
        let out =
            with_file_access_mode(FileAccessMode::Workspace, Tool::execute(&tool, &args)).await;
        assert!(
            out.success,
            "workspace mode should allow writing to a new sub-directory within the workspace: {}",
            out.output
        );

        let content = tokio::fs::read_to_string(&new_subdir_file).await.unwrap();
        assert!(content.contains("created-in-new-subdir"));
    }

    #[tokio::test]
    async fn write_file_workspace_rejects_dotdot_traversal_to_new_dir() {
        let cwd = std::env::current_dir().expect("current dir");
        let traversal_path = cwd
            .join("..")
            .join("..")
            .join("_traversal_test_dir")
            .join("evil.txt");

        let tool = WriteFileTool;
        let args = serde_json::json!({
            "file_path": traversal_path.to_string_lossy(),
            "content": "should-not-be-created"
        })
        .to_string();
        let out =
            with_file_access_mode(FileAccessMode::Workspace, Tool::execute(&tool, &args)).await;
        assert!(
            !out.success,
            "workspace mode must reject ../../ traversal even when parent doesn't exist: {}",
            out.output
        );
    }

    #[test]
    fn lexical_clean_resolves_dotdot_components() {
        let cleaned = lexical_clean(Path::new("/a/b/c/../../d"));
        assert_eq!(cleaned, PathBuf::from("/a/d"));

        let cleaned2 = lexical_clean(Path::new("/workspace/project/../../etc/passwd"));
        assert_eq!(cleaned2, PathBuf::from("/etc/passwd"));

        let cleaned3 = lexical_clean(Path::new("/workspace/./subdir/../subdir/file.txt"));
        assert_eq!(cleaned3, PathBuf::from("/workspace/subdir/file.txt"));
    }
}

#[cfg(test)]
mod new_feature_tests {
    use super::*;
    use xiaolin_core::tool::Tool;
    use tempfile::tempdir_in;

    #[test]
    fn fuzzy_match_normalizes_whitespace() {
        let content = "fn main() {\n    let x = 1;\n    let y = 2;\n}\n";
        // old_string with extra spaces
        let old = "fn  main() {\n  let  x = 1;\n  let y  = 2;\n}";
        match try_fuzzy_match(content, old) {
            FuzzyMatchResult::UniqueMatch { start, end } => {
                assert_eq!(start, 0);
                assert!(end > 0, "end should be > 0, got {end}");
            }
            FuzzyMatchResult::NoMatch => panic!("expected fuzzy match but got NoMatch"),
            FuzzyMatchResult::MultipleMatches(n) => panic!("expected unique match but got {n}"),
        }
    }

    #[test]
    fn fuzzy_match_returns_no_match_for_different_content() {
        let content = "fn main() {\n    let x = 1;\n}\n";
        let old = "fn other() {\n    let y = 2;\n}";
        assert!(matches!(
            try_fuzzy_match(content, old),
            FuzzyMatchResult::NoMatch
        ));
    }

    #[test]
    fn normalize_whitespace_collapses_tabs_and_spaces() {
        assert_eq!(normalize_whitespace("  hello   world  "), "hello world");
        assert_eq!(normalize_whitespace("\t\thello\t\tworld"), "hello world");
        assert_eq!(normalize_whitespace("a  b\n  c  d"), "a b\nc d");
    }

    #[tokio::test]
    async fn edit_file_fuzzy_match_succeeds_with_whitespace_diff() {
        let cwd = std::env::current_dir().expect("current dir");
        let tmp = tempdir_in(&cwd).expect("temp dir in workspace");
        let file_path = tmp.path().join("fuzzy.rs");
        let original = "fn greet() {\n    println!(\"hello\");\n}\n";
        tokio::fs::write(&file_path, original).await.expect("write");

        let tool = EditFileTool;
        // old_string with slightly different whitespace (2 spaces instead of 4)
        let args = serde_json::json!({
            "path": file_path.to_string_lossy(),
            "old_string": "fn greet() {\n  println!(\"hello\");\n}",
            "new_string": "fn greet() {\n    println!(\"world\");\n}"
        })
        .to_string();
        let out = Tool::execute(&tool, &args).await;
        assert!(out.success, "fuzzy edit should succeed: {}", out.output);
        assert!(
            out.output.contains("\"fuzzyMatch\":true")
                || out.output.contains("\"fuzzyMatch\": true"),
            "should report fuzzy match: {}",
            out.output
        );

        let updated = tokio::fs::read_to_string(&file_path).await.expect("read");
        assert!(
            updated.contains("world"),
            "file should contain the new text"
        );
    }

    #[tokio::test]
    async fn read_file_truncates_large_files() {
        let cwd = std::env::current_dir().expect("current dir");
        let tmp = tempdir_in(&cwd).expect("temp dir in workspace");
        let file_path = tmp.path().join("large.txt");
        let content: String = (0..3000).map(|i| format!("line {i}\n")).collect();
        tokio::fs::write(&file_path, &content).await.expect("write");

        let tool = ReadFileTool;
        let args = serde_json::json!({ "file_path": file_path.to_string_lossy() }).to_string();
        let out = Tool::execute(&tool, &args).await;
        assert!(out.success, "read should succeed: {}", out.output);
        assert!(
            out.output.contains("File truncated"),
            "should contain truncation message: ...{}...",
            &out.output[out.output.len().saturating_sub(300)..],
        );
        if let Some(meta) = &out.metadata {
            assert_eq!(meta["isTruncated"], true);
            assert_eq!(meta["totalLines"], 3000);
            assert_eq!(meta["linesShown"], 2000);
            assert_eq!(meta["encoding"], "utf-8");
        }
    }

    #[tokio::test]
    async fn search_case_insensitive_by_default() {
        let cwd = std::env::current_dir().expect("current dir");
        let tmp = tempdir_in(&cwd).expect("temp dir in workspace");
        let file_path = tmp.path().join("case.txt");
        tokio::fs::write(&file_path, "Hello World\nhello world\nHELLO WORLD\n")
            .await
            .expect("write");

        let tool = SearchInFilesTool;
        let args = serde_json::json!({
            "pattern": "hello world",
            "path": tmp.path().to_string_lossy()
        })
        .to_string();
        let out = with_work_dir(Some(cwd), Tool::execute(&tool, &args)).await;
        assert!(out.success, "search should succeed: {}", out.output);
        // Should find all 3 lines (case-insensitive default)
        assert!(
            out.output.contains("3 matches") || out.output.contains("Found 3"),
            "should find 3 case-insensitive matches: {}",
            out.output
        );
    }

    #[tokio::test]
    async fn edit_file_diff_output_in_display() {
        let cwd = std::env::current_dir().expect("current dir");
        let tmp = tempdir_in(&cwd).expect("temp dir in workspace");
        let file_path = tmp.path().join("diff_test.rs");
        tokio::fs::write(&file_path, "fn old_name() {}\n")
            .await
            .expect("write");

        let tool = EditFileTool;
        let args = serde_json::json!({
            "path": file_path.to_string_lossy(),
            "old_string": "fn old_name() {}",
            "new_string": "fn new_name() {}"
        })
        .to_string();
        let out = Tool::execute(&tool, &args).await;
        assert!(out.success, "edit should succeed: {}", out.output);

        // display_output should contain unified diff
        if let Some(ref display) = out.display_output {
            assert!(
                display.contains("diff")
                    || display.contains("-fn old_name")
                    || display.contains("+fn new_name"),
                "display should contain diff info: {}",
                display
            );
        }
    }
}

#[cfg(test)]
mod user_friendly_error_tests {
    use super::*;

    #[test]
    fn test_create_user_friendly_error_messages() {
        let path = "test_file.txt";

        assert!(
            create_user_friendly_error(ToolErrorType::FileNotFound, path)
                .contains("does not exist")
        );
        assert!(
            create_user_friendly_error(ToolErrorType::FileWriteFailure, path)
                .contains("Check file permissions")
        );
        assert!(
            create_user_friendly_error(ToolErrorType::ReadContentFailure, path)
                .contains("Could not read")
        );

        let perm_msg = create_user_friendly_error(ToolErrorType::PermissionDenied, path);
        assert!(
            perm_msg.contains("Permission denied"),
            "should mention denial: {perm_msg}"
        );
        assert!(
            perm_msg.contains("allowed locations"),
            "should mention allowed locations: {perm_msg}"
        );

        let workspace_msg = create_user_friendly_error(ToolErrorType::PathNotInWorkspace, path);
        assert!(
            workspace_msg.contains("Allowed locations"),
            "should list allowed locations: {workspace_msg}"
        );
        assert!(
            workspace_msg.contains("skill directories"),
            "should mention skill dirs: {workspace_msg}"
        );
        assert!(
            workspace_msg.contains("Full (YOLO)"),
            "should guide user to YOLO mode: {workspace_msg}"
        );
    }
}

#[cfg(test)]
mod find_similar_files_tests {
    use super::*;
    use tempfile::tempdir_in;

    #[test]
    fn finds_matching_file_in_subdirectory() {
        let tmp = tempdir_in(".").unwrap();
        let sub = tmp.path().join("src").join("utils");
        std::fs::create_dir_all(&sub).unwrap();
        let target = sub.join("helper.rs");
        std::fs::write(&target, "fn main() {}").unwrap();

        let results = find_similar_files("helper.rs", tmp.path(), 3, 3);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], target);
    }

    #[test]
    fn returns_empty_when_no_match() {
        let tmp = tempdir_in(".").unwrap();
        std::fs::write(tmp.path().join("other.rs"), "").unwrap();

        let results = find_similar_files("missing.rs", tmp.path(), 3, 3);
        assert!(results.is_empty());
    }

    #[test]
    fn respects_max_results() {
        let tmp = tempdir_in(".").unwrap();
        for i in 0..5 {
            let d = tmp.path().join(format!("d{i}"));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("mod.rs"), "").unwrap();
        }

        let results = find_similar_files("mod.rs", tmp.path(), 3, 2);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn skips_ignored_directories() {
        let tmp = tempdir_in(".").unwrap();
        let git_dir = tmp.path().join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(git_dir.join("config.rs"), "").unwrap();

        let node_dir = tmp.path().join("node_modules");
        std::fs::create_dir_all(&node_dir).unwrap();
        std::fs::write(node_dir.join("config.rs"), "").unwrap();

        let src_dir = tmp.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(src_dir.join("config.rs"), "valid").unwrap();

        let results = find_similar_files("config.rs", tmp.path(), 3, 5);
        assert_eq!(results.len(), 1);
        assert!(results[0].to_str().unwrap().contains("src"));
    }

    #[test]
    fn format_message_with_suggestions() {
        let suggestions = vec![
            PathBuf::from("/workspace/src/main.rs"),
            PathBuf::from("/workspace/tests/main.rs"),
        ];
        let msg = format_not_found_with_suggestions("main.rs", &suggestions);
        assert!(msg.contains("Did you mean"));
        assert!(msg.contains("/workspace/src/main.rs"));
        assert!(msg.contains("/workspace/tests/main.rs"));
    }

    #[test]
    fn format_message_without_suggestions() {
        let msg = format_not_found_with_suggestions("main.rs", &[]);
        assert!(msg.contains("does not exist"));
        assert!(msg.contains("list_directory"));
    }

    #[tokio::test]
    async fn format_message_includes_cwd_hint() {
        with_work_dir(Some(PathBuf::from("/tmp/myrepo")), async {
            let msg = format_not_found_with_suggestions("missing.rs", &[]);
            assert!(msg.contains("Current working directory: /tmp/myrepo"));
        }).await;
    }
}

#[cfg(test)]
mod suggest_path_under_cwd_tests {
    use super::*;
    use tempfile::tempdir_in;

    #[tokio::test]
    async fn corrects_path_missing_repo_dir() {
        let tmp = tempdir_in(".").unwrap();
        let repo = tmp.path().join("myrepo");
        let src = repo.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("lib.rs"), "// code").unwrap();

        let parent = tmp.path().to_path_buf();
        let wrong_path = parent.join("src").join("lib.rs");
        let expected = src.join("lib.rs");

        with_work_dir(Some(repo), async move {
            let result = suggest_path_under_cwd(&wrong_path);
            assert!(result.is_some(), "should suggest correction");
            assert_eq!(result.unwrap(), expected);
        }).await;
    }

    #[tokio::test]
    async fn returns_none_when_path_already_under_cwd() {
        let tmp = tempdir_in(".").unwrap();
        let repo = tmp.path().join("myrepo");
        let src = repo.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("lib.rs"), "// code").unwrap();

        let already_correct = src.join("lib.rs");

        with_work_dir(Some(repo), async move {
            let result = suggest_path_under_cwd(&already_correct);
            assert!(result.is_none(), "should return None for correct paths");
        }).await;
    }

    #[tokio::test]
    async fn returns_none_when_corrected_path_does_not_exist() {
        let tmp = tempdir_in(".").unwrap();
        let repo = tmp.path().join("myrepo");
        std::fs::create_dir_all(&repo).unwrap();

        let parent = tmp.path().to_path_buf();
        let wrong_path = parent.join("nonexistent").join("file.rs");

        with_work_dir(Some(repo), async move {
            let result = suggest_path_under_cwd(&wrong_path);
            assert!(result.is_none(), "should return None when correction doesn't exist");
        }).await;
    }

    #[tokio::test]
    async fn returns_none_for_completely_unrelated_path() {
        let tmp = tempdir_in(".").unwrap();
        let repo = tmp.path().join("myrepo");
        std::fs::create_dir_all(&repo).unwrap();

        with_work_dir(Some(repo), async {
            let result = suggest_path_under_cwd(Path::new("/etc/passwd"));
            assert!(result.is_none(), "should return None for unrelated paths");
        }).await;
    }
}

#[cfg(test)]
mod edit_error_code_tests {
    use super::*;

    #[test]
    fn format_error_contains_json_fields() {
        let msg = EditErrorCode::NoChange.format_error("/tmp/test.rs", "old == new");
        assert!(msg.contains("\"errorCode\":1"));
        assert!(msg.contains("\"errorType\":\"no_change\""));
        assert!(msg.contains("\"recovery_hint\":"));
        assert!(msg.contains("\"file\":\"/tmp/test.rs\""));
    }

    #[test]
    fn all_error_codes_produce_valid_json() {
        let codes = [
            EditErrorCode::NoChange,
            EditErrorCode::FileExists,
            EditErrorCode::NotFound,
            EditErrorCode::Stale,
            EditErrorCode::NotMatched,
            EditErrorCode::Ambiguous,
        ];
        for code in codes {
            let msg = code.format_error("test.rs", "detail");
            let parsed: serde_json::Value = serde_json::from_str(&msg)
                .unwrap_or_else(|e| panic!("Invalid JSON for {:?}: {e}\n{msg}", code));
            assert!(parsed.get("errorCode").unwrap().is_number());
            assert!(parsed.get("errorType").unwrap().is_string());
            assert!(parsed.get("recovery_hint").unwrap().is_string());
            assert!(parsed.get("message").unwrap().is_string());
            assert!(parsed.get("file").unwrap().is_string());
        }
    }

    #[test]
    fn error_code_numeric_values() {
        assert_eq!(EditErrorCode::NoChange as u8, 1);
        assert_eq!(EditErrorCode::FileExists as u8, 3);
        assert_eq!(EditErrorCode::NotFound as u8, 4);
        assert_eq!(EditErrorCode::Stale as u8, 7);
        assert_eq!(EditErrorCode::NotMatched as u8, 8);
        assert_eq!(EditErrorCode::Ambiguous as u8, 9);
    }

    #[test]
    fn format_error_escapes_special_chars() {
        let msg = EditErrorCode::NotMatched.format_error(
            "test.rs",
            "Line 1\nLine 2 with \"quotes\"",
        );
        let parsed: serde_json::Value = serde_json::from_str(&msg)
            .expect("should be valid JSON even with special chars");
        assert!(parsed.get("message").unwrap().as_str().unwrap().contains("Line 1"));
    }
}

#[cfg(test)]
mod multi_edit_tests {
    use super::*;
    use xiaolin_core::tool::Tool;
    use tempfile::tempdir_in;

    #[tokio::test]
    async fn multi_edit_atomic_success() {
        let cwd = std::env::current_dir().expect("current dir");
        let tmp = tempdir_in(&cwd).expect("temp dir in workspace");
        let file_a = tmp.path().join("a.txt");
        let file_b = tmp.path().join("b.txt");
        tokio::fs::write(&file_a, "hello world\n").await.unwrap();
        tokio::fs::write(&file_b, "foo bar baz\n").await.unwrap();

        let tool = MultiEditTool;
        let args = serde_json::json!({
            "edits": [
                {
                    "path": file_a.to_string_lossy(),
                    "changes": [{"old_string": "hello", "new_string": "goodbye"}]
                },
                {
                    "path": file_b.to_string_lossy(),
                    "changes": [{"old_string": "foo", "new_string": "qux"}]
                }
            ]
        })
        .to_string();

        let result = Tool::execute(&tool, &args).await;
        assert!(
            result.success,
            "multi_edit should succeed: {}",
            result.output
        );

        let a_content = tokio::fs::read_to_string(&file_a).await.unwrap();
        assert_eq!(a_content, "goodbye world\n");
        let b_content = tokio::fs::read_to_string(&file_b).await.unwrap();
        assert_eq!(b_content, "qux bar baz\n");
    }

    #[tokio::test]
    async fn multi_edit_atomic_rollback_on_failure() {
        let cwd = std::env::current_dir().expect("current dir");
        let tmp = tempdir_in(&cwd).expect("temp dir in workspace");
        let file_a = tmp.path().join("a2.txt");
        let file_b = tmp.path().join("b2.txt");
        tokio::fs::write(&file_a, "original_a\n").await.unwrap();
        tokio::fs::write(&file_b, "original_b\n").await.unwrap();

        let tool = MultiEditTool;
        let args = serde_json::json!({
            "edits": [
                {
                    "path": file_a.to_string_lossy(),
                    "changes": [{"old_string": "original_a", "new_string": "modified_a"}]
                },
                {
                    "path": file_b.to_string_lossy(),
                    "changes": [{"old_string": "NONEXISTENT_STRING", "new_string": "anything"}]
                }
            ]
        })
        .to_string();

        let result = Tool::execute(&tool, &args).await;
        assert!(
            !result.success,
            "multi_edit should fail when edit not found"
        );
        assert!(result.output.contains("Transaction aborted"));

        // File A should NOT be modified (atomic rollback)
        let a_content = tokio::fs::read_to_string(&file_a).await.unwrap();
        assert_eq!(
            a_content, "original_a\n",
            "file A should be untouched after failed transaction"
        );
    }

    #[tokio::test]
    async fn multi_edit_dry_run() {
        let cwd = std::env::current_dir().expect("current dir");
        let tmp = tempdir_in(&cwd).expect("temp dir in workspace");
        let file_a = tmp.path().join("dry.txt");
        tokio::fs::write(&file_a, "dry run content\n")
            .await
            .unwrap();

        let tool = MultiEditTool;
        let args = serde_json::json!({
            "dry_run": true,
            "edits": [{
                "path": file_a.to_string_lossy(),
                "changes": [{"old_string": "dry run", "new_string": "wet run"}]
            }]
        })
        .to_string();

        let result = Tool::execute(&tool, &args).await;
        assert!(result.success, "dry_run should succeed: {}", result.output);
        assert!(result.output.contains("dry_run"));

        // File should NOT be modified
        let content = tokio::fs::read_to_string(&file_a).await.unwrap();
        assert_eq!(
            content, "dry run content\n",
            "dry_run should not modify files"
        );
    }

    // ── Line-range matching tests ──────────────────────────────────

    #[test]
    fn line_range_context_match_finds_old_string_near_range() {
        let content = "line1\nline2\nfn foo() {\n    bar()\n}\nline6\n";
        let result = try_line_range_match(content, "fn foo() {", 3, 5);
        match result {
            LineRangeMatchResult::ContextMatch { start, end } => {
                assert_eq!(&content[start..end], "fn foo() {");
            }
            other => panic!(
                "expected ContextMatch, got {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }

    #[test]
    fn line_range_overwrite_when_old_string_not_found() {
        let content = "aaa\nbbb\nccc\nddd\neee\n";
        let result = try_line_range_match(content, "NONEXISTENT", 2, 3);
        match result {
            LineRangeMatchResult::Overwrite { extracted, .. } => {
                assert!(extracted.contains("bbb"), "extracted={extracted}");
                assert!(extracted.contains("ccc"), "extracted={extracted}");
            }
            other => panic!(
                "expected Overwrite, got {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }

    #[test]
    fn line_range_out_of_bounds() {
        let content = "one\ntwo\nthree\n";
        let result = try_line_range_match(content, "anything", 10, 20);
        assert!(matches!(result, LineRangeMatchResult::OutOfBounds { .. }));
    }

    #[test]
    fn line_range_splice_replaces_correctly() {
        let content = "aaa\nbbb\nccc\nddd\n";
        // Overwrite lines 2-3 (bbb\nccc\n) with "XXX\n"
        let lines: Vec<&str> = content.lines().collect();
        let (start, end) = compute_byte_range(&lines, 1, 2, true, content.len());
        let result = apply_line_range_splice(content, start, end, "XXX\n");
        assert_eq!(result, "aaa\nXXX\nddd\n");
    }

    #[test]
    fn apply_single_change_exact_match_works() {
        let content = "fn main() {\n    println!(\"hello\");\n}\n";
        match apply_single_change(
            content,
            "    println!(\"hello\");",
            "    println!(\"world\");",
            false,
            None,
            MatchMode::Exact,
            None,
            None,
            "test",
        ) {
            ApplyChangeResult::Ok { new_content, .. } => {
                assert!(new_content.contains("println!(\"world\")"));
                assert!(!new_content.contains("println!(\"hello\")"));
            }
            ApplyChangeResult::Err(e) => panic!("unexpected error: {e}"),
        }
    }

    #[test]
    fn apply_single_change_falls_back_to_line_range() {
        let content = "alpha\nbeta\ngamma\ndelta\n";
        match apply_single_change(
            content,
            "NONEXISTENT",
            "REPLACEMENT\n",
            false,
            None,
            MatchMode::Exact,
            Some(2),
            Some(3),
            "test",
        ) {
            ApplyChangeResult::Ok {
                new_content,
                log_entry,
            } => {
                assert!(new_content.contains("REPLACEMENT"));
                assert!(!new_content.contains("beta"));
                assert!(!new_content.contains("gamma"));
                let json_str = log_entry.to_string();
                assert!(
                    json_str.contains("line_range"),
                    "log should indicate line_range fallback: {json_str}"
                );
            }
            ApplyChangeResult::Err(e) => panic!("expected line-range fallback to succeed: {e}"),
        }
    }

    #[test]
    fn apply_single_change_no_line_range_returns_error() {
        let content = "alpha\nbeta\ngamma\n";
        match apply_single_change(
            content,
            "NONEXISTENT",
            "REPLACEMENT",
            false,
            None,
            MatchMode::Exact,
            None,
            None,
            "test",
        ) {
            ApplyChangeResult::Err(e) => {
                assert!(e.contains("not found"), "error should say not found: {e}");
            }
            ApplyChangeResult::Ok { .. } => panic!("should have failed without line range hint"),
        }
    }

    #[tokio::test]
    async fn edit_file_line_range_fallback_single_edit() {
        let cwd = std::env::current_dir().expect("current dir");
        let tmp = tempdir_in(&cwd).expect("temp dir in workspace");
        let file_path = tmp.path().join("lr_edit.txt");
        tokio::fs::write(&file_path, "line1\nline2\nline3\nline4\nline5\n")
            .await
            .unwrap();

        // First read the file to populate the cache
        let read_tool = ReadFileTool;
        let read_args = serde_json::json!({ "file_path": file_path.to_string_lossy() }).to_string();
        let _ = Tool::execute(&read_tool, &read_args).await;

        let tool = EditFileTool;
        let args = serde_json::json!({
            "file_path": file_path.to_string_lossy(),
            "old_string": "DOES_NOT_EXIST",
            "new_string": "REPLACED\n",
            "start_line": 2,
            "end_line": 3
        })
        .to_string();

        let result = Tool::execute(&tool, &args).await;
        assert!(
            result.success,
            "should succeed via line-range fallback: {}",
            result.output
        );

        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert!(content.contains("REPLACED"));
        assert!(!content.contains("line2"));
        assert!(!content.contains("line3"));
        assert!(content.contains("line1"));
        assert!(content.contains("line4"));
    }

    // ── Semantic search & smart read tests ──────────────────────────────

    #[test]
    fn parse_rg_line_extracts_file_line_content() {
        let (file, line, rest) = parse_rg_line("src/main.rs:42:fn main() {").unwrap();
        assert_eq!(file, "src/main.rs");
        assert_eq!(line, 42);
        assert_eq!(rest, "fn main() {");
    }

    #[test]
    fn parse_rg_line_handles_context_dash() {
        let (file, line, rest) = parse_rg_line("src/lib.rs:10-    use std::io;").unwrap();
        assert_eq!(file, "src/lib.rs");
        assert_eq!(line, 10);
        assert_eq!(rest, "    use std::io;");
    }

    #[test]
    fn parse_rg_line_returns_none_for_no_match() {
        assert!(parse_rg_line("-- separator --").is_none());
        assert!(parse_rg_line("").is_none());
    }

    #[test]
    fn extract_file_symbols_for_nonexistent_returns_empty() {
        let syms = extract_file_symbols("/nonexistent/file.rs");
        assert!(syms.is_empty());
    }

    #[test]
    fn generate_compact_outline_for_nonexistent_returns_none() {
        let result = generate_compact_outline(Path::new("/nonexistent/file.rs"), 500);
        assert!(result.is_none());
    }

    #[test]
    fn generate_compact_outline_includes_symbol_lines() {
        let tmp = tempdir_in(".").unwrap();
        let file = tmp.path().join("test_outline.rs");
        std::fs::write(
            &file,
            "\
fn alpha() {
    println!(\"a\");
}

struct Beta {
    x: i32,
}

fn gamma() {
    let _ = 1;
    let _ = 2;
    let _ = 3;
}
",
        )
        .unwrap();
        let ts_available = xiaolin_treesitter::CodeParser::detect_language(&file)
            .is_some_and(|l| xiaolin_treesitter::CodeParser::is_language_available(&l));
        let outline = generate_compact_outline(&file, 250);
        if ts_available {
            let text = outline.expect("outline should be Some when tree-sitter is available");
            assert!(
                text.contains("alpha"),
                "outline should contain 'alpha': {text}"
            );
            assert!(
                text.contains("fn"),
                "outline should contain kind label 'fn': {text}"
            );
            assert!(
                text.contains("250 lines"),
                "outline should mention total lines: {text}"
            );
        } else {
            assert!(
                outline.is_none(),
                "outline should be None without tree-sitter"
            );
        }
    }

    #[test]
    fn annotate_match_line_without_symbols() {
        let mut cache = std::collections::HashMap::new();
        let annotated = annotate_match_line(
            "/nonexistent/file.rs:10:let x = 1;",
            Path::new("/nonexistent"),
            &mut cache,
        );
        assert_eq!(annotated, "/nonexistent/file.rs:10:let x = 1;");
    }

    #[test]
    fn enrich_search_preserves_non_match_lines() {
        let input = "Found 1 match in 1 file\n--\nsrc/foo.rs:10:hello";
        let root = Path::new("/nonexistent");
        let result = enrich_search_with_symbols(input, root);
        assert!(result.contains("Found 1 match"));
    }

    #[tokio::test]
    async fn search_in_files_semantic_context_parameter() {
        let tmp = tempdir_in(".").unwrap();
        let file = tmp.path().join("sample.rs");
        std::fs::write(
            &file,
            "\
fn target_function() {
    let needle = 42;
}

fn other_function() {
    println!(\"no match here\");
}
",
        )
        .unwrap();

        let tool = SearchInFilesTool;
        let args = serde_json::json!({
            "pattern": "needle",
            "path": tmp.path().to_string_lossy(),
            "semantic_context": true
        })
        .to_string();

        let result = Tool::execute(&tool, &args).await;
        assert!(result.success, "search should succeed: {}", result.output);
        assert!(result.output.contains("needle"), "should find the match");

        // Semantic annotations depend on tree-sitter availability.
        let ts_available = xiaolin_treesitter::CodeParser::detect_language(&file)
            .is_some_and(|l| xiaolin_treesitter::CodeParser::is_language_available(&l));
        if ts_available && result.output.contains("[in ") {
            assert!(
                result.output.contains("target_function"),
                "should annotate with enclosing function: {}",
                result.output
            );
        }
    }

    #[tokio::test]
    async fn read_file_auto_outline_for_large_file() {
        let tmp = tempdir_in(".").unwrap();
        let file = tmp.path().join("large.rs");
        let mut content = String::new();
        content.push_str("fn first_function() {\n");
        for i in 0..100 {
            content.push_str(&format!("    let var_{i} = {i};\n"));
        }
        content.push_str("}\n\n");
        content.push_str("struct MyStruct {\n    x: i32,\n}\n\n");
        content.push_str("fn second_function() {\n");
        for i in 0..100 {
            content.push_str(&format!("    let other_{i} = {i};\n"));
        }
        content.push_str("}\n");
        std::fs::write(&file, &content).unwrap();

        let line_count = content.lines().count();
        assert!(
            line_count >= SMART_READ_OUTLINE_THRESHOLD,
            "test file should be >= {SMART_READ_OUTLINE_THRESHOLD} lines, got {line_count}"
        );

        // Tree-sitter language availability varies across test environments.
        let ts_available = xiaolin_treesitter::CodeParser::detect_language(&file)
            .is_some_and(|l| xiaolin_treesitter::CodeParser::is_language_available(&l));

        let tool = ReadFileTool;
        let args = serde_json::json!({
            "file_path": file.to_string_lossy()
        })
        .to_string();

        let result = Tool::execute(&tool, &args).await;
        assert!(result.success, "read should succeed: {}", result.output);

        if ts_available {
            let preview = &result.output[..result.output.len().min(600)];
            assert!(
                result.output.contains("File outline") || result.output.contains("── File outline"),
                "should have auto outline in output. Preview: {preview}"
            );
            if let Some(meta) = &result.metadata {
                assert_eq!(meta["hasOutline"], true);
            }
        } else {
            assert!(
                !result.output.contains("── File outline"),
                "without tree-sitter, outline should not appear"
            );
            if let Some(meta) = &result.metadata {
                assert_eq!(meta["hasOutline"], false);
            }
        }
    }

    #[tokio::test]
    async fn read_file_no_outline_for_small_file() {
        let tmp = tempdir_in(".").unwrap();
        let file = tmp.path().join("small.rs");
        std::fs::write(&file, "fn main() {\n    println!(\"hello\");\n}\n").unwrap();

        let tool = ReadFileTool;
        let args = serde_json::json!({
            "file_path": file.to_string_lossy()
        })
        .to_string();

        let result = Tool::execute(&tool, &args).await;
        assert!(result.success);
        assert!(
            !result.output.contains("File outline"),
            "small file should NOT have outline: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn read_file_no_outline_for_offset_read() {
        let tmp = tempdir_in(".").unwrap();
        let file = tmp.path().join("big.rs");
        let mut content = String::new();
        for i in 0..300 {
            content.push_str(&format!("// line {i}\n"));
        }
        std::fs::write(&file, &content).unwrap();

        let tool = ReadFileTool;
        let args = serde_json::json!({
            "file_path": file.to_string_lossy(),
            "offset": 10,
            "limit": 20
        })
        .to_string();

        let result = Tool::execute(&tool, &args).await;
        assert!(result.success);
        assert!(
            !result.output.contains("File outline"),
            "offset reads should NOT get outline: {}",
            result.output
        );
    }
}
