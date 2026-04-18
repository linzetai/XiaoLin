use std::collections::HashMap;

use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolResult};
use serde::Deserialize;

use super::filesystem::{ReadFileTool, SearchInFilesTool};
use super::lsp_manager::LspSessionManager;

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
    let parsed: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("invalid search result JSON: {e}"))?;
    Ok(parsed
        .get("matches")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default())
}

fn parse_read_line(raw: &str) -> String {
    raw.lines().next().unwrap_or_default().to_string()
}

pub struct WorkspaceSymbolsTool;

#[async_trait]
impl Tool for WorkspaceSymbolsTool {
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
        let escaped = regex::escape(args.query.trim());
        let pattern = format!(
            r"\b(fn|struct|enum|trait|impl|class|interface|type)\s+{escaped}\b"
        );
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
        props.insert("symbol".to_string(), serde_json::json!({"type":"string","description":"Optional explicit symbol override."}));
        props.insert("search_path".to_string(), serde_json::json!({"type":"string","description":"Optional workspace scope."}));
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
        props.insert("search_path".to_string(), serde_json::json!({"type":"string"}));
        props.insert("glob".to_string(), serde_json::json!({"type":"string"}));
        props.insert("include_declaration".to_string(), serde_json::json!({"type":"boolean"}));
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

        let scope_path = args
            .search_path
            .clone()
            .unwrap_or_else(|| ".".to_string());
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
        assert!(out.success, "workspace_symbols should succeed: {}", out.output);
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
        assert!(result.is_some(), "expected lsp workspace_symbols to be available");
        let stats = lsp.stats_snapshot();
        let total = stats
            .get("requestsTotal")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        assert!(total > 0, "expected lsp request counter to increase: {}", stats);
    }
}
