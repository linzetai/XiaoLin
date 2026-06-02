//! On-demand documentation injection for agent prompts.
//!
//! Loads Markdown documentation files from `~/.xiaolin/docs/` (or a configured
//! directory), builds a lightweight keyword index from headers, and selects
//! relevant snippets to inject into the system prompt when the agent's query
//! references specific libraries/frameworks.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A single indexed documentation entry.
#[derive(Debug, Clone)]
pub struct DocEntry {
    pub id: String,
    pub title: String,
    #[allow(dead_code)] // TODO(integrate): expose in doc search diagnostics
    pub headers: Vec<String>,
    pub content: String,
    #[allow(dead_code)] // TODO(integrate): expose in doc search diagnostics
    pub source_path: PathBuf,
    #[allow(dead_code)] // TODO(integrate): expose in doc search diagnostics
    pub char_count: usize,
}

/// Index over loaded documentation files.
#[derive(Debug, Clone, Default)]
pub struct DocIndex {
    entries: Vec<DocEntry>,
    keyword_map: HashMap<String, Vec<usize>>,
}

/// Configuration for the magic docs feature.
#[derive(Debug, Clone)]
pub struct MagicDocsConfig {
    pub enabled: bool,
    pub docs_dir: PathBuf,
    #[allow(dead_code)] // TODO(integrate): use as default for select_for_injection
    pub max_injection_chars: usize,
}

impl Default for MagicDocsConfig {
    fn default() -> Self {
        let docs_dir = dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(".xiaolin")
            .join("docs");
        Self {
            enabled: true,
            docs_dir,
            max_injection_chars: 4000,
        }
    }
}

impl DocIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load all .md files from the given directory (non-recursive).
    pub fn load_from_dir(dir: &Path) -> Self {
        let mut index = Self::new();

        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return index,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }

            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            let headers = extract_headers(&content);
            let title = headers.first().cloned().unwrap_or_else(|| id.clone());
            let char_count = content.len();

            index.entries.push(DocEntry {
                id: id.clone(),
                title,
                headers: headers.clone(),
                content,
                source_path: path,
                char_count,
            });

            let entry_idx = index.entries.len() - 1;
            for header in &headers {
                for word in header.split_whitespace() {
                    let kw = word.to_lowercase();
                    if kw.len() >= 3 {
                        index.keyword_map.entry(kw).or_default().push(entry_idx);
                    }
                }
            }
            for word in id.split(['-', '_']) {
                let kw = word.to_lowercase();
                if kw.len() >= 3 {
                    index.keyword_map.entry(kw).or_default().push(entry_idx);
                }
            }
        }

        index
    }

    /// Find relevant doc entries for a query string.
    /// Returns entries sorted by relevance (number of keyword hits).
    pub fn search(&self, query: &str) -> Vec<&DocEntry> {
        if self.entries.is_empty() {
            return Vec::new();
        }

        let query_words: Vec<String> = query
            .split_whitespace()
            .map(|w| w.to_lowercase())
            .filter(|w| w.len() >= 3)
            .collect();

        if query_words.is_empty() {
            return Vec::new();
        }

        let mut scores: HashMap<usize, usize> = HashMap::new();
        for word in &query_words {
            if let Some(indices) = self.keyword_map.get(word) {
                for &idx in indices {
                    *scores.entry(idx).or_default() += 1;
                }
            }
            for (idx, entry) in self.entries.iter().enumerate() {
                let content_lower = entry.content.to_lowercase();
                if content_lower.contains(word.as_str()) {
                    *scores.entry(idx).or_default() += 1;
                }
            }
        }

        let mut ranked: Vec<(usize, usize)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1));

        ranked.iter().map(|(idx, _)| &self.entries[*idx]).collect()
    }

    /// Select and format doc snippets for injection, respecting token limits.
    pub fn select_for_injection(&self, query: &str, max_chars: usize) -> Option<String> {
        let results = self.search(query);
        if results.is_empty() {
            return None;
        }

        let mut output = String::from("## Relevant Documentation\n\n");
        let mut remaining = max_chars.saturating_sub(output.len());

        for entry in results {
            if remaining < 100 {
                break;
            }

            let header = format!("### {} ({})\n\n", entry.title, entry.id);
            if header.len() >= remaining {
                break;
            }
            remaining -= header.len();
            output.push_str(&header);

            let snippet = if entry.content.len() <= remaining {
                entry.content.clone()
            } else {
                let boundary = entry.content[..remaining].rfind('\n').unwrap_or(remaining);
                format!("{}...\n", &entry.content[..boundary])
            };
            remaining = remaining.saturating_sub(snippet.len());
            output.push_str(&snippet);
            output.push_str("\n\n");
        }

        if output.len() > 40 {
            Some(output)
        } else {
            None
        }
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    #[allow(dead_code)] // TODO(integrate): skip injection when index empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn extract_headers(content: &str) -> Vec<String> {
    content
        .lines()
        .filter(|line| line.starts_with('#'))
        .map(|line| line.trim_start_matches('#').trim().to_string())
        .filter(|h| !h.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn load_from_empty_dir_returns_empty_index() {
        let tmp = tempfile::TempDir::new().unwrap();
        let index = DocIndex::load_from_dir(tmp.path());
        assert!(index.is_empty());
        assert_eq!(index.entry_count(), 0);
    }

    #[test]
    fn load_and_search_docs() {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::write(
            tmp.path().join("tokio-guide.md"),
            "# Tokio Runtime\n\n## Spawning Tasks\n\nUse `tokio::spawn` to run async tasks.\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join("serde-tips.md"),
            "# Serde Serialization\n\n## Custom Deserialize\n\nImplement `Deserialize` trait.\n",
        )
        .unwrap();

        let index = DocIndex::load_from_dir(tmp.path());
        assert_eq!(index.entry_count(), 2);

        let results = index.search("tokio spawn");
        assert!(!results.is_empty());
        assert_eq!(results[0].id, "tokio-guide");
    }

    #[test]
    fn select_for_injection_truncates() {
        let tmp = tempfile::TempDir::new().unwrap();
        let long_content = format!("# API Reference\n\n{}\n", "x".repeat(10000));
        fs::write(tmp.path().join("api-ref.md"), &long_content).unwrap();

        let index = DocIndex::load_from_dir(tmp.path());
        let injection = index.select_for_injection("api reference", 500);
        assert!(injection.is_some());
        let text = injection.unwrap();
        assert!(text.len() <= 600);
        assert!(text.contains("API Reference"));
    }

    #[test]
    fn no_match_returns_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::write(
            tmp.path().join("rust-guide.md"),
            "# Rust Basics\n\n## Ownership\n\nRust has ownership rules.\n",
        )
        .unwrap();

        let index = DocIndex::load_from_dir(tmp.path());
        let result = index.select_for_injection("kubernetes helm", 4000);
        assert!(result.is_none());
    }
}
