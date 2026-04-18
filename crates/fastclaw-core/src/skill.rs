use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Where a skill was loaded from (lower ordinal = lower priority).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SkillLayer {
    Extension = 0,
    Project = 1,
    Global = 2,
    AgentWorkspace = 3,
}

/// A parsed SKILL.md entry with optional YAML frontmatter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub content: String,
    pub source_path: PathBuf,
    #[serde(default)]
    pub frontmatter: SkillFrontmatter,
    #[serde(default = "default_layer")]
    pub layer: SkillLayer,
}

fn default_layer() -> SkillLayer {
    SkillLayer::Project
}

/// YAML frontmatter from a SKILL.md file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillFrontmatter {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Registry of loaded skills keyed by id.
#[derive(Debug, Clone, Default)]
pub struct SkillRegistry {
    skills: HashMap<String, SkillEntry>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
        }
    }

    pub fn register(&mut self, skill: SkillEntry) {
        tracing::debug!(skill_id = %skill.id, name = %skill.name, "registered skill");
        self.skills.insert(skill.id.clone(), skill);
    }

    pub fn get(&self, id: &str) -> Option<&SkillEntry> {
        self.skills.get(id)
    }

    pub fn list(&self) -> Vec<&SkillEntry> {
        self.skills.values().collect()
    }

    pub fn count(&self) -> usize {
        self.skills.len()
    }

    /// Merge another registry into this one. Skills from `other` override
    /// those already present with the same ID (regardless of layer).
    pub fn merge_from(&mut self, other: SkillRegistry) {
        for (id, skill) in other.skills {
            self.skills.insert(id, skill);
        }
    }

    /// Return true if the registry contains a skill with the given ID.
    pub fn contains(&self, id: &str) -> bool {
        self.skills.contains_key(id)
    }

    /// Sanitize a deny list by removing IDs that don't exist in this registry.
    /// Returns (cleaned list, removed IDs) so callers can log what was cleaned.
    pub fn sanitize_deny_list(&self, deny: &[String]) -> (Vec<String>, Vec<String>) {
        let mut kept = Vec::new();
        let mut removed = Vec::new();
        for id in deny {
            if self.contains(id) {
                kept.push(id.clone());
            } else {
                removed.push(id.clone());
            }
        }
        (kept, removed)
    }

    /// Return a new registry containing only the skills that pass global
    /// allow/deny lists and an optional per-agent allowlist.
    pub fn filtered(
        &self,
        global_allow: &[String],
        global_deny: &[String],
        agent_allow: Option<&[String]>,
    ) -> SkillRegistry {
        let mut out = SkillRegistry::new();
        for (id, skill) in &self.skills {
            if !global_deny.is_empty() && global_deny.iter().any(|d| d == id) {
                continue;
            }
            if !global_allow.is_empty() && !global_allow.iter().any(|a| a == id) {
                continue;
            }
            if let Some(agent_list) = agent_allow {
                if !agent_list.is_empty() && !agent_list.iter().any(|a| a == id) {
                    continue;
                }
            }
            out.skills.insert(id.clone(), skill.clone());
        }
        out
    }

    /// Format all enabled skills into a prompt section for system prompt injection.
    /// Uses the "full" mode by default. See `format_for_prompt_mode` for other modes.
    pub fn format_for_prompt(&self) -> String {
        self.format_for_prompt_mode(&crate::config::SkillPromptMode::Full)
    }

    /// Format skills into a prompt section using the specified mode.
    ///
    /// - **Full**: Complete SKILL.md content (highest accuracy, most tokens)
    /// - **Compact**: Name + one-line description only (~50% token savings)
    /// - **Lazy**: Minimal header only; model uses list_skills/read_skill tools
    pub fn format_for_prompt_mode(&self, mode: &crate::config::SkillPromptMode) -> String {
        let enabled: Vec<&SkillEntry> = self
            .skills
            .values()
            .filter(|s| s.frontmatter.enabled.unwrap_or(true))
            .collect();

        if enabled.is_empty() {
            return String::new();
        }

        match mode {
            crate::config::SkillPromptMode::Full => self.format_full(&enabled),
            crate::config::SkillPromptMode::Compact => self.format_compact(&enabled),
            crate::config::SkillPromptMode::Lazy => self.format_lazy(&enabled),
        }
    }

    fn format_full(&self, skills: &[&SkillEntry]) -> String {
        let mut buf = String::from("## Available Skills\n\n");
        buf.push_str(
            "The following skills describe capabilities you have and how to use them:\n\n",
        );

        for skill in skills {
            buf.push_str(&format!("### {}\n\n", skill.name));
            if let Some(ref desc) = skill.description {
                buf.push_str(&format!("{}\n\n", desc));
            }
            buf.push_str(&skill.content);
            buf.push_str("\n\n---\n\n");
        }
        buf
    }

    fn format_compact(&self, skills: &[&SkillEntry]) -> String {
        let mut buf = String::from("## Available Skills\n\n");
        buf.push_str("You have the following skills. Use `read_skill` tool with the skill ID to get full instructions.\n\n");

        for skill in skills {
            let desc = skill.description.as_deref().unwrap_or("(no description)");
            let one_line = desc.lines().next().unwrap_or(desc);
            buf.push_str(&format!(
                "- **{}** (`{}`): {}\n",
                skill.name, skill.id, one_line
            ));
        }
        buf.push('\n');
        buf
    }

    fn format_lazy(&self, skills: &[&SkillEntry]) -> String {
        let mut buf = String::from("## Skills\n\n");
        buf.push_str(&format!(
            "You have {} skills available. Use the `list_skills` tool to see them, \
             and `read_skill` tool with a skill ID to get detailed instructions.\n\n",
            skills.len()
        ));
        buf.push_str("Skill IDs: ");
        let ids: Vec<&str> = skills.iter().map(|s| s.id.as_str()).collect();
        buf.push_str(&ids.join(", "));
        buf.push_str("\n\n");
        buf
    }
}

/// Load all SKILL.md files from a list of directories.
/// Directories searched later override earlier ones (last wins).
pub fn load_skills_from_dirs(dirs: &[&Path]) -> SkillRegistry {
    load_skills_from_dirs_with_layer(dirs, SkillLayer::Project)
}

/// Load skills from directories, tagging them with the given layer.
pub fn load_skills_from_dirs_with_layer(dirs: &[&Path], layer: SkillLayer) -> SkillRegistry {
    let mut registry = SkillRegistry::new();

    for dir in dirs {
        if !dir.exists() || !dir.is_dir() {
            continue;
        }

        match std::fs::read_dir(dir) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_dir() {
                        continue;
                    }
                    let skill_file = path.join("SKILL.md");
                    if skill_file.exists() {
                        match parse_skill_file(&skill_file, &path) {
                            Ok(mut skill) => {
                                skill.layer = layer;
                                registry.register(skill);
                            }
                            Err(e) => tracing::warn!(
                                path = %skill_file.display(),
                                error = %e,
                                "failed to parse SKILL.md"
                            ),
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(dir = %dir.display(), error = %e, "failed to read skills dir");
            }
        }
    }

    tracing::info!(count = registry.count(), layer = ?layer, "loaded skills");
    registry
}

/// Resolve the global shared skills directory: `~/.fastclaw/skills/`.
pub fn resolve_global_skills_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".fastclaw")
        .join("skills")
}

/// Build a per-agent SkillRegistry by merging layers in priority order.
///
/// Loading order (later overrides earlier):
/// 1. Extension plugin skills
/// 2. Project-level `./skills/`
/// 3. Global `~/.fastclaw/skills/`
/// 4. Agent workspace `workspace/<agent>/skills/`
pub struct SkillRegistryBuilder {
    base: SkillRegistry,
}

impl SkillRegistryBuilder {
    /// Start from a pre-loaded base registry (typically extension + project skills).
    pub fn new(base: SkillRegistry) -> Self {
        Self { base }
    }

    /// Start from an empty registry.
    pub fn empty() -> Self {
        Self {
            base: SkillRegistry::new(),
        }
    }

    /// Merge skills from a directory with a given layer. Higher-layer skills
    /// override lower-layer skills with the same ID.
    pub fn add_dir(mut self, dir: &Path, layer: SkillLayer) -> Self {
        let overlay = load_skills_from_dirs_with_layer(&[dir], layer);
        self.base.merge_from(overlay);
        self
    }

    /// Merge skills from multiple directories with a given layer.
    pub fn add_dirs(mut self, dirs: &[&Path], layer: SkillLayer) -> Self {
        let overlay = load_skills_from_dirs_with_layer(dirs, layer);
        self.base.merge_from(overlay);
        self
    }

    /// Merge another registry (unconditionally overrides on ID collision).
    pub fn merge(mut self, other: SkillRegistry) -> Self {
        self.base.merge_from(other);
        self
    }

    pub fn build(self) -> SkillRegistry {
        self.base
    }
}

fn parse_skill_file(path: &Path, dir: &Path) -> anyhow::Result<SkillEntry> {
    let raw = std::fs::read_to_string(path)?;

    let dir_name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let (frontmatter, content) = parse_frontmatter(&raw);

    let name = frontmatter
        .name
        .clone()
        .unwrap_or_else(|| extract_title_from_md(&content).unwrap_or_else(|| dir_name.to_string()));

    let description = frontmatter
        .description
        .clone()
        .or_else(|| extract_first_paragraph(&content));

    Ok(SkillEntry {
        id: dir_name.to_string(),
        name,
        description,
        content,
        source_path: path.to_path_buf(),
        frontmatter,
        layer: SkillLayer::Project,
    })
}

fn parse_frontmatter(raw: &str) -> (SkillFrontmatter, String) {
    if !raw.starts_with("---") {
        return (SkillFrontmatter::default(), raw.to_string());
    }

    let rest = &raw[3..];
    if let Some(end) = rest.find("\n---") {
        let yaml_str = &rest[..end].trim();
        let body = rest[end + 4..].trim_start().to_string();

        match serde_yaml_ng::from_str::<SkillFrontmatter>(yaml_str) {
            Ok(fm) => (fm, body),
            Err(_) => (SkillFrontmatter::default(), raw.to_string()),
        }
    } else {
        (SkillFrontmatter::default(), raw.to_string())
    }
}

fn extract_title_from_md(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("# ") {
            return Some(title.trim().to_string());
        }
    }
    None
}

fn extract_first_paragraph(content: &str) -> Option<String> {
    let mut lines = content.lines().peekable();
    while let Some(line) = lines.peek() {
        if line.trim().is_empty() || line.trim().starts_with('#') {
            lines.next();
        } else {
            break;
        }
    }

    let mut paragraph = String::new();
    for line in lines {
        if line.trim().is_empty() {
            break;
        }
        if !paragraph.is_empty() {
            paragraph.push(' ');
        }
        paragraph.push_str(line.trim());
    }

    if paragraph.is_empty() {
        None
    } else {
        Some(paragraph)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SkillPromptMode;
    use std::fs;
    use tempfile::TempDir;

    // ── parse_frontmatter ──────────────────────────────────────────

    #[test]
    fn parse_frontmatter_with_yaml() {
        let raw = "---\nname: Test Skill\ndescription: A test\nenabled: true\ntags:\n  - demo\n---\n# Body\n\nHello world";
        let (fm, body) = parse_frontmatter(raw);
        assert_eq!(fm.name.as_deref(), Some("Test Skill"));
        assert_eq!(fm.description.as_deref(), Some("A test"));
        assert_eq!(fm.enabled, Some(true));
        assert_eq!(fm.tags, vec!["demo"]);
        assert!(body.starts_with("# Body"));
    }

    #[test]
    fn parse_frontmatter_without_yaml() {
        let raw = "# Just Markdown\n\nNo frontmatter here.";
        let (fm, body) = parse_frontmatter(raw);
        assert!(fm.name.is_none());
        assert_eq!(body, raw);
    }

    #[test]
    fn parse_frontmatter_invalid_yaml_returns_default() {
        let raw = "---\n: bad: yaml: [unclosed\n---\nBody text";
        let (fm, body) = parse_frontmatter(raw);
        assert!(fm.name.is_none());
        assert_eq!(body, raw);
    }

    #[test]
    fn parse_frontmatter_unclosed_fence_returns_raw() {
        let raw = "---\nname: Orphan\nno closing fence";
        let (fm, body) = parse_frontmatter(raw);
        assert!(fm.name.is_none());
        assert_eq!(body, raw);
    }

    #[test]
    fn parse_frontmatter_empty_yaml_block() {
        let raw = "---\n---\n# Title\n\nContent";
        let (fm, body) = parse_frontmatter(raw);
        assert!(fm.name.is_none());
        assert!(fm.tools.is_empty());
        assert!(body.starts_with("# Title"));
    }

    // ── extract_title_from_md ──────────────────────────────────────

    #[test]
    fn extract_title_basic() {
        assert_eq!(
            extract_title_from_md("# My Title\n\nBody"),
            Some("My Title".into())
        );
    }

    #[test]
    fn extract_title_skips_h2() {
        assert_eq!(
            extract_title_from_md("## Not H1\n\n# Actual Title"),
            Some("Actual Title".into())
        );
    }

    #[test]
    fn extract_title_none_when_missing() {
        assert_eq!(extract_title_from_md("No headings at all"), None);
    }

    #[test]
    fn extract_title_strips_whitespace() {
        assert_eq!(
            extract_title_from_md("#   Padded   \n"),
            Some("Padded".into())
        );
    }

    // ── extract_first_paragraph ────────────────────────────────────

    #[test]
    fn first_paragraph_after_heading() {
        let md = "# Title\n\nFirst paragraph line one.\nLine two.\n\nSecond paragraph.";
        assert_eq!(
            extract_first_paragraph(md),
            Some("First paragraph line one. Line two.".into())
        );
    }

    #[test]
    fn first_paragraph_no_heading() {
        let md = "Immediate text here.\nContinued.";
        assert_eq!(
            extract_first_paragraph(md),
            Some("Immediate text here. Continued.".into())
        );
    }

    #[test]
    fn first_paragraph_empty_content() {
        assert_eq!(extract_first_paragraph(""), None);
        assert_eq!(extract_first_paragraph("# Title\n\n"), None);
    }

    // ── SkillRegistry basic ops ────────────────────────────────────

    fn make_skill(id: &str, name: &str, enabled: Option<bool>) -> SkillEntry {
        SkillEntry {
            id: id.into(),
            name: name.into(),
            description: Some(format!("{name} description")),
            content: format!("Content of {name}"),
            source_path: PathBuf::from(format!("/fake/{id}/SKILL.md")),
            frontmatter: SkillFrontmatter {
                name: Some(name.into()),
                enabled,
                ..Default::default()
            },
            layer: SkillLayer::Project,
        }
    }

    #[test]
    fn registry_register_and_get() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill("alpha", "Alpha Skill", None));
        reg.register(make_skill("beta", "Beta Skill", Some(true)));

        assert_eq!(reg.count(), 2);
        assert!(reg.get("alpha").is_some());
        assert!(reg.get("beta").is_some());
        assert!(reg.get("gamma").is_none());
    }

    #[test]
    fn registry_overwrite_by_id() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill("x", "Version 1", None));
        reg.register(make_skill("x", "Version 2", None));

        assert_eq!(reg.count(), 1);
        assert_eq!(reg.get("x").unwrap().name, "Version 2");
    }

    #[test]
    fn registry_list_returns_all() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill("a", "A", None));
        reg.register(make_skill("b", "B", None));
        reg.register(make_skill("c", "C", None));

        assert_eq!(reg.list().len(), 3);
    }

    // ── format_for_prompt_mode ─────────────────────────────────────

    #[test]
    fn format_empty_registry_returns_empty() {
        let reg = SkillRegistry::new();
        assert_eq!(reg.format_for_prompt_mode(&SkillPromptMode::Full), "");
        assert_eq!(reg.format_for_prompt_mode(&SkillPromptMode::Compact), "");
        assert_eq!(reg.format_for_prompt_mode(&SkillPromptMode::Lazy), "");
    }

    #[test]
    fn format_skips_disabled_skills() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill("on", "Enabled", Some(true)));
        reg.register(make_skill("off", "Disabled", Some(false)));

        let full = reg.format_for_prompt_mode(&SkillPromptMode::Full);
        assert!(full.contains("Enabled"));
        assert!(!full.contains("Disabled"));
    }

    #[test]
    fn format_full_includes_content() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill("demo", "Demo Skill", None));

        let output = reg.format_for_prompt_mode(&SkillPromptMode::Full);
        assert!(output.contains("## Available Skills"));
        assert!(output.contains("### Demo Skill"));
        assert!(output.contains("Content of Demo Skill"));
    }

    #[test]
    fn format_compact_has_id_and_name() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill("my-tool", "My Tool", None));

        let output = reg.format_for_prompt_mode(&SkillPromptMode::Compact);
        assert!(output.contains("**My Tool**"));
        assert!(output.contains("`my-tool`"));
        assert!(output.contains("read_skill"));
        assert!(!output.contains("Content of My Tool"));
    }

    #[test]
    fn format_lazy_shows_count_and_ids() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill("a", "Alpha", None));
        reg.register(make_skill("b", "Beta", None));

        let output = reg.format_for_prompt_mode(&SkillPromptMode::Lazy);
        assert!(output.contains("2 skills"));
        assert!(output.contains("list_skills"));
        assert!(!output.contains("Content of"));
    }

    #[test]
    fn format_default_uses_full() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill("x", "X", None));

        let default_out = reg.format_for_prompt();
        let full_out = reg.format_for_prompt_mode(&SkillPromptMode::Full);
        assert_eq!(default_out, full_out);
    }

    // ── load_skills_from_dirs ──────────────────────────────────────

    fn write_skill_md(base: &Path, skill_id: &str, content: &str) {
        let dir = base.join(skill_id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), content).unwrap();
    }

    #[test]
    fn load_from_single_dir() {
        let tmp = TempDir::new().unwrap();
        write_skill_md(
            tmp.path(),
            "hello",
            "# Hello Skill\n\nGreets the user.\n\n## Usage\n\nJust say hi.",
        );

        let reg = load_skills_from_dirs(&[tmp.path()]);
        assert_eq!(reg.count(), 1);

        let skill = reg.get("hello").unwrap();
        assert_eq!(skill.name, "Hello Skill");
        assert_eq!(skill.description.as_deref(), Some("Greets the user."));
    }

    #[test]
    fn load_with_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let content = "---\nname: Custom Name\ndescription: Custom desc\ntags:\n  - tag1\n  - tag2\n---\n# Ignored Title\n\nBody text here.";
        write_skill_md(tmp.path(), "custom", content);

        let reg = load_skills_from_dirs(&[tmp.path()]);
        let skill = reg.get("custom").unwrap();
        assert_eq!(skill.name, "Custom Name");
        assert_eq!(skill.description.as_deref(), Some("Custom desc"));
        assert_eq!(skill.frontmatter.tags, vec!["tag1", "tag2"]);
    }

    #[test]
    fn load_later_dir_overrides_earlier() {
        let dir_a = TempDir::new().unwrap();
        let dir_b = TempDir::new().unwrap();

        write_skill_md(dir_a.path(), "dup", "# Version A\n\nFrom dir A.");
        write_skill_md(dir_b.path(), "dup", "# Version B\n\nFrom dir B.");

        let reg = load_skills_from_dirs(&[dir_a.path(), dir_b.path()]);
        assert_eq!(reg.count(), 1);
        assert_eq!(reg.get("dup").unwrap().name, "Version B");
    }

    #[test]
    fn load_nonexistent_dir_is_ignored() {
        let reg = load_skills_from_dirs(&[Path::new("/nonexistent/path/12345")]);
        assert_eq!(reg.count(), 0);
    }

    #[test]
    fn load_skips_files_not_dirs() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("not_a_dir.md"), "stray file").unwrap();
        write_skill_md(tmp.path(), "real", "# Real\n\nReal skill.");

        let reg = load_skills_from_dirs(&[tmp.path()]);
        assert_eq!(reg.count(), 1);
        assert!(reg.get("real").is_some());
    }

    #[test]
    fn load_skips_dir_without_skill_md() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("empty-skill")).unwrap();
        fs::write(
            tmp.path().join("empty-skill").join("README.md"),
            "not a skill",
        )
        .unwrap();

        let reg = load_skills_from_dirs(&[tmp.path()]);
        assert_eq!(reg.count(), 0);
    }

    #[test]
    fn load_multiple_skills() {
        let tmp = TempDir::new().unwrap();
        write_skill_md(tmp.path(), "skill-a", "# A\n\nSkill A desc.");
        write_skill_md(tmp.path(), "skill-b", "# B\n\nSkill B desc.");
        write_skill_md(tmp.path(), "skill-c", "# C\n\nSkill C desc.");

        let reg = load_skills_from_dirs(&[tmp.path()]);
        assert_eq!(reg.count(), 3);
    }

    // ── SkillLayer ─────────────────────────────────────────────────

    #[test]
    fn layer_ordering() {
        assert!(SkillLayer::Extension < SkillLayer::Project);
        assert!(SkillLayer::Project < SkillLayer::Global);
        assert!(SkillLayer::Global < SkillLayer::AgentWorkspace);
    }

    #[test]
    fn load_with_layer_tags_entries() {
        let tmp = TempDir::new().unwrap();
        write_skill_md(tmp.path(), "x", "# X\n\nDesc.");

        let reg = load_skills_from_dirs_with_layer(&[tmp.path()], SkillLayer::Extension);
        assert_eq!(reg.get("x").unwrap().layer, SkillLayer::Extension);

        let reg2 = load_skills_from_dirs_with_layer(&[tmp.path()], SkillLayer::AgentWorkspace);
        assert_eq!(reg2.get("x").unwrap().layer, SkillLayer::AgentWorkspace);
    }

    // ── merge_from ─────────────────────────────────────────────────

    #[test]
    fn merge_from_adds_new_skills() {
        let mut base = SkillRegistry::new();
        base.register(make_skill("a", "A", None));

        let mut overlay = SkillRegistry::new();
        overlay.register(make_skill("b", "B", None));

        base.merge_from(overlay);
        assert_eq!(base.count(), 2);
        assert!(base.get("a").is_some());
        assert!(base.get("b").is_some());
    }

    #[test]
    fn merge_from_overrides_existing() {
        let mut base = SkillRegistry::new();
        base.register(make_skill("shared", "Base Version", None));

        let mut overlay = SkillRegistry::new();
        overlay.register(make_skill("shared", "Overlay Version", None));

        base.merge_from(overlay);
        assert_eq!(base.count(), 1);
        assert_eq!(base.get("shared").unwrap().name, "Overlay Version");
    }

    // ── SkillRegistryBuilder ───────────────────────────────────────

    #[test]
    fn builder_layered_merge() {
        let ext_dir = TempDir::new().unwrap();
        let project_dir = TempDir::new().unwrap();
        let agent_dir = TempDir::new().unwrap();

        write_skill_md(
            ext_dir.path(),
            "common",
            "# Extension Common\n\nFrom extension.",
        );
        write_skill_md(
            ext_dir.path(),
            "ext-only",
            "# Ext Only\n\nExtension exclusive.",
        );
        write_skill_md(
            project_dir.path(),
            "common",
            "# Project Common\n\nFrom project.",
        );
        write_skill_md(
            project_dir.path(),
            "proj-only",
            "# Proj Only\n\nProject exclusive.",
        );
        write_skill_md(
            agent_dir.path(),
            "common",
            "# Agent Common\n\nFrom agent workspace.",
        );
        write_skill_md(
            agent_dir.path(),
            "agent-only",
            "# Agent Only\n\nAgent private.",
        );

        let reg = SkillRegistryBuilder::empty()
            .add_dir(ext_dir.path(), SkillLayer::Extension)
            .add_dir(project_dir.path(), SkillLayer::Project)
            .add_dir(agent_dir.path(), SkillLayer::AgentWorkspace)
            .build();

        assert_eq!(reg.count(), 4);
        assert_eq!(reg.get("common").unwrap().name, "Agent Common");
        assert_eq!(reg.get("common").unwrap().layer, SkillLayer::AgentWorkspace);
        assert!(reg.get("ext-only").is_some());
        assert!(reg.get("proj-only").is_some());
        assert!(reg.get("agent-only").is_some());
    }

    #[test]
    fn builder_from_base() {
        let mut base = SkillRegistry::new();
        base.register(make_skill("pre", "Pre-loaded", None));

        let extra_dir = TempDir::new().unwrap();
        write_skill_md(extra_dir.path(), "new", "# New\n\nNew skill.");

        let reg = SkillRegistryBuilder::new(base)
            .add_dir(extra_dir.path(), SkillLayer::Global)
            .build();

        assert_eq!(reg.count(), 2);
        assert!(reg.get("pre").is_some());
        assert!(reg.get("new").is_some());
    }

    // ── resolve_global_skills_dir ──────────────────────────────────

    #[test]
    fn global_dir_ends_with_fastclaw_skills() {
        let dir = resolve_global_skills_dir();
        assert!(dir.ends_with(".fastclaw/skills"));
    }
}
