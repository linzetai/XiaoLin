use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A project-level rule loaded from `.xiaolin/rules/*.md`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub content: String,
    pub source_path: PathBuf,
    #[serde(default)]
    pub frontmatter: RuleFrontmatter,
}

/// YAML frontmatter for rule files. Compatible with Cursor `.cursor/rules/*.mdc` format.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleFrontmatter {
    #[serde(default)]
    pub always_apply: Option<bool>,
    #[serde(default)]
    pub globs: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

/// Collection of loaded rules.
#[derive(Debug, Clone, Default)]
pub struct RuleRegistry {
    rules: Vec<Rule>,
}

impl RuleRegistry {
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    pub fn add(&mut self, rule: Rule) {
        self.rules.push(rule);
    }

    pub fn count(&self) -> usize {
        self.rules.len()
    }

    /// Return rules that should always be injected into the system prompt.
    pub fn always_apply_rules(&self) -> Vec<&Rule> {
        self.rules
            .iter()
            .filter(|r| r.frontmatter.always_apply == Some(true))
            .collect()
    }

    /// Return rules whose glob patterns match the given file path.
    pub fn matching_rules(&self, file_path: &str) -> Vec<&Rule> {
        self.rules
            .iter()
            .filter(|r| {
                if r.frontmatter.globs.is_empty() {
                    return false;
                }
                r.frontmatter.globs.iter().any(|g| glob_match(g, file_path))
            })
            .collect()
    }

    /// Format always-apply rules into a system prompt section.
    pub fn format_always_apply_for_prompt(&self) -> String {
        let rules = self.always_apply_rules();
        if rules.is_empty() {
            return String::new();
        }
        let mut buf = String::from("\n<project_rules>\n");
        for rule in &rules {
            let name = rule.frontmatter.name.as_deref().unwrap_or(&rule.id);
            buf.push_str(&format!(
                "\n<rule name=\"{}\">\n{}\n</rule>\n",
                name, rule.content
            ));
        }
        buf.push_str("</project_rules>\n");
        buf
    }

    pub fn all_rules(&self) -> &[Rule] {
        &self.rules
    }

    pub fn merge_from(&mut self, other: RuleRegistry) {
        self.rules.extend(other.rules);
    }
}

fn glob_match(pattern: &str, path: &str) -> bool {
    let pattern = if !pattern.starts_with("**/") && !pattern.starts_with('/') {
        format!("**/{pattern}")
    } else {
        pattern.to_string()
    };
    glob::Pattern::new(&pattern)
        .map(|p| p.matches(path))
        .unwrap_or(false)
}

/// Parse a rule file (Markdown with optional YAML frontmatter).
fn parse_rule_file(path: &Path) -> anyhow::Result<Rule> {
    let raw = std::fs::read_to_string(path)?;
    let (frontmatter, content) = parse_frontmatter(&raw);

    let id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    Ok(Rule {
        id,
        content,
        source_path: path.to_path_buf(),
        frontmatter,
    })
}

fn parse_frontmatter(raw: &str) -> (RuleFrontmatter, String) {
    if !raw.starts_with("---") {
        return (RuleFrontmatter::default(), raw.to_string());
    }
    if let Some(end) = raw[3..].find("---") {
        let yaml_str = &raw[3..3 + end];
        let rest = &raw[3 + end + 3..];
        match serde_yaml_ng::from_str::<RuleFrontmatter>(yaml_str.trim()) {
            Ok(fm) => (fm, rest.trim_start().to_string()),
            Err(e) => {
                tracing::warn!(error = %e, "invalid YAML frontmatter in rule file, falling back to defaults");
                (RuleFrontmatter::default(), raw.to_string())
            }
        }
    } else {
        (RuleFrontmatter::default(), raw.to_string())
    }
}

/// Load all rules from a directory (`.xiaolin/rules/` or `.cursor/rules/`).
pub fn load_rules_from_dir(dir: &Path) -> RuleRegistry {
    let mut registry = RuleRegistry::new();
    if !dir.exists() || !dir.is_dir() {
        return registry;
    }
    match std::fs::read_dir(dir) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if ext != "md" && ext != "mdc" {
                    continue;
                }
                match parse_rule_file(&path) {
                    Ok(rule) => {
                        tracing::debug!(
                            rule_id = %rule.id,
                            always_apply = ?rule.frontmatter.always_apply,
                            globs = ?rule.frontmatter.globs,
                            path = %path.display(),
                            "loaded project rule"
                        );
                        registry.add(rule);
                    }
                    Err(e) => {
                        tracing::warn!(
                            path = %path.display(),
                            error = %e,
                            "failed to parse rule file"
                        );
                    }
                }
            }
        }
        Err(e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "failed to read rules dir");
        }
    }
    if registry.count() > 0 {
        tracing::info!(
            count = registry.count(),
            always_apply = registry.always_apply_rules().len(),
            dir = %dir.display(),
            "loaded project rules"
        );
    }
    registry
}

/// Load rules from the workspace root, scanning both `.xiaolin/rules/` and `.cursor/rules/`.
pub fn load_project_rules(workspace_root: &Path) -> RuleRegistry {
    let mut registry = RuleRegistry::new();
    let xiaolin_rules = load_rules_from_dir(&workspace_root.join(".xiaolin/rules"));
    let cursor_rules = load_rules_from_dir(&workspace_root.join(".cursor/rules"));
    registry.merge_from(cursor_rules);
    registry.merge_from(xiaolin_rules);
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_frontmatter_with_always_apply() {
        let raw = "---\nalwaysApply: true\nname: test-rule\n---\n# My Rule\nDo this.";
        let (fm, content) = parse_frontmatter(raw);
        assert_eq!(fm.always_apply, Some(true));
        assert_eq!(fm.name.as_deref(), Some("test-rule"));
        assert!(content.contains("# My Rule"));
    }

    #[test]
    fn parse_frontmatter_without() {
        let raw = "# Just content\nNo frontmatter here.";
        let (fm, content) = parse_frontmatter(raw);
        assert!(fm.always_apply.is_none());
        assert!(content.contains("# Just content"));
    }

    #[test]
    fn glob_match_basic() {
        assert!(glob_match("*.rs", "src/main.rs"));
        assert!(glob_match("**/*.ts", "lib/stores/session-store.ts"));
        assert!(!glob_match("*.py", "main.rs"));
    }
}
