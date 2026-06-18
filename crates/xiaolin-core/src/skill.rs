use globset::{Glob, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Where a skill was loaded from (lower ordinal = lower priority).
///
/// Loading order: Extension < SharedAgents < UserCodex < UserCursor < UserFastclaw(Global)
/// < ProjectCursor < ProjectFastclaw < AgentWorkspace.
/// Skills from higher-priority layers override those from lower layers with the same ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SkillLayer {
    Extension = 0,
    SharedAgents = 1,
    UserCodex = 2,
    UserCursor = 3,
    Project = 4,
    Global = 5,
    ProjectCursor = 6,
    ProjectFastclaw = 7,
    AgentWorkspace = 8,
}

/// Origin tool that owns a skill directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillOrigin {
    XiaoLin,
    Cursor,
    Codex,
    SharedAgents,
    Extension,
}

/// Provenance metadata attached to every discovered skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSource {
    pub origin: SkillOrigin,
    pub layer: SkillLayer,
    pub path: PathBuf,
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
    #[serde(default)]
    pub source: Option<SkillSource>,
}

fn default_layer() -> SkillLayer {
    SkillLayer::Project
}

impl SkillEntry {
    /// A skill is conditional if it has non-empty `paths` without a catch-all `**` pattern.
    /// Conditional skills are only injected when touched files match the glob patterns.
    pub fn is_conditional(&self) -> bool {
        let paths = &self.frontmatter.paths;
        if paths.is_empty() {
            return false;
        }
        !paths.iter().any(|p| p.trim() == "**")
    }
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
    /// File path globs for conditional activation. Empty = unconditional (always on).
    /// Patterns are gitignore-style relative paths matched against workspace files.
    #[serde(default)]
    pub paths: Vec<String>,
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

    /// Consume the registry and return all skill entries.
    pub fn into_entries(self) -> Vec<SkillEntry> {
        self.skills.into_values().collect()
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

    /// Return a new registry containing only skills relevant for the given touched file paths.
    ///
    /// - **Unconditional** skills (`paths: []` or `paths: ["**"]`) are always included.
    /// - **Conditional** skills are included only if at least one touched path matches
    ///   any of the skill's `paths` globs.
    /// - Skills already excluded by deny list (via `filtered()`) stay excluded — call
    ///   `filtered()` first, then `filter_for_paths()`.
    pub fn filter_for_paths(&self, touched: &[&str]) -> SkillRegistry {
        if touched.is_empty() {
            let mut out = SkillRegistry::new();
            for (id, skill) in &self.skills {
                if !skill.is_conditional() {
                    out.skills.insert(id.clone(), skill.clone());
                }
            }
            return out;
        }

        let mut out = SkillRegistry::new();
        for (id, skill) in &self.skills {
            if !skill.is_conditional() {
                out.skills.insert(id.clone(), skill.clone());
                continue;
            }
            if skill_matches_paths(skill, touched) {
                out.skills.insert(id.clone(), skill.clone());
            }
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
        self.format_with_budget(mode, None).0
    }

    /// Format skills with an optional character budget.
    ///
    /// Returns `(prompt_text, truncation_info)`.
    /// `truncation_info` is `Some(SkillTruncationInfo)` when any truncation occurred.
    pub fn format_with_budget(
        &self,
        mode: &crate::config::SkillPromptMode,
        char_budget: Option<usize>,
    ) -> (String, Option<SkillTruncationInfo>) {
        let mut enabled: Vec<&SkillEntry> = self
            .skills
            .values()
            .filter(|s| s.frontmatter.enabled.unwrap_or(true))
            .collect();

        if enabled.is_empty() {
            return (String::new(), None);
        }

        // Sort by layer descending (highest priority first) for budget truncation
        enabled.sort_by(|a, b| b.layer.cmp(&a.layer));

        let budget = match char_budget {
            Some(0) | None => {
                let output = match mode {
                    crate::config::SkillPromptMode::Full => self.format_full(&enabled),
                    crate::config::SkillPromptMode::Compact => self.format_compact(&enabled),
                    crate::config::SkillPromptMode::Lazy => self.format_lazy(&enabled),
                };
                return (output, None);
            }
            Some(b) => b,
        };

        let initial = match mode {
            crate::config::SkillPromptMode::Full => self.format_full(&enabled),
            crate::config::SkillPromptMode::Compact => self.format_compact(&enabled),
            crate::config::SkillPromptMode::Lazy => self.format_lazy(&enabled),
        };

        if initial.len() <= budget {
            return (initial, None);
        }

        // Stage 1: truncate descriptions to first line (compact/full only, lazy already minimal)
        if !matches!(mode, crate::config::SkillPromptMode::Lazy) {
            let truncated = self.format_compact_truncated(&enabled);
            if truncated.len() <= budget {
                return (
                    truncated,
                    Some(SkillTruncationInfo {
                        stage: TruncationStage::DescriptionShortened,
                        total_skills: enabled.len(),
                        included_skills: enabled.len(),
                        omitted_skills: 0,
                    }),
                );
            }
        }

        // Stage 2: omit lowest-priority skills until within budget
        let mut included = enabled.clone();
        let mut omitted = 0usize;
        while included.len() > 1 {
            included.pop(); // remove lowest-priority (last after sort)
            omitted += 1;
            let candidate = if matches!(mode, crate::config::SkillPromptMode::Lazy) {
                self.format_lazy(&included)
            } else {
                self.format_compact_truncated(&included)
            };
            if candidate.len() <= budget {
                return (
                    candidate,
                    Some(SkillTruncationInfo {
                        stage: TruncationStage::SkillsOmitted,
                        total_skills: enabled.len(),
                        included_skills: included.len(),
                        omitted_skills: omitted,
                    }),
                );
            }
        }

        // Even with 1 skill it exceeds budget — return it anyway with warning
        let final_output = if matches!(mode, crate::config::SkillPromptMode::Lazy) {
            self.format_lazy(&included)
        } else {
            self.format_compact_truncated(&included)
        };
        (
            final_output,
            Some(SkillTruncationInfo {
                stage: TruncationStage::SkillsOmitted,
                total_skills: enabled.len(),
                included_skills: included.len(),
                omitted_skills: omitted,
            }),
        )
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

    /// Compact format with descriptions truncated to first line only (Stage 1 truncation).
    fn format_compact_truncated(&self, skills: &[&SkillEntry]) -> String {
        let mut buf = String::from("## Available Skills\n\n");
        buf.push_str("You have the following skills. Use `read_skill` tool with the skill ID to get full instructions.\n\n");

        for skill in skills {
            let first_line = skill
                .description
                .as_deref()
                .and_then(|d| d.lines().next())
                .unwrap_or("(no description)");
            let truncated = if first_line.chars().count() > 80 {
                let s: String = first_line.chars().take(77).collect();
                format!("{s}…")
            } else {
                first_line.to_string()
            };
            buf.push_str(&format!(
                "- **{}** (`{}`): {}\n",
                skill.name, skill.id, truncated
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

/// Information about skill prompt truncation due to context budget.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillTruncationInfo {
    pub stage: TruncationStage,
    pub total_skills: usize,
    pub included_skills: usize,
    pub omitted_skills: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TruncationStage {
    /// Descriptions were shortened to first line.
    DescriptionShortened,
    /// Some skills were omitted to fit budget.
    SkillsOmitted,
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

/// Resolve the global shared skills directory based on the current build mode.
pub fn resolve_global_skills_dir() -> PathBuf {
    let mode = crate::config::ConfigMode::from_flags(false, None);
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));

    let state_dir = match mode {
        crate::config::ConfigMode::Development => home.join(".xiaolin-dev"),
        crate::config::ConfigMode::Profile(name) => home.join(format!(".xiaolin-{name}")),
        crate::config::ConfigMode::Production => home.join(".xiaolin"),
    };

    state_dir.join("skills")
}

/// Scan path descriptor for cross-tool skill discovery.
#[derive(Debug)]
pub struct SkillScanPath {
    pub path: PathBuf,
    pub layer: SkillLayer,
    pub origin: SkillOrigin,
}

/// Build the full list of skill scan paths (user-level + project-level).
///
/// Scan order (lowest priority first → highest last):
/// 1. `~/.agents/skills/`       (SharedAgents)
/// 2. `~/.codex/skills/`        (UserCodex)
/// 3. `~/.cursor/skills/`       (UserCursor)
/// 4. `~/.cursor/skills-cursor/` (UserCursor — Cursor built-in skills)
/// 5. `~/.xiaolin/skills/`     (Global / UserFastclaw)
/// 6. `<ws>/.cursor/skills/`    (ProjectCursor)
/// 7. `<ws>/.xiaolin/skills/`  (ProjectFastclaw)
pub fn build_skill_scan_paths(workspace_root: Option<&Path>) -> Vec<SkillScanPath> {
    let mut paths = Vec::new();
    if let Some(home) = dirs::home_dir() {
        paths.push(SkillScanPath {
            path: home.join(".agents/skills"),
            layer: SkillLayer::SharedAgents,
            origin: SkillOrigin::SharedAgents,
        });
        paths.push(SkillScanPath {
            path: home.join(".codex/skills"),
            layer: SkillLayer::UserCodex,
            origin: SkillOrigin::Codex,
        });
        paths.push(SkillScanPath {
            path: home.join(".cursor/skills"),
            layer: SkillLayer::UserCursor,
            origin: SkillOrigin::Cursor,
        });
        paths.push(SkillScanPath {
            path: home.join(".cursor/skills-cursor"),
            layer: SkillLayer::UserCursor,
            origin: SkillOrigin::Cursor,
        });
    }
    let global_dir = resolve_global_skills_dir();
    paths.push(SkillScanPath {
        path: global_dir,
        layer: SkillLayer::Global,
        origin: SkillOrigin::XiaoLin,
    });
    if let Some(ws) = workspace_root {
        paths.push(SkillScanPath {
            path: ws.join(".cursor/skills"),
            layer: SkillLayer::ProjectCursor,
            origin: SkillOrigin::Cursor,
        });
        paths.push(SkillScanPath {
            path: ws.join(".xiaolin/skills"),
            layer: SkillLayer::ProjectFastclaw,
            origin: SkillOrigin::XiaoLin,
        });
    }
    paths
}

/// Built-in skill: teaches agents how to manage `.xiaolin/` project configuration.
const BUILTIN_CONFIG_MANAGER_SKILL: &str = r#"---
name: XiaoLin Config Manager
description: Manage .xiaolin/ project configuration (skills, MCP servers, rules, config)
---

# XiaoLin Config Manager

You can help users manage their XiaoLin project configuration. The `.xiaolin/` directory in the workspace root contains project-level settings.

## Directory Structure

```
<workspace_root>/.xiaolin/
├── config.json          # Project-level configuration (overrides user/global config)
├── mcp.json             # Project-level MCP server definitions (Cursor-compatible format)
├── skills/              # Project-level skills (SKILL.md files)
│   └── <skill-id>/
│       └── SKILL.md
└── rules/               # Project-level rules (Markdown with YAML frontmatter)
    └── *.md
```

## Configuration Layers (highest to lowest priority)

1. `.xiaolin/config.json` — project-level (workspace root)
2. `config/default.json` — local project config (cwd-relative)
3. `~/.xiaolin/config/default.json` — user-level
4. `~/.openclaw/openclaw.json` — legacy compatibility

## MCP Server Format

`.xiaolin/mcp.json` uses the same format as Cursor's `.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "server-id": {
      "command": "npx",
      "args": ["-y", "@some/mcp-server"],
      "env": { "API_KEY": "..." },
      "disabled": false
    }
  }
}
```

Set `"disabled": true` to disable a user-level MCP server for this project.

## Rules Format

`.xiaolin/rules/*.md` files support YAML frontmatter:

```yaml
---
alwaysApply: true        # Always inject into system prompt
name: my-rule            # Display name
description: Rule desc
globs:                   # Only inject when matching files are involved
  - "*.rs"
  - "src/**/*.ts"
---
# Rule content here
```

## Skill Discovery

Skills are loaded from multiple sources (lower to higher priority):
- `~/.agents/skills/` (shared agents)
- `~/.codex/skills/` (Codex user-level)
- `~/.cursor/skills/` (Cursor user-level)
- `~/.xiaolin/skills/` (XiaoLin user-level)
- `<workspace>/.cursor/skills/` (Cursor project-level, read-only)
- `<workspace>/.xiaolin/skills/` (XiaoLin project-level)

When creating skills, always write to `.xiaolin/skills/` — never modify other tools' directories.

## Actions

- **Create skill**: Write `<workspace>/.xiaolin/skills/<id>/SKILL.md`
- **Add MCP server**: Edit `.xiaolin/mcp.json`
- **Add rule**: Create `.xiaolin/rules/<name>.md`
- **Override config**: Edit `.xiaolin/config.json`
"#;

/// Register the built-in config manager skill into a registry.
pub fn register_builtin_skills(registry: &mut SkillRegistry) {
    registry.register(SkillEntry {
        id: "xiaolin-config-manager".to_string(),
        name: "XiaoLin Config Manager".to_string(),
        description: Some(
            "Manage .xiaolin/ project configuration (skills, MCP servers, rules, config)"
                .to_string(),
        ),
        content: BUILTIN_CONFIG_MANAGER_SKILL.to_string(),
        source_path: PathBuf::from("(builtin)"),
        frontmatter: SkillFrontmatter {
            name: Some("XiaoLin Config Manager".to_string()),
            description: Some(
                "Manage .xiaolin/ project configuration".to_string(),
            ),
            ..Default::default()
        },
        layer: SkillLayer::Extension,
        source: Some(SkillSource {
            origin: SkillOrigin::XiaoLin,
            layer: SkillLayer::Extension,
            path: PathBuf::from("(builtin)"),
        }),
    });
}

/// Load skills from all cross-tool scan paths and merge into a single registry.
pub fn load_skills_cross_tool(workspace_root: Option<&Path>) -> SkillRegistry {
    let scan_paths = build_skill_scan_paths(workspace_root);
    let mut registry = SkillRegistry::new();
    for sp in &scan_paths {
        if !sp.path.exists() || !sp.path.is_dir() {
            continue;
        }
        let layer_reg = load_skills_from_dirs_with_layer(&[sp.path.as_path()], sp.layer);
        let count = layer_reg.count();
        if count > 0 {
            tracing::info!(
                count,
                layer = ?sp.layer,
                origin = ?sp.origin,
                path = %sp.path.display(),
                "discovered skills from cross-tool directory"
            );
        }
        for skill in layer_reg.into_entries() {
            let mut s = skill;
            s.source = Some(SkillSource {
                origin: sp.origin,
                layer: sp.layer,
                path: sp.path.clone(),
            });
            registry.register(s);
        }
    }
    registry
}

/// Build a per-agent SkillRegistry by merging layers in priority order.
///
/// Loading order (later overrides earlier):
/// 1. Extension plugin skills
/// 2. Cross-tool skills (SharedAgents, UserCodex, UserCursor, Global, ProjectCursor, ProjectFastclaw)
/// 3. Agent workspace `workspace/<agent>/skills/`
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
        source: None,
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

/// Check if any of the touched paths match the conditional skill's `paths` globs.
fn skill_matches_paths(skill: &SkillEntry, touched: &[&str]) -> bool {
    let mut builder = GlobSetBuilder::new();
    let mut has_patterns = false;

    for pattern in &skill.frontmatter.paths {
        let trimmed = pattern.trim();
        if trimmed.is_empty() {
            continue;
        }
        match Glob::new(trimmed) {
            Ok(g) => {
                builder.add(g);
                has_patterns = true;
            }
            Err(e) => {
                tracing::warn!(
                    skill_id = %skill.id,
                    pattern = %trimmed,
                    error = %e,
                    "invalid glob pattern in skill frontmatter paths"
                );
            }
        }
    }

    if !has_patterns {
        return false;
    }

    let globset = match builder.build() {
        Ok(gs) => gs,
        Err(e) => {
            tracing::warn!(skill_id = %skill.id, error = %e, "failed to build globset");
            return false;
        }
    };

    touched.iter().any(|path| globset.is_match(path))
}

/// Extract file paths mentioned in conversation messages' tool calls.
///
/// Scans tool call arguments for common path-bearing parameters (`path`, `file_path`,
/// `target_path`, `directory`) and returns them as a deduplicated list.
pub fn extract_touched_paths(messages: &[crate::types::ChatMessage]) -> Vec<String> {
    use std::collections::HashSet;
    let mut paths = HashSet::new();

    for msg in messages {
        if let Some(ref tool_calls) = msg.tool_calls {
            for tc in tool_calls {
                if let Ok(args) = serde_json::from_str::<serde_json::Value>(&tc.function.arguments)
                {
                    if let Some(obj) = args.as_object() {
                        for key in &["path", "file_path", "target_path", "directory", "file", "filename"] {
                            if let Some(serde_json::Value::String(p)) = obj.get(*key) {
                                if !p.is_empty() {
                                    paths.insert(p.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    paths.into_iter().collect()
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
    fn parse_frontmatter_with_tools() {
        let raw = "---\nname: Restricted\ntools:\n  - read_file\n  - write_file\n---\n# Body";
        let (fm, _body) = parse_frontmatter(raw);
        assert_eq!(fm.tools, vec!["read_file", "write_file"]);
    }

    #[test]
    fn parse_frontmatter_empty_tools() {
        let raw = "---\nname: Unrestricted\ntools: []\n---\n# Body";
        let (fm, _body) = parse_frontmatter(raw);
        assert!(fm.tools.is_empty());
    }

    #[test]
    fn parse_frontmatter_no_tools_field() {
        let raw = "---\nname: NoTools\n---\n# Body";
        let (fm, _body) = parse_frontmatter(raw);
        assert!(fm.tools.is_empty());
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
            source: None,
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

    // ── format_with_budget ─────────────────────────────────────────

    fn make_skill_with_layer(
        id: &str,
        name: &str,
        desc: &str,
        layer: SkillLayer,
    ) -> SkillEntry {
        SkillEntry {
            id: id.into(),
            name: name.into(),
            description: Some(desc.into()),
            content: format!("Content of {name}"),
            source_path: PathBuf::from(format!("/fake/{id}/SKILL.md")),
            frontmatter: SkillFrontmatter {
                name: Some(name.into()),
                description: Some(desc.into()),
                ..Default::default()
            },
            layer,
            source: None,
        }
    }

    #[test]
    fn budget_none_returns_all_no_truncation() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill("a", "Alpha", None));
        reg.register(make_skill("b", "Beta", None));

        let (output, info) = reg.format_with_budget(&SkillPromptMode::Compact, None);
        assert!(output.contains("Alpha"));
        assert!(output.contains("Beta"));
        assert!(info.is_none());
    }

    #[test]
    fn budget_zero_returns_all_no_truncation() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill("a", "Alpha", None));

        let (output, info) = reg.format_with_budget(&SkillPromptMode::Compact, Some(0));
        assert!(output.contains("Alpha"));
        assert!(info.is_none());
    }

    #[test]
    fn budget_large_enough_no_truncation() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill("a", "Alpha", None));

        let (output, info) = reg.format_with_budget(&SkillPromptMode::Compact, Some(100_000));
        assert!(output.contains("Alpha"));
        assert!(info.is_none());
    }

    #[test]
    fn budget_triggers_description_shortening() {
        let mut reg = SkillRegistry::new();
        let long_desc = "A very long description that goes on and on. ".repeat(5);
        reg.register(make_skill_with_layer("a", "Alpha", &long_desc, SkillLayer::Project));

        let full_output = reg.format_for_prompt_mode(&SkillPromptMode::Compact);
        // Budget just under the full compact output forces Stage 1 truncation
        let budget = full_output.len() - 10;
        let (output, info) = reg.format_with_budget(&SkillPromptMode::Compact, Some(budget));
        assert!(output.contains("Alpha"));
        let info = info.expect("should have truncation info");
        assert_eq!(info.stage, TruncationStage::DescriptionShortened);
        assert_eq!(info.total_skills, 1);
        assert_eq!(info.included_skills, 1);
        assert_eq!(info.omitted_skills, 0);
    }

    #[test]
    fn budget_triggers_skill_omission() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill_with_layer(
            "ext", "Extension Skill", "Low priority", SkillLayer::Extension,
        ));
        reg.register(make_skill_with_layer(
            "proj", "Project Skill", "Medium priority", SkillLayer::Project,
        ));
        reg.register(make_skill_with_layer(
            "ws", "Workspace Skill", "High priority", SkillLayer::AgentWorkspace,
        ));

        // Very tight budget — only room for 1-2 skills
        let (output, info) = reg.format_with_budget(&SkillPromptMode::Compact, Some(200));
        let info = info.expect("should have truncation info");
        assert_eq!(info.stage, TruncationStage::SkillsOmitted);
        assert!(info.omitted_skills > 0);
        // Highest priority skill should be retained
        assert!(output.contains("Workspace Skill"));
    }

    #[test]
    fn budget_disabled_with_zero_percent() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill("a", "Alpha", None));
        reg.register(make_skill("b", "Beta", None));

        // context_budget_percent=0 → char_budget=None → no truncation
        let (output, info) = reg.format_with_budget(&SkillPromptMode::Compact, None);
        assert!(output.contains("Alpha"));
        assert!(output.contains("Beta"));
        assert!(info.is_none());
    }

    // ── resolve_global_skills_dir ──────────────────────────────────

    #[test]
    fn global_dir_ends_with_xiaolin_skills() {
        let dir = resolve_global_skills_dir();
        if cfg!(debug_assertions) {
            assert!(
                dir.ends_with(".xiaolin-dev/skills"),
                "got {}",
                dir.display()
            );
        } else {
            assert!(dir.ends_with(".xiaolin/skills"), "got {}", dir.display());
        }
    }

    // ── Conditional activation (paths) ────────────────────────────

    fn make_conditional_skill(id: &str, name: &str, paths: Vec<&str>) -> SkillEntry {
        SkillEntry {
            id: id.into(),
            name: name.into(),
            description: Some(format!("{name} description")),
            content: format!("Content of {name}"),
            source_path: PathBuf::from(format!("/fake/{id}/SKILL.md")),
            frontmatter: SkillFrontmatter {
                name: Some(name.into()),
                paths: paths.into_iter().map(String::from).collect(),
                ..Default::default()
            },
            layer: SkillLayer::Project,
            source: None,
        }
    }

    #[test]
    fn is_conditional_empty_paths() {
        assert!(!make_conditional_skill("a", "A", vec![]).is_conditional());
    }

    #[test]
    fn is_conditional_star_star() {
        assert!(!make_conditional_skill("a", "A", vec!["**"]).is_conditional());
    }

    #[test]
    fn is_conditional_with_patterns() {
        assert!(make_conditional_skill("a", "A", vec!["*.rs"]).is_conditional());
    }

    #[test]
    fn is_conditional_star_star_among_others() {
        assert!(!make_conditional_skill("a", "A", vec!["*.rs", "**"]).is_conditional());
    }

    #[test]
    fn filter_paths_no_touched_returns_unconditional_only() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill("always", "Always On", None));
        reg.register(make_conditional_skill("rs-only", "Rust Only", vec!["*.rs"]));

        let filtered = reg.filter_for_paths(&[]);
        assert_eq!(filtered.count(), 1);
        assert!(filtered.get("always").is_some());
        assert!(filtered.get("rs-only").is_none());
    }

    #[test]
    fn filter_paths_matching_includes_conditional() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill("always", "Always On", None));
        reg.register(make_conditional_skill("rs-only", "Rust Only", vec!["*.rs"]));

        let filtered = reg.filter_for_paths(&["src/main.rs"]);
        assert_eq!(filtered.count(), 2);
        assert!(filtered.get("always").is_some());
        assert!(filtered.get("rs-only").is_some());
    }

    #[test]
    fn filter_paths_no_match_excludes_conditional() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill("always", "Always On", None));
        reg.register(make_conditional_skill("rs-only", "Rust Only", vec!["*.rs"]));

        let filtered = reg.filter_for_paths(&["package.json", "src/app.tsx"]);
        assert_eq!(filtered.count(), 1);
        assert!(filtered.get("always").is_some());
    }

    #[test]
    fn filter_paths_deny_takes_priority() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill("always", "Always On", None));
        reg.register(make_conditional_skill("rs-only", "Rust Only", vec!["*.rs"]));

        let denied = reg.filtered(&[], &["rs-only".to_string()], None);
        let result = denied.filter_for_paths(&["main.rs"]);
        assert_eq!(result.count(), 1);
        assert!(result.get("always").is_some());
        assert!(result.get("rs-only").is_none());
    }

    #[test]
    fn filter_paths_multiple_globs() {
        let mut reg = SkillRegistry::new();
        reg.register(make_conditional_skill("web", "Web", vec!["*.tsx", "*.css", "*.html"]));

        assert_eq!(reg.filter_for_paths(&["app.tsx"]).count(), 1);
        assert_eq!(reg.filter_for_paths(&["style.css"]).count(), 1);
        assert_eq!(reg.filter_for_paths(&["page.html"]).count(), 1);
        assert_eq!(reg.filter_for_paths(&["main.rs"]).count(), 0);
    }

    #[test]
    fn filter_paths_directory_glob() {
        let mut reg = SkillRegistry::new();
        reg.register(make_conditional_skill("tests", "Test", vec!["tests/**", "**/*_test.rs"]));

        assert_eq!(reg.filter_for_paths(&["tests/unit.rs"]).count(), 1);
        assert_eq!(reg.filter_for_paths(&["src/foo_test.rs"]).count(), 1);
        assert_eq!(reg.filter_for_paths(&["src/main.rs"]).count(), 0);
    }

    #[test]
    fn frontmatter_paths_round_trip() {
        let yaml = "---\nname: Cond\npaths:\n  - \"*.rs\"\n  - \"src/**/*.toml\"\n---\n# Body";
        let (fm, _) = parse_frontmatter(yaml);
        assert_eq!(fm.paths, vec!["*.rs", "src/**/*.toml"]);
    }

    #[test]
    fn extract_touched_paths_from_tool_calls() {
        use crate::types::{ChatMessage, FunctionCall, Role, ToolCall};

        let msgs = vec![
            ChatMessage {
                role: Role::User,
                content: Some(serde_json::Value::String("help".into())),
                ..Default::default()
            },
            ChatMessage {
                role: Role::Assistant,
                tool_calls: Some(vec![ToolCall {
                    id: "tc1".into(),
                    call_type: "function".into(),
                    function: FunctionCall {
                        name: "read_file".into(),
                        arguments: r#"{"path": "src/main.rs"}"#.into(),
                    },
                    output: None,
                    success: None,
                    duration_ms: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: Role::Assistant,
                tool_calls: Some(vec![ToolCall {
                    id: "tc2".into(),
                    call_type: "function".into(),
                    function: FunctionCall {
                        name: "write_file".into(),
                        arguments: r#"{"file_path": "Cargo.toml", "content": "..."}"#.into(),
                    },
                    output: None,
                    success: None,
                    duration_ms: None,
                }]),
                ..Default::default()
            },
        ];

        let paths = extract_touched_paths(&msgs);
        assert!(paths.contains(&"src/main.rs".to_string()));
        assert!(paths.contains(&"Cargo.toml".to_string()));
        assert_eq!(paths.len(), 2);
    }
}
