use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use fastclaw_core::agent_config::FileAccessMode;
use fastclaw_core::tool::{Tool, ToolErrorType, ToolKind, ToolParameterSchema, ToolResult};
use regex::RegexBuilder;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;

tokio::task_local! {
    static FILE_ACCESS_MODE: FileAccessMode;
    static EFFECTIVE_WORK_DIR: Option<PathBuf>;
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

fn ensure_within_workspace(path: &Path, must_exist: bool) -> std::io::Result<PathBuf> {
    match current_file_access_mode() {
        FileAccessMode::None => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "file access is disabled by agent policy (file_access=none)",
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
                return absolute.canonicalize();
            }
            return if absolute.exists() {
                absolute.canonicalize()
            } else {
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
    let resolved = if must_exist {
        absolute.canonicalize()?
    } else if absolute.exists() {
        absolute.canonicalize()?
    } else {
        let parent = absolute
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.clone())
            .canonicalize()?;
        parent.join(
            absolute
                .file_name()
                .map(PathBuf::from)
                .unwrap_or_default(),
        )
    };
    if resolved.starts_with(&root) {
        return Ok(resolved);
    }
    if let Some(state_root) = state_dir_root() {
        if resolved.starts_with(&state_root) {
            return Ok(resolved);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        format!(
            "path '{}' resolves outside workspace root '{}'",
            path.display(),
            root.display()
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
    path: String,
    offset: Option<i64>,
    limit: Option<usize>,
    #[serde(default)]
    number_lines: bool,
    max_chars: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct WriteFileArgs {
    path: String,
    content: String,
    mode: Option<String>,
    expected_content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EditFileArgs {
    path: String,
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
    path: String,
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

fn normalize_line(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let mut prev_ws = false;
    for ch in line.chars() {
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

fn try_fuzzy_match(file_content: &str, old_string: &str) -> FuzzyMatchResult {
    let file_lines: Vec<&str> = file_content.lines().collect();
    let norm_file_lines: Vec<String> = file_lines.iter().map(|l| normalize_line(l)).collect();
    let norm_old_lines: Vec<String> = old_string.lines().map(|l| normalize_line(l)).collect();
    let old_line_count = norm_old_lines.len();
    if old_line_count == 0 || norm_old_lines.iter().all(|l| l.is_empty()) {
        return FuzzyMatchResult::NoMatch;
    }

    // Count line-based fuzzy matches and find the first match position
    let mut match_count = 0usize;
    let mut first_match_line: Option<usize> = None;
    for start_line in 0..file_lines.len() {
        if start_line + old_line_count > file_lines.len() {
            break;
        }
        let all_match = (0..old_line_count)
            .all(|i| norm_file_lines[start_line + i] == norm_old_lines[i]);
        if all_match {
            match_count += 1;
            if first_match_line.is_none() {
                first_match_line = Some(start_line);
            }
        }
    }

    if match_count == 0 {
        return FuzzyMatchResult::NoMatch;
    }
    if match_count > 1 {
        return FuzzyMatchResult::MultipleMatches(match_count);
    }

    if let Some(start_line) = first_match_line {
        let mut byte_offset = 0usize;
        for line in file_lines.iter().take(start_line) {
            byte_offset += line.len() + 1;
        }
        let start_byte = byte_offset;
        for line in file_lines.iter().skip(start_line).take(old_line_count) {
            byte_offset += line.len() + 1;
        }
        let end_byte = byte_offset.saturating_sub(1);
        if old_string.ends_with('\n') {
            return FuzzyMatchResult::UniqueMatch { start: start_byte, end: byte_offset.min(file_content.len()) };
        }
        return FuzzyMatchResult::UniqueMatch { start: start_byte, end: end_byte.min(file_content.len()) };
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
                "Permission denied accessing '{}'. \
                 Possible causes: \
                 (1) The file_access mode is set to 'none' — the user can change this in the agent settings panel under '文件访问权限'. \
                 (2) OS-level file permissions prevent access — suggest the user check ownership/permissions. \
                 (3) The file is locked by another process.",
                path,
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
                "Cannot access path '{}': it is outside the current workspace root. \
                 Solutions: \
                 (1) Use a path relative to the workspace root. \
                 (2) Ask the user to change the working directory via the folder icon (工作目录) at the bottom of the chat input. \
                 (3) If full filesystem access is needed, the user can set file_access to 'full' in the agent settings panel under '文件访问权限'.",
                path,
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
    if pat.starts_with('/') {
        let clean_pat = &pat[1..];  // Remove leading slash
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
            if prefix.is_empty() && rel_path.contains(suffix) {
                return true;
            } else if suffix.is_empty() && rel_path.starts_with(prefix) {
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

async fn atomic_write_text(path: &Path, content: &str) -> std::io::Result<()> {
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
    tmp.write_all(content.as_bytes()).await?;
    tmp.flush().await?;
    drop(tmp);
    tokio::fs::rename(&tmp_path, path).await
}

/// Read a file and return its contents.
pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn kind(&self) -> ToolKind { ToolKind::Read }
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read a file's content. Large files auto-truncated to 2000 lines by default. \
         Use offset/limit to read specific line ranges for large files. \
         Handles text files; binary and non-UTF-8 files return an error. \
         Also truncated if content exceeds max_chars (default 32768, max 256000). \
         Use number_lines=true to get line-numbered output for precise references."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "path".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "File path (absolute or relative to cwd)."
            }),
        );
        props.insert(
            "offset".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Optional starting line (1-indexed). Negative values count from file end (-1 is last line). If omitted, starts from line 1."
            }),
        );
        props.insert(
            "limit".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Optional maximum number of lines to return (used with offset). If omitted, returns all remaining lines."
            }),
        );
        props.insert(
            "number_lines".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "Optional. When true, prefixes each returned line as '<line>|<content>'. Useful for precise references."
            }),
        );
        props.insert(
            "max_chars".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Optional output cap in characters for non-sliced reads. Defaults to 32768, hard max 256000."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["path".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: ReadFileArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "read_file arguments are not valid JSON: {e}. \
                 Pass a JSON object like {{\"path\": \"path/to/file.rs\"}}; optional fields: offset, limit, number_lines, max_chars."
            )),
        };
        let path = args.path.as_str();

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
                            ToolErrorType::PermissionDenied => "The user may need to adjust file_access in agent settings ('文件访问权限'), or set a different working directory via the folder icon at the bottom of the chat.",
                            _ => "The file may be binary or locked—do not retry read_file on binary content."
                        }
                    ),
                );
            }
        };

        let file_size = raw_bytes.len();

        if looks_like_binary(&raw_bytes, 8192) {
            return ToolResult::typed_err(
                ToolErrorType::ReadContentFailure,
                format!(
                    "read_file: '{path}' appears to be a binary file ({file_size} bytes). \
                     Binary files are not supported. Use shell_exec to inspect binary content \
                     (e.g. `file {path}`, `hexdump -C {path} | head`) or request a text export."
                ),
            );
        }

        let content = match String::from_utf8(raw_bytes) {
            Ok(s) => s,
            Err(_) => {
                return ToolResult::typed_err(
                    ToolErrorType::ReadContentFailure,
                    format!(
                        "read_file: '{path}' contains invalid UTF-8 ({file_size} bytes). \
                         The file may use a non-UTF-8 encoding. \
                         Try shell_exec with iconv or file to check encoding."
                    ),
                );
            }
        };

        let total_lines = content.lines().count();
        let line_ending = detect_line_ending(&content);

        if args.offset.is_some() || args.limit.is_some() || args.number_lines {
            return ToolResult::ok(render_line_slice(
                &content,
                args.offset,
                args.limit,
                args.number_lines,
            ));
        }

        let max_chars = args
            .max_chars
            .unwrap_or(DEFAULT_READ_FILE_MAX_CHARS)
            .clamp(1, ABSOLUTE_READ_FILE_MAX_CHARS);

        // Apply default line limit for large files (like Qwen Code's 2000-line default)
        let line_truncated = total_lines > DEFAULT_READ_FILE_MAX_LINES;
        let content = if line_truncated {
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
            "totalLines": total_lines,
            "fileSize": file_size,
            "lineEnding": line_ending,
            "truncated": is_truncated,
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
        "Write content to a file. Modes: overwrite (default), append, create_new. \
         Parent directories are created automatically. \
         Use expected_content for optimistic locking (write only if current content matches)."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "path".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "File path (absolute or relative to cwd). Parent dirs auto-created."
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
            required: vec!["path".to_string(), "content".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: WriteFileArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "write_file arguments are not valid JSON: {e}. \
                 Pass {{\"path\": \"...\", \"content\": \"...\"}}; optional fields: mode, expected_content."
            )),
        };
        let path = args.path.as_str();
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
            if let Err(_) = tokio::fs::create_dir_all(parent).await {
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

        let write_result = match mode {
            "overwrite" => atomic_write_text(&validated, content).await,
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
                    atomic_write_text(&validated, content).await
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
                ToolResult::ok(
                    serde_json::json!({
                        "written": true,
                        "path": path,
                        "mode": mode,
                        "bytes": final_bytes,
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
        "Replace text in a file, or create a new file (empty old_string). \
         old_string should match exactly one location (include 3+ lines of context for uniqueness). \
         Set replace_all=true to replace every occurrence. Line endings auto-preserved. \
         If exact match fails, falls back to whitespace-normalized fuzzy matching. \
         CRITICAL: old_string must be the literal text from the file including indentation."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "path".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Target file path (absolute or relative to cwd)."
            }),
        );
        props.insert(
            "old_string".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Exact existing text to locate in the file."
            }),
        );
        props.insert(
            "new_string".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Replacement text."
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
                "path".to_string(),
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
                     Pass {{\"path\":\"...\", \"old_string\":\"...\", \"new_string\":\"...\"}}."
                ))
            }
        };

        // When old_string is empty, this is a "create new file" operation
        if args.old_string.is_empty() {
            if args.new_string.is_empty() {
                return ToolResult::typed_err(
                    ToolErrorType::InvalidToolParams,
                    "edit_file: both old_string and new_string are empty. Nothing to do.",
                );
            }
            let validated = match ensure_within_workspace(Path::new(&args.path), false) {
                Ok(p) => p,
                Err(_) => {
                    return ToolResult::typed_err(
                        ToolErrorType::PathNotInWorkspace,
                        create_user_friendly_error(ToolErrorType::PathNotInWorkspace, &args.path),
                    )
                }
            };
            if validated.exists() {
                return ToolResult::typed_err(
                    ToolErrorType::AttemptToCreateExistingFile,
                    create_user_friendly_error(ToolErrorType::AttemptToCreateExistingFile, &args.path),
                );
            }
            if let Some(parent) = validated.parent() {
                if let Err(_) = tokio::fs::create_dir_all(parent).await {
                    return ToolResult::typed_err(
                        ToolErrorType::FileWriteFailure,
                        format!("Could not create parent directories for '{}'. Check directory permissions.", args.path),
                    );
                }
            }
            let new_lines = args.new_string.lines().count();
            return match atomic_write_text(&validated, &args.new_string).await {
                Ok(()) => {
                    ToolResult::ok(
                        serde_json::json!({
                            "created": true,
                            "path": args.path,
                            "bytes": args.new_string.len(),
                            "linesAdded": new_lines,
                            "linesRemoved": 0,
                            "diffStat": format!("+{new_lines} -0 lines"),
                        })
                        .to_string(),
                    )
                }
                Err(e) => ToolResult::typed_err(
                    ToolErrorType::FileWriteFailure,
                    format!("edit_file failed to create '{}': {e}", args.path),
                ),
            };
        }

        if args.old_string == args.new_string {
            return ToolResult::typed_err(
                ToolErrorType::EditNoChange,
                create_user_friendly_error(ToolErrorType::EditNoChange, &args.path),
            );
        }

        let validated = match ensure_within_workspace(Path::new(&args.path), true) {
            Ok(p) => p,
            Err(_) => {
                return ToolResult::typed_err(
                    ToolErrorType::PathNotInWorkspace,
                    create_user_friendly_error(ToolErrorType::PathNotInWorkspace, &args.path),
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
                
                return ToolResult::typed_err(err_type, create_user_friendly_error(err_type, &args.path));
            }
        };

        let current = match String::from_utf8(raw_bytes) {
            Ok(s) => s,
            Err(_) => {
                return ToolResult::typed_err(
                    ToolErrorType::ReadContentFailure,
                    format!("File '{}' contains binary data which cannot be edited. Use appropriate tools for binary files.", args.path),
                );
            }
        };

        let original_line_ending = detect_line_ending(&current);

        // Normalize to LF for matching, then restore original line ending
        let normalized = current.replace("\r\n", "\n");
        let old_normalized = args.old_string.replace("\r\n", "\n");
        let new_normalized = args.new_string.replace("\r\n", "\n");

        let match_count = normalized.matches(&old_normalized).count();
        if let Some(expected) = args.expected_replacements {
            if match_count != expected {
                return ToolResult::typed_err(
                    ToolErrorType::EditMultipleOccurrences,
                    create_user_friendly_error(ToolErrorType::EditMultipleOccurrences, &args.path),
                );
            }
        }

        // --- Fuzzy matching fallback (inspired by Qwen Code's multi-stage correction) ---
        // If exact match fails, try whitespace-normalized matching before giving up.
        let (updated_normalized, replaced, fuzzy_used) = if match_count == 0 {
            match try_fuzzy_match(&normalized, &old_normalized) {
                FuzzyMatchResult::UniqueMatch { start, end } => {
                    // Replace the original text range with new_string
                    let mut result = String::with_capacity(normalized.len());
                    result.push_str(&normalized[..start]);
                    result.push_str(&new_normalized);
                    result.push_str(&normalized[end..]);
                    (result, 1usize, true)
                }
                FuzzyMatchResult::NoMatch => {
                    // Provide helpful context: show the first few lines of the file
                    let file_preview: String = normalized.lines().take(20)
                        .enumerate()
                        .map(|(i, l)| format!("{}|{}", i + 1, l))
                        .collect::<Vec<_>>()
                        .join("\n");
                    return ToolResult::typed_err(
                        ToolErrorType::EditNoOccurrenceFound,
                        format!(
                            "In file '{}': Could not find the specified text to replace \
                             (neither exact nor whitespace-normalized match). \
                             The file may have changed or the old_string is incorrect.\n\
                             Hint: re-read the file with read_file to get the current content, \
                             then retry with the exact text.\n\
                             File preview (first 20 lines):\n{file_preview}",
                            args.path
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
                            args.path
                        ),
                    );
                }
            }
        } else if !args.replace_all && args.expected_replacements.is_none() && match_count > 1 {
            return ToolResult::typed_err(
                ToolErrorType::EditMultipleOccurrences,
                format!(
                    "In file '{}': Found {} exact matches. Provide more surrounding context \
                     to uniquely identify the location, or set replace_all=true.",
                    args.path, match_count
                ),
            );
        } else {
            let result = if args.replace_all {
                normalized.replace(&old_normalized, &new_normalized)
            } else {
                normalized.replacen(&old_normalized, &new_normalized, 1)
            };
            let count = if args.replace_all { match_count } else { 1 };
            (result, count, false)
        };

        // Restore original line ending style
        let updated = if original_line_ending == "crlf" {
            updated_normalized.replace('\n', "\r\n")
        } else {
            updated_normalized
        };

        let old_lines = normalized.lines().count();
        let new_lines = updated.lines().count();
        let added = new_lines.saturating_sub(old_lines);
        let removed = old_lines.saturating_sub(new_lines);

        // Build both a context snippet and a unified-diff snippet for verification
        let snippet = build_edit_snippet(&updated, &new_normalized, 4);
        let diff = build_diff_snippet(&old_normalized, &new_normalized, &args.path);

        match atomic_write_text(&validated, &updated).await {
            Ok(()) => {
                let diff_stat = format!("+{} -{} lines", added, removed);
                let mut result = ToolResult::ok(
                    serde_json::json!({
                        "edited": true,
                        "path": args.path,
                        "replacements": replaced,
                        "bytes": updated.len(),
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
                        "path": args.path,
                        "replacements": replaced,
                        "diffStat": diff_stat,
                        "fuzzyMatch": fuzzy_used,
                        "diff": diff,
                        "snippet": snippet,
                    })
                    .to_string(),
                );
                result.metadata = Some(serde_json::json!({
                    "lineEnding": original_line_ending,
                    "totalLines": new_lines,
                    "fuzzyMatch": fuzzy_used,
                }));
                result
            }
            Err(_) => ToolResult::typed_err(
                ToolErrorType::FileWriteFailure,
                format!("Could not write to file '{}'. Check file permissions or disk space.", args.path),
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
                    for i in ctx_start..line_no {
                        text_output.push_str(&format!("{rel}-{}-{}\n", i + 1, all_lines[i]));
                    }
                }

                text_output.push_str(&format!("{rel}:{}:{}\n", line_no + 1, line));

                if context_lines > 0 {
                    let ctx_end = (line_no + context_lines + 1).min(all_lines.len());
                    for i in (line_no + 1)..ctx_end {
                        text_output.push_str(&format!("{rel}-{}-{}\n", i + 1, all_lines[i]));
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

    fn description(&self) -> &str {
        "Search files using regex. Returns matches with file paths and line numbers in ripgrep-style format. \
         Uses ripgrep (rg) for fast search when available; falls back to built-in implementation. \
         Case-insensitive by default. Supports glob filter, context lines (0-5). Respects .gitignore."
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
            required: vec!["path".to_string(), "edits".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: ApplyPatchArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::err(format!(
                    "apply_patch arguments are not valid JSON: {e}. \
                     Pass {{\"path\":\"...\", \"edits\":[{{\"old_string\":\"...\", \"new_string\":\"...\"}}]}}."
                ))
            }
        };

        if args.edits.is_empty() {
            return ToolResult::err("apply_patch requires at least one edit.".to_string());
        }

        let validated = match ensure_within_workspace(Path::new(&args.path), true) {
            Ok(p) => p,
            Err(_) => {
                return ToolResult::typed_err(
                    ToolErrorType::PathNotInWorkspace,
                    create_user_friendly_error(ToolErrorType::PathNotInWorkspace, &args.path),
                )
            }
        };

        let mut current = match tokio::fs::read_to_string(&validated).await {
            Ok(c) => c,
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
                    create_user_friendly_error(err_type, &args.path),
                );
            }
        };

        if let Some(expected) = args.expected_content.as_deref() {
            if current != expected {
                return ToolResult::err(format!(
                    "apply_patch optimistic lock failed for '{}': content differs from expected_content.",
                    args.path
                ));
            }
        }

        let original_line_ending = detect_line_ending(&current);
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

        let final_content = if original_line_ending == "crlf" {
            working.replace('\n', "\r\n")
        } else {
            working
        };

        match atomic_write_text(&validated, &final_content).await {
            Ok(()) => ToolResult::ok(
                serde_json::json!({
                    "patched": true,
                    "path": args.path,
                    "edits_applied": applied,
                    "bytes": final_content.len(),
                })
                .to_string(),
            ),
            Err(_) => ToolResult::typed_err(
                ToolErrorType::FileWriteFailure,
                format!("Could not write to file '{}'. Check file permissions or disk space.", args.path),
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
    path: String,
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

        // Phase 1: Read all files and compute edits in memory
        let mut staged: Vec<(PathBuf, String, String, Vec<serde_json::Value>)> = Vec::new();

        for (file_idx, entry) in args.edits.iter().enumerate() {
            let validated = match ensure_within_workspace(Path::new(&entry.path), true) {
                Ok(p) => p,
                Err(_) => return ToolResult::err(format!(
                    "multi_edit: file #{file_idx} '{}' is outside the workspace. Transaction aborted, no files modified.",
                    entry.path
                )),
            };

            let raw_bytes = match tokio::fs::read(&validated).await {
                Ok(b) => b,
                Err(e) => return ToolResult::err(format!(
                    "multi_edit: could not read file #{file_idx} '{}': {e}. Transaction aborted, no files modified.",
                    entry.path
                )),
            };

            let original = match String::from_utf8(raw_bytes) {
                Ok(s) => s,
                Err(_) => return ToolResult::err(format!(
                    "multi_edit: file #{file_idx} '{}' contains non-UTF8 data. Transaction aborted.",
                    entry.path
                )),
            };

            let line_ending = detect_line_ending(&original);
            let mut current = original.replace("\r\n", "\n");
            let mut change_log = Vec::new();

            for (change_idx, change) in entry.changes.iter().enumerate() {
                if change.old_string.is_empty() {
                    return ToolResult::err(format!(
                        "multi_edit: file #{file_idx} '{}', change #{change_idx} has empty old_string. Transaction aborted.",
                        entry.path
                    ));
                }

                let old_norm = change.old_string.replace("\r\n", "\n");
                let new_norm = change.new_string.replace("\r\n", "\n");
                let match_count = current.matches(&old_norm).count();

                if match_count == 0 {
                    return ToolResult::err(format!(
                        "multi_edit: file #{file_idx} '{}', change #{change_idx}: old_string not found. \
                         Transaction aborted, no files modified. Re-read the file to get current content.",
                        entry.path
                    ));
                }

                if !change.replace_all && match_count > 1 {
                    return ToolResult::err(format!(
                        "multi_edit: file #{file_idx} '{}', change #{change_idx}: found {match_count} matches. \
                         Set replace_all=true or provide more context. Transaction aborted.",
                        entry.path
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

            let final_content = if line_ending == "crlf" {
                current.replace('\n', "\r\n")
            } else {
                current
            };

            staged.push((validated, entry.path.clone(), final_content, change_log));
        }

        // Phase 2: Write all files (only if all edits succeeded)
        if args.dry_run {
            let results: Vec<serde_json::Value> = staged.iter().map(|(_, path, content, log)| {
                serde_json::json!({
                    "path": path,
                    "changes_applied": log,
                    "result_bytes": content.len(),
                    "result_lines": content.lines().count(),
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
        for (validated_path, display_path, content, change_log) in &staged {
            match atomic_write_text(validated_path, content).await {
                Ok(()) => {
                    written.push(serde_json::json!({
                        "path": display_path,
                        "changes_applied": change_log,
                        "bytes": content.len(),
                        "lines": content.lines().count(),
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
                            .filter_map(|w| w.get("path").and_then(|p| p.as_str()))
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
        let args = serde_json::json!({ "path": file_path.to_string_lossy() }).to_string();
        let out = with_file_access_mode(FileAccessMode::None, Tool::execute(&tool, &args)).await;
        assert!(!out.success, "read_file should be blocked in none mode");
        assert!(
            out.output.contains("outside the current workspace root") || out.output.contains("file access is disabled"),
            "unexpected error output: {}",
            out.output
        );
    }

    #[tokio::test]
    async fn read_file_workspace_blocks_outside_path() {
        let mut temp = tempfile::NamedTempFile::new().expect("tmp file");
        writeln!(temp, "outside").expect("write outside file");
        let outside_path = temp.path().to_string_lossy().to_string();

        let tool = ReadFileTool;
        let args = serde_json::json!({ "path": outside_path }).to_string();
        let out = with_file_access_mode(FileAccessMode::Workspace, Tool::execute(&tool, &args)).await;
        assert!(!out.success, "workspace mode should block outside path");
        assert!(
            out.output.contains("outside the current workspace root"),
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
        let args = serde_json::json!({ "path": outside_path }).to_string();
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
        assert!(out.output.contains("outside the current workspace root") || out.output.contains("file access is disabled"));
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
        assert!(out.output.contains("outside the current workspace root"), "unexpected: {}", out.output);
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
        assert!(out.output.contains("outside the current workspace root") || out.output.contains("file access is disabled"),
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
        let args = serde_json::json!({ "path": file_path.to_string_lossy() }).to_string();
        let out = Tool::execute(&tool, &args).await;
        assert!(out.success, "read should succeed: {}", out.output);
        assert!(
            out.output.contains("File content truncated"),
            "should contain truncation message: ...{}...",
            &out.output[out.output.len().saturating_sub(200)..],
        );
        // Verify metadata indicates truncation
        if let Some(meta) = &out.metadata {
            assert_eq!(meta["truncated"], true);
            assert_eq!(meta["totalLines"], 3000);
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
        assert!(perm_msg.contains("文件访问权限"), "should guide user to setting: {perm_msg}");

        let workspace_msg = create_user_friendly_error(ToolErrorType::PathNotInWorkspace, path);
        assert!(workspace_msg.contains("outside the current workspace root"), "should explain boundary: {workspace_msg}");
        assert!(workspace_msg.contains("工作目录"), "should guide user to work_dir: {workspace_msg}");
        assert!(workspace_msg.contains("文件访问权限"), "should guide user to setting: {workspace_msg}");
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
