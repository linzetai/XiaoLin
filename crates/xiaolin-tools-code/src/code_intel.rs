use std::collections::HashMap;

use async_trait::async_trait;
use xiaolin_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolResult};
use serde::Deserialize;

use xiaolin_tools_fs::filesystem::{ReadFileTool, SearchInFilesTool};
use crate::lsp_manager::LspSessionManager;

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

fn detect_symbol_kind_from_line(line: &str, query: &str) -> String {
    let kinds = [
        ("fn", "function"),
        ("struct", "struct"),
        ("enum", "enum"),
        ("trait", "trait"),
        ("impl", "impl"),
        ("class", "class"),
        ("interface", "interface"),
        ("type", "type"),
    ];
    let lowered = line.to_lowercase();
    for (kw, kind) in kinds {
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
            Err(e) => return ToolResult::err(format!("workspace_symbols invalid JSON: {e}")),
        };
        if args.query.trim().is_empty() {
            return ToolResult::err("workspace_symbols requires non-empty query.".to_string());
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
                let limited = lsp_symbols
                    .into_iter()
                    .take(args.limit.unwrap_or(50).clamp(1, 500))
                    .map(|s| {
                        serde_json::json!({
                            "name": args.query,
                            "kind": "symbol",
                            "path": s.path,
                            "line": s.line,
                            "column": s.column,
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
            Err(e) => return ToolResult::err(e),
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
            Err(e) => return ToolResult::err(format!("go_to_definition invalid JSON: {e}")),
        };
        let file_path = args.path.clone();
        let symbol = if let Some(s) = args.symbol {
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
                None => {
                    return ToolResult::err(format!(
                        "go_to_definition could not extract symbol at line {}, column {}.",
                        args.line, args.column
                    ))
                }
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
                        let defs = locs
                            .into_iter()
                            .map(|d| {
                                serde_json::json!({
                                    "name": symbol,
                                    "kind": "symbol",
                                    "path": d.path,
                                    "line": d.line,
                                    "column": d.column,
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
            Err(e) => return ToolResult::err(format!("find_references invalid JSON: {e}")),
        };
        let file_path = args.path.clone();

        let symbol = if let Some(s) = args.symbol {
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
                None => {
                    return ToolResult::err(format!(
                        "find_references could not extract symbol at line {}, column {}.",
                        args.line, args.column
                    ))
                }
            }
        };

        // Try local symbol index first for fast reference lookup.
        let index_refs = crate::symbol_index::SymbolIndex::global().find_references(&symbol);
        if !index_refs.is_empty() {
            let limit = args.limit.unwrap_or(200).clamp(1, 2000);
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
                        let arr = refs
                            .into_iter()
                            .map(|r| {
                                serde_json::json!({
                                    "path": r.path,
                                    "line": r.line,
                                    "column": r.column,
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
            "max_results": args.limit.unwrap_or(200).clamp(1, 2000),
        })
        .to_string();
        let result = SearchInFilesTool.execute(&search_args).await;
        if !result.success {
            return result;
        }
        let mut refs = match parse_search_matches(&result.output) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(e),
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
        "Unified LSP tool supporting multiple code intelligence operations: \
         goToDefinition, findReferences, hover, documentSymbol, workspaceSymbol, \
         goToImplementation, diagnostics, workspaceDiagnostics, codeActions. \
         Requires file_path for file-scoped operations, and line+character for position-scoped ones."
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
            Err(e) => return ToolResult::err(format!("lsp tool invalid JSON: {e}")),
        };

        match args.operation.as_str() {
            "goToDefinition" => {
                let (path, line, col) = match require_position(&args) {
                    Ok(v) => v,
                    Err(e) => return ToolResult::err(e),
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
                    Err(e) => return ToolResult::err(e),
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
                        return ToolResult::err(
                            "workspaceSymbol requires a non-empty 'query' parameter.".to_string(),
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
                    Err(e) => return ToolResult::err(e),
                };
                execute_hover(&path, line, col).await
            }
            "documentSymbol" => {
                let path = match &args.file_path {
                    Some(p) if !p.trim().is_empty() => p.clone(),
                    _ => return ToolResult::err("documentSymbol requires 'filePath'.".to_string()),
                };
                execute_document_symbol(&path).await
            }
            "goToImplementation" => {
                let (path, line, col) = match require_position(&args) {
                    Ok(v) => v,
                    Err(e) => return ToolResult::err(e),
                };
                execute_go_to_implementation(&path, line, col).await
            }
            "diagnostics" => {
                let path = match &args.file_path {
                    Some(p) if !p.trim().is_empty() => p.clone(),
                    _ => return ToolResult::err("diagnostics requires 'filePath'.".to_string()),
                };
                execute_diagnostics(&path).await
            }
            "workspaceDiagnostics" => execute_workspace_diagnostics().await,
            "codeActions" => {
                let (path, line, col) = match require_position(&args) {
                    Ok(v) => v,
                    Err(e) => return ToolResult::err(e),
                };
                execute_code_actions(&path, line, col).await
            }
            other => ToolResult::err(format!(
                "Unknown LSP operation: '{}'. Supported: goToDefinition, findReferences, hover, \
                 documentSymbol, workspaceSymbol, goToImplementation, diagnostics, \
                 workspaceDiagnostics, codeActions.",
                other
            )),
        }
    }
}

fn require_position(args: &UnifiedLspArgs) -> Result<(String, usize, usize), String> {
    let path = args
        .file_path
        .as_deref()
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| format!("{} requires 'filePath'.", args.operation))?;
    let line = args
        .line
        .ok_or_else(|| format!("{} requires 'line'.", args.operation))?;
    let col = args
        .character
        .ok_or_else(|| format!("{} requires 'character'.", args.operation))?;
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
        ToolResult::err(format!("hover: could not read {}:{}:{}", path, line, col))
    }
}

async fn execute_document_symbol(path: &str) -> ToolResult {
    let file_path = std::path::Path::new(path);
    if let Some(lang) = xiaolin_treesitter::CodeParser::detect_language(file_path) {
        if xiaolin_treesitter::CodeParser::is_language_available(&lang) {
            match xiaolin_treesitter::CodeParser::parse_file(file_path) {
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
        Err(e) => return ToolResult::err(e),
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
            return ToolResult::err(format!(
                "goToImplementation: no symbol at {}:{}:{}",
                path, line, col
            ))
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
        Err(e) => return ToolResult::err(e),
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
        "Extract a structured outline of symbols (functions, classes, structs, etc.) \
         from a source file using AST parsing. Returns symbol names, kinds, line \
         ranges, and signatures — but NOT the source code itself. Use this to \
         quickly understand file structure before targeted reading. \
         For reading actual code in semantic blocks, use `code_sections` instead."
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
            Err(e) => return ToolResult::err(format!("file_outline invalid JSON: {e}")),
        };
        let file_path = std::path::Path::new(&args.path);
        let lang = match xiaolin_treesitter::CodeParser::detect_language(file_path) {
            Some(l) => l,
            None => {
                return ToolResult::err(format!(
                    "file_outline: unsupported file type '{}'",
                    file_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("unknown")
                ))
            }
        };

        if !xiaolin_treesitter::CodeParser::is_language_available(&lang) {
            return ToolResult::err(format!(
                "file_outline: tree-sitter language '{lang}' not available"
            ));
        }

        let parsed = match xiaolin_treesitter::CodeParser::parse_file(file_path) {
            Ok(p) => p,
            Err(e) => return ToolResult::err(format!("file_outline parse error: {e}")),
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
        "Split a source file into semantic sections using AST parsing. \
         Each section is a logical unit (function, class, impl block, etc.) \
         with its line range and label. Use this to plan targeted `read_file` \
         calls on large files — first get the section map, then read only the \
         sections you need. Unlike `file_outline` which lists symbol metadata, \
         this tool shows how the file is divided into readable blocks."
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
            Err(e) => return ToolResult::err(format!("code_sections invalid JSON: {e}")),
        };
        let file_path = std::path::Path::new(&args.path);
        let lang = match xiaolin_treesitter::CodeParser::detect_language(file_path) {
            Some(l) => l,
            None => {
                return ToolResult::err(format!(
                    "code_sections: unsupported file type '{}'",
                    file_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("unknown")
                ))
            }
        };

        if !xiaolin_treesitter::CodeParser::is_language_available(&lang) {
            return ToolResult::err(format!(
                "code_sections: tree-sitter language '{lang}' not available"
            ));
        }

        let parsed = match xiaolin_treesitter::CodeParser::parse_file(file_path) {
            Ok(p) => p,
            Err(e) => return ToolResult::err(format!("code_sections parse error: {e}")),
        };

        let max_lines = args.max_chunk_lines.unwrap_or(80).clamp(10, 500);
        let chunks =
            xiaolin_treesitter::chunk_file(&parsed.tree, &parsed.source, &lang, max_lines);

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
