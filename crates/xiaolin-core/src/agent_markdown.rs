use std::path::Path;

use serde::Deserialize;

use crate::agent_config::{
    PermissionMode, SubAgentDef, SubAgentDefSource, SubAgentMode, SubAgentToolFilter,
};

/// Frontmatter fields for a markdown agent definition.
/// All fields except `id` are optional and fall back to SubAgentDef defaults.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentFrontmatter {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    background: bool,
    #[serde(default)]
    concurrency_safe: bool,
    #[serde(default = "default_max_context_messages")]
    max_context_messages: usize,
    #[serde(default)]
    permission_mode: PermissionMode,
    #[serde(default)]
    mode: SubAgentMode,
    #[serde(default)]
    tools: ToolsSection,
}

fn default_max_context_messages() -> usize {
    20
}

#[derive(Debug, Default, Deserialize)]
struct ToolsSection {
    #[serde(default)]
    allowed: Vec<String>,
    #[serde(default)]
    denied: Vec<String>,
    #[serde(default)]
    profile: Option<String>,
}

/// Parse a markdown agent definition from file content.
///
/// Expected format:
/// ```text
/// ---
/// id: my-agent
/// name: My Agent
/// description: Does something
/// background: false
/// tools:
///   allowed: [read_file, grep]
///   denied: [write_file]
/// ---
/// You are a custom agent that...
/// ```
///
/// The body (everything after the closing `---`) becomes the `system_prompt`.
pub fn parse_agent_markdown(content: &str, path: &Path) -> Result<SubAgentDef, String> {
    let (frontmatter_str, body) = split_frontmatter(content).ok_or_else(|| {
        format!(
            "missing YAML frontmatter delimiters (---) in {}",
            path.display()
        )
    })?;

    let fm: AgentFrontmatter = serde_yaml_ng::from_str(frontmatter_str)
        .map_err(|e| format!("invalid frontmatter in {}: {e}", path.display()))?;

    if fm.id.is_empty() {
        return Err(format!(
            "frontmatter `id` must be non-empty in {}",
            path.display()
        ));
    }

    let system_prompt = {
        let trimmed = body.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    };

    Ok(SubAgentDef {
        id: fm.id,
        name: fm.name,
        description: fm.description,
        model: None,
        tools: SubAgentToolFilter {
            allowed: fm.tools.allowed,
            denied: fm.tools.denied,
            profile: fm.tools.profile,
        },
        system_prompt,
        background: fm.background,
        concurrency_safe: fm.concurrency_safe,
        max_context_messages: fm.max_context_messages,
        max_result_chars: None,
        permission_mode: fm.permission_mode,
        mode: fm.mode,
        source: SubAgentDefSource::MarkdownFile(path.to_path_buf()),
    })
}

/// Load all valid markdown agent definitions from a directory.
/// Invalid files are skipped with a warning log.
pub fn load_agents_from_dir(dir: &Path) -> Vec<SubAgentDef> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut defs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "md" && ext != "markdown" {
            continue;
        }

        match std::fs::read_to_string(&path) {
            Ok(content) => match parse_agent_markdown(&content, &path) {
                Ok(def) => defs.push(def),
                Err(e) => {
                    tracing::warn!("skipping invalid agent file {}: {e}", path.display());
                }
            },
            Err(e) => {
                tracing::warn!("failed to read agent file {}: {e}", path.display());
            }
        }
    }

    defs
}

/// Split content into (frontmatter, body) using `---` delimiters.
fn split_frontmatter(content: &str) -> Option<(&str, &str)> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }

    let after_open = &trimmed[3..];
    let after_open = after_open.strip_prefix('\n').unwrap_or(after_open);

    let close_pos = after_open.find("\n---")?;
    let frontmatter = &after_open[..close_pos];
    let body = &after_open[close_pos + 4..];
    let body = body.strip_prefix('\n').unwrap_or(body);

    Some((frontmatter, body))
}

/// Merge multiple definition sources, later entries override earlier ones by `id`.
/// Preserves insertion order: first occurrence determines position.
pub fn merge_subagent_defs(layers: Vec<Vec<SubAgentDef>>) -> Vec<SubAgentDef> {
    let mut result: Vec<SubAgentDef> = Vec::new();
    for layer in layers {
        for def in layer {
            if let Some(existing) = result.iter_mut().find(|d| d.id == def.id) {
                *existing = def;
            } else {
                result.push(def);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parse_basic_agent_markdown() {
        let content = r#"---
id: test-agent
name: Test Agent
description: A test agent
background: true
concurrency_safe: true
tools:
  allowed:
    - read_file
    - grep
  denied:
    - write_file
---
You are a test agent. Follow instructions carefully.
"#;
        let path = PathBuf::from("test.md");
        let def = parse_agent_markdown(content, &path).unwrap();
        assert_eq!(def.id, "test-agent");
        assert_eq!(def.name.as_deref(), Some("Test Agent"));
        assert_eq!(def.description.as_deref(), Some("A test agent"));
        assert!(def.background);
        assert!(def.concurrency_safe);
        assert_eq!(def.tools.allowed, vec!["read_file", "grep"]);
        assert_eq!(def.tools.denied, vec!["write_file"]);
        assert_eq!(
            def.system_prompt.as_deref(),
            Some("You are a test agent. Follow instructions carefully.")
        );
        assert!(matches!(def.source, SubAgentDefSource::MarkdownFile(_)));
    }

    #[test]
    fn parse_minimal_agent_markdown() {
        let content = "---\nid: minimal\n---\n";
        let path = PathBuf::from("minimal.md");
        let def = parse_agent_markdown(content, &path).unwrap();
        assert_eq!(def.id, "minimal");
        assert_eq!(def.name, None);
        assert!(!def.background);
        assert_eq!(def.system_prompt, None);
    }

    #[test]
    fn parse_missing_frontmatter_fails() {
        let content = "No frontmatter here\nJust text.";
        let path = PathBuf::from("bad.md");
        let result = parse_agent_markdown(content, &path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing YAML frontmatter"));
    }

    #[test]
    fn parse_empty_id_fails() {
        let content = "---\nid: \"\"\n---\n";
        let path = PathBuf::from("empty-id.md");
        let result = parse_agent_markdown(content, &path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("non-empty"));
    }

    #[test]
    fn parse_unknown_field_fails() {
        let content = "---\nid: test\nunknown_field: true\n---\n";
        let path = PathBuf::from("unknown.md");
        let result = parse_agent_markdown(content, &path);
        assert!(result.is_err());
    }

    #[test]
    fn merge_layers_override_by_id() {
        let builtin = vec![SubAgentDef {
            id: "explore".into(),
            name: Some("Explorer".into()),
            description: Some("original".into()),
            ..Default::default()
        }];
        let user = vec![SubAgentDef {
            id: "explore".into(),
            name: Some("My Explorer".into()),
            description: Some("custom".into()),
            ..Default::default()
        }];
        let merged = merge_subagent_defs(vec![builtin, user]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].name.as_deref(), Some("My Explorer"));
        assert_eq!(merged[0].description.as_deref(), Some("custom"));
    }

    #[test]
    fn merge_layers_preserve_order() {
        let builtin = vec![
            SubAgentDef {
                id: "a".into(),
                ..Default::default()
            },
            SubAgentDef {
                id: "b".into(),
                ..Default::default()
            },
        ];
        let user = vec![SubAgentDef {
            id: "c".into(),
            ..Default::default()
        }];
        let merged = merge_subagent_defs(vec![builtin, user]);
        let ids: Vec<&str> = merged.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }
}
