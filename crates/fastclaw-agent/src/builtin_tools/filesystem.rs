use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use base64::Engine as _;
use chardetng::EncodingDetector;
use encoding_rs::Encoding;
use fastclaw_core::agent_config::FileAccessMode;
use fastclaw_core::tool::{Tool, ToolErrorType, ToolKind, ToolParameterSchema, ToolResult};
use regex::RegexBuilder;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;

tokio::task_local! {
    static FILE_ACCESS_MODE: FileAccessMode;
    static EFFECTIVE_WORK_DIR: Option<PathBuf>;
    static ADDITIONAL_ALLOWED_PATHS: Vec<PathBuf>;
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

fn workspace_root() -> std::io::Result<PathBuf> {
    if let Ok(Some(dir)) = EFFECTIVE_WORK_DIR.try_with(|d| d.clone()) {
        if dir.is_dir() {
            return Ok(dir);
        }
    }
    std::env::current_dir()
}

fn state_dir_root() -> Option<PathBuf> {
    fastclaw_core::paths::resolve_state_dir_from(None).canonicalize().ok()
}

fn current_file_access_mode() -> FileAccessMode {
    FILE_ACCESS_MODE
        .try_with(|m| *m)
        .unwrap_or(FileAccessMode::Workspace)
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
        locs.push(format!("  • FastClaw data: {}", state_root.display()));
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

fn ensure_within_workspace(path: &Path, must_exist: bool) -> std::io::Result<PathBuf> {
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
            Ok(canon_parent) => Ok(canon_parent.join(
                absolute.file_name().map(PathBuf::from).unwrap_or_default(),
            )),
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
        Err(e) if must_exist => return Err(e),
        Err(_) => {
            // Parent doesn't exist yet — check whitelist using raw absolute path
            if is_path_under_allowed_prefixes(&absolute, &absolute) {
                if let Some(parent) = absolute.parent() {
                    if !parent.exists() {
                        std::fs::create_dir_all(parent)?;
                    }
                }
                return Ok(absolute);
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
    #[serde(default)]
    number_lines: bool,
    max_chars: Option<usize>,
    pages: Option<String>,
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
        "svg" => return DetectedFileType::Image { mime: "image/svg+xml" },
        "bmp" => return DetectedFileType::Image { mime: "image/bmp" },
        "ico" => return DetectedFileType::Image { mime: "image/x-icon" },
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
        let head: String = selected_text.chars().take(ABSOLUTE_READ_FILE_MAX_CHARS).collect();
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
        return Err(format!("requested {} pages, max 20 per request", indices.len()));
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

    let mut output = format!("Jupyter Notebook: {path} ({} cells, kernel: {kernel})\n", nb.cells.len());
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
        output.push_str("─".repeat(40usize.saturating_sub(cell_label.len())).as_str());
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
        let evalue = out
            .get("evalue")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        return format!(
            "{}: {}",
            ename.as_str().unwrap_or("Error"),
            evalue
        );
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

#[derive(Debug, Deserialize)]
struct EditFileArgs {
    #[serde(alias = "path")]
    file_path: String,
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
    expected_replacements: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct SearchInFilesArgs {
    pattern: String,
    path: Option<String>,
    glob: Option<String>,
    case_sensitive: Option<bool>,
    max_results: Option<usize>,
    context_lines: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ApplyPatchArgs {
    #[serde(alias = "path")]
    file_path: String,
    edits: Vec<ApplyPatchEdit>,
    expected_content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApplyPatchEdit {
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
    expected_replacements: Option<usize>,
}

fn compute_slice_bounds(total_lines: usize, offset: Option<i64>, limit: Option<usize>) -> (usize, usize) {
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

fn render_line_slice(content: &str, offset: Option<i64>, limit: Option<usize>, number_lines: bool) -> String {
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
        rendered.push_str(&format!(
            "\n[Showing lines {}-{} of {} total]",
            start + 1,
            end,
            total
        ));
    }
    rendered
}

fn looks_like_binary(bytes: &[u8], sample_size: usize) -> bool {
    let check = &bytes[..bytes.len().min(sample_size)];
    let null_count = check.iter().filter(|&&b| b == 0).count();
    null_count > 0
}

fn detect_line_ending(content: &str) -> &'static str {
    if content.contains("\r\n") { "crlf" } else { "lf" }
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
    s.lines()
        .map(|line| normalize_line(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn maybe_augment_old_string_for_deletion<'a>(file_content: &str, old_string: &'a str, new_string: &str) -> std::borrow::Cow<'a, str> {
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
        '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}' | '\u{2212}' => '-',
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
            if !prev_ws { result.push(' '); }
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

fn count_line_sequence<F>(file_lines: &[&str], pattern_lines: &[&str], transform: F) -> (usize, Option<usize>)
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

fn compute_byte_range(file_lines: &[&str], start_line: usize, line_count: usize, old_ends_with_newline: bool, file_len: usize) -> (usize, usize) {
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
        let (start, end) = compute_byte_range(&file_lines, start_line, line_count, old_string.ends_with('\n'), file_content.len());
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
        let (count4, first4) = count_line_sequence(&file_lines, &trimmed_old, |l| l.trim_end().to_string());
        if count4 == 1 {
            return build_result(first4.unwrap(), trimmed_old.len());
        }
        let (count5, first5) = count_line_sequence(&file_lines, &trimmed_old, normalize_line);
        if count5 == 1 {
            return build_result(first5.unwrap(), trimmed_old.len());
        }
    }

    if count > 1 {
        return FuzzyMatchResult::MultipleMatches(count);
    }

    FuzzyMatchResult::NoMatch
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

/// Creates user-friendly error messages based on the error type.
/// This helps prevent exposing internal system details to the LLM.
fn create_user_friendly_error(error_type: ToolErrorType, path: &str) -> String {
    match error_type {
        ToolErrorType::FileNotFound => {
            format!("The file '{}' does not exist. Please verify the file path.", path)
        }
        ToolErrorType::FileWriteFailure => {
            format!("Could not write to file '{}'. Check file permissions or disk space.", path)
        }
        ToolErrorType::ReadContentFailure => {
            format!("Could not read file '{}'. The file may be locked, corrupted, or inaccessible.", path)
        }
        ToolErrorType::AttemptToCreateExistingFile => {
            format!("File '{}' already exists. To modify an existing file, provide non-empty old_string and new_string parameters.", path)
        }
        ToolErrorType::PermissionDenied => {
            format!(
                "Permission denied accessing '{path}'. \
                 Possible causes: \
                 (1) The path is outside all allowed locations (workspace, FastClaw data, skill directories). \
                 (2) The execution mode is set to Plan (read-only) — the user can change this in Settings → Security → Execution Mode. \
                 (3) OS-level file permissions prevent access. \
                 Suggestion: use a path within the workspace, or ask the user to switch to Full (YOLO) mode in Settings → Security.",
            )
        }
        ToolErrorType::NoSpaceLeft => {
            format!("No space left on device while writing to '{}'. Free up some disk space.", path)
        }
        ToolErrorType::TargetIsDirectory => {
            format!("Expected a file but '{}' is a directory. Please specify a file path.", path)
        }
        ToolErrorType::PathNotInWorkspace => {
            format!(
                "Cannot access path '{path}': it is outside all allowed locations. \
                 Allowed locations include: workspace root, FastClaw data directory (~/.fastclaw/), \
                 skill directories (~/.cursor/skills/, ~/.agents/skills/, ~/.codex/skills/), \
                 and any user-configured additional_allowed_paths. \
                 Solutions: \
                 (1) Use a path within an allowed location. \
                 (2) Ask the user to change the working directory via the folder icon at the bottom of the chat input. \
                 (3) If full filesystem access is needed, the user can switch to Full (YOLO) mode in Settings → Security.",
            )
        }
        ToolErrorType::SearchPathNotFound => {
            format!("Search path '{}' does not exist or is not accessible.", path)
        }
        ToolErrorType::SearchPathNotADirectory => {
            format!("Search path '{}' is not a directory. Please specify a directory path.", path)
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
            format!("In file '{}': Old and new content are identical. No changes were made.", path)
        }
        _ => {
            format!("An error occurred while processing '{}'.", path)
        }
    }
}

fn is_skippable_dir_name(name: &str) -> bool {
    matches!(name, ".git" | "target" | "node_modules" | ".idea" | ".cursor")
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
        let dir_pat = &pat[..pat.len()-1];  // Remove trailing slash
        // Match if path starts with the directory pattern
        return rel_path.starts_with(&format!("{}/", dir_pat)) || 
               simple_glob_match(dir_pat, rel_path);
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
    let tmp_name = format!(".fastclaw-write-{}-{now}.tmp", std::process::id());
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

    let (content, enc_name, has_bom) = if let Some((decoded, bom_enc)) = try_decode_with_bom(raw_bytes) {
        let is_bom = bom_enc.contains("bom") || bom_enc.starts_with("utf-16") || bom_enc.starts_with("utf-32");
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
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let dotted = format!(".{}", ext.to_ascii_lowercase());
    UTF8_BOM_EXTENSIONS.contains(&dotted.as_str())
}

/// Read a file and return its contents.
pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn kind(&self) -> ToolKind { ToolKind::Read }
    fn name(&self) -> &str {
        "read_file"
    }

    fn max_result_size_chars(&self) -> usize { DEFAULT_READ_FILE_MAX_CHARS }

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
- You can optionally specify a line offset and limit (especially handy for long files), \
but it's recommended to read the whole file by not providing these parameters\n\
- Results are returned with line numbers starting at 1, using the format: LINE_NUMBER|LINE_CONTENT\n\
- This tool can read images (PNG, JPG, GIF, WEBP, SVG, BMP). When reading an image file \
the contents are presented visually as the model is multimodal\n\
- This tool can read PDF files (.pdf). For large PDFs (more than 10 pages), you MUST provide \
the pages parameter to read specific page ranges (e.g., pages: \"1-5\"). Maximum 20 pages per request\n\
- This tool can read Jupyter notebooks (.ipynb) and returns all cells with their outputs\n\
- This tool can only read files, not directories. To list a directory, use `list_directory` or `shell_exec`\n\
- If you read a file that exists but has empty contents you will receive a system reminder warning\n\
- When you already know which part of the file you need, only read that part using offset/limit. \
This is important for larger files".to_string()
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
            "offset".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Optional: starting line number (1-indexed). Negative values count from end (-1 = last line). Requires 'limit' to be set for pagination."
            }),
        );
        props.insert(
            "limit".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Optional: maximum number of lines to return. Use with 'offset' to paginate through large files."
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
        let args: ReadFileArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "read_file arguments are not valid JSON: {e}. \
                 Pass a JSON object like {{\"file_path\": \"/absolute/path/to/file.rs\"}}; \
                 optional fields: offset, limit, number_lines, max_chars, pages."
            )),
        };
        let path = args.file_path.as_str();

        let validated = match ensure_within_workspace(Path::new(path), true) {
            Ok(p) => p,
            Err(_) => {
                return ToolResult::typed_err(
                    ToolErrorType::PathNotInWorkspace,
                    create_user_friendly_error(ToolErrorType::PathNotInWorkspace, path),
                )
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
                    format!(
                        "read_file failed for path '{path}': {e}. \
                         Recovery: {recovery}",
                        recovery = match err_type {
                            ToolErrorType::FileNotFound => "Run list_directory on the parent to find the correct filename.",
                            ToolErrorType::PermissionDenied => "The user may need to switch execution mode in Settings → Security, or set a different working directory via the folder icon at the bottom of the chat.",
                            _ => "Check file permissions or retry. For binary files, use shell_exec to inspect."
                        }
                    ),
                );
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
            let mut result = ToolResult::ok(render_line_slice(
                &content,
                args.offset,
                args.limit,
                args.number_lines,
            ));
            result.metadata = Some(serde_json::json!({
                "fileType": "text",
                "totalLines": total_lines,
                "fileSize": file_size,
                "lineEnding": line_ending,
                "encoding": encoding,
            }));
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
            format!(
                "{truncated_text}\n\
                 [File content truncated: showing lines 1-{DEFAULT_READ_FILE_MAX_LINES} of {total_lines} total lines. \
                 Use offset/limit parameters to read remaining content.]"
            )
        } else {
            lines_shown = total_lines;
            content
        };

        let char_count = content.chars().count();
        let char_truncated = char_count > max_chars;
        let text = if char_truncated {
            let head: String = content.chars().take(max_chars).collect();
            format!("{head}\n[truncated, showing {max_chars} of {char_count} chars, {total_lines} total lines]")
        } else {
            content
        };

        let is_truncated = line_truncated || char_truncated;

        let mut result = ToolResult::ok(text);
        result.metadata = Some(serde_json::json!({
            "fileType": "text",
            "totalLines": total_lines,
            "linesShown": lines_shown,
            "fileSize": file_size,
            "lineEnding": line_ending,
            "encoding": encoding,
            "isTruncated": is_truncated,
        }));
        result
    }
}

/// Write content to a file, creating it if needed.
pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn kind(&self) -> ToolKind { ToolKind::Edit }
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
Only use this tool to create new files or for complete rewrites\n\
- NEVER create documentation files (*.md) or README files unless explicitly requested by the User\n\
- Only use emojis if the user explicitly requests it. Avoid writing emojis to files unless asked\n\
- Parent directories are created automatically if they don't exist\n\
- Modes: overwrite (default — replaces entire file), append (add to end), create_new (fail if exists)\n\
- Preserves existing file encoding (BOM, UTF-16, line endings) when overwriting".to_string()
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
            Err(_) => {
                return ToolResult::typed_err(
                    ToolErrorType::PathNotInWorkspace,
                    create_user_friendly_error(ToolErrorType::PathNotInWorkspace, path),
                )
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

        let existing_meta = if validated.exists() && mode == "overwrite" {
            match tokio::fs::read(&validated).await {
                Ok(raw) => {
                    let (_, meta) = detect_file_encoding_meta(&raw);
                    Some(meta)
                }
                Err(_) => None,
            }
        } else {
            None
        };

        let write_result = match mode {
            "overwrite" => {
                if let Some(ref meta) = existing_meta {
                    let bytes = encode_with_meta(content, meta);
                    atomic_write_bytes(&validated, &bytes).await
                } else {
                    let meta = if needs_utf8_bom(&validated) {
                        FileEncodingMeta { has_bom: true, ..FileEncodingMeta::default() }
                    } else {
                        FileEncodingMeta::default()
                    };
                    let bytes = encode_with_meta(content, &meta);
                    atomic_write_bytes(&validated, &bytes).await
                }
            }
            "append" => async {
                let mut f = tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&validated)
                    .await?;
                f.write_all(content.as_bytes()).await?;
                f.flush().await
            }
            .await,
            "create_new" => {
                if validated.exists() {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        "target file already exists",
                    ))
                } else {
                    let meta = if needs_utf8_bom(&validated) {
                        FileEncodingMeta { has_bom: true, ..FileEncodingMeta::default() }
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
                let final_bytes = match tokio::fs::metadata(&validated).await {
                    Ok(meta) => meta.len() as usize,
                    Err(_) => content.len(),
                };
                let enc_info = existing_meta.as_ref().map_or("utf-8", |m| m.encoding);
                ToolResult::ok(
                    serde_json::json!({
                        "written": true,
                        "file_path": path,
                        "mode": mode,
                        "bytes": final_bytes,
                        "encoding": enc_info,
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
                    format!("Could not write to file '{path}'. Check file permissions or disk space."),
                )
            }
        }
    }
}

/// Edit text in a file by replacing an exact snippet.
pub struct EditFileTool;

#[async_trait]
impl Tool for EditFileTool {
    fn kind(&self) -> ToolKind { ToolKind::Edit }
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Replaces text within a file. By default, replaces a single occurrence. \
         Set replace_all=true to replace every instance. Use empty old_string to create a new file. \
         old_string MUST be the exact literal text from the file including all whitespace and indentation. \
         Include at least 3 lines of context BEFORE and AFTER for unique identification. \
         Preserves file encoding (BOM, line endings) automatically. \
         Falls back to Unicode-normalized and whitespace-normalized fuzzy matching if exact match fails."
    }

    fn prompt(&self) -> String {
        "Performs exact string replacements in files.\n\n\
Usage:\n\
- You must use the `read_file` tool at least once in the conversation before editing. \
This tool will error if you attempt an edit without reading the file first\n\
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
if you want to rename a variable for instance".to_string()
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
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec![
                "file_path".to_string(),
                "old_string".to_string(),
                "new_string".to_string(),
            ],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: EditFileArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::err(format!(
                    "edit_file arguments are not valid JSON: {e}. \
                     Pass {{\"file_path\":\"/absolute/path\", \"old_string\":\"...\", \"new_string\":\"...\"}}."
                ))
            }
        };

        if args.old_string.is_empty() {
            if args.new_string.is_empty() {
                return ToolResult::typed_err(
                    ToolErrorType::InvalidToolParams,
                    "edit_file: both old_string and new_string are empty. Nothing to do.",
                );
            }
            let validated = match ensure_within_workspace(Path::new(&args.file_path), false) {
                Ok(p) => p,
                Err(_) => {
                    return ToolResult::typed_err(
                        ToolErrorType::PathNotInWorkspace,
                        create_user_friendly_error(ToolErrorType::PathNotInWorkspace, &args.file_path),
                    )
                }
            };
            if validated.exists() {
                return ToolResult::typed_err(
                    ToolErrorType::AttemptToCreateExistingFile,
                    create_user_friendly_error(ToolErrorType::AttemptToCreateExistingFile, &args.file_path),
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
                FileEncodingMeta { has_bom: true, ..FileEncodingMeta::default() }
            } else {
                FileEncodingMeta::default()
            };
            let bytes = encode_with_meta(&args.new_string, &meta);
            return match atomic_write_bytes(&validated, &bytes).await {
                Ok(()) => {
                    ToolResult::ok(
                        serde_json::json!({
                            "created": true,
                            "file_path": args.file_path,
                            "bytes": bytes.len(),
                            "linesAdded": new_lines,
                            "linesRemoved": 0,
                            "diffStat": format!("+{new_lines} -0 lines"),
                        })
                        .to_string(),
                    )
                }
                Err(e) => ToolResult::typed_err(
                    ToolErrorType::FileWriteFailure,
                    format!("edit_file failed to create '{}': {e}", args.file_path),
                ),
            };
        }

        if args.old_string == args.new_string {
            return ToolResult::typed_err(
                ToolErrorType::EditNoChange,
                create_user_friendly_error(ToolErrorType::EditNoChange, &args.file_path),
            );
        }

        let validated = match ensure_within_workspace(Path::new(&args.file_path), true) {
            Ok(p) => p,
            Err(_) => {
                return ToolResult::typed_err(
                    ToolErrorType::PathNotInWorkspace,
                    create_user_friendly_error(ToolErrorType::PathNotInWorkspace, &args.file_path),
                )
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
                return ToolResult::typed_err(err_type, create_user_friendly_error(err_type, &args.file_path));
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

        let old_augmented = maybe_augment_old_string_for_deletion(&normalized, &old_normalized, &new_normalized);

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
                    create_user_friendly_error(ToolErrorType::EditMultipleOccurrences, &args.file_path),
                );
            }
        }

        let (updated_normalized, replaced, fuzzy_used) = if match_count == 0 {
            match try_fuzzy_match(&normalized, &old_normalized) {
                FuzzyMatchResult::UniqueMatch { start, end } => {
                    let mut result = String::with_capacity(normalized.len());
                    result.push_str(&normalized[..start]);
                    result.push_str(&new_normalized);
                    result.push_str(&normalized[end..]);
                    (result, 1usize, true)
                }
                FuzzyMatchResult::NoMatch => {
                    let file_preview: String = normalized.lines().take(20)
                        .enumerate()
                        .map(|(i, l)| format!("{}|{}", i + 1, l))
                        .collect::<Vec<_>>()
                        .join("\n");
                    return ToolResult::typed_err(
                        ToolErrorType::EditNoOccurrenceFound,
                        format!(
                            "In file '{}': Could not find the specified text to replace \
                             (neither exact nor whitespace/Unicode-normalized match). \
                             The file may have changed or the old_string is incorrect.\n\
                             Hint: re-read the file with read_file to get the current content, \
                             then retry with the exact text.\n\
                             File preview (first 20 lines):\n{file_preview}",
                            args.file_path
                        ),
                    );
                }
                FuzzyMatchResult::MultipleMatches(n) => {
                    return ToolResult::typed_err(
                        ToolErrorType::EditMultipleOccurrences,
                        format!(
                            "In file '{}': Found {n} whitespace-normalized matches \
                             (exact match found 0). Provide more surrounding context \
                             to uniquely identify the location to edit, \
                             or set replace_all=true to replace all occurrences.",
                            args.file_path
                        ),
                    );
                }
            }
        } else if !args.replace_all && args.expected_replacements.is_none() && match_count > 1 {
            return ToolResult::typed_err(
                ToolErrorType::EditMultipleOccurrences,
                format!(
                    "In file '{}': Found {} {} matches. Provide more surrounding context \
                     to uniquely identify the location, or set replace_all=true.",
                    args.file_path, match_count,
                    if unicode_match { "Unicode-normalized" } else { "exact" }
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
        };

        let old_lines = normalized.lines().count();
        let new_lines = updated_normalized.lines().count();
        let added = new_lines.saturating_sub(old_lines);
        let removed = old_lines.saturating_sub(new_lines);

        let snippet = build_edit_snippet(&updated_normalized, &new_normalized, 4);
        let diff = build_diff_snippet(&old_normalized, &new_normalized, &args.file_path);

        let write_bytes = encode_with_meta(&updated_normalized, &enc_meta);

        match atomic_write_bytes(&validated, &write_bytes).await {
            Ok(()) => {
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
                        "fuzzyMatch": fuzzy_used,
                        "diff": diff,
                        "snippet": snippet,
                    })
                    .to_string(),
                );
                result.metadata = Some(serde_json::json!({
                    "lineEnding": enc_meta.line_ending,
                    "encoding": enc_meta.encoding,
                    "totalLines": new_lines,
                    "fuzzyMatch": fuzzy_used,
                }));
                result
            }
            Err(_) => ToolResult::typed_err(
                ToolErrorType::FileWriteFailure,
                format!("Could not write to file '{}'. Check file permissions or disk space.", args.file_path),
            ),
        }
    }
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

    let output = tokio::time::timeout(
        tokio::time::Duration::from_secs(30),
        cmd.output(),
    )
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
    fn kind(&self) -> ToolKind { ToolKind::Search }
    fn name(&self) -> &str {
        "search_in_files"
    }

    fn max_result_size_chars(&self) -> usize { 20_000 }

    fn description(&self) -> &str {
        "Search files using regex. Returns matches with file paths and line numbers in ripgrep-style format. \
         Uses ripgrep (rg) for fast search when available; falls back to built-in implementation. \
         Case-insensitive by default. Supports glob filter, context lines (0-5). Respects .gitignore."
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
- Respects .gitignore automatically".to_string()
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
                "description": "Optional number of lines to show before and after each match (0-5). Default 0. Useful for understanding match context without a separate read_file call."
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
            Err(_) => {
                return ToolResult::typed_err(
                    ToolErrorType::PathNotInWorkspace,
                    create_user_friendly_error(ToolErrorType::PathNotInWorkspace, scope),
                )
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
        let ctx = args.context_lines.unwrap_or(0).min(5);

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
                        Err(e) => return ToolResult::typed_err(ToolErrorType::GrepExecutionError, e),
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

        // LLM-friendly text output (like ripgrep format)
        let header = format!(
            "Found {} matches in {} files for pattern \"{}\" in \"{}\"{}:\n",
            match_count,
            matched_files,
            args.pattern,
            scope,
            if let Some(ref g) = args.glob { format!(" (filter: \"{}\")", g) } else { String::new() },
        );
        let truncation_note = if truncated {
            format!("\n[Results truncated at {} matches]", max_results)
        } else {
            String::new()
        };
        let llm_output = format!("{header}{text_output}{truncation_note}");

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
pub struct ApplyPatchTool;

#[async_trait]
impl Tool for ApplyPatchTool {
    fn kind(&self) -> ToolKind { ToolKind::Edit }
    fn name(&self) -> &str {
        "apply_patch"
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
            Err(_) => {
                return ToolResult::typed_err(
                    ToolErrorType::PathNotInWorkspace,
                    create_user_friendly_error(ToolErrorType::PathNotInWorkspace, &args.file_path),
                )
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
                        let file_preview: String = working.lines().take(15)
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

        let write_bytes = encode_with_meta(&working, &enc_meta);

        match atomic_write_bytes(&validated, &write_bytes).await {
            Ok(()) => ToolResult::ok(
                serde_json::json!({
                    "patched": true,
                    "file_path": args.file_path,
                    "edits_applied": applied,
                    "bytes": write_bytes.len(),
                    "encoding": enc_meta.encoding,
                })
                .to_string(),
            ),
            Err(_) => ToolResult::typed_err(
                ToolErrorType::FileWriteFailure,
                format!("Could not write to file '{}'. Check file permissions or disk space.", args.file_path),
            ),
        }
    }
}

/// List files and directories at a given path.
pub struct ListDirectoryTool;

#[async_trait]
impl Tool for ListDirectoryTool {
    fn kind(&self) -> ToolKind { ToolKind::Read }
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
            Err(e) => return ToolResult::err(format!(
                "list_directory arguments are not valid JSON: {e}. \
                 Pass exactly {{\"path\": \"your/dir\"}} with a string path, then retry."
            )),
        };

        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolResult::err(
                "list_directory is missing string field 'path'. \
                 Example: {\"path\": \".\"} or {\"path\": \"src/components\"}. \
                 Use read_file if you need the contents of a single file."
                    .to_string(),
            ),
        };

        let mut entries = Vec::new();
        let validated = match ensure_within_workspace(Path::new(path), true) {
            Ok(p) => p,
            Err(_) => {
                return ToolResult::typed_err(
                    ToolErrorType::PathNotInWorkspace,
                    create_user_friendly_error(ToolErrorType::PathNotInWorkspace, path),
                )
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
    fn kind(&self) -> ToolKind { ToolKind::Read }
    fn name(&self) -> &str { "glob" }

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
        props.insert("path".to_string(), serde_json::json!({
            "type": "string",
            "description": "Directory to search in (default '.'). Relative to workspace root."
        }));
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
            _ => return ToolResult::err(
                "glob requires a non-empty 'pattern'. Example: {\"pattern\": \"*.rs\"}".to_string()
            ),
        };
        let base = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        let base_dir = match ensure_within_workspace(Path::new(base), true) {
            Ok(p) => p,
            Err(_) => return ToolResult::typed_err(
                ToolErrorType::PathNotInWorkspace,
                create_user_friendly_error(ToolErrorType::PathNotInWorkspace, base),
            ),
        };

        let root = workspace_root().and_then(|p| p.canonicalize()).unwrap_or_else(|_| base_dir.clone());
        let gitignore = load_gitignore_patterns(&root);

        let full_pattern = if pattern.starts_with("**/") || pattern.contains('/') {
            base_dir.join(pattern).to_string_lossy().to_string()
        } else {
            base_dir.join("**").join(pattern).to_string_lossy().to_string()
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
                    if rel_lower.contains("node_modules/") || rel_lower.contains(".git/") || rel_lower.contains("/target/") {
                        continue;
                    }
                    let mtime = entry.metadata()
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    entries.push((entry, mtime));
                    if entries.len() >= 200 { break; }
                }
            }
            Err(e) => return ToolResult::err(format!("glob: invalid pattern: {e}")),
        }

        entries.sort_by(|a, b| b.1.cmp(&a.1));
        let max_results = 100;
        let truncated = entries.len() > max_results;
        entries.truncate(max_results);

        let cwd = std::env::current_dir().unwrap_or_default();
        let file_list: Vec<String> = entries.iter().map(|(p, _)| {
            p.strip_prefix(&cwd).unwrap_or(p).to_string_lossy().to_string()
        }).collect();

        let total = file_list.len();

        ToolResult::ok(serde_json::json!({
            "pattern": pattern,
            "path": base,
            "files": file_list,
            "count": total,
            "truncated": truncated,
        }).to_string())
    }
}

// ─── Multi-file Atomic Edit ─────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct MultiEditArgs {
    edits: Vec<MultiEditFileEntry>,
    #[serde(default)]
    dry_run: bool,
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
}

/// Atomically apply edits across multiple files.
///
/// All files are validated and patched in memory first.
/// Only if every file's edits succeed are the results written to disk.
/// If any edit in any file fails, no files are modified (all-or-nothing).
pub struct MultiEditTool;

#[async_trait]
impl Tool for MultiEditTool {
    fn kind(&self) -> ToolKind { ToolKind::Edit }
    fn name(&self) -> &str { "multi_edit" }

    fn description(&self) -> &str {
        "Atomically apply edits across multiple files. All edits are validated in memory first; \
         only if every edit succeeds are results written to disk. If any edit fails, no files are \
         modified (all-or-nothing transaction). Supports dry_run mode to preview changes."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert("edits".to_string(), serde_json::json!({
            "type": "array",
            "description": "List of file edit entries. Each entry has 'path' and 'changes' (array of {old_string, new_string, replace_all?})."
        }));
        props.insert("dry_run".to_string(), serde_json::json!({
            "type": "boolean",
            "description": "If true, validate all edits but do not write. Default false."
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

        let mut staged: Vec<(PathBuf, String, Vec<u8>, Vec<serde_json::Value>)> = Vec::new();

        for (file_idx, entry) in args.edits.iter().enumerate() {
            let validated = match ensure_within_workspace(Path::new(&entry.file_path), true) {
                Ok(p) => p,
                Err(_) => return ToolResult::err(format!(
                    "multi_edit: file #{file_idx} '{}' is outside the workspace. Transaction aborted, no files modified.",
                    entry.file_path
                )),
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
                if change.old_string.is_empty() {
                    return ToolResult::err(format!(
                        "multi_edit: file #{file_idx} '{}', change #{change_idx} has empty old_string. Transaction aborted.",
                        entry.file_path
                    ));
                }

                let old_norm = change.old_string.replace("\r\n", "\n");
                let new_norm = change.new_string.replace("\r\n", "\n");
                let match_count = current.matches(&old_norm).count();

                if match_count == 0 {
                    return ToolResult::err(format!(
                        "multi_edit: file #{file_idx} '{}', change #{change_idx}: old_string not found. \
                         Transaction aborted, no files modified. Re-read the file to get current content.",
                        entry.file_path
                    ));
                }

                if !change.replace_all && match_count > 1 {
                    return ToolResult::err(format!(
                        "multi_edit: file #{file_idx} '{}', change #{change_idx}: found {match_count} matches. \
                         Set replace_all=true or provide more context. Transaction aborted.",
                        entry.file_path
                    ));
                }

                let replaced = if change.replace_all { match_count } else { 1 };
                current = if change.replace_all {
                    current.replace(&old_norm, &new_norm)
                } else {
                    current.replacen(&old_norm, &new_norm, 1)
                };

                change_log.push(serde_json::json!({
                    "change_index": change_idx,
                    "replacements": replaced,
                }));
            }

            let final_bytes = encode_with_meta(&current, &enc_meta);

            staged.push((validated, entry.file_path.clone(), final_bytes, change_log));
        }

        if args.dry_run {
            let results: Vec<serde_json::Value> = staged.iter().map(|(_, fp, bytes, log)| {
                serde_json::json!({
                    "file_path": fp,
                    "changes_applied": log,
                    "result_bytes": bytes.len(),
                })
            }).collect();

            return ToolResult::ok(serde_json::json!({
                "dry_run": true,
                "files": results,
                "count": results.len(),
                "status": "all edits valid, no files written",
            }).to_string());
        }

        let mut written = Vec::new();
        for (validated_path, display_path, bytes, change_log) in &staged {
            match atomic_write_bytes(validated_path, bytes).await {
                Ok(()) => {
                    written.push(serde_json::json!({
                        "file_path": display_path,
                        "changes_applied": change_log,
                        "bytes": bytes.len(),
                    }));
                }
                Err(e) => {
                    return ToolResult::err(format!(
                        "multi_edit: CRITICAL — failed to write '{}' after {} files already written: {e}. \
                         Earlier files in the transaction ({}) were already modified. \
                         Manual recovery may be needed.",
                        display_path,
                        written.len(),
                        written.iter()
                            .filter_map(|w| w.get("file_path").and_then(|p| p.as_str()))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
        }

        ToolResult::ok(serde_json::json!({
            "success": true,
            "files": written,
            "count": written.len(),
        }).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastclaw_core::tool::Tool;
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

    #[tokio::test]
    async fn read_file_allowed_in_cursor_skills_dir() {
        let home = dirs::home_dir().expect("home dir");
        let skill_dir = home.join(".cursor").join("skills").join("_test_whitelist_skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let skill_file = skill_dir.join("SKILL.md");
        tokio::fs::write(&skill_file, "# Test Skill\nContent here.").await.unwrap();

        let tool = ReadFileTool;
        let args = serde_json::json!({ "file_path": skill_file.to_string_lossy() }).to_string();
        let out = with_file_access_mode(FileAccessMode::Workspace, Tool::execute(&tool, &args)).await;
        assert!(out.success, "workspace mode should allow reading ~/.cursor/skills/: {}", out.output);
        assert!(out.output.contains("Test Skill"));

        // Cleanup
        let _ = std::fs::remove_dir_all(&skill_dir);
    }

    #[tokio::test]
    async fn write_file_allowed_in_cursor_skills_dir() {
        let home = dirs::home_dir().expect("home dir");
        let skill_dir = home.join(".cursor").join("skills").join("_test_whitelist_write");
        let skill_file = skill_dir.join("SKILL.md");

        let tool = WriteFileTool;
        let args = serde_json::json!({
            "file_path": skill_file.to_string_lossy(),
            "content": "# New Skill\nCreated by test."
        }).to_string();
        let out = with_file_access_mode(FileAccessMode::Workspace, Tool::execute(&tool, &args)).await;
        assert!(out.success, "workspace mode should allow writing ~/.cursor/skills/: {}", out.output);

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
        let out = with_file_access_mode(FileAccessMode::Workspace, Tool::execute(&tool, &args)).await;
        assert!(!out.success, "workspace mode should reject ~/Desktop/ access");
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
        tokio::fs::write(&file_path, "fn hello_world() {}\n").await.expect("write sample");

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
        tokio::fs::write(&file_path, "blocked\n").await.expect("write sample");

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
        tokio::fs::write(&file_path, "backward-compat\n").await.expect("write sample");

        let tool = ReadFileTool;
        let args = serde_json::json!({ "path": file_path.to_string_lossy() }).to_string();
        let out = Tool::execute(&tool, &args).await;
        assert!(out.success, "old 'path' param should still work: {}", out.output);
        assert!(out.output.contains("backward-compat"));
    }

    #[tokio::test]
    async fn read_file_workspace_blocks_outside_path() {
        let mut temp = tempfile::NamedTempFile::new().expect("tmp file");
        writeln!(temp, "outside").expect("write outside file");
        let outside_path = temp.path().to_string_lossy().to_string();

        let tool = ReadFileTool;
        let args = serde_json::json!({ "file_path": outside_path }).to_string();
        let out = with_file_access_mode(FileAccessMode::Workspace, Tool::execute(&tool, &args)).await;
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
        assert!(out.success, "full mode should allow outside path: {}", out.output);
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
        let out = with_file_access_mode(FileAccessMode::Workspace, Tool::execute(&tool, &args)).await;
        assert!(!out.success, "workspace mode should block outside edit");
        assert!(out.output.contains("outside") || out.output.contains("Allowed locations"), "unexpected: {}", out.output);
    }

    #[tokio::test]
    async fn search_in_files_denied_when_file_access_none() {
        let cwd = std::env::current_dir().expect("current dir");
        let tmp = tempdir_in(&cwd).expect("temp dir in workspace");
        let file_path = tmp.path().join("search.txt");
        tokio::fs::write(&file_path, "needle\n").await.expect("write sample");

        let tool = SearchInFilesTool;
        let args = serde_json::json!({
            "pattern": "needle",
            "path": tmp.path().to_string_lossy()
        })
        .to_string();
        let out = with_file_access_mode(FileAccessMode::None, Tool::execute(&tool, &args)).await;
        assert!(!out.success, "search_in_files should be blocked in none mode");
        assert!(out.output.contains("outside") || out.output.contains("file access is disabled"),
            "unexpected: {}", out.output);
    }
}

#[cfg(test)]
mod new_feature_tests {
    use super::*;
    use fastclaw_core::tool::Tool;
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
        assert!(matches!(try_fuzzy_match(content, old), FuzzyMatchResult::NoMatch));
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
        assert!(out.output.contains("\"fuzzyMatch\":true") || out.output.contains("\"fuzzyMatch\": true"),
            "should report fuzzy match: {}", out.output);

        let updated = tokio::fs::read_to_string(&file_path).await.expect("read");
        assert!(updated.contains("world"), "file should contain the new text");
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
            out.output.contains("File content truncated"),
            "should contain truncation message: ...{}...",
            &out.output[out.output.len().saturating_sub(200)..],
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
        let out = Tool::execute(&tool, &args).await;
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
        tokio::fs::write(&file_path, "fn old_name() {}\n").await.expect("write");

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
            assert!(display.contains("diff") || display.contains("-fn old_name") || display.contains("+fn new_name"),
                "display should contain diff info: {}", display);
        }
    }
}

#[cfg(test)]
mod user_friendly_error_tests {
    use super::*;

    #[test]
    fn test_create_user_friendly_error_messages() {
        let path = "test_file.txt";
        
        assert!(create_user_friendly_error(ToolErrorType::FileNotFound, path)
            .contains("does not exist"));
        assert!(create_user_friendly_error(ToolErrorType::FileWriteFailure, path)
            .contains("Check file permissions"));
        assert!(create_user_friendly_error(ToolErrorType::ReadContentFailure, path)
            .contains("Could not read"));

        let perm_msg = create_user_friendly_error(ToolErrorType::PermissionDenied, path);
        assert!(perm_msg.contains("Permission denied"), "should mention denial: {perm_msg}");
        assert!(perm_msg.contains("allowed locations"), "should mention allowed locations: {perm_msg}");

        let workspace_msg = create_user_friendly_error(ToolErrorType::PathNotInWorkspace, path);
        assert!(workspace_msg.contains("Allowed locations"), "should list allowed locations: {workspace_msg}");
        assert!(workspace_msg.contains("skill directories"), "should mention skill dirs: {workspace_msg}");
        assert!(workspace_msg.contains("Full (YOLO)"), "should guide user to YOLO mode: {workspace_msg}");
    }
}

#[cfg(test)]
mod multi_edit_tests {
    use super::*;
    use fastclaw_core::tool::Tool;
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
        }).to_string();

        let result = Tool::execute(&tool, &args).await;
        assert!(result.success, "multi_edit should succeed: {}", result.output);

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
        }).to_string();

        let result = Tool::execute(&tool, &args).await;
        assert!(!result.success, "multi_edit should fail when edit not found");
        assert!(result.output.contains("Transaction aborted"));

        // File A should NOT be modified (atomic rollback)
        let a_content = tokio::fs::read_to_string(&file_a).await.unwrap();
        assert_eq!(a_content, "original_a\n", "file A should be untouched after failed transaction");
    }

    #[tokio::test]
    async fn multi_edit_dry_run() {
        let cwd = std::env::current_dir().expect("current dir");
        let tmp = tempdir_in(&cwd).expect("temp dir in workspace");
        let file_a = tmp.path().join("dry.txt");
        tokio::fs::write(&file_a, "dry run content\n").await.unwrap();

        let tool = MultiEditTool;
        let args = serde_json::json!({
            "dry_run": true,
            "edits": [{
                "path": file_a.to_string_lossy(),
                "changes": [{"old_string": "dry run", "new_string": "wet run"}]
            }]
        }).to_string();

        let result = Tool::execute(&tool, &args).await;
        assert!(result.success, "dry_run should succeed: {}", result.output);
        assert!(result.output.contains("dry_run"));

        // File should NOT be modified
        let content = tokio::fs::read_to_string(&file_a).await.unwrap();
        assert_eq!(content, "dry run content\n", "dry_run should not modify files");
    }
}
