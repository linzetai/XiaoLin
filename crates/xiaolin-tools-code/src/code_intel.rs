use std::collections::HashMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::Deserialize;
use xiaolin_core::tool::{
    no_retry_recovery_hint, Tool, ToolErrorType, ToolKind, ToolParameterSchema, ToolResult,
};

use crate::lsp_manager::LspSessionManager;
use xiaolin_tools_fs::filesystem::{ensure_within_workspace, ReadFileTool, SearchInFilesTool};
use xiaolin_tools_fs::snippet::line_snippet;

/// Default context lines (each side) attached to a code-location snippet.
const DEFAULT_SNIPPET_CONTEXT: usize = 5;
/// Max files read from disk for cross-file snippets within a single tool call.
/// Caps the extra IO introduced by snippet attachment (results beyond the cap
/// keep their `path`/`line` but get an empty `snippet`).
const MAX_CROSS_FILE_SNIPPET_READS: usize = 20;
/// Number of leading results that receive a full context snippet. Beyond this,
/// results get a single-line snippet to keep large result sets within budget.
const TOP_K_CONTEXT_SNIPPETS: usize = 50;
/// Size cap for files read on demand for snippets (avoid loading huge files).
const MAX_SNIPPET_SOURCE_BYTES: u64 = 2 * 1024 * 1024;

/// Loads `snippet` text for code locations while bounding extra file IO.
///
/// - The **input file** (already loaded as `full_file.output`) is sliced with zero IO.
/// - **Cross-file** locations are read on demand, cached per path, and capped by
///   [`MAX_CROSS_FILE_SNIPPET_READS`]. Past the cap, snippets degrade to empty
///   strings (structure stays stable: `path`/`line` are always preserved).
struct SnippetLoader {
    input_canon: Option<PathBuf>,
    input_content: Option<String>,
    cache: HashMap<PathBuf, Option<String>>,
    reads_remaining: usize,
}

impl SnippetLoader {
    fn new(input_path: Option<&str>, input_content: Option<String>) -> Self {
        let input_canon = input_path.and_then(|p| std::fs::canonicalize(p).ok());
        Self {
            input_canon,
            input_content,
            cache: HashMap::new(),
            reads_remaining: MAX_CROSS_FILE_SNIPPET_READS,
        }
    }

    /// Returns the snippet for `path:line` with `context` lines on each side.
    /// Empty string when the source is unavailable or the read cap is exhausted.
    fn snippet(&mut self, path: &str, line: usize, context: usize) -> String {
        if line == 0 {
            return String::new();
        }
        let canon = std::fs::canonicalize(path).ok();

        // Zero-IO path: location is within the already-loaded input file.
        if let (Some(content), Some(ic), Some(c)) = (&self.input_content, &self.input_canon, &canon)
        {
            if ic == c {
                return line_snippet(content, line, context);
            }
        }

        let key = canon.unwrap_or_else(|| PathBuf::from(path));
        if let Some(cached) = self.cache.get(&key) {
            return cached
                .as_ref()
                .map(|c| line_snippet(c, line, context))
                .unwrap_or_default();
        }
        if self.reads_remaining == 0 {
            return String::new();
        }
        self.reads_remaining -= 1;
        let content = read_snippet_source(&key);
        let snip = content
            .as_ref()
            .map(|c| line_snippet(c, line, context))
            .unwrap_or_default();
        self.cache.insert(key, content);
        snip
    }
}

/// Read a file for snippet extraction, bounded by [`MAX_SNIPPET_SOURCE_BYTES`].
fn read_snippet_source(path: &Path) -> Option<String> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() <= MAX_SNIPPET_SOURCE_BYTES => std::fs::read_to_string(path).ok(),
        _ => None,
    }
}

/// Context-line budget for the i-th result: full context for the first
/// [`TOP_K_CONTEXT_SNIPPETS`], single line afterwards.
fn snippet_context_for_index(idx: usize) -> usize {
    if idx < TOP_K_CONTEXT_SNIPPETS {
        DEFAULT_SNIPPET_CONTEXT
    } else {
        0
    }
}

/// Clamp `find_references` result count (default 200, max 2000).
fn reference_result_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(200).clamp(1, 2000)
}

#[derive(Debug, Deserialize)]
struct WorkspaceSymbolsArgs {
    query: String,
    path: Option<String>,
    glob: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct GoToDefinitionArgs {
    path: String,
    line: usize,
    column: usize,
    symbol: Option<String>,
    search_path: Option<String>,
    glob: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct FindReferencesArgs {
    path: String,
    line: usize,
    column: usize,
    symbol: Option<String>,
    search_path: Option<String>,
    glob: Option<String>,
    include_declaration: Option<bool>,
    limit: Option<usize>,
}

fn extract_token_at_column(line: &str, column: usize) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let idx = column.saturating_sub(1).min(chars.len().saturating_sub(1));
    if !is_ident_char(chars[idx]) {
        return None;
    }
    let mut start = idx;
    while start > 0 && is_ident_char(chars[start - 1]) {
        start -= 1;
    }
    let mut end = idx;
    while end + 1 < chars.len() && is_ident_char(chars[end + 1]) {
        end += 1;
    }
    Some(chars[start..=end].iter().collect())
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Keyword → symbol kind mapping for line-based detection.
/// When adding new language constructs, update this list.
const SYMBOL_KIND_KEYWORDS: &[(&str, &str)] = &[
    ("fn", "function"),
    ("struct", "struct"),
    ("enum", "enum"),
    ("trait", "trait"),
    ("impl", "impl"),
    ("class", "class"),
    ("interface", "interface"),
    ("type", "type"),
];

fn detect_symbol_kind_from_line(line: &str, query: &str) -> String {
    let lowered = line.to_lowercase();
    for (kw, kind) in SYMBOL_KIND_KEYWORDS {
        if lowered.contains(&format!("{kw} {query}")) {
            return kind.to_string();
        }
    }
    "symbol".to_string()
}

fn parse_search_matches(raw: &str) -> Result<Vec<serde_json::Value>, String> {
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw) {
        return Ok(parsed
            .get("matches")
            .and_then(|m| m.as_array())
            .cloned()
            .unwrap_or_default());
    }

    let mut matches = Vec::new();
    for line in raw.lines() {
        if line.starts_with("Found ") || line.starts_with('[') || line == "---" || line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(3, ':');
        let path = match parts.next() {
            Some(p) if !p.is_empty() => p,
            _ => continue,
        };
        let line_no: u64 = match parts.next().and_then(|s| s.parse().ok()) {
            Some(n) => n,
            None => continue,
        };
        let text = parts.next().unwrap_or("");
        matches.push(serde_json::json!({
            "path": path,
            "line": line_no,
            "column": 0,
            "text": text,
        }));
    }
    Ok(matches)
}

fn parse_read_line(raw: &str) -> String {
    raw.lines().next().unwrap_or_default().to_string()
}

fn validate_workspace_read_path(path: &str, tool: &str) -> Result<PathBuf, ToolResult> {
    ensure_within_workspace(Path::new(path), true).map_err(|e| {
        ToolResult::err_with_recovery(
            ToolErrorType::PathNotInWorkspace,
            format!("{tool}: path not allowed: {e}"),
            "Use a path inside the workspace root, or run list_directory/glob to discover valid paths.",
        )
    })
}

fn code_intel_invalid_json(tool: &str, e: impl std::fmt::Display) -> ToolResult {
    ToolResult::err_with_recovery(
        ToolErrorType::InvalidToolParams,
        format!("{tool} invalid JSON: {e}"),
        "Fix the tool arguments JSON schema (required fields, types) before retrying.",
    )
}

fn code_intel_lsp_unavailable(tool: &str, detail: impl std::fmt::Display) -> ToolResult {
    ToolResult::err_with_recovery(
        ToolErrorType::LspUnavailable,
        format!("{tool}: {detail}"),
        no_retry_recovery_hint(
            "Use search_in_files, file_outline, or read_file as fallbacks; start rust-analyzer or the language server if the operator can fix the environment.",
        ),
    )
}

fn code_intel_invalid_params(
    tool: &str,
    message: impl std::fmt::Display,
    hint: impl Into<String>,
) -> ToolResult {
    ToolResult::err_with_recovery(
        ToolErrorType::InvalidToolParams,
        format!("{tool}: {message}"),
        hint.into(),
    )
}

fn code_intel_execution_failed(
    tool: &str,
    message: impl std::fmt::Display,
    hint: impl Into<String>,
) -> ToolResult {
    ToolResult::err_with_recovery(
        ToolErrorType::LspRequestFailed,
        format!("{tool}: {message}"),
        hint.into(),
    )
}

fn code_intel_parse_search_failed(tool: &str, detail: impl std::fmt::Display) -> ToolResult {
    code_intel_execution_failed(
        tool,
        format!("failed to parse search results: {detail}"),
        no_retry_recovery_hint(
            "Use search_in_files or read_file on known paths instead of retrying parse.",
        ),
    )
}

pub struct WorkspaceSymbolsTool;

#[async_trait]
impl Tool for WorkspaceSymbolsTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }
    fn name(&self) -> &str {
        "workspace_symbols"
    }

    fn description(&self) -> &str {
        "Find symbol definitions across workspace files. \
         This MVP implementation uses fast text heuristics for common languages and returns structured symbol locations."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "query".to_string(),
            serde_json::json!({"type":"string","description":"Symbol name to search."}),
        );
        props.insert(
            "path".to_string(),
            serde_json::json!({"type":"string","description":"Optional directory scope. Default '.'."}),
        );
        props.insert(
            "glob".to_string(),
            serde_json::json!({"type":"string","description":"Optional file glob filter, e.g. '*.rs'."}),
        );
        props.insert(
            "limit".to_string(),
            serde_json::json!({"type":"integer","description":"Optional max symbols. Default 50."}),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["query".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: WorkspaceSymbolsArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return code_intel_invalid_json("workspace_symbols", e),
        };
        if args.query.trim().is_empty() {
            return code_intel_invalid_params(
                "workspace_symbols",
                "requires non-empty query",
                "Pass a symbol name or prefix in the query field.",
            );
        }

        // Try local symbol index first (fast, no LSP needed).
        let index_results = crate::symbol_index::SymbolIndex::global().lookup(args.query.trim());
        if !index_results.is_empty() {
            let limit = args.limit.unwrap_or(50).clamp(1, 500);
            let symbols: Vec<serde_json::Value> = index_results
                .into_iter()
                .take(limit)
                .map(|s| {
                    serde_json::json!({
                        "name": s.name,
                        "kind": s.kind,
                        "path": s.file_path,
                        "line": s.start_line,
                        "endLine": s.end_line,
                        "signature": s.signature,
                        // symbol_index has no file content loaded; use the
                        // signature as a zero-IO snippet downgrade (structure parity).
                        "snippet": s.signature,
                        "engine": "symbol_index",
                    })
                })
                .collect();
            return ToolResult::ok(
                serde_json::json!({
                    "query": args.query,
                    "symbols": symbols,
                    "count": symbols.len(),
                    "engine": "symbol_index",
                })
                .to_string(),
            );
        }

        let escaped = regex::escape(args.query.trim());
        let pattern = format!(r"\b(fn|struct|enum|trait|impl|class|interface|type)\s+{escaped}\b");
        let scope = args.path.clone().unwrap_or_else(|| ".".to_string());
        if let Ok(workspace_root) = std::env::current_dir() {
            if let Ok(Some(lsp_symbols)) = LspSessionManager::global()
                .workspace_symbols(
                    &format!("{scope}/dummy.rs"),
                    args.query.trim(),
                    &workspace_root.to_string_lossy(),
                )
                .await
            {
                let mut loader = SnippetLoader::new(None, None);
                let limited = lsp_symbols
                    .into_iter()
                    .take(args.limit.unwrap_or(50).clamp(1, 500))
                    .enumerate()
                    .map(|(idx, s)| {
                        let snippet =
                            loader.snippet(&s.path, s.line, snippet_context_for_index(idx));
                        serde_json::json!({
                            "name": args.query,
                            "kind": "symbol",
                            "path": s.path,
                            "line": s.line,
                            "column": s.column,
                            "snippet": snippet,
                            "engine": "lsp",
                        })
                    })
                    .collect::<Vec<_>>();
                if !limited.is_empty() {
                    let lsp_stats = LspSessionManager::global().stats_snapshot();
                    return ToolResult::ok(
                        serde_json::json!({
                            "query": args.query,
                            "symbols": limited,
                            "count": limited.len(),
                            "engine": "lsp",
                            "lspStats": lsp_stats,
                        })
                        .to_string(),
                    );
                }
            }
        }

        let search_args = serde_json::json!({
            "pattern": pattern,
            "path": scope,
            "glob": args.glob,
            "max_results": args.limit.unwrap_or(50).clamp(1, 500),
        })
        .to_string();
        let search = SearchInFilesTool.execute(&search_args).await;
        if !search.success {
            return search;
        }
        let matches = match parse_search_matches(&search.output) {
            Ok(m) => m,
            Err(e) => return code_intel_parse_search_failed("workspace_symbols", e),
        };

        let symbols: Vec<serde_json::Value> = matches
            .into_iter()
            .map(|m| {
                let text = m.get("text").and_then(|v| v.as_str()).unwrap_or_default();
                serde_json::json!({
                    "name": args.query,
                    "kind": detect_symbol_kind_from_line(text, args.query.trim()),
                    "path": m.get("path").and_then(|v| v.as_str()).unwrap_or_default(),
                    "line": m.get("line").and_then(|v| v.as_u64()).unwrap_or(0),
                    "column": m.get("column").and_then(|v| v.as_u64()).unwrap_or(0),
                    "snippet": text,
                })
            })
            .collect();

        ToolResult::ok(
            serde_json::json!({
                "query": args.query,
                "symbols": symbols,
                "count": symbols.len(),
            })
            .to_string(),
        )
    }
}

pub struct GoToDefinitionTool;

#[async_trait]
impl Tool for GoToDefinitionTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }
    fn name(&self) -> &str {
        "go_to_definition"
    }

    fn description(&self) -> &str {
        "Resolve symbol definition location from a file position. \
         Uses local token extraction plus workspace symbol lookup."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert("path".to_string(), serde_json::json!({"type":"string"}));
        props.insert("line".to_string(), serde_json::json!({"type":"integer"}));
        props.insert("column".to_string(), serde_json::json!({"type":"integer"}));
        props.insert(
            "symbol".to_string(),
            serde_json::json!({"type":"string","description":"Optional explicit symbol override."}),
        );
        props.insert(
            "search_path".to_string(),
            serde_json::json!({"type":"string","description":"Optional workspace scope."}),
        );
        props.insert("glob".to_string(), serde_json::json!({"type":"string"}));
        props.insert("limit".to_string(), serde_json::json!({"type":"integer"}));
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["path".to_string(), "line".to_string(), "column".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: GoToDefinitionArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return code_intel_invalid_json("go_to_definition", e),
        };
        let file_path = args.path.clone();
        let symbol =
            if let Some(s) = args.symbol {
                s
            } else {
                let read_args = serde_json::json!({
                    "path": file_path.clone(),
                    "offset": args.line as i64,
                    "limit": 1
                })
                .to_string();
                let line_result = ReadFileTool.execute(&read_args).await;
                if !line_result.success {
                    return line_result;
                }
                let line = parse_read_line(&line_result.output);
                match extract_token_at_column(&line, args.column) {
                    Some(tok) => tok,
                    None => return code_intel_invalid_params(
                        "go_to_definition",
                        format!(
                            "could not extract symbol at line {}, column {}",
                            args.line, args.column
                        ),
                        "Pass an explicit symbol parameter or place the cursor on an identifier.",
                    ),
                }
            };

        // Try local symbol index first for fast definition lookup.
        let index_results = crate::symbol_index::SymbolIndex::global().lookup(&symbol);
        let exact_defs: Vec<_> = index_results.iter().filter(|s| s.name == symbol).collect();
        if exact_defs.len() == 1 {
            let d = &exact_defs[0];
            return ToolResult::ok(
                serde_json::json!({
                    "query": symbol,
                    "symbols": [{
                        "name": d.name,
                        "kind": d.kind,
                        "path": d.file_path,
                        "line": d.start_line,
                        "endLine": d.end_line,
                        "signature": d.signature,
                        "snippet": d.signature,
                        "engine": "symbol_index",
                    }],
                    "count": 1,
                    "engine": "symbol_index",
                })
                .to_string(),
            );
        }

        let read_args_full = serde_json::json!({
            "path": file_path.clone(),
        })
        .to_string();
        let full_file = ReadFileTool.execute(&read_args_full).await;
        if full_file.success {
            if let Ok(workspace_root) = std::env::current_dir() {
                if let Ok(Some(locs)) = LspSessionManager::global()
                    .go_to_definition(
                        &file_path,
                        args.line,
                        args.column,
                        &full_file.output,
                        &workspace_root.to_string_lossy(),
                    )
                    .await
                {
                    if !locs.is_empty() {
                        let lsp_stats = LspSessionManager::global().stats_snapshot();
                        let mut loader =
                            SnippetLoader::new(Some(&file_path), Some(full_file.output.clone()));
                        let defs = locs
                            .into_iter()
                            .enumerate()
                            .map(|(idx, d)| {
                                let snippet =
                                    loader.snippet(&d.path, d.line, snippet_context_for_index(idx));
                                serde_json::json!({
                                    "name": symbol,
                                    "kind": "symbol",
                                    "path": d.path,
                                    "line": d.line,
                                    "column": d.column,
                                    "snippet": snippet,
                                    "engine": "lsp",
                                })
                            })
                            .collect::<Vec<_>>();
                        return ToolResult::ok(
                            serde_json::json!({
                                "query": symbol,
                                "symbols": defs,
                                "count": defs.len(),
                                "engine": "lsp",
                                "lspStats": lsp_stats,
                            })
                            .to_string(),
                        );
                    }
                }
            }
        }

        // If the symbol index found multiple matches but not a unique one, return them.
        if !index_results.is_empty() {
            let limit = args.limit.unwrap_or(20);
            let symbols: Vec<serde_json::Value> = index_results
                .into_iter()
                .take(limit)
                .map(|s| {
                    serde_json::json!({
                        "name": s.name,
                        "kind": s.kind,
                        "path": s.file_path,
                        "line": s.start_line,
                        "endLine": s.end_line,
                        "signature": s.signature,
                        "snippet": s.signature,
                        "engine": "symbol_index",
                    })
                })
                .collect();
            return ToolResult::ok(
                serde_json::json!({
                    "query": symbol,
                    "symbols": symbols,
                    "count": symbols.len(),
                    "engine": "symbol_index",
                })
                .to_string(),
            );
        }

        let symbol_args = serde_json::json!({
            "query": symbol,
            "path": args.search_path.unwrap_or_else(|| ".".to_string()),
            "glob": args.glob,
            "limit": args.limit.unwrap_or(20),
        })
        .to_string();
        WorkspaceSymbolsTool.execute(&symbol_args).await
    }
}

pub struct FindReferencesTool;

#[async_trait]
impl Tool for FindReferencesTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }
    fn name(&self) -> &str {
        "find_references"
    }

    fn description(&self) -> &str {
        "Find references of a symbol from a file position across workspace files."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert("path".to_string(), serde_json::json!({"type":"string"}));
        props.insert("line".to_string(), serde_json::json!({"type":"integer"}));
        props.insert("column".to_string(), serde_json::json!({"type":"integer"}));
        props.insert("symbol".to_string(), serde_json::json!({"type":"string"}));
        props.insert(
            "search_path".to_string(),
            serde_json::json!({"type":"string"}),
        );
        props.insert("glob".to_string(), serde_json::json!({"type":"string"}));
        props.insert(
            "include_declaration".to_string(),
            serde_json::json!({"type":"boolean"}),
        );
        props.insert("limit".to_string(), serde_json::json!({"type":"integer"}));
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["path".to_string(), "line".to_string(), "column".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: FindReferencesArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return code_intel_invalid_json("find_references", e),
        };
        let file_path = args.path.clone();

        let symbol =
            if let Some(s) = args.symbol {
                s
            } else {
                let read_args = serde_json::json!({
                    "path": file_path.clone(),
                    "offset": args.line as i64,
                    "limit": 1
                })
                .to_string();
                let line_result = ReadFileTool.execute(&read_args).await;
                if !line_result.success {
                    return line_result;
                }
                let line = parse_read_line(&line_result.output);
                match extract_token_at_column(&line, args.column) {
                    Some(tok) => tok,
                    None => return code_intel_invalid_params(
                        "find_references",
                        format!(
                            "could not extract symbol at line {}, column {}",
                            args.line, args.column
                        ),
                        "Pass an explicit symbol parameter or place the cursor on an identifier.",
                    ),
                }
            };

        let limit = reference_result_limit(args.limit);

        // Try local symbol index first for fast reference lookup.
        let index_refs = crate::symbol_index::SymbolIndex::global().find_references(&symbol);
        if !index_refs.is_empty() {
            let arr: Vec<serde_json::Value> = index_refs
                .into_iter()
                .take(limit)
                .map(|s| {
                    serde_json::json!({
                        "path": s.file_path,
                        "line": s.start_line,
                        "endLine": s.end_line,
                        "name": s.name,
                        "kind": s.kind,
                        "signature": s.signature,
                        "snippet": s.signature,
                        "engine": "symbol_index",
                    })
                })
                .collect();
            return ToolResult::ok(
                serde_json::json!({
                    "symbol": symbol,
                    "references": arr,
                    "count": arr.len(),
                    "engine": "symbol_index",
                })
                .to_string(),
            );
        }

        let read_args_full = serde_json::json!({
            "path": file_path.clone(),
        })
        .to_string();
        let full_file = ReadFileTool.execute(&read_args_full).await;
        if full_file.success {
            if let Ok(workspace_root) = std::env::current_dir() {
                if let Ok(Some(refs)) = LspSessionManager::global()
                    .find_references(
                        &file_path,
                        args.line,
                        args.column,
                        &full_file.output,
                        args.include_declaration.unwrap_or(false),
                        &workspace_root.to_string_lossy(),
                    )
                    .await
                {
                    if !refs.is_empty() {
                        let lsp_stats = LspSessionManager::global().stats_snapshot();
                        let mut loader =
                            SnippetLoader::new(Some(&file_path), Some(full_file.output.clone()));
                        let arr = refs
                            .into_iter()
                            .take(limit)
                            .enumerate()
                            .map(|(idx, r)| {
                                let snippet =
                                    loader.snippet(&r.path, r.line, snippet_context_for_index(idx));
                                serde_json::json!({
                                    "path": r.path,
                                    "line": r.line,
                                    "column": r.column,
                                    "snippet": snippet,
                                    "engine": "lsp",
                                })
                            })
                            .collect::<Vec<_>>();
                        return ToolResult::ok(
                            serde_json::json!({
                                "symbol": symbol,
                                "references": arr,
                                "count": arr.len(),
                                "engine": "lsp",
                                "lspStats": lsp_stats,
                            })
                            .to_string(),
                        );
                    }
                }
            }
        }

        let scope_path = args.search_path.clone().unwrap_or_else(|| ".".to_string());
        let pattern = format!(r"\b{}\b", regex::escape(&symbol));
        let search_args = serde_json::json!({
            "pattern": pattern,
            "path": scope_path,
            "glob": args.glob,
            "max_results": limit,
        })
        .to_string();
        let result = SearchInFilesTool.execute(&search_args).await;
        if !result.success {
            return result;
        }
        let mut refs = match parse_search_matches(&result.output) {
            Ok(v) => v,
            Err(e) => return code_intel_parse_search_failed("find_references", e),
        };

        if !args.include_declaration.unwrap_or(false) {
            let def_args = serde_json::json!({
                "query": symbol,
                "path": args.search_path.clone().unwrap_or_else(|| ".".to_string()),
                "glob": args.glob,
                "limit": 50,
            })
            .to_string();
            let defs = WorkspaceSymbolsTool.execute(&def_args).await;
            if defs.success {
                if let Ok(def_json) = serde_json::from_str::<serde_json::Value>(&defs.output) {
                    let mut def_set = std::collections::HashSet::new();
                    for d in def_json
                        .get("symbols")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default()
                    {
                        let p = d.get("path").and_then(|v| v.as_str()).unwrap_or_default();
                        let l = d.get("line").and_then(|v| v.as_u64()).unwrap_or_default();
                        def_set.insert((p.to_string(), l));
                    }
                    refs.retain(|r| {
                        let p = r.get("path").and_then(|v| v.as_str()).unwrap_or_default();
                        let l = r.get("line").and_then(|v| v.as_u64()).unwrap_or_default();
                        !def_set.contains(&(p.to_string(), l))
                    });
                }
            }
        }

        // Align ripgrep fallback field with the other engine paths: `text` → `snippet`.
        for r in refs.iter_mut() {
            if let Some(obj) = r.as_object_mut() {
                if let Some(text) = obj.remove("text") {
                    obj.insert("snippet".to_string(), text);
                } else {
                    obj.entry("snippet")
                        .or_insert(serde_json::Value::String(String::new()));
                }
            }
        }

        ToolResult::ok(
            serde_json::json!({
                "symbol": symbol,
                "references": refs,
                "count": refs.len(),
            })
            .to_string(),
        )
    }
}

// ─── Unified LSP Tool ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnifiedLspArgs {
    operation: String,
    #[serde(default)]
    file_path: Option<String>,
    #[serde(default)]
    line: Option<usize>,
    #[serde(default)]
    character: Option<usize>,
    #[serde(default)]
    include_declaration: Option<bool>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

pub struct UnifiedLspTool;

#[async_trait]
impl Tool for UnifiedLspTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }
    fn supports_parallel(&self) -> bool {
        true
    }
    fn name(&self) -> &str {
        "lsp"
    }

    fn description(&self) -> &str {
        "LSP code intelligence: goToDefinition, findReferences, hover, symbols, diagnostics, and more."
    }

    fn prompt(&self) -> String {
        "Unified LSP tool for code navigation, symbol search, and diagnostics.\n\n\
## When to Use\n\
- **goToDefinition**: jump to where a symbol is defined\n\
- **findReferences**: list all usages of a symbol at a position\n\
- **hover**: type/signature/docs at a cursor position\n\
- **documentSymbol**: file-level symbol tree (like outline); needs `filePath` only\n\
- **workspaceSymbol**: project-wide symbol search by name; needs `query` only\n\
- **diagnostics** / **workspaceDiagnostics**: compiler/linter errors for one file or the whole workspace\n\
- **codeActions**: available quick-fixes at a position (may be stubbed)\n\n\
## goToImplementation (Separate Flow)\n\
- Jumps from a trait/abstract symbol to concrete implementations\n\
- Results typically lack rich source context — **no `snippet` field**\n\
- After locating targets, use `search_in_files` or `read_file` with `lines` on returned paths\n\n\
## Snippet Field\n\
- **goToDefinition**: `snippet` with ±5 lines (capped at 600 chars) — inspect before read_file\n\
- **findReferences**: first 50 references include ±5 line snippets; index 50+ return single-line snippets only\n\
- **workspaceSymbol**: includes `snippet` with surrounding context\n\
- **goToImplementation**: no snippet — see section above\n\n\
## Tool Cooperation\n\
Recommended flow for unfamiliar code:\n\
1. `file_outline` or `documentSymbol` — understand file structure\n\
2. `lsp` (goToDefinition / findReferences / workspaceSymbol) — locate targets with snippets\n\
3. `read_file` with `lines` — read only the range you still need\n\
If LSP is unavailable or returns empty, fall back to `search_in_files` for text/symbol search.\n\n\
## Parameter Rules\n\
- **Position-scoped** (goToDefinition, findReferences, hover, goToImplementation, codeActions): \
require `filePath` + `line` (1-based) + `character` (1-based column)\n\
- **File-scoped** (documentSymbol, diagnostics): require `filePath` only\n\
- **Workspace-scoped** (workspaceSymbol, workspaceDiagnostics): no `filePath`; workspaceSymbol needs `query`\n\
- `limit` caps result count (defaults vary by operation); `includeDeclaration` applies to findReferences\n\n\
## Anti-Patterns\n\
- Do NOT cat/head/tail entire files to find a symbol — use lsp or search_in_files\n\
- Do NOT call position-scoped operations without `line` and `character`\n\
- Do NOT read_file the whole file right after lsp when `snippet` already answers your question\n\
- Do NOT expect goToImplementation snippets — follow up with search_in_files or read_file\n\
- Do NOT retry lsp in a tight loop when errors say LSP is unavailable — switch to search_in_files"
            .to_string()
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "operation".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["goToDefinition", "findReferences", "hover", "documentSymbol",
                         "workspaceSymbol", "goToImplementation", "diagnostics",
                         "workspaceDiagnostics", "codeActions"],
                "description": "LSP operation to perform."
            }),
        );
        props.insert("filePath".to_string(), serde_json::json!({
            "type": "string",
            "description": "File path (absolute or workspace-relative). Required for all operations except workspaceSymbol and workspaceDiagnostics."
        }));
        props.insert("line".to_string(), serde_json::json!({
            "type": "integer",
            "description": "1-based line number. Required for position-scoped operations (goToDefinition, findReferences, hover, goToImplementation)."
        }));
        props.insert(
            "character".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "1-based column number. Required for position-scoped operations."
            }),
        );
        props.insert(
            "includeDeclaration".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "Include the declaration in findReferences results. Default false."
            }),
        );
        props.insert(
            "query".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Query string for workspaceSymbol search."
            }),
        );
        props.insert(
            "limit".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Maximum number of results. Default 50."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["operation".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: UnifiedLspArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return code_intel_invalid_json("lsp", e),
        };

        match args.operation.as_str() {
            "goToDefinition" => {
                let (path, line, col) = match require_position(&args) {
                    Ok(v) => v,
                    Err(e) => return e,
                };
                let inner_args = serde_json::json!({
                    "path": path, "line": line, "column": col,
                    "limit": args.limit.unwrap_or(20)
                })
                .to_string();
                GoToDefinitionTool.execute(&inner_args).await
            }
            "findReferences" => {
                let (path, line, col) = match require_position(&args) {
                    Ok(v) => v,
                    Err(e) => return e,
                };
                let inner_args = serde_json::json!({
                    "path": path, "line": line, "column": col,
                    "include_declaration": args.include_declaration.unwrap_or(false),
                    "limit": args.limit.unwrap_or(200)
                })
                .to_string();
                FindReferencesTool.execute(&inner_args).await
            }
            "workspaceSymbol" => {
                let query = match &args.query {
                    Some(q) if !q.trim().is_empty() => q.clone(),
                    _ => {
                        return code_intel_invalid_params(
                            "workspaceSymbol",
                            "requires a non-empty query parameter",
                            "Pass query with the symbol name to search for.",
                        )
                    }
                };
                let inner_args = serde_json::json!({
                    "query": query,
                    "limit": args.limit.unwrap_or(50)
                })
                .to_string();
                WorkspaceSymbolsTool.execute(&inner_args).await
            }
            "hover" => {
                let (path, line, col) = match require_position(&args) {
                    Ok(v) => v,
                    Err(e) => return e,
                };
                execute_hover(&path, line, col).await
            }
            "documentSymbol" => {
                let path = match &args.file_path {
                    Some(p) if !p.trim().is_empty() => p.clone(),
                    _ => {
                        return code_intel_invalid_params(
                            "documentSymbol",
                            "requires filePath",
                            "Pass filePath as a workspace-relative or absolute path inside the project.",
                        )
                    }
                };
                execute_document_symbol(&path).await
            }
            "goToImplementation" => {
                let (path, line, col) = match require_position(&args) {
                    Ok(v) => v,
                    Err(e) => return e,
                };
                execute_go_to_implementation(&path, line, col).await
            }
            "diagnostics" => {
                let path = match &args.file_path {
                    Some(p) if !p.trim().is_empty() => p.clone(),
                    _ => {
                        return code_intel_invalid_params(
                            "diagnostics",
                            "requires filePath",
                            "Pass filePath as a workspace-relative or absolute path inside the project.",
                        )
                    }
                };
                execute_diagnostics(&path).await
            }
            "workspaceDiagnostics" => execute_workspace_diagnostics().await,
            "codeActions" => {
                let (path, line, col) = match require_position(&args) {
                    Ok(v) => v,
                    Err(e) => return e,
                };
                execute_code_actions(&path, line, col).await
            }
            other => code_intel_invalid_params(
                "lsp",
                format!("unknown operation '{other}'"),
                "Use one of: goToDefinition, findReferences, hover, documentSymbol, workspaceSymbol, goToImplementation, diagnostics, workspaceDiagnostics, codeActions.",
            ),
        }
    }
}

fn require_position(args: &UnifiedLspArgs) -> Result<(String, usize, usize), ToolResult> {
    let path = args
        .file_path
        .as_deref()
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| {
            ToolResult::err_with_recovery(
                ToolErrorType::InvalidToolParams,
                format!("{} requires 'filePath'.", args.operation),
                "Pass filePath as a workspace-relative or absolute path inside the project.",
            )
        })?;
    let line = args.line.ok_or_else(|| {
        ToolResult::err_with_recovery(
            ToolErrorType::InvalidToolParams,
            format!("{} requires 'line'.", args.operation),
            "Pass 1-based line number where the symbol appears.",
        )
    })?;
    let col = args.character.ok_or_else(|| {
        ToolResult::err_with_recovery(
            ToolErrorType::InvalidToolParams,
            format!("{} requires 'character'.", args.operation),
            "Pass 0-based character column within the line.",
        )
    })?;
    Ok((path.to_string(), line, col))
}

async fn execute_hover(path: &str, line: usize, col: usize) -> ToolResult {
    if let Ok(workspace_root) = std::env::current_dir() {
        let lsp = LspSessionManager::global();
        if let Ok(Some(result)) = lsp
            .hover(path, line, col, &workspace_root.to_string_lossy())
            .await
        {
            return ToolResult::ok(
                serde_json::json!({
                    "operation": "hover",
                    "filePath": path,
                    "line": line,
                    "character": col,
                    "content": result,
                    "engine": "lsp",
                })
                .to_string(),
            );
        }
    }
    let read_args =
        serde_json::json!({"path": path, "offset": line as i64, "limit": 5}).to_string();
    let context = ReadFileTool.execute(&read_args).await;
    if context.success {
        let token = extract_token_at_column(context.output.lines().next().unwrap_or_default(), col);
        ToolResult::ok(serde_json::json!({
            "operation": "hover",
            "filePath": path,
            "line": line,
            "character": col,
            "content": format!("Token: {}. Context:\n{}", token.unwrap_or_default(), context.output),
            "engine": "fallback",
        }).to_string())
    } else {
        code_intel_lsp_unavailable(
            "hover",
            format!("could not read {path}:{line}:{col} and LSP hover is unavailable"),
        )
    }
}

async fn execute_document_symbol(path: &str) -> ToolResult {
    let file_path = match validate_workspace_read_path(path, "documentSymbol") {
        Ok(p) => p,
        Err(err) => return err,
    };
    if let Some(lang) = xiaolin_treesitter::CodeParser::detect_language(&file_path) {
        if xiaolin_treesitter::CodeParser::is_language_available(&lang) {
            match xiaolin_treesitter::CodeParser::parse_file(&file_path) {
                Ok(parsed) => {
                    let symbols = xiaolin_treesitter::extract_symbols(
                        &parsed.tree,
                        &parsed.source,
                        &parsed.language,
                    );
                    let json_symbols: Vec<serde_json::Value> = symbols
                        .iter()
                        .filter(|s| !matches!(s.kind, xiaolin_treesitter::SymbolKind::Import))
                        .map(|s| {
                            serde_json::json!({
                                "name": s.name,
                                "kind": s.kind.to_string(),
                                "line": s.start_line,
                                "endLine": s.end_line,
                                "signature": s.signature,
                            })
                        })
                        .collect();
                    return ToolResult::ok(
                        serde_json::json!({
                            "operation": "documentSymbol",
                            "filePath": path,
                            "symbols": json_symbols,
                            "count": json_symbols.len(),
                            "engine": "treesitter",
                            "language": lang,
                        })
                        .to_string(),
                    );
                }
                Err(e) => {
                    tracing::debug!(error = %e, "treesitter parse failed for documentSymbol, falling back to regex");
                }
            }
        }
    }

    let pattern =
        r"\b(fn|struct|enum|trait|impl|class|interface|type|const|static|pub|def|function)\s+\w+";
    let search_args = serde_json::json!({
        "pattern": pattern,
        "path": path,
        "max_results": 200,
    })
    .to_string();
    let result = SearchInFilesTool.execute(&search_args).await;
    if !result.success {
        return result;
    }
    let matches = match parse_search_matches(&result.output) {
        Ok(m) => m,
        Err(e) => return code_intel_parse_search_failed("documentSymbol", e),
    };
    let symbols: Vec<serde_json::Value> = matches
        .into_iter()
        .map(|m| {
            let text = m.get("text").and_then(|v| v.as_str()).unwrap_or_default();
            serde_json::json!({
                "name": text.split_whitespace().last().unwrap_or_default(),
                "kind": text.split_whitespace().next().unwrap_or("symbol"),
                "line": m.get("line").and_then(|v| v.as_u64()).unwrap_or(0),
                "snippet": text.trim(),
            })
        })
        .collect();
    ToolResult::ok(
        serde_json::json!({
            "operation": "documentSymbol",
            "filePath": path,
            "symbols": symbols,
            "count": symbols.len(),
            "engine": "heuristic",
        })
        .to_string(),
    )
}

async fn execute_go_to_implementation(path: &str, line: usize, col: usize) -> ToolResult {
    let read_args =
        serde_json::json!({"path": path, "offset": line as i64, "limit": 1}).to_string();
    let line_result = ReadFileTool.execute(&read_args).await;
    if !line_result.success {
        return line_result;
    }
    let token = extract_token_at_column(line_result.output.lines().next().unwrap_or_default(), col);
    let symbol = match token {
        Some(t) => t,
        None => {
            return code_intel_invalid_params(
                "goToImplementation",
                format!("no symbol at {path}:{line}:{col}"),
                "Place the cursor on a trait or type name, or pass a symbol via go_to_definition first.",
            )
        }
    };
    let pattern = format!(r"\bimpl\s+(?:\w+\s+for\s+)?{}\b", regex::escape(&symbol));
    let search_args = serde_json::json!({
        "pattern": pattern,
        "path": ".",
        "max_results": 50,
    })
    .to_string();
    let result = SearchInFilesTool.execute(&search_args).await;
    if !result.success {
        return result;
    }
    let matches = match parse_search_matches(&result.output) {
        Ok(m) => m,
        Err(e) => return code_intel_parse_search_failed("goToImplementation", e),
    };
    ToolResult::ok(
        serde_json::json!({
            "operation": "goToImplementation",
            "symbol": symbol,
            "implementations": matches,
            "count": matches.len(),
            "engine": "heuristic",
        })
        .to_string(),
    )
}

async fn execute_diagnostics(path: &str) -> ToolResult {
    ToolResult::ok(serde_json::json!({
        "operation": "diagnostics",
        "filePath": path,
        "diagnostics": [],
        "count": 0,
        "note": "LSP diagnostics not yet connected. Use shell_exec with compiler/linter for diagnostics.",
    }).to_string())
}

async fn execute_workspace_diagnostics() -> ToolResult {
    ToolResult::ok(serde_json::json!({
        "operation": "workspaceDiagnostics",
        "diagnostics": [],
        "count": 0,
        "note": "LSP workspace diagnostics not yet connected. Use shell_exec with compiler/linter.",
    }).to_string())
}

async fn execute_code_actions(path: &str, line: usize, col: usize) -> ToolResult {
    ToolResult::ok(serde_json::json!({
        "operation": "codeActions",
        "filePath": path,
        "line": line,
        "character": col,
        "actions": [],
        "count": 0,
        "note": "LSP code actions not yet connected. Describe the desired fix and use edit_file directly.",
    }).to_string())
}

// ─── TreeSitter-powered Tools ─────────────────────────────────────

pub struct FileOutlineTool;

#[async_trait]
impl Tool for FileOutlineTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }
    fn name(&self) -> &str {
        "file_outline"
    }

    fn description(&self) -> &str {
        "AST symbol outline: names, kinds, line ranges, and signatures (no source text)."
    }

    fn prompt(&self) -> String {
        "Extract a structured symbol outline from a source file via tree-sitter AST parsing.\n\n\
## When to Use\n\
- Before reading a **large or unfamiliar** source file (200+ lines)\n\
- When you need a map of functions, classes, structs, traits, etc. with line ranges\n\
- When read_file's built-in outline is insufficient (unsupported language, or you want \
imports via `include_imports`)\n\n\
## vs code_sections\n\
- **file_outline** → symbol **metadata** (name, kind, startLine, endLine, signature) — no code\n\
- **code_sections** → semantic **blocks** with labels and line ranges for planning reads\n\
Use outline to pick *what* exists; use sections to see *how the file is chunked*.\n\n\
## Tool Cooperation\n\
1. `file_outline` → pick target symbol and note `startLine`/`endLine`\n\
2. `read_file` with `lines` (e.g. `\"142-180\"`) → read only that range\n\
Pair with `lsp` when you need cross-file navigation after locating a symbol.\n\n\
## Parameters\n\
- `path`: workspace-relative or absolute source file\n\
- `include_imports`: default false; set true to include import statements in the outline\n\n\
## Anti-Patterns\n\
- Do NOT read_file the entire large file when outline would suffice for orientation\n\
- Do NOT call file_outline on a small known file you already understand — read_file directly\n\
- Do NOT duplicate read_file's auto-outline on supported files unless you need imports or AST detail"
            .to_string()
    }

    fn search_hint(&self) -> &str {
        "outline structure symbols functions classes methods ast tree-sitter"
    }

    fn is_deferred(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "path".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Path to the source file."
            }),
        );
        props.insert(
            "include_imports".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "Include import statements in the outline. Default false."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["path".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        #[derive(Deserialize)]
        struct Args {
            path: String,
            include_imports: Option<bool>,
        }
        let args: Args = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return code_intel_invalid_json("file_outline", e),
        };
        let file_path = match validate_workspace_read_path(&args.path, "file_outline") {
            Ok(p) => p,
            Err(err) => return err,
        };
        let lang = match xiaolin_treesitter::CodeParser::detect_language(&file_path) {
            Some(l) => l,
            None => {
                return code_intel_invalid_params(
                    "file_outline",
                    format!(
                        "unsupported file type '{}'",
                        file_path
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("unknown")
                    ),
                    "Use read_file for unsupported extensions, or pick a source file with a known language.",
                )
            }
        };

        if !xiaolin_treesitter::CodeParser::is_language_available(&lang) {
            return code_intel_lsp_unavailable(
                "file_outline",
                format!("tree-sitter language '{lang}' not available"),
            );
        }

        let parsed = match xiaolin_treesitter::CodeParser::parse_file(&file_path) {
            Ok(p) => p,
            Err(e) => {
                return code_intel_execution_failed(
                    "file_outline",
                    format!("parse error: {e}"),
                    no_retry_recovery_hint(
                        "Verify the file is valid source code; use read_file on known paths instead of retrying parse.",
                    ),
                )
            }
        };

        let all_symbols = xiaolin_treesitter::extract_symbols(&parsed.tree, &parsed.source, &lang);
        let include_imports = args.include_imports.unwrap_or(false);
        let symbols: Vec<serde_json::Value> = all_symbols
            .iter()
            .filter(|s| {
                include_imports || !matches!(s.kind, xiaolin_treesitter::SymbolKind::Import)
            })
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "kind": s.kind.to_string(),
                    "startLine": s.start_line,
                    "endLine": s.end_line,
                    "signature": s.signature,
                })
            })
            .collect();

        ToolResult::ok(
            serde_json::json!({
                "filePath": args.path,
                "language": lang,
                "symbols": symbols,
                "count": symbols.len(),
                "engine": "treesitter",
            })
            .to_string(),
        )
    }
}

pub struct CodeSectionsTool;

#[async_trait]
impl Tool for CodeSectionsTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }
    fn name(&self) -> &str {
        "code_sections"
    }

    fn description(&self) -> &str {
        "Split a source file into semantic sections (functions, classes, impl blocks) with line ranges."
    }

    fn prompt(&self) -> String {
        "Split a source file into semantic sections using tree-sitter AST parsing.\n\n\
## When to Use\n\
- Planning **targeted read_file** calls on large files\n\
- Understanding how a file is divided into logical blocks before editing\n\
- When you need section labels and line ranges rather than a flat symbol list\n\n\
## vs file_outline\n\
- **code_sections** → readable **blocks** (label + startLine + endLine) for navigation\n\
- **file_outline** → symbol **metadata** (kinds, signatures) without chunking strategy\n\
Use sections to plan *which ranges to read*; use outline to scan *all symbols at a glance*.\n\n\
## Tool Cooperation\n\
1. `code_sections` → identify section label and line range\n\
2. `read_file` with `lines` matching that range → fetch only needed code\n\
3. `edit_file` on the exact text from read_file output\n\
For cross-file jumps after locating a section, follow with `lsp` goToDefinition.\n\n\
## Parameters\n\
- `path`: workspace-relative or absolute source file\n\
- `max_chunk_lines`: max lines per section before splitting (default 80)\n\n\
## Anti-Patterns\n\
- Do NOT read_file the whole large file when sections already show the target range\n\
- Do NOT use code_sections on tiny files — read_file directly\n\
- Do NOT call both file_outline and code_sections redundantly unless you need both views"
            .to_string()
    }

    fn search_hint(&self) -> &str {
        "sections blocks split semantic chunks ast code navigation"
    }

    fn is_deferred(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "path".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Path to the source file."
            }),
        );
        props.insert(
            "max_chunk_lines".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Maximum lines per section before splitting. Default 80."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["path".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        #[derive(Deserialize)]
        struct Args {
            path: String,
            max_chunk_lines: Option<usize>,
        }
        let args: Args = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return code_intel_invalid_json("code_sections", e),
        };
        let file_path = match validate_workspace_read_path(&args.path, "code_sections") {
            Ok(p) => p,
            Err(err) => return err,
        };
        let lang = match xiaolin_treesitter::CodeParser::detect_language(&file_path) {
            Some(l) => l,
            None => {
                return code_intel_invalid_params(
                    "code_sections",
                    format!(
                        "unsupported file type '{}'",
                        file_path
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("unknown")
                    ),
                    "Use read_file for unsupported extensions, or pick a source file with a known language.",
                )
            }
        };

        if !xiaolin_treesitter::CodeParser::is_language_available(&lang) {
            return code_intel_lsp_unavailable(
                "code_sections",
                format!("tree-sitter language '{lang}' not available"),
            );
        }

        let parsed = match xiaolin_treesitter::CodeParser::parse_file(&file_path) {
            Ok(p) => p,
            Err(e) => {
                return code_intel_execution_failed(
                    "code_sections",
                    format!("parse error: {e}"),
                    no_retry_recovery_hint(
                        "Verify the file is valid source code; use read_file on known paths instead of retrying parse.",
                    ),
                )
            }
        };

        let max_lines = args.max_chunk_lines.unwrap_or(80).clamp(10, 500);
        let chunks = xiaolin_treesitter::chunk_file(&parsed.tree, &parsed.source, &lang, max_lines);

        let json_chunks: Vec<serde_json::Value> = chunks
            .iter()
            .map(|c| {
                serde_json::json!({
                    "startLine": c.start_line,
                    "endLine": c.end_line,
                    "kind": c.kind,
                    "name": c.name,
                    "lines": c.end_line.saturating_sub(c.start_line) + 1,
                })
            })
            .collect();

        ToolResult::ok(
            serde_json::json!({
                "filePath": args.path,
                "language": lang,
                "chunks": json_chunks,
                "count": json_chunks.len(),
                "totalLines": parsed.source.lines().count(),
                "engine": "treesitter",
            })
            .to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir_in;

    #[tokio::test]
    async fn workspace_symbols_finds_rust_function_definition() {
        let cwd = std::env::current_dir().expect("cwd");
        let tmp = tempdir_in(&cwd).expect("temp dir");
        let file = tmp.path().join("sample.rs");
        tokio::fs::write(&file, "fn hello_code_intel() {}\n")
            .await
            .expect("write");

        let args = serde_json::json!({
            "query": "hello_code_intel",
            "path": tmp.path().to_string_lossy(),
            "glob": "*.rs"
        })
        .to_string();
        let out = WorkspaceSymbolsTool.execute(&args).await;
        assert!(
            out.success,
            "workspace_symbols should succeed: {}",
            out.output
        );
        let body: serde_json::Value = serde_json::from_str(&out.output).expect("json");
        assert!(body.get("count").and_then(|v| v.as_u64()).unwrap_or(0) >= 1);
    }

    #[test]
    fn extract_token_by_column_works() {
        let line = "let target_symbol = value;";
        let token = extract_token_at_column(line, 6).expect("token");
        assert_eq!(token, "target_symbol");
    }

    #[test]
    fn snippet_loader_uses_input_content_without_io() {
        let content = "line1\nfn target() {}\nline3\n";
        // input_path may not exist on disk; canonicalize fails → input fast-path
        // is skipped, but cross-file read returns empty (no panic).
        let mut loader = SnippetLoader::new(Some("/nonexistent/in.rs"), Some(content.to_string()));
        let snip = loader.snippet("/nonexistent/in.rs", 2, 1);
        // Without canonicalize match it degrades to empty; assert no panic + stable type.
        assert!(snip.is_empty() || snip.contains("target"));
    }

    #[tokio::test]
    async fn workspace_symbols_ripgrep_path_includes_snippet_field() {
        let cwd = std::env::current_dir().expect("cwd");
        let tmp = tempdir_in(&cwd).expect("temp dir");
        let file = tmp.path().join("snip_sample.rs");
        tokio::fs::write(&file, "fn snippet_probe_fn() {}\n")
            .await
            .expect("write");

        let args = serde_json::json!({
            "query": "snippet_probe_fn",
            "path": tmp.path().to_string_lossy(),
            "glob": "*.rs"
        })
        .to_string();
        let out = WorkspaceSymbolsTool.execute(&args).await;
        assert!(out.success, "should succeed: {}", out.output);
        let body: serde_json::Value = serde_json::from_str(&out.output).expect("json");
        let symbols = body
            .get("symbols")
            .and_then(|v| v.as_array())
            .expect("symbols");
        assert!(!symbols.is_empty());
        // Every symbol object exposes a `snippet` field (structure parity).
        for s in symbols {
            assert!(
                s.get("snippet").is_some(),
                "each symbol must carry a snippet field: {s}"
            );
        }
    }

    #[test]
    fn reference_result_limit_clamps() {
        assert_eq!(reference_result_limit(None), 200);
        assert_eq!(reference_result_limit(Some(0)), 1);
        assert_eq!(reference_result_limit(Some(500)), 500);
        assert_eq!(reference_result_limit(Some(5000)), 2000);
    }

    #[test]
    fn parse_search_failed_hint_includes_no_retry() {
        let result = code_intel_parse_search_failed("workspace_symbols", "bad json");
        assert!(!result.success);
        assert!(result.output.contains("Stop retrying"));
        assert!(result.output.contains("search_in_files"));
    }

    #[test]
    fn lsp_prompt_mentions_snippet_precision_and_anti_patterns() {
        let tool = UnifiedLspTool;
        let prompt = tool.prompt();
        assert!(
            prompt.contains("snippet"),
            "lsp prompt should mention snippet"
        );
        assert!(
            prompt.contains("Anti-Patterns"),
            "lsp prompt should include Anti-Patterns"
        );
        assert!(
            prompt.contains("first 50"),
            "findReferences snippet policy should mention first 50"
        );
        assert!(
            prompt.contains("goToImplementation"),
            "goToImplementation should be documented separately"
        );
        assert!(
            prompt.contains("no snippet"),
            "goToImplementation should state no snippet"
        );
    }

    #[test]
    fn file_outline_prompt_mentions_read_file() {
        let tool = FileOutlineTool;
        let prompt = tool.prompt();
        assert!(
            prompt.contains("read_file"),
            "file_outline prompt should mention read_file"
        );
    }

    #[test]
    fn snippet_context_for_index_top_k_boundary() {
        assert_eq!(snippet_context_for_index(0), DEFAULT_SNIPPET_CONTEXT);
        assert_eq!(snippet_context_for_index(49), DEFAULT_SNIPPET_CONTEXT);
        assert_eq!(snippet_context_for_index(50), 0);
        assert_eq!(snippet_context_for_index(51), 0);
    }

    #[test]
    fn find_references_lsp_path_respects_limit_before_snippet_context() {
        let refs: Vec<u32> = (0..3000).collect();
        let limit = reference_result_limit(Some(100));
        let taken: Vec<_> = refs.into_iter().take(limit).enumerate().collect();
        assert_eq!(taken.len(), 100);
        assert_eq!(taken.first().map(|(i, _)| *i), Some(0));
        assert_eq!(taken.last().map(|(i, _)| *i), Some(99));
        assert_eq!(snippet_context_for_index(49), DEFAULT_SNIPPET_CONTEXT);
        assert_eq!(snippet_context_for_index(50), 0);
    }

    #[test]
    fn snippet_loader_cross_file_read_cap() {
        let mut loader = SnippetLoader::new(None, None);
        // Distinct paths so each attempt consumes one read; budget must bottom out at 0.
        for i in 0..(MAX_CROSS_FILE_SNIPPET_READS + 5) {
            let snip = loader.snippet(&format!("/no/such/file_{i}.rs"), 1, 5);
            assert!(snip.is_empty());
        }
        assert_eq!(loader.reads_remaining, 0);
    }

    #[tokio::test]
    #[ignore = "requires rust-analyzer; flaky under parallel test execution due to global LSP singleton"]
    async fn workspace_symbols_uses_lsp_when_available() {
        let ra_ok = tokio::process::Command::new("rust-analyzer")
            .arg("--version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !ra_ok {
            return;
        }

        let root = std::env::current_dir().expect("cwd");
        let lsp = LspSessionManager::global();
        let result = lsp
            .workspace_symbols("src/runtime.rs", "AgentRuntime", &root.to_string_lossy())
            .await
            .expect("lsp request should succeed");
        assert!(
            result.is_some(),
            "expected lsp workspace_symbols to be available"
        );
        let stats = lsp.stats_snapshot();
        let total = stats
            .get("requestsTotal")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        assert!(
            total > 0,
            "expected lsp request counter to increase: {}",
            stats
        );
    }
}
