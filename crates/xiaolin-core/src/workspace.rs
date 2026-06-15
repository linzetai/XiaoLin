use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

fn validate_skill_id(skill_id: &str) -> anyhow::Result<()> {
    if skill_id.is_empty() {
        anyhow::bail!("skill_id must not be empty");
    }
    if skill_id.contains("..") || skill_id.contains('/') || skill_id.contains('\\') {
        anyhow::bail!(
            "skill_id '{}' contains path traversal characters (.. / \\)",
            skill_id
        );
    }
    if !skill_id
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        anyhow::bail!(
            "skill_id '{}' contains disallowed characters; only [a-zA-Z0-9._-] are permitted",
            skill_id
        );
    }
    Ok(())
}

pub const DEFAULT_IDENTITY_FILENAME: &str = "IDENTITY.md";
pub const DEFAULT_USER_FILENAME: &str = "USER.md";

/// Filenames under the repo `prompts/` directory (see `XIAOLIN_PROMPTS_DIR` or cwd `./prompts`).
pub const PROMPTS_REPO_SYSTEM_BASE: &str = "system-base.md";
pub const PROMPTS_REPO_TOOL_USAGE_GUIDE: &str = "tool-usage-guide.md";

/// Embedded copy of `prompts/system-base.md` (used when no workspace / compile-time default).
pub const EMBEDDED_SYSTEM_BASE_PROMPT: &str = include_str!("../../../prompts/system-base.md");
/// Embedded copy of `prompts/tool-usage-guide.md`.
pub const EMBEDDED_TOOL_USAGE_GUIDE: &str = include_str!("../../../prompts/tool-usage-guide.md");
/// Canonical name for the tool usage guide text (embedded, or read from [`PROMPTS_REPO_TOOL_USAGE_GUIDE`]).
pub const DEFAULT_TOOL_USAGE_GUIDE: &str = EMBEDDED_TOOL_USAGE_GUIDE;

const WORKSPACE_FILES: &[&str] = &[
    DEFAULT_IDENTITY_FILENAME,
    DEFAULT_USER_FILENAME,
];

/// Prompt context ordering — lower numbers appear first in the system prompt.
const CONTEXT_FILE_ORDER: &[(&str, u32)] = &[
    ("identity.md", 10),
    ("user.md", 20),
];

/// Loaded workspace identity files for an agent.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceIdentity {
    pub identity: Option<String>,
    pub user: Option<String>,
    pub extras: Vec<(String, String)>,
}

impl WorkspaceIdentity {
    fn context_priority(filename: &str) -> u32 {
        CONTEXT_FILE_ORDER
            .iter()
            .find(|(name, _)| *name == filename)
            .map(|(_, p)| *p)
            .unwrap_or(100)
    }

    /// Format all identity content for system prompt injection, ordered by priority.
    pub fn format_for_prompt(&self) -> String {
        let mut sections: Vec<(u32, &str, &str)> = Vec::new();

        if let Some(ref identity) = self.identity {
            sections.push((Self::context_priority("identity.md"), "Identity", identity));
        }
        if let Some(ref user) = self.user {
            sections.push((Self::context_priority("user.md"), "User Context", user));
        }

        for (idx, (name, content)) in self.extras.iter().enumerate() {
            let lower = name.to_lowercase();
            let prio = CONTEXT_FILE_ORDER
                .iter()
                .find(|(n, _)| *n == lower.as_str())
                .map(|(_, p)| *p)
                .unwrap_or(200 + idx as u32);
            sections.push((prio, name.as_str(), content.as_str()));
        }

        sections.sort_by_key(|s| s.0);

        let mut buf = String::new();
        for (_, label, content) in &sections {
            buf.push_str(&format!("## {}\n\n{}\n\n", label, content));
        }

        buf
    }
}

/// An agent workspace directory with bootstrap files.
#[derive(Debug, Clone)]
pub struct AgentWorkspace {
    pub root: PathBuf,
    pub agent_id: String,
}

impl AgentWorkspace {
    pub fn new(root: impl Into<PathBuf>, agent_id: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            agent_id: agent_id.into(),
        }
    }

    /// Load all identity files from this workspace.
    pub fn load_identity(&self) -> WorkspaceIdentity {
        let mut bs = WorkspaceIdentity::default();

        for &fname in WORKSPACE_FILES {
            let content = self.read_file(fname);
            match fname {
                f if f == DEFAULT_IDENTITY_FILENAME => bs.identity = content,
                f if f == DEFAULT_USER_FILENAME => bs.user = content,
                _ => {}
            }
        }

        bs
    }

    /// Write or update a bootstrap file.
    pub fn write_file(&self, filename: &str, content: &str) -> anyhow::Result<()> {
        let path = self.safe_resolve(filename)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// Read a single file from the workspace.
    fn read_file(&self, filename: &str) -> Option<String> {
        let path = match self.safe_resolve(filename) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(filename, error = %e, "workspace: blocked path traversal in read_file");
                return None;
            }
        };
        match std::fs::read_to_string(&path) {
            Ok(content) if !content.trim().is_empty() => Some(content),
            _ => None,
        }
    }

    fn safe_resolve(&self, filename: &str) -> anyhow::Result<PathBuf> {
        if filename.contains("..") {
            tracing::warn!(
                filename = %filename,
                workspace = %self.root.display(),
                agent = %self.agent_id,
                "workspace: path traversal attempt blocked"
            );
            anyhow::bail!("filename must not contain '..'");
        }
        let joined = self.root.join(filename);
        let canon_root = self
            .root
            .canonicalize()
            .unwrap_or_else(|_| self.root.clone());
        let canon_target = if joined.exists() {
            joined.canonicalize()?
        } else {
            let parent = joined
                .parent()
                .map(|p| {
                    if p.exists() {
                        p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
                    } else {
                        p.to_path_buf()
                    }
                })
                .unwrap_or_else(|| canon_root.clone());
            parent.join(joined.file_name().unwrap_or_default())
        };
        if !canon_target.starts_with(&canon_root) {
            tracing::warn!(
                filename = %filename,
                resolved = %canon_target.display(),
                root = %canon_root.display(),
                agent = %self.agent_id,
                "workspace: resolved path escapes root boundary"
            );
            anyhow::bail!(
                "resolved path escapes workspace root: {}",
                canon_target.display()
            );
        }
        Ok(joined)
    }

    /// Path to the agent's private skills directory.
    pub fn skills_dir(&self) -> PathBuf {
        self.root.join("skills")
    }

    /// Ensure the skills directory exists.
    pub fn ensure_skills_dir(&self) -> anyhow::Result<PathBuf> {
        let dir = self.skills_dir();
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// Write a SKILL.md file into the agent's workspace skills directory.
    /// Creates `skills/<skill_id>/SKILL.md`.
    pub fn write_skill(&self, skill_id: &str, content: &str) -> anyhow::Result<PathBuf> {
        validate_skill_id(skill_id)?;
        let skill_dir = self.skills_dir().join(skill_id);
        std::fs::create_dir_all(&skill_dir)?;
        let path = skill_dir.join("SKILL.md");
        std::fs::write(&path, content)?;
        tracing::info!(
            agent_id = %self.agent_id,
            skill_id = %skill_id,
            path = %path.display(),
            "wrote skill to agent workspace"
        );
        Ok(path)
    }

    /// Initialize the workspace with default identity files if they don't exist.
    pub fn ensure_workspace(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.root)?;

        let identity_path = self.root.join(DEFAULT_IDENTITY_FILENAME);
        if !identity_path.exists() {
            std::fs::write(&identity_path, DEFAULT_IDENTITY_TEMPLATE)?;
            tracing::info!(path = %identity_path.display(), "created default IDENTITY.md");
        }

        let user_path = self.root.join(DEFAULT_USER_FILENAME);
        if !user_path.exists() {
            std::fs::write(&user_path, DEFAULT_USER_TEMPLATE)?;
            tracing::info!(path = %user_path.display(), "created default USER.md");
        }

        Ok(())
    }
}

/// Candidate roots for repo-style `prompts/` (see `XIAOLIN_PROMPTS_DIR` or `./prompts` under cwd).
fn prompts_dir_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(dir) = std::env::var("XIAOLIN_PROMPTS_DIR") {
        out.push(PathBuf::from(dir));
    }
    if let Ok(cwd) = std::env::current_dir() {
        out.push(cwd.join("prompts"));
    }
    out
}

static CACHED_BASE_PROMPTS: OnceLock<Option<String>> = OnceLock::new();
static CACHED_ROLE_PROMPTS: OnceLock<RwLock<HashMap<String, Option<String>>>> = OnceLock::new();
static SKILL_PROMPT_MODE: OnceLock<crate::config::SkillPromptMode> = OnceLock::new();

fn role_prompt_cache() -> &'static RwLock<HashMap<String, Option<String>>> {
    CACHED_ROLE_PROMPTS.get_or_init(|| RwLock::new(HashMap::new()))
}

fn try_load_prompts_from_filesystem() -> Option<(String, String)> {
    fn read_pair(dir: &Path) -> Option<(String, String)> {
        let base = std::fs::read_to_string(dir.join(PROMPTS_REPO_SYSTEM_BASE)).ok()?;
        let tools = std::fs::read_to_string(dir.join(PROMPTS_REPO_TOOL_USAGE_GUIDE)).ok()?;
        if base.trim().is_empty() || tools.trim().is_empty() {
            return None;
        }
        Some((base, tools))
    }

    for p in prompts_dir_candidates() {
        if let Some(pair) = read_pair(&p) {
            return Some(pair);
        }
    }
    None
}

/// `agent_id` must be a single path segment (e.g. `main`, `code-assistant`) to load `agents/<id>.md`.
fn sanitize_agent_prompt_filename(agent_id: &str) -> Option<&str> {
    if agent_id.is_empty() || agent_id.contains("..") {
        return None;
    }
    if !agent_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some(agent_id)
}

fn try_load_agent_role_prompt_from_filesystem(agent_id: &str) -> Option<String> {
    let key = sanitize_agent_prompt_filename(agent_id)?;
    for root in prompts_dir_candidates() {
        let path = root.join("agents").join(format!("{key}.md"));
        if let Ok(s) = std::fs::read_to_string(&path) {
            if !s.trim().is_empty() {
                return Some(s);
            }
        }
    }
    None
}

/// Embedded per-role instructions when no workspace and no `prompts/agents/` on disk.
fn embedded_agent_role_prompt(agent_id: &str) -> Option<&'static str> {
    match agent_id {
        "main" => Some(include_str!("../../../prompts/agents/main.md")),
        "general-assistant" => Some(include_str!("../../../prompts/agents/general-assistant.md")),
        "code-assistant" => Some(include_str!("../../../prompts/agents/code-assistant.md")),
        "code-reviewer" => Some(include_str!("../../../prompts/agents/code-reviewer.md")),
        "devops" => Some(include_str!("../../../prompts/agents/devops.md")),
        "data-analyst" => Some(include_str!("../../../prompts/agents/data-analyst.md")),
        "writing" => Some(include_str!("../../../prompts/agents/writing.md")),
        "research" => Some(include_str!("../../../prompts/agents/research.md")),
        "qa-tester" => Some(include_str!("../../../prompts/agents/qa-tester.md")),
        "product-manager" => Some(include_str!("../../../prompts/agents/product-manager.md")),
        "security-auditor" => Some(include_str!("../../../prompts/agents/security-auditor.md")),
        "tutor" => Some(include_str!("../../../prompts/agents/tutor.md")),
        "api-builder" => Some(include_str!("../../../prompts/agents/api-builder.md")),
        "customer-support" => Some(include_str!("../../../prompts/agents/customer-support.md")),
        _ => None,
    }
}

/// Optional role layer from `prompts/agents/<agent_id>.md` (filesystem first, else embedded for known ids).
/// Result is cached per agent_id after first load.
pub fn resolve_agent_role_prompt(agent_id: &str) -> Option<String> {
    let cache = role_prompt_cache();
    if let Ok(guard) = cache.read() {
        if let Some(cached) = guard.get(agent_id) {
            return cached.clone();
        }
    }
    let result = try_load_agent_role_prompt_from_filesystem(agent_id)
        .or_else(|| embedded_agent_role_prompt(agent_id).map(str::to_string));
    if let Ok(mut guard) = cache.write() {
        guard.insert(agent_id.to_string(), result.clone());
    }
    result
}

/// Set the global skill prompt mode so that [`default_runtime_system_prompt`] can strip
/// the `## Skills` section from the tool usage guide when skill tools are not registered
/// (i.e. when `prompt_mode` is `Full`). Call once during gateway/runtime initialisation.
pub fn set_skill_prompt_mode(mode: crate::config::SkillPromptMode) {
    let _ = SKILL_PROMPT_MODE.set(mode);
}

/// Strip a Markdown `## <heading>` section (including its body, up to the next `## ` or EOF).
fn strip_md_section(text: &str, heading: &str) -> String {
    let marker = format!("## {heading}");
    let Some(start) = text.find(&marker) else {
        return text.to_string();
    };
    let after_marker = start + marker.len();
    let end = text[after_marker..]
        .find("\n## ")
        .map(|offset| after_marker + offset)
        .unwrap_or(text.len());
    let mut out = text[..start].trim_end().to_string();
    let tail = text[end..].trim_start_matches('\n');
    if !tail.is_empty() {
        out.push_str("\n\n");
        out.push_str(tail);
    }
    out
}

/// Apply prompt-mode filtering to the tool usage guide text.
fn filter_tool_guide(guide: &str) -> String {
    let mode = SKILL_PROMPT_MODE
        .get()
        .unwrap_or(&crate::config::SkillPromptMode::Full);
    match mode {
        crate::config::SkillPromptMode::Full => strip_md_section(guide, "Skills"),
        _ => guide.to_string(),
    }
}

/// Default system message when an agent has no `systemPrompt` in config: base + tool guide.
/// Cached after first load to avoid repeated filesystem reads.
/// When skill prompt mode is `Full`, the `## Skills` section in the tool guide is stripped
/// because those tools are not registered (skill content is injected directly).
pub fn default_runtime_system_prompt() -> String {
    let cached = CACHED_BASE_PROMPTS.get_or_init(|| {
        try_load_prompts_from_filesystem().map(|(base, tools)| {
            let filtered = filter_tool_guide(&tools);
            format!("{}\n\n{}", base.trim_end(), filtered.trim_end())
        })
    });
    match cached {
        Some(s) => s.clone(),
        None => {
            let filtered = filter_tool_guide(EMBEDDED_TOOL_USAGE_GUIDE);
            format!(
                "{}\n\n{}",
                EMBEDDED_SYSTEM_BASE_PROMPT.trim_end(),
                filtered.trim_end()
            )
        }
    }
}

/// Same as [`default_runtime_system_prompt`] plus optional `prompts/agents/<agent_id>.md` role layer.
/// Role prompts are cached per agent_id after first load.
pub fn default_runtime_system_prompt_for_agent(agent_id: &str) -> String {
    let mut body = default_runtime_system_prompt();
    if let Some(role) = resolve_agent_role_prompt(agent_id) {
        let r = role.trim();
        if !r.is_empty() {
            body.push_str("\n\n---\n\n");
            body.push_str(r);
        }
    }
    body
}

/// Invalidate cached prompts so they are re-read from disk on next access.
pub fn invalidate_prompt_cache() {
    if let Ok(mut guard) = role_prompt_cache().write() {
        guard.clear();
    }
}

/// Resolve the workspace root for a given agent.
pub fn resolve_workspace_root(
    state_dir: &Path,
    agent_id: &str,
    explicit_workspace: Option<&Path>,
) -> PathBuf {
    if let Some(ws) = explicit_workspace {
        return ws.to_path_buf();
    }
    if agent_id == "main" {
        state_dir.join("workspace")
    } else {
        state_dir.join(format!("workspace-{}", agent_id))
    }
}

/// Project-root marker files, ordered by priority (highest first).
///
/// `.xiaolin/` is the strongest signal; `.git/` is the de-facto standard for
/// version-controlled projects; language-specific manifest files act as fallback.
const ROOT_MARKERS_HIGH: &[&str] = &[".git"];
const XIAOLIN_DIR_REQUIRED_FILES: &[&str] = &["mcp.json", "config.json"];
const ROOT_MARKERS_LANG: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "Makefile",
    ".hg",
    ".svn",
];

/// Walk up from `start` looking for project-root markers.
///
/// Priority: `.xiaolin/` > `.git/` > language manifests.
/// If nothing is found, returns `start` unchanged.
pub fn detect_workspace_root(start: &Path) -> PathBuf {
    let start = start
        .canonicalize()
        .unwrap_or_else(|_| start.to_path_buf());

    let mut best: Option<(PathBuf, u8)> = None; // (path, priority) — lower is higher

    let high_count = ROOT_MARKERS_HIGH.len() as u8;
    let mut dir = start.as_path();
    loop {
        // Priority 0 (highest): .xiaolin/ with actual config files
        let xiaolin_dir = dir.join(".xiaolin");
        if xiaolin_dir.is_dir()
            && XIAOLIN_DIR_REQUIRED_FILES
                .iter()
                .any(|f| xiaolin_dir.join(f).exists())
        {
            return dir.to_path_buf();
        }

        // Priority 1+: .git and other high-priority markers
        for (prio, marker) in ROOT_MARKERS_HIGH.iter().enumerate() {
            if dir.join(marker).exists() {
                let p = prio as u8;
                if best.as_ref().is_none_or(|(_, bp)| p < *bp) {
                    best = Some((dir.to_path_buf(), p));
                }
            }
        }
        for marker in ROOT_MARKERS_LANG {
            if dir.join(marker).exists() && best.is_none() {
                best = Some((dir.to_path_buf(), high_count));
            }
        }
        // NOTE: we intentionally do NOT break when .git is found. We keep
        // climbing so that .xiaolin/ with config files (absolute highest
        // priority, triggers early `return` above) can be discovered at any
        // ancestor. The first .git is already recorded in `best` and won't be
        // replaced by a higher-level .git (same priority, not strictly less).
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent,
            _ => break,
        }
    }

    best.map(|(p, _)| p).unwrap_or(start)
}

/// Write a SKILL.md file into the global shared skills directory (`~/.xiaolin/skills/`).
pub fn write_global_skill(skill_id: &str, content: &str) -> anyhow::Result<PathBuf> {
    validate_skill_id(skill_id)?;
    let global_dir = crate::skill::resolve_global_skills_dir();
    let skill_dir = global_dir.join(skill_id);
    std::fs::create_dir_all(&skill_dir)?;
    let path = skill_dir.join("SKILL.md");
    std::fs::write(&path, content)?;
    tracing::info!(
        skill_id = %skill_id,
        path = %path.display(),
        "wrote skill to global directory"
    );
    Ok(path)
}

const DEFAULT_IDENTITY_TEMPLATE: &str = r#"# Who I Am

- **Name:** 小林
- **Creature:** AI assistant
- **Vibe:** 简洁直接，有判断力
- **Emoji:** 🌲
- **Avatar:**

## Style

简洁优先，必要时详尽。有自己的判断，不盲目附和。遇到问题先自己查，然后再问。

## Operating Rules

- In group chats, respond only when @mentioned
- Don't execute dangerous operations without confirmation
- Don't forward private messages to other conversations

---
_This file is yours to evolve. As you learn who you are, update it._
"#;

const DEFAULT_USER_TEMPLATE: &str = r#"# About Your User

_Get to know who you're helping. Update this over time._

- **Name:**
- **Preferred address:**
- **Timezone:**
- **Notes:**

## Background

_(What do they care about? What projects are they working on? Accumulate over time.)_
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_workspace_creates_identity_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = AgentWorkspace::new(tmp.path(), "test-agent");
        ws.ensure_workspace().unwrap();

        assert!(tmp.path().join(DEFAULT_IDENTITY_FILENAME).exists());
        assert!(tmp.path().join(DEFAULT_USER_FILENAME).exists());
        assert!(!tmp.path().join("BOOTSTRAP.md").exists());
        assert!(!tmp.path().join("SOUL.md").exists());
        assert!(!tmp.path().join("AGENTS.md").exists());
        assert!(!tmp.path().join("TOOLS.md").exists());
        assert!(!tmp.path().join("SYSTEM_BASE.md").exists());

        let identity =
            std::fs::read_to_string(tmp.path().join(DEFAULT_IDENTITY_FILENAME)).unwrap();
        let user = std::fs::read_to_string(tmp.path().join(DEFAULT_USER_FILENAME)).unwrap();
        assert!(!identity.trim().is_empty());
        assert!(!user.trim().is_empty());
    }

    #[test]
    fn load_identity_returns_workspace_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = AgentWorkspace::new(tmp.path(), "test-agent");
        ws.ensure_workspace().unwrap();

        let bs = ws.load_identity();
        assert!(bs.identity.is_some());
        assert!(bs.user.is_some());
    }

    #[test]
    fn detect_workspace_root_ignores_bare_xiaolin_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let sub = tmp.path().join("sub");
        std::fs::create_dir_all(sub.join(".xiaolin/agents")).unwrap();
        std::fs::create_dir_all(sub.join(".git")).unwrap();

        let result = detect_workspace_root(&sub);
        assert_eq!(result, sub, "bare .xiaolin/ (no config files) should NOT be highest priority; .git should win");
    }

    #[test]
    fn detect_workspace_root_finds_xiaolin_with_mcp_json() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("project");
        let deep = root.join("src/app");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::create_dir_all(root.join(".xiaolin")).unwrap();
        std::fs::write(root.join(".xiaolin/mcp.json"), "{}").unwrap();

        let result = detect_workspace_root(&deep);
        assert_eq!(
            result.canonicalize().unwrap(),
            root.canonicalize().unwrap(),
            ".xiaolin/ with mcp.json should be detected as project root"
        );
    }

    #[test]
    fn detect_workspace_root_prefers_xiaolin_config_over_git() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("project");
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir_all(sub.join(".git")).unwrap();
        std::fs::create_dir_all(root.join(".xiaolin")).unwrap();
        std::fs::write(root.join(".xiaolin/config.json"), "{}").unwrap();

        let result = detect_workspace_root(&sub);
        assert_eq!(
            result.canonicalize().unwrap(),
            root.canonicalize().unwrap(),
            ".xiaolin/ with config.json at parent should be preferred over .git in child"
        );
    }

    #[test]
    fn detect_workspace_root_falls_back_to_git_when_no_xiaolin() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("project");
        let deep = root.join("src/deep/nested");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]").unwrap();

        let result = detect_workspace_root(&deep);
        assert_eq!(
            result.canonicalize().unwrap(),
            root.canonicalize().unwrap(),
            "should fall back to .git when no .xiaolin/config"
        );
    }
}
