use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use fastclaw_core::agent_config::FileAccessMode;
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolResult};
use regex::RegexBuilder;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;

tokio::task_local! {
    static FILE_ACCESS_MODE: FileAccessMode;
}

pub async fn with_file_access_mode<F, T>(mode: FileAccessMode, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    FILE_ACCESS_MODE.scope(mode, fut).await
}

fn workspace_root() -> std::io::Result<PathBuf> {
    std::env::current_dir()
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
        Ok(resolved)
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "path '{}' resolves outside workspace root '{}'",
                path.display(),
                root.display()
            ),
        ))
    }
}

const DEFAULT_READ_FILE_MAX_CHARS: usize = 32_768;
const ABSOLUTE_READ_FILE_MAX_CHARS: usize = 256_000;

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
    let lines: Vec<&str> = content.lines().collect();
    let (start, end) = compute_slice_bounds(lines.len(), offset, limit);
    let mut rendered = String::new();
    for (idx, line) in lines[start..end].iter().enumerate() {
        if number_lines {
            rendered.push_str(&format!("{}|", start + idx + 1));
        }
        rendered.push_str(line);
        if idx + 1 < end.saturating_sub(start) {
            rendered.push('\n');
        }
    }
    rendered
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

fn collect_text_files(base: &Path, max_files: usize) -> std::io::Result<Vec<PathBuf>> {
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
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read one UTF-8 text file from the gateway host with optional line-window slicing. \
         By default, read_file returns full text (capped by max_chars); with offset/limit it returns only the requested line range, which is much faster for large files. \
         Use read_file when you need the authoritative on-disk text before quoting, explaining, or merging edits: source, configs, READMEs, or logs. \
         When you are unsure of cwd or spelling, call list_directory on \".\" or the parent folder first; wrong paths fail fast and burn a turn. \
         To find a symbol or string across the repo, use shell_exec with ripgrep (rg) or grep—do not open many files with read_file in a loop. web_search and web_fetch only reach the public internet; they never read this workspace. \
         Never pass http(s) URLs to read_file; for web pages use web_search to discover links, then web_fetch or http_fetch. \
         Hard cap defaults to 32,768 characters; you can raise max_chars up to 256,000 for one call. \
         Non-UTF-8 and binary content are not supported; request a text export or an approved binary path. Symlinks are followed; broken links fail with a read error—fix the target or use a resolved path. \
         Anti-pattern: using read_file as a workspace-wide search; anti-pattern: re-reading huge files without offset/limit. \
         Examples: {\"path\": \"src/main.rs\"}, {\"path\": \"src/main.rs\", \"offset\": 200, \"limit\": 80, \"number_lines\": true}."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "path".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Single existing file path (absolute or relative to the gateway cwd). Good: 'src/main.rs', './config/default.json', '/var/log/app.log'. Bad: a folder path (use list_directory); bad: 'https://…' (use web_fetch). If ENOENT, list_directory the parent and correct spelling. Symlinks are followed—if you get odd errors, the link target may be missing."
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
            Err(e) => {
                return ToolResult::err(format!(
                    "read_file blocked by workspace boundary policy for '{path}': {e}"
                ))
            }
        };

        match tokio::fs::read_to_string(&validated).await {
            Ok(content) => {
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
                let char_count = content.chars().count();
                let truncated = if char_count > max_chars {
                    let head: String = content.chars().take(max_chars).collect();
                    format!("{head}... [truncated, {char_count} chars total]")
                } else {
                    content
                };
                ToolResult::ok(truncated)
            }
            Err(e) => ToolResult::err(format!(
                "read_file failed for path '{path}': {e}. \
                 What went wrong: the gateway could not open or decode the file as UTF-8 text. \
                 What to do next: if ENOENT (not found), run list_directory on the parent directory or '.' to fix the path; if EACCES (permission denied), pick a readable path or use shell_exec 'ls -la <dir>' if policy allows; if the message mentions invalid UTF-8, the file is binary or wrong encoding—do not retry read_file; ask for a text export or use a binary-safe workflow."
            )),
        }
    }
}

/// Write content to a file, creating it if needed.
pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write one UTF-8 text file on the gateway host with explicit mode control. \
         Modes: overwrite (default, atomic full replace), append (add content at file end), create_new (fail if file exists). \
         For safe concurrent edits, use expected_content to enforce optimistic locking: write proceeds only when the current file exactly matches your expected baseline. \
         Missing parent directories are created automatically; you usually do not need a separate mkdir. \
         Before overwriting important files, read_file the current version and merge edits carefully. \
         Prefer write_file for source, docs, and configs you own; avoid bulk-rewriting vendor trees or lockfiles unless the user explicitly asked. \
         Runs as the gateway OS user; permission denied means pick a writable location or ask the operator—do not assume sudo. \
         Anti-pattern: megabyte payloads in one JSON string—split files or stream via shell if policy allows. \
         Examples: {\"path\": \"notes.md\", \"content\": \"...\"}, {\"path\": \"log.txt\", \"content\": \"line\\n\", \"mode\": \"append\"}."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "path".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "File path to write (absolute or relative to gateway cwd). Examples: 'src/lib.rs', 'notes/meeting.md'. Parent dirs are auto-created. The path must denote a file, not an existing directory. To create only a directory, use shell_exec 'mkdir -p …' if allowed, or write a placeholder file such as .gitkeep."
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
            Err(e) => {
                return ToolResult::err(format!(
                    "write_file blocked by workspace boundary policy for '{path}': {e}"
                ))
            }
        };
        let file_path = validated.as_path();
        if let Some(parent) = file_path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return ToolResult::err(format!(
                    "write_file could not create parent directories for '{path}': {e}. \
                     Pick a path under a writable root, shorten the path, or create parents with shell_exec (mkdir -p) only if policy allows. \
                     Check disk space and permissions if this persists."
                ));
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
            Err(e) => ToolResult::err(format!(
                "write_file failed for '{path}': {e}. \
                 What went wrong: the gateway could not write bytes (wrong path type, full disk, permissions, or OS error). \
                 What to do next: ensure the path is a file target (not an existing directory), check free space and write permission; if the file already existed, read_file it first and merge before retrying so you do not clobber content."
            )),
        }
    }
}

/// Edit text in a file by replacing an exact snippet.
pub struct EditFileTool;

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Edit one UTF-8 text file using exact string replacement, similar to patch-style edits but simpler for LLMs. \
         Provide old_string and new_string; by default exactly one replacement is applied. \
         If old_string appears multiple times, set replace_all=true or expected_replacements to avoid ambiguity. \
         This avoids full-file rewrites and reduces accidental clobbering. \
         Example: {\"path\": \"src/lib.rs\", \"old_string\": \"fn old()\", \"new_string\": \"fn new()\"}."
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

        if args.old_string.is_empty() {
            return ToolResult::err(
                "edit_file requires non-empty old_string to avoid ambiguous global insertions."
                    .to_string(),
            );
        }

        let validated = match ensure_within_workspace(Path::new(&args.path), true) {
            Ok(p) => p,
            Err(e) => {
                return ToolResult::err(format!(
                    "edit_file blocked by workspace boundary policy for '{}': {e}",
                    args.path
                ))
            }
        };

        let current = match tokio::fs::read_to_string(&validated).await {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::err(format!(
                    "edit_file failed to read '{}': {e}",
                    args.path
                ))
            }
        };

        let match_count = current.matches(&args.old_string).count();
        if let Some(expected) = args.expected_replacements {
            if match_count != expected {
                return ToolResult::err(format!(
                    "edit_file expected {expected} matches but found {match_count} in '{}'.",
                    args.path
                ));
            }
        }
        if match_count == 0 {
            return ToolResult::err(format!(
                "edit_file found no matches for old_string in '{}'.",
                args.path
            ));
        }
        if !args.replace_all && args.expected_replacements.is_none() && match_count > 1 {
            return ToolResult::err(format!(
                "edit_file found {match_count} matches in '{}'; set replace_all=true or expected_replacements to disambiguate.",
                args.path
            ));
        }

        let updated = if args.replace_all {
            current.replace(&args.old_string, &args.new_string)
        } else {
            current.replacen(&args.old_string, &args.new_string, 1)
        };
        let replaced = if args.replace_all { match_count } else { 1 };

        match atomic_write_text(&validated, &updated).await {
            Ok(()) => ToolResult::ok(
                serde_json::json!({
                    "edited": true,
                    "path": args.path,
                    "replacements": replaced,
                    "bytes": updated.len(),
                })
                .to_string(),
            ),
            Err(e) => ToolResult::err(format!(
                "edit_file failed to write '{}': {e}",
                args.path
            )),
        }
    }
}

/// Search text across files under a directory.
pub struct SearchInFilesTool;

#[async_trait]
impl Tool for SearchInFilesTool {
    fn name(&self) -> &str {
        "search_in_files"
    }

    fn description(&self) -> &str {
        "Search text in workspace files quickly, similar to ripgrep-style lookup. \
         Supports regex pattern, optional directory scope, and optional glob filter. \
         Returns structured matches with file path, line, column, and matching line text. \
         Best practice: use search_in_files first to locate symbols, then read_file with offset/limit."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "pattern".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Regex pattern to search for. Example: \"fn\\s+main\"."
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
                "description": "Optional simple glob filter for relative paths, e.g. '*.rs' or 'src/*.ts'."
            }),
        );
        props.insert(
            "case_sensitive".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "Optional. Defaults to true."
            }),
        );
        props.insert(
            "max_results".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Optional cap on returned matches. Default 200, max 2000."
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
                     Pass {{\"pattern\":\"...\"}} with optional path/glob/case_sensitive/max_results."
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
                return ToolResult::err(format!(
                    "search_in_files blocked by workspace boundary policy for '{scope}': {e}"
                ))
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

        let regex = match RegexBuilder::new(&args.pattern)
            .case_insensitive(!args.case_sensitive.unwrap_or(true))
            .build()
        {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::err(format!(
                    "search_in_files invalid regex pattern '{}': {e}",
                    args.pattern
                ))
            }
        };

        let max_results = args.max_results.unwrap_or(200).clamp(1, 2000);
        let mut files = if validated.is_file() {
            vec![validated.clone()]
        } else {
            match collect_text_files(&validated, 50_000) {
                Ok(v) => v,
                Err(e) => {
                    return ToolResult::err(format!(
                        "search_in_files failed to enumerate files under '{}': {e}",
                        scope
                    ))
                }
            }
        };
        files.sort();

        let mut results = Vec::new();
        let mut scanned_files = 0usize;
        let mut matched_files = 0usize;
        let mut truncated = false;
        for file in files {
            if results.len() >= max_results {
                truncated = true;
                break;
            }
            let rel = file
                .strip_prefix(&root)
                .unwrap_or(file.as_path())
                .to_string_lossy()
                .to_string();
            if let Some(glob) = args.glob.as_deref() {
                if !simple_glob_match(glob, &rel) {
                    continue;
                }
            }

            scanned_files += 1;
            let content = match fs::read_to_string(&file) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let mut file_hit = false;
            for (line_no, line) in content.lines().enumerate() {
                if results.len() >= max_results {
                    truncated = true;
                    break;
                }
                for m in regex.find_iter(line) {
                    results.push(serde_json::json!({
                        "path": rel,
                        "line": line_no + 1,
                        "column": m.start() + 1,
                        "match": m.as_str(),
                        "text": line,
                    }));
                    file_hit = true;
                    if results.len() >= max_results {
                        truncated = true;
                        break;
                    }
                }
                if results.len() >= max_results {
                    break;
                }
            }
            if file_hit {
                matched_files += 1;
            }
        }

        ToolResult::ok(
            serde_json::json!({
                "pattern": args.pattern,
                "scope": scope,
                "glob": args.glob,
                "case_sensitive": args.case_sensitive.unwrap_or(true),
                "matches": results,
                "count": results.len(),
                "scanned_files": scanned_files,
                "matched_files": matched_files,
                "truncated": truncated,
            })
            .to_string(),
        )
    }
}

/// Apply multiple string replacement edits to one file atomically.
pub struct ApplyPatchTool;

#[async_trait]
impl Tool for ApplyPatchTool {
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
            Err(e) => {
                return ToolResult::err(format!(
                    "apply_patch blocked by workspace boundary policy for '{}': {e}",
                    args.path
                ))
            }
        };

        let mut current = match tokio::fs::read_to_string(&validated).await {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::err(format!(
                    "apply_patch failed to read '{}': {e}",
                    args.path
                ))
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

        let mut applied = Vec::new();
        for (idx, edit) in args.edits.iter().enumerate() {
            if edit.old_string.is_empty() {
                return ToolResult::err(format!(
                    "apply_patch edit #{idx} has empty old_string, which is not allowed."
                ));
            }
            let match_count = current.matches(&edit.old_string).count();
            if let Some(expected) = edit.expected_replacements {
                if match_count != expected {
                    return ToolResult::err(format!(
                        "apply_patch edit #{idx} expected {expected} matches but found {match_count}."
                    ));
                }
            }
            if match_count == 0 {
                return ToolResult::err(format!(
                    "apply_patch edit #{idx} found no matches for old_string."
                ));
            }
            if !edit.replace_all && edit.expected_replacements.is_none() && match_count > 1 {
                return ToolResult::err(format!(
                    "apply_patch edit #{idx} found {match_count} matches; set replace_all=true or expected_replacements to disambiguate."
                ));
            }
            let replaced = if edit.replace_all { match_count } else { 1 };
            current = if edit.replace_all {
                current.replace(&edit.old_string, &edit.new_string)
            } else {
                current.replacen(&edit.old_string, &edit.new_string, 1)
            };
            applied.push(serde_json::json!({
                "edit_index": idx,
                "replacements": replaced,
            }));
        }

        match atomic_write_text(&validated, &current).await {
            Ok(()) => ToolResult::ok(
                serde_json::json!({
                    "patched": true,
                    "path": args.path,
                    "edits_applied": applied,
                    "bytes": current.len(),
                })
                .to_string(),
            ),
            Err(e) => ToolResult::err(format!(
                "apply_patch failed to write '{}': {e}",
                args.path
            )),
        }
    }
}

/// List files and directories at a given path.
pub struct ListDirectoryTool;

#[async_trait]
impl Tool for ListDirectoryTool {
    fn name(&self) -> &str {
        "list_directory"
    }

    fn description(&self) -> &str {
        "List immediate children of one directory on the gateway host. The JSON response includes each child's name, type (file, directory, or symlink), and size in bytes; names are sorted lexicographically for stable output across calls. \
         Use list_directory to discover layout, confirm spelling before read_file or write_file, or see sibling modules next to a path the user mentioned. \
         Non-recursive by design—one level per call. To go deeper, call list_directory on subdirectories, or use shell_exec with find/rg when policy allows for large discovery. \
         Do not use this on a file path (ENOTDIR)—use read_file for file bodies. Dotfiles appear if the OS lists them; they are not hidden by default. \
         Anti-pattern: expecting a full tree in one shot; anti-pattern: listing huge trees like node_modules without filtering—narrow with rg or ask the user. \
         Example: {\"path\": \".\"} at the repo root; {\"path\": \"crates/fastclaw-agent\"} before editing files in that crate."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "path".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Directory that must exist (absolute or relative to gateway cwd). Examples: '.', 'src/components', '/tmp/out'. Not for files—if you need file contents, use read_file. Symlinked dirs show type 'symlink'. Sort order is lexical, not semantic priority."
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
            Err(e) => {
                return ToolResult::err(format!(
                    "list_directory blocked by workspace boundary policy for '{path}': {e}"
                ))
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

        let payload: serde_json::Value =
            serde_json::from_str(&out.output).expect("json payload");
        let count = payload
            .get("count")
            .and_then(|v| v.as_u64())
            .unwrap_or_default();
        assert!(count >= 1, "should return at least one match");
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
            out.output.contains("file access is disabled"),
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
            out.output.contains("workspace boundary policy"),
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
        assert!(out.output.contains("file access is disabled"));
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
        assert!(out.output.contains("workspace boundary policy"));
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
        assert!(out.output.contains("file access is disabled"));
    }
}
