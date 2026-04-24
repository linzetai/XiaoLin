use std::path::{Path, PathBuf};

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

pub const DEFAULT_SOUL_FILENAME: &str = "SOUL.md";
pub const DEFAULT_USER_FILENAME: &str = "USER.md";
pub const DEFAULT_AGENTS_FILENAME: &str = "AGENTS.md";
pub const DEFAULT_MEMORY_FILENAME: &str = "MEMORY.md";
pub const DEFAULT_TOOLS_FILENAME: &str = "TOOLS.md";
/// Workspace copy of the repo `prompts/system-base.md` (created by [`AgentWorkspace::ensure_bootstrap`]).
pub const DEFAULT_SYSTEM_BASE_FILENAME: &str = "SYSTEM_BASE.md";
/// Filenames under the repo `prompts/` directory (see `FASTCLAW_PROMPTS_DIR` or cwd `./prompts`).
pub const PROMPTS_REPO_SYSTEM_BASE: &str = "system-base.md";
pub const PROMPTS_REPO_TOOL_USAGE_GUIDE: &str = "tool-usage-guide.md";

/// Embedded copy of `prompts/system-base.md` (used when no workspace / compile-time default).
pub const EMBEDDED_SYSTEM_BASE_PROMPT: &str = include_str!("../../../prompts/system-base.md");
/// Embedded copy of `prompts/tool-usage-guide.md`.
pub const EMBEDDED_TOOL_USAGE_GUIDE: &str = include_str!("../../../prompts/tool-usage-guide.md");
/// Canonical name for the tool usage guide text (embedded, or read from [`PROMPTS_REPO_TOOL_USAGE_GUIDE`]).
pub const DEFAULT_TOOL_USAGE_GUIDE: &str = EMBEDDED_TOOL_USAGE_GUIDE;

const BOOTSTRAP_FILES: &[&str] = &[
    DEFAULT_SYSTEM_BASE_FILENAME,
    DEFAULT_AGENTS_FILENAME,
    DEFAULT_SOUL_FILENAME,
    DEFAULT_USER_FILENAME,
    DEFAULT_MEMORY_FILENAME,
    DEFAULT_TOOLS_FILENAME,
];

/// Prompt context ordering — lower numbers appear first in the system prompt.
const CONTEXT_FILE_ORDER: &[(&str, u32)] = &[
    ("system_base.md", 5),
    ("agents.md", 10),
    ("soul.md", 20),
    ("user.md", 40),
    ("tools.md", 50),
    ("memory.md", 60),
];

/// Loaded bootstrap files for an agent workspace.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceBootstrap {
    pub system_base: Option<String>,
    pub soul: Option<String>,
    pub user: Option<String>,
    pub agents: Option<String>,
    pub memory: Option<String>,
    pub tools: Option<String>,
    pub extras: Vec<(String, String)>,
}

impl WorkspaceBootstrap {
    fn context_priority(filename: &str) -> u32 {
        CONTEXT_FILE_ORDER
            .iter()
            .find(|(name, _)| *name == filename)
            .map(|(_, p)| *p)
            .unwrap_or(100)
    }

    /// Format all bootstrap content for system prompt injection, ordered by priority.
    pub fn format_for_prompt(&self) -> String {
        let mut sections: Vec<(u32, &str, &str)> = Vec::new();

        if let Some(ref system_base) = self.system_base {
            sections.push((
                Self::context_priority("system_base.md"),
                "System Base",
                system_base,
            ));
        }
        if let Some(ref agents) = self.agents {
            sections.push((
                Self::context_priority("agents.md"),
                "Operating Rules",
                agents,
            ));
        }
        if let Some(ref soul) = self.soul {
            sections.push((Self::context_priority("soul.md"), "Personality", soul));
        }
        if let Some(ref user) = self.user {
            sections.push((
                Self::context_priority("user.md"),
                "User Context",
                user,
            ));
        }
        if let Some(ref tools) = self.tools {
            sections.push((Self::context_priority("tools.md"), "Tool Usage", tools));
        }
        if let Some(ref memory) = self.memory {
            sections.push((Self::context_priority("memory.md"), "Memory", memory));
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

    /// Load all bootstrap files from this workspace.
    pub fn load_bootstrap(&self) -> WorkspaceBootstrap {
        let mut bs = WorkspaceBootstrap::default();

        for &fname in BOOTSTRAP_FILES {
            let content = self.read_file(fname);
            match fname {
                f if f == DEFAULT_SYSTEM_BASE_FILENAME => bs.system_base = content,
                f if f == DEFAULT_SOUL_FILENAME => bs.soul = content,
                f if f == DEFAULT_USER_FILENAME => bs.user = content,
                f if f == DEFAULT_AGENTS_FILENAME => bs.agents = content,
                f if f == DEFAULT_MEMORY_FILENAME => bs.memory = content,
                f if f == DEFAULT_TOOLS_FILENAME => bs.tools = content,
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
        let canon_root = self.root.canonicalize().unwrap_or_else(|_| self.root.clone());
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

    /// Initialize the workspace with default bootstrap files if they don't exist.
    pub fn ensure_bootstrap(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.root)?;

        let (bootstrap_system_base, bootstrap_tools) = try_load_prompts_from_filesystem()
            .unwrap_or_else(|| {
                (
                    EMBEDDED_SYSTEM_BASE_PROMPT.to_string(),
                    EMBEDDED_TOOL_USAGE_GUIDE.to_string(),
                )
            });

        let system_base_path = self.root.join(DEFAULT_SYSTEM_BASE_FILENAME);
        if !system_base_path.exists() {
            std::fs::write(&system_base_path, &bootstrap_system_base)?;
            tracing::info!(path = %system_base_path.display(), "created default SYSTEM_BASE.md");
        }

        let soul_path = self.root.join(DEFAULT_SOUL_FILENAME);
        if !soul_path.exists() {
            std::fs::write(&soul_path, DEFAULT_SOUL_TEMPLATE)?;
            tracing::info!(path = %soul_path.display(), "created default SOUL.md");
        }

        let user_path = self.root.join(DEFAULT_USER_FILENAME);
        if !user_path.exists() {
            std::fs::write(&user_path, DEFAULT_USER_TEMPLATE)?;
            tracing::info!(path = %user_path.display(), "created default USER.md");
        }

        let agents_path = self.root.join(DEFAULT_AGENTS_FILENAME);
        if !agents_path.exists() {
            std::fs::write(&agents_path, DEFAULT_AGENTS_TEMPLATE)?;
            tracing::info!(path = %agents_path.display(), "created default AGENTS.md");
        }

        let tools_path = self.root.join(DEFAULT_TOOLS_FILENAME);
        if !tools_path.exists() {
            std::fs::write(&tools_path, &bootstrap_tools)?;
            tracing::info!(path = %tools_path.display(), "created default TOOLS.md");
        }

        Ok(())
    }
}

/// Candidate roots for repo-style `prompts/` (see `FASTCLAW_PROMPTS_DIR` or `./prompts` under cwd).
fn prompts_dir_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(dir) = std::env::var("FASTCLAW_PROMPTS_DIR") {
        out.push(PathBuf::from(dir));
    }
    if let Ok(cwd) = std::env::current_dir() {
        out.push(cwd.join("prompts"));
    }
    out
}

/// Try to read [`PROMPTS_REPO_SYSTEM_BASE`] and [`PROMPTS_REPO_TOOL_USAGE_GUIDE`] from `FASTCLAW_PROMPTS_DIR` or `./prompts` (cwd).
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
pub fn resolve_agent_role_prompt(agent_id: &str) -> Option<String> {
    try_load_agent_role_prompt_from_filesystem(agent_id)
        .or_else(|| embedded_agent_role_prompt(agent_id).map(str::to_string))
}

/// Default system message when an agent has no `systemPrompt` in config: base + tool guide.
/// Prefers live files from `FASTCLAW_PROMPTS_DIR` or `./prompts`, else embedded copies from the build.
pub fn default_runtime_system_prompt() -> String {
    if let Some((base, tools)) = try_load_prompts_from_filesystem() {
        format!("{}\n\n{}", base.trim_end(), tools.trim_end())
    } else {
        format!(
            "{}\n\n{}",
            EMBEDDED_SYSTEM_BASE_PROMPT.trim_end(),
            EMBEDDED_TOOL_USAGE_GUIDE.trim_end()
        )
    }
}

/// Same as [`default_runtime_system_prompt`] plus optional `prompts/agents/<agent_id>.md` role layer.
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

/// Write a SKILL.md file into the global shared skills directory (`~/.fastclaw/skills/`).
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

const DEFAULT_SOUL_TEMPLATE: &str = r#"# SOUL — personality layer

You are an AI assistant powered by FastClaw; full engineering defaults (principles, tool rules, anti-patterns) live in **`prompts/system-base.md`**, and the detailed tool decision tree in **`prompts/tool-usage-guide.md`**.

All agents inherit those layers from the repo (workspace copies: **`SYSTEM_BASE.md`**, **`TOOLS.md`**). [`AgentWorkspace::ensure_bootstrap`] materializes them when missing. At runtime, if the agent has no `systemPrompt` in config, [`default_runtime_system_prompt_for_agent`] injects base + tool guide (live `FASTCLAW_PROMPTS_DIR` / `./prompts` when readable, else the embedded build copy from `include_str!`).

## 你是谁

有立场的智能体，不是客套聊天机器人。

## 风格

**真正有用，而不是表演有用。** 简洁与深入随任务切换；有主见，但用证据说话。

## 与用户协作

先尝试解决，卡住再提问；重要假设要说明白。

---
_随你更了解自己与用户，更新本文件。_
"#;

const DEFAULT_USER_TEMPLATE: &str = r#"# USER.md - 关于你的用户

_了解你帮助的人。随时间更新这个文件。_

- **名字:**
- **称呼:**
- **时区:**
- **备注:**

## 背景

_(他们关心什么？在做什么项目？随时间积累。)_
"#;

const DEFAULT_AGENTS_TEMPLATE: &str = r#"# AGENTS.md - 运行规则

## 工具使用

- 优先使用现有工具完成任务
- 工具调用失败时，尝试替代方案而非直接报错

## 记忆管理

- 用户说"记住"/"remember"/"别忘了"时，立即调用 memory_store
- 学到用户偏好、项目规则、架构决策时，主动存储为 fact
- 对话结束前，将关键结论和决策存储为 episode
- 回答历史相关问题前，先 memory_search 查询
- 禁止存储密码、密钥、token 等敏感信息

## 消息规范

- 群聊中被 @提及 时才回复
- 私聊中始终回复
- 回复保持简洁，切中要害

## 安全边界

- 不执行未经确认的危险操作
- 不转发私密消息到其他会话
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_bootstrap_creates_identity_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = AgentWorkspace::new(tmp.path(), "test-agent");
        ws.ensure_bootstrap().unwrap();

        assert!(tmp.path().join(DEFAULT_SOUL_FILENAME).exists());
        assert!(tmp.path().join(DEFAULT_USER_FILENAME).exists());
        assert!(tmp.path().join(DEFAULT_AGENTS_FILENAME).exists());

        let soul = std::fs::read_to_string(tmp.path().join(DEFAULT_SOUL_FILENAME)).unwrap();
        let user = std::fs::read_to_string(tmp.path().join(DEFAULT_USER_FILENAME)).unwrap();
        let agents = std::fs::read_to_string(tmp.path().join(DEFAULT_AGENTS_FILENAME)).unwrap();
        assert!(!soul.trim().is_empty());
        assert!(!user.trim().is_empty());
        assert!(!agents.trim().is_empty());
    }
}
