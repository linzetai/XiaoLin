use std::path::Path;
use std::sync::OnceLock;

use dashmap::DashMap;
use tree_sitter::Tree;

static LANG_CACHE: OnceLock<DashMap<String, bool>> = OnceLock::new();

fn lang_availability() -> &'static DashMap<String, bool> {
    LANG_CACHE.get_or_init(DashMap::new)
}

pub struct ParsedTree {
    pub tree: Tree,
    pub source: String,
    pub language: String,
}

pub struct CodeParser;

impl CodeParser {
    pub fn detect_language(path: &Path) -> Option<String> {
        let ext = path.extension()?.to_str()?;
        let lang = match ext {
            "rs" => "rust",
            "py" | "pyw" => "python",
            "js" | "mjs" | "cjs" => "javascript",
            "ts" | "mts" | "cts" => "typescript",
            "tsx" => "tsx",
            "jsx" => "jsx",
            "go" => "go",
            "java" => "java",
            "c" | "h" => "c",
            "cpp" | "cc" | "cxx" | "hpp" | "hxx" => "cpp",
            "cs" => "c_sharp",
            "rb" => "ruby",
            "php" => "php",
            "swift" => "swift",
            "kt" | "kts" => "kotlin",
            "scala" => "scala",
            "lua" => "lua",
            "r" | "R" => "r",
            "zig" => "zig",
            "toml" => "toml",
            "yaml" | "yml" => "yaml",
            "json" => "json",
            "md" | "markdown" => "markdown",
            "html" | "htm" => "html",
            "css" => "css",
            "scss" => "scss",
            "sql" => "sql",
            "sh" | "bash" | "zsh" => "bash",
            "ps1" => "powershell",
            "dockerfile" | "Dockerfile" => "dockerfile",
            "tf" => "hcl",
            "proto" => "proto",
            "graphql" | "gql" => "graphql",
            "vue" => "vue",
            "svelte" => "svelte",
            "dart" => "dart",
            "ex" | "exs" => "elixir",
            "erl" | "hrl" => "erlang",
            "hs" => "haskell",
            "ml" | "mli" => "ocaml",
            "nim" => "nim",
            _ => return None,
        };
        Some(lang.to_string())
    }

    pub fn is_language_available(lang: &str) -> bool {
        let cache = lang_availability();
        if let Some(entry) = cache.get(lang) {
            return *entry;
        }
        let available = tree_sitter_language_pack::has_language(lang);
        cache.insert(lang.to_string(), available);
        available
    }

    pub fn parse(source: &str, language: &str) -> anyhow::Result<ParsedTree> {
        let mut parser = tree_sitter_language_pack::get_parser(language)
            .map_err(|e| anyhow::anyhow!("failed to get parser for {language}: {e}"))?;

        let tree = parser
            .parse(source, None)
            .ok_or_else(|| anyhow::anyhow!("tree-sitter parse returned None for {language}"))?;

        Ok(ParsedTree {
            tree,
            source: source.to_string(),
            language: language.to_string(),
        })
    }

    pub fn parse_file(path: &Path) -> anyhow::Result<ParsedTree> {
        let lang = Self::detect_language(path)
            .ok_or_else(|| anyhow::anyhow!("unsupported file extension: {}", path.display()))?;

        if !Self::is_language_available(&lang) {
            anyhow::bail!("tree-sitter language '{lang}' not available (may need download)");
        }

        let source = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", path.display()))?;

        Self::parse(&source, &lang)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_language_common_extensions() {
        assert_eq!(CodeParser::detect_language(Path::new("main.rs")), Some("rust".into()));
        assert_eq!(CodeParser::detect_language(Path::new("app.py")), Some("python".into()));
        assert_eq!(CodeParser::detect_language(Path::new("index.ts")), Some("typescript".into()));
        assert_eq!(CodeParser::detect_language(Path::new("main.go")), Some("go".into()));
        assert_eq!(CodeParser::detect_language(Path::new("App.tsx")), Some("tsx".into()));
        assert_eq!(CodeParser::detect_language(Path::new("no_ext")), None);
    }

    #[test]
    fn parse_rust_snippet() {
        if !CodeParser::is_language_available("rust") {
            return; // skip if parser not downloaded
        }
        let result = CodeParser::parse("fn main() { println!(\"hello\"); }", "rust");
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert!(!parsed.tree.root_node().has_error());
    }
}
