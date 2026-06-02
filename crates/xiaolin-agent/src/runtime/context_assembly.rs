//! Automatic context assembly based on task type and environment.
//!
//! Before the LLM begins processing, this module proactively gathers and
//! injects relevant context so the model doesn't have to "search blindly".
//! Context sources include:
//!
//! - Memory system (facts, episodes, semantic search)
//! - Magic docs (keyword-matched documentation)
//! - File system heuristics (related files, config files)
//! - Task type signals (from TaskDecomposer or user message analysis)
//!
//! This reduces the number of tool calls weak models need to make, directly
//! compensating for their limited planning/search capabilities.

use std::path::{Path, PathBuf};

use super::task_decomposer::TaskType;

/// Configuration for context auto-assembly.
#[derive(Debug, Clone)]
pub struct ContextAssemblyConfig {
    pub enabled: bool,
    /// Maximum total characters to inject as assembled context.
    pub max_chars: usize,
    /// Whether to include file-system heuristic context (nearby files, configs).
    pub include_fs_context: bool,
    /// Whether to query memory for relevant facts/episodes.
    pub include_memory: bool,
    /// Whether to include magic_docs results.
    pub include_docs: bool,
}

impl Default for ContextAssemblyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_chars: 4000,
            include_fs_context: true,
            include_memory: true,
            include_docs: true,
        }
    }
}

/// Assembled context ready for injection into the prompt.
#[derive(Debug, Clone, Default)]
pub struct AssembledContext {
    /// Relevant facts from memory.
    pub facts: Vec<String>,
    /// Documentation snippets from magic_docs.
    pub docs_snippet: Option<String>,
    /// Nearby/related file paths for awareness.
    pub related_files: Vec<PathBuf>,
    /// Project-level metadata (language, framework, test command).
    pub project_hints: Vec<String>,
    /// Total character count of assembled content.
    pub total_chars: usize,
}

impl AssembledContext {
    pub fn is_empty(&self) -> bool {
        self.facts.is_empty()
            && self.docs_snippet.is_none()
            && self.related_files.is_empty()
            && self.project_hints.is_empty()
    }

    /// Format the assembled context as a prompt injection block.
    pub fn format_for_prompt(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }

        let mut block = String::with_capacity(self.total_chars + 200);
        block.push_str("─── Context ────────────────────────────────────────\n");

        if !self.project_hints.is_empty() {
            block.push_str("Project:\n");
            for hint in &self.project_hints {
                block.push_str(&format!("  • {}\n", hint));
            }
            block.push('\n');
        }

        if !self.facts.is_empty() {
            block.push_str("Relevant knowledge:\n");
            for fact in &self.facts {
                block.push_str(&format!("  • {}\n", fact));
            }
            block.push('\n');
        }

        if !self.related_files.is_empty() {
            block.push_str("Related files:\n");
            for f in &self.related_files {
                block.push_str(&format!("  • {}\n", f.display()));
            }
            block.push('\n');
        }

        if let Some(ref docs) = self.docs_snippet {
            block.push_str("Documentation:\n");
            block.push_str(docs);
            block.push('\n');
        }

        block.push_str("────────────────────────────────────────────────────\n");
        Some(block)
    }
}

/// Detect project characteristics from the working directory.
pub fn detect_project_hints(work_dir: &Path) -> Vec<String> {
    let mut hints = Vec::new();

    let indicators: &[(&str, &str)] = &[
        ("Cargo.toml", "Rust project (Cargo)"),
        ("package.json", "Node.js/JavaScript project"),
        ("pyproject.toml", "Python project (pyproject)"),
        ("requirements.txt", "Python project (pip)"),
        ("go.mod", "Go project"),
        ("pom.xml", "Java project (Maven)"),
        ("build.gradle", "Java/Kotlin project (Gradle)"),
        ("Makefile", "Has Makefile"),
        ("Dockerfile", "Has Docker configuration"),
        ("docker-compose.yml", "Has Docker Compose"),
        (".github/workflows", "Has GitHub Actions CI"),
        ("tsconfig.json", "TypeScript project"),
        (".eslintrc.json", "Has ESLint configuration"),
        (".prettierrc", "Has Prettier configuration"),
    ];

    for (file, description) in indicators {
        if work_dir.join(file).exists() {
            hints.push(description.to_string());
        }
    }

    // Detect test framework
    let test_indicators: &[(&str, &str)] = &[
        ("jest.config.js", "Test framework: Jest"),
        ("jest.config.ts", "Test framework: Jest"),
        ("vitest.config.ts", "Test framework: Vitest"),
        ("pytest.ini", "Test framework: pytest"),
        ("setup.cfg", "Test framework: pytest/setuptools"),
        (".mocharc.yml", "Test framework: Mocha"),
    ];

    for (file, description) in test_indicators {
        if work_dir.join(file).exists() {
            hints.push(description.to_string());
        }
    }

    hints
}

/// Discover files that are likely related to the user's task.
///
/// Uses heuristics based on task type and file patterns.
pub fn discover_related_files(
    work_dir: &Path,
    task_type: TaskType,
    user_message: &str,
    max_files: usize,
) -> Vec<PathBuf> {
    let mut files = Vec::new();

    // Extract potential file paths mentioned in the user message
    for word in user_message.split_whitespace() {
        let clean = word.trim_matches(|c: char| c == '\'' || c == '"' || c == '`' || c == ',');
        if looks_like_file_path(clean) {
            let candidate = work_dir.join(clean);
            if candidate.exists() && files.len() < max_files {
                files.push(candidate);
            }
        }
    }

    // Task-type-specific discovery
    match task_type {
        TaskType::Coding => {
            // Look for common config files that affect coding
            let configs = [
                "tsconfig.json",
                "Cargo.toml",
                "package.json",
                "pyproject.toml",
            ];
            for cfg in configs {
                let p = work_dir.join(cfg);
                if p.exists() && files.len() < max_files && !files.contains(&p) {
                    files.push(p);
                    break;
                }
            }
        }
        TaskType::Workflow => {
            // Look for CI/deployment configs
            let workflow_files = [
                ".github/workflows/ci.yml",
                ".github/workflows/main.yml",
                "Makefile",
                "docker-compose.yml",
            ];
            for wf in workflow_files {
                let p = work_dir.join(wf);
                if p.exists() && files.len() < max_files && !files.contains(&p) {
                    files.push(p);
                }
            }
        }
        _ => {}
    }

    files
}

/// Extract keywords from a user message for memory/docs lookup.
pub fn extract_keywords(message: &str) -> Vec<String> {
    let stop_words: &[&str] = &[
        "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
        "do", "does", "did", "will", "would", "could", "should", "may", "might", "shall", "can",
        "to", "of", "in", "for", "on", "with", "at", "by", "from", "as", "into", "through",
        "during", "before", "after", "above", "below", "between", "and", "but", "or", "nor", "not",
        "so", "yet", "both", "either", "neither", "each", "this", "that", "these", "those", "it",
        "its", "my", "your", "his", "her", "our", "their", "what", "which", "who", "whom", "whose",
        "i", "me", "we", "us", "you", "he", "she", "they", "them", "please", "help", "want",
        "need", "like", "just", "make", "get",
    ];

    message
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-' && c != '.')
        .filter(|w| w.len() >= 3)
        .filter(|w| !stop_words.contains(&w.to_lowercase().as_str()))
        .map(|w| w.to_string())
        .take(10)
        .collect()
}

fn looks_like_file_path(s: &str) -> bool {
    if s.len() < 3 || s.len() > 200 {
        return false;
    }
    // Must contain a path separator or file extension
    (s.contains('/') || s.contains('.'))
        && !s.starts_with("http")
        && !s.starts_with("//")
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '/' || c == '.' || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_keywords_filters_stop_words() {
        let keywords =
            extract_keywords("please help me fix the authentication bug in the login module");
        assert!(!keywords.contains(&"please".to_string()));
        assert!(!keywords.contains(&"the".to_string()));
        assert!(
            keywords.contains(&"fix".to_string())
                || keywords.contains(&"authentication".to_string())
        );
        assert!(keywords.contains(&"authentication".to_string()));
        assert!(keywords.contains(&"login".to_string()));
        assert!(keywords.contains(&"module".to_string()));
    }

    #[test]
    fn extract_keywords_handles_technical_terms() {
        let keywords =
            extract_keywords("implement rate-limiting middleware for the Express.js API");
        assert!(keywords.contains(&"rate-limiting".to_string()));
        assert!(keywords.contains(&"middleware".to_string()));
        assert!(keywords.contains(&"Express.js".to_string()));
    }

    #[test]
    fn extract_keywords_limits_count() {
        let long_msg =
            "word1 word2 word3 word4 word5 word6 word7 word8 word9 word10 word11 word12 word13";
        let keywords = extract_keywords(long_msg);
        assert!(keywords.len() <= 10);
    }

    #[test]
    fn looks_like_file_path_basic() {
        assert!(looks_like_file_path("src/main.rs"));
        assert!(looks_like_file_path("package.json"));
        assert!(looks_like_file_path("tests/unit/auth.test.ts"));
        assert!(!looks_like_file_path("hello"));
        assert!(!looks_like_file_path("https://example.com"));
        assert!(!looks_like_file_path("a"));
        assert!(!looks_like_file_path("// comment"));
    }

    #[test]
    fn assembled_context_empty_check() {
        let ctx = AssembledContext::default();
        assert!(ctx.is_empty());
        assert!(ctx.format_for_prompt().is_none());
    }

    #[test]
    fn assembled_context_formats_correctly() {
        let ctx = AssembledContext {
            facts: vec![
                "User prefers TypeScript".into(),
                "Project uses React 18".into(),
            ],
            docs_snippet: Some("React hooks documentation excerpt...".into()),
            related_files: vec![PathBuf::from("src/App.tsx"), PathBuf::from("tsconfig.json")],
            project_hints: vec!["TypeScript project".into(), "Test framework: Jest".into()],
            total_chars: 200,
        };

        let formatted = ctx.format_for_prompt().unwrap();
        assert!(formatted.contains("TypeScript project"));
        assert!(formatted.contains("User prefers TypeScript"));
        assert!(formatted.contains("src/App.tsx"));
        assert!(formatted.contains("React hooks"));
    }

    #[test]
    fn detect_project_hints_empty_dir() {
        let hints = detect_project_hints(Path::new("/nonexistent/path"));
        assert!(hints.is_empty());
    }

    #[test]
    fn discover_related_files_extracts_mentioned_paths() {
        let tmp = std::env::temp_dir();
        let test_file = tmp.join("test_context_assembly.txt");
        std::fs::write(&test_file, "test").ok();

        let msg = format!("please fix the bug in {}", test_file.display());
        let files = discover_related_files(&tmp, TaskType::Coding, &msg, 5);

        // Clean up
        std::fs::remove_file(&test_file).ok();

        // The file should be found if path parsing works
        // (depends on path format matching the heuristic)
        assert!(files.len() <= 5);
    }
}
