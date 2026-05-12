use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;

use dashmap::DashMap;
use fastclaw_treesitter::{
    extract_callees, extract_symbols, extract_trait_impls, CodeParser, SymbolKind,
};

static GLOBAL_CACHE: OnceLock<CodeGraphCache> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct FileCodeContext {
    pub pub_symbols: Vec<(String, SymbolKind)>,
    pub callees: Vec<String>,
    pub trait_impls: Vec<(String, String)>,
    pub imports: Vec<String>,
    pub updated_at: Instant,
}

pub struct CodeGraphCache {
    cache: DashMap<PathBuf, FileCodeContext>,
}

impl Default for CodeGraphCache {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeGraphCache {
    pub fn new() -> Self {
        Self {
            cache: DashMap::new(),
        }
    }

    pub fn global() -> &'static CodeGraphCache {
        GLOBAL_CACHE.get_or_init(CodeGraphCache::new)
    }

    pub fn extract_context(path: &Path, source: &str, language: &str) -> FileCodeContext {
        let parsed = match CodeParser::parse(source, language) {
            Ok(p) => p,
            Err(_) => {
                return FileCodeContext {
                    pub_symbols: Vec::new(),
                    callees: Vec::new(),
                    trait_impls: Vec::new(),
                    imports: Vec::new(),
                    updated_at: Instant::now(),
                };
            }
        };

        let all_symbols = extract_symbols(&parsed.tree, source, language);

        let pub_symbols: Vec<(String, SymbolKind)> = all_symbols
            .iter()
            .filter(|s| is_pub_symbol(s, source, language))
            .map(|s| (s.name.clone(), s.kind.clone()))
            .collect();

        let imports: Vec<String> = all_symbols
            .iter()
            .filter(|s| matches!(s.kind, SymbolKind::Import))
            .map(|s| {
                let sig = &s.signature;
                if sig.is_empty() {
                    s.name.clone()
                } else {
                    sig.clone()
                }
            })
            .collect();

        let callees = extract_callees(&parsed.tree, source, language);
        let trait_impls = extract_trait_impls(&parsed.tree, source, language);

        let _ = path;

        FileCodeContext {
            pub_symbols,
            callees,
            trait_impls,
            imports,
            updated_at: Instant::now(),
        }
    }

    pub fn extract_and_store(&self, path: &Path, source: &str, language: &str) {
        let ctx = Self::extract_context(path, source, language);
        self.cache.insert(path.to_path_buf(), ctx);
    }

    pub fn get(&self, path: &Path) -> Option<FileCodeContext> {
        self.cache.get(path).map(|r| r.value().clone())
    }

    /// Render the N most recently accessed files into a compact text block
    /// that fits within `max_tokens` (approximated at ~4 chars per token).
    pub fn format_for_prompt(&self, max_tokens: usize) -> Option<String> {
        let max_chars = max_tokens * 4;

        let mut entries: Vec<(PathBuf, FileCodeContext)> = self
            .cache
            .iter()
            .map(|r| (r.key().clone(), r.value().clone()))
            .collect();

        if entries.is_empty() {
            return None;
        }

        entries.sort_by(|a, b| b.1.updated_at.cmp(&a.1.updated_at));

        let now = Instant::now();
        let mut parts = vec!["<code_context>".to_string()];
        let mut char_budget = max_chars.saturating_sub(30);

        for (path, ctx) in entries.iter().take(8) {
            let age = now.duration_since(ctx.updated_at);
            let age_label = if age.as_secs() < 10 {
                "just read".to_string()
            } else if age.as_secs() < 120 {
                format!("read ~{}s ago", age.as_secs())
            } else {
                format!("read ~{}m ago", age.as_secs() / 60)
            };

            let path_str = path
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string());

            let mut section = format!("## {} ({})\n", path_str, age_label);

            if !ctx.pub_symbols.is_empty() {
                let syms: Vec<String> = ctx
                    .pub_symbols
                    .iter()
                    .take(15)
                    .map(|(name, kind)| format!("{name}({kind})"))
                    .collect();
                section.push_str(&format!("Exports: {}\n", syms.join(", ")));
            }

            if !ctx.callees.is_empty() {
                let calls: Vec<&str> = ctx.callees.iter().take(15).map(String::as_str).collect();
                section.push_str(&format!("Calls: {}\n", calls.join(", ")));
            }

            if !ctx.trait_impls.is_empty() {
                let impls: Vec<String> = ctx
                    .trait_impls
                    .iter()
                    .take(10)
                    .map(|(ty, tr)| format!("{ty}: {tr}"))
                    .collect();
                section.push_str(&format!("Impls: {}\n", impls.join(", ")));
            }

            if section.len() > char_budget {
                break;
            }
            char_budget = char_budget.saturating_sub(section.len());
            parts.push(section);
        }

        parts.push("</code_context>".to_string());

        let result = parts.join("\n");
        if result.lines().count() <= 2 {
            return None;
        }

        Some(result)
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

fn is_pub_symbol(sym: &fastclaw_treesitter::Symbol, source: &str, language: &str) -> bool {
    if matches!(sym.kind, SymbolKind::Import) {
        return false;
    }

    match language {
        "rust" => {
            let lines: Vec<&str> = source.lines().collect();
            if sym.start_line > 0 && sym.start_line <= lines.len() {
                let line = lines[sym.start_line - 1];
                line.trim_start().starts_with("pub ") || line.trim_start().starts_with("pub(")
            } else {
                false
            }
        }
        "python" => !sym.name.starts_with('_'),
        "javascript" | "typescript" | "tsx" | "jsx" => {
            let lines: Vec<&str> = source.lines().collect();
            if sym.start_line > 0 && sym.start_line <= lines.len() {
                let line = lines[sym.start_line - 1];
                line.contains("export ")
            } else {
                false
            }
        }
        "go" => sym
            .name
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_rust_code_context() {
        if !CodeParser::is_language_available("rust") {
            return;
        }
        let source = r#"
use std::io;

pub struct Config {
    pub name: String,
}

pub fn process(input: &str) -> String {
    let result = input.to_uppercase();
    println!("{}", result);
    result
}

fn private_helper() {}

impl Default for Config {
    fn default() -> Self {
        Config { name: String::new() }
    }
}
"#;
        let ctx = CodeGraphCache::extract_context(Path::new("test.rs"), source, "rust");
        let pub_names: Vec<&str> = ctx.pub_symbols.iter().map(|(n, _)| n.as_str()).collect();
        assert!(
            pub_names.contains(&"Config"),
            "missing Config: {pub_names:?}"
        );
        assert!(
            pub_names.contains(&"process"),
            "missing process: {pub_names:?}"
        );
        assert!(
            !pub_names.contains(&"private_helper"),
            "private should be excluded: {pub_names:?}"
        );
        assert!(!ctx.callees.is_empty(), "should have callees");
        assert!(!ctx.trait_impls.is_empty(), "should have trait impls");
    }

    #[test]
    fn cache_stores_and_retrieves() {
        if !CodeParser::is_language_available("rust") {
            return;
        }
        let cache = CodeGraphCache::new();
        let source = "pub fn hello() { println!(\"hi\"); }";
        cache.extract_and_store(Path::new("hello.rs"), source, "rust");

        assert_eq!(cache.len(), 1);
        let ctx = cache.get(Path::new("hello.rs")).unwrap();
        assert!(!ctx.pub_symbols.is_empty());
    }

    #[test]
    fn format_prompt_empty_cache() {
        let cache = CodeGraphCache::new();
        assert!(cache.format_for_prompt(2000).is_none());
    }

    #[test]
    fn format_prompt_has_context() {
        if !CodeParser::is_language_available("rust") {
            return;
        }
        let cache = CodeGraphCache::new();
        let source = r#"
pub fn alpha() { beta(); }
pub fn beta() { gamma(); }
fn gamma() {}
"#;
        cache.extract_and_store(Path::new("funcs.rs"), source, "rust");

        let prompt = cache.format_for_prompt(2000);
        assert!(prompt.is_some());
        let text = prompt.unwrap();
        assert!(text.contains("<code_context>"));
        assert!(text.contains("funcs.rs"));
    }
}
