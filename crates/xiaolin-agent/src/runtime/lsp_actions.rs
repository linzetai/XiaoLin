//! LSP-driven deterministic code actions.
//!
//! Wraps Language Server Protocol code actions (rename, extract, organize
//! imports, etc.) so the LLM can invoke them declaratively without doing
//! manual text editing. The LSP server handles all mechanical details
//! (line offsets, import updates, cross-file references) deterministically.
//!
//! This eliminates an entire class of errors where LLMs miscalculate line
//! numbers, forget to update imports, or produce malformed edits.

use std::path::Path;

/// Supported deterministic code actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeAction {
    /// Rename a symbol across the entire workspace.
    Rename {
        file: String,
        line: u32,
        column: u32,
        new_name: String,
    },
    /// Get diagnostics for a file (post-edit validation).
    GetDiagnostics { file: String },
    /// Find all references to a symbol.
    FindReferences {
        file: String,
        line: u32,
        column: u32,
    },
    /// Go to definition of a symbol.
    GoToDefinition {
        file: String,
        line: u32,
        column: u32,
    },
    /// Request code actions available at a position (quick fixes, refactors).
    AvailableActions {
        file: String,
        line: u32,
        column: u32,
    },
    /// Organize imports / auto-import.
    OrganizeImports { file: String },
    /// Format a file using the LSP formatter.
    Format { file: String },
}

/// A text edit produced by an LSP action.
#[derive(Debug, Clone)]
pub struct TextEdit {
    pub file: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub new_text: String,
}

/// Result of executing a code action.
#[derive(Debug, Clone)]
pub enum ActionResult {
    /// Edits to apply (rename, format, organize imports).
    Edits(Vec<TextEdit>),
    /// Locations found (references, definition).
    Locations(Vec<Location>),
    /// Available actions at a position.
    Actions(Vec<ActionItem>),
    /// Diagnostics for a file.
    Diagnostics(Vec<Diagnostic>),
    /// The action is not supported for this file/language.
    NotSupported(String),
    /// LSP server error.
    Error(String),
}

/// A source location.
#[derive(Debug, Clone)]
pub struct Location {
    pub file: String,
    pub line: u32,
    pub column: u32,
}

/// A code action item available at a position.
#[derive(Debug, Clone)]
pub struct ActionItem {
    pub title: String,
    pub kind: Option<String>,
    pub is_preferred: bool,
}

/// A diagnostic (error/warning) from the LSP.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

/// Configuration for LSP actions.
#[derive(Debug, Clone)]
pub struct LspActionsConfig {
    pub enabled: bool,
    /// Timeout for LSP requests in milliseconds.
    pub timeout_ms: u64,
    /// Whether to auto-format after rename/refactor.
    pub format_after_action: bool,
}

impl Default for LspActionsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            timeout_ms: 10_000,
            format_after_action: true,
        }
    }
}

/// Determine if a file has LSP support based on extension.
pub fn has_lsp_support(file_path: &Path) -> bool {
    let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    matches!(
        ext,
        "rs" | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "py"
            | "go"
            | "java"
            | "kt"
            | "c"
            | "cpp"
            | "h"
            | "hpp"
            | "cs"
            | "rb"
            | "swift"
            | "vue"
            | "svelte"
    )
}

/// Determine the LSP server command for a given file extension.
pub fn lsp_server_for_extension(ext: &str) -> Option<&'static str> {
    match ext {
        "rs" => Some("rust-analyzer"),
        "ts" | "tsx" | "js" | "jsx" => Some("typescript-language-server"),
        "py" => Some("pyright-langserver"),
        "go" => Some("gopls"),
        "java" | "kt" => Some("jdtls"),
        "c" | "cpp" | "h" | "hpp" => Some("clangd"),
        "cs" => Some("omnisharp"),
        "rb" => Some("solargraph"),
        "swift" => Some("sourcekit-lsp"),
        "vue" => Some("vls"),
        "svelte" => Some("svelte-language-server"),
        _ => None,
    }
}

/// Format a code action as a tool-call-like instruction for the LLM.
///
/// When the LLM wants to perform a refactoring, it can express it as a
/// CodeAction, and the harness will execute it via LSP without the LLM
/// needing to compute exact text edits.
pub fn format_action_result_for_prompt(action: &CodeAction, result: &ActionResult) -> String {
    match result {
        ActionResult::Edits(edits) => {
            let mut out = String::new();
            out.push_str(&format!(
                "✅ {} completed — {} file(s) modified:\n",
                action_name(action),
                count_unique_files(edits)
            ));
            for edit in edits.iter().take(10) {
                out.push_str(&format!(
                    "  {}:{}:{} → {}\n",
                    edit.file,
                    edit.start_line,
                    edit.start_col,
                    truncate(&edit.new_text, 60)
                ));
            }
            if edits.len() > 10 {
                out.push_str(&format!("  ... and {} more edits\n", edits.len() - 10));
            }
            out
        }
        ActionResult::Locations(locs) => {
            let mut out = format!("Found {} reference(s):\n", locs.len());
            for loc in locs.iter().take(15) {
                out.push_str(&format!("  {}:{}:{}\n", loc.file, loc.line, loc.column));
            }
            if locs.len() > 15 {
                out.push_str(&format!("  ... and {} more\n", locs.len() - 15));
            }
            out
        }
        ActionResult::Actions(items) => {
            let mut out = format!("Available code actions ({}):\n", items.len());
            for item in items.iter().take(10) {
                let preferred = if item.is_preferred {
                    " [preferred]"
                } else {
                    ""
                };
                let kind = item.kind.as_deref().unwrap_or("");
                out.push_str(&format!("  • {} ({}){}\n", item.title, kind, preferred));
            }
            out
        }
        ActionResult::Diagnostics(diags) => {
            let errors = diags
                .iter()
                .filter(|d| d.severity == DiagnosticSeverity::Error)
                .count();
            let warnings = diags
                .iter()
                .filter(|d| d.severity == DiagnosticSeverity::Warning)
                .count();
            let mut out = format!(
                "Diagnostics: {} error(s), {} warning(s)\n",
                errors, warnings
            );
            for diag in diags.iter().take(10) {
                let icon = match diag.severity {
                    DiagnosticSeverity::Error => "❌",
                    DiagnosticSeverity::Warning => "⚠️",
                    DiagnosticSeverity::Information => "ℹ️",
                    DiagnosticSeverity::Hint => "💡",
                };
                out.push_str(&format!(
                    "  {} {}:{} — {}\n",
                    icon, diag.line, diag.column, diag.message
                ));
            }
            out
        }
        ActionResult::NotSupported(reason) => {
            format!("⚠️ Action not supported: {}\n", reason)
        }
        ActionResult::Error(err) => {
            format!("❌ LSP error: {}\n", err)
        }
    }
}

fn action_name(action: &CodeAction) -> &str {
    match action {
        CodeAction::Rename { .. } => "Rename",
        CodeAction::GetDiagnostics { .. } => "Get diagnostics",
        CodeAction::FindReferences { .. } => "Find references",
        CodeAction::GoToDefinition { .. } => "Go to definition",
        CodeAction::AvailableActions { .. } => "Available actions",
        CodeAction::OrganizeImports { .. } => "Organize imports",
        CodeAction::Format { .. } => "Format",
    }
}

fn count_unique_files(edits: &[TextEdit]) -> usize {
    let mut seen = std::collections::HashSet::new();
    for edit in edits {
        seen.insert(&edit.file);
    }
    seen.len()
}

fn truncate(s: &str, max: usize) -> String {
    let first_line = s.lines().next().unwrap_or(s);
    if first_line.len() <= max {
        first_line.to_string()
    } else {
        format!("{}...", &first_line[..max - 3])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn has_lsp_support_common_extensions() {
        assert!(has_lsp_support(&PathBuf::from("main.rs")));
        assert!(has_lsp_support(&PathBuf::from("app.tsx")));
        assert!(has_lsp_support(&PathBuf::from("utils.py")));
        assert!(has_lsp_support(&PathBuf::from("server.go")));
        assert!(!has_lsp_support(&PathBuf::from("readme.md")));
        assert!(!has_lsp_support(&PathBuf::from("data.csv")));
    }

    #[test]
    fn lsp_server_for_common_extensions() {
        assert_eq!(lsp_server_for_extension("rs"), Some("rust-analyzer"));
        assert_eq!(
            lsp_server_for_extension("ts"),
            Some("typescript-language-server")
        );
        assert_eq!(lsp_server_for_extension("py"), Some("pyright-langserver"));
        assert_eq!(lsp_server_for_extension("go"), Some("gopls"));
        assert_eq!(lsp_server_for_extension("md"), None);
    }

    #[test]
    fn format_rename_result() {
        let action = CodeAction::Rename {
            file: "src/main.rs".into(),
            line: 10,
            column: 5,
            new_name: "new_name".into(),
        };
        let result = ActionResult::Edits(vec![
            TextEdit {
                file: "src/main.rs".into(),
                start_line: 10,
                start_col: 5,
                end_line: 10,
                end_col: 13,
                new_text: "new_name".into(),
            },
            TextEdit {
                file: "src/lib.rs".into(),
                start_line: 25,
                start_col: 10,
                end_line: 25,
                end_col: 18,
                new_text: "new_name".into(),
            },
        ]);

        let formatted = format_action_result_for_prompt(&action, &result);
        assert!(formatted.contains("Rename completed"));
        assert!(formatted.contains("2 file(s) modified"));
        assert!(formatted.contains("src/main.rs"));
        assert!(formatted.contains("src/lib.rs"));
    }

    #[test]
    fn format_diagnostics_result() {
        let action = CodeAction::GetDiagnostics {
            file: "src/main.rs".into(),
        };
        let result = ActionResult::Diagnostics(vec![
            Diagnostic {
                file: "src/main.rs".into(),
                line: 42,
                column: 5,
                severity: DiagnosticSeverity::Error,
                message: "type mismatch".into(),
                code: Some("E0308".into()),
            },
            Diagnostic {
                file: "src/main.rs".into(),
                line: 55,
                column: 1,
                severity: DiagnosticSeverity::Warning,
                message: "unused variable".into(),
                code: None,
            },
        ]);

        let formatted = format_action_result_for_prompt(&action, &result);
        assert!(formatted.contains("1 error(s), 1 warning(s)"));
        assert!(formatted.contains("type mismatch"));
        assert!(formatted.contains("unused variable"));
    }

    #[test]
    fn format_not_supported() {
        let action = CodeAction::Rename {
            file: "readme.md".into(),
            line: 1,
            column: 1,
            new_name: "x".into(),
        };
        let result = ActionResult::NotSupported("No LSP server for .md files".into());
        let formatted = format_action_result_for_prompt(&action, &result);
        assert!(formatted.contains("not supported"));
    }

    #[test]
    fn count_unique_files_deduplicates() {
        let edits = vec![
            TextEdit {
                file: "a.rs".into(),
                start_line: 1,
                start_col: 0,
                end_line: 1,
                end_col: 5,
                new_text: "x".into(),
            },
            TextEdit {
                file: "a.rs".into(),
                start_line: 2,
                start_col: 0,
                end_line: 2,
                end_col: 5,
                new_text: "y".into(),
            },
            TextEdit {
                file: "b.rs".into(),
                start_line: 1,
                start_col: 0,
                end_line: 1,
                end_col: 5,
                new_text: "z".into(),
            },
        ];
        assert_eq!(count_unique_files(&edits), 2);
    }
}
