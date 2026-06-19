use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use xiaolin_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolResult};
use xiaolin_core::workspace::AgentWorkspace;

// --- Skill tools for lazy/compact modes ---

/// List all available skills with their names and descriptions.
pub struct ListSkillsTool {
    registry: Arc<xiaolin_core::skill::SkillRegistry>,
}

impl ListSkillsTool {
    pub fn new(registry: Arc<xiaolin_core::skill::SkillRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for ListSkillsTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }

    fn name(&self) -> &str {
        "list_skills"
    }

    fn description(&self) -> &str {
        "Return JSON listing enabled skills: id, name, short description, and tags from each SKILL.md frontmatter. \
         Call list_skills before read_skill whenever procedures might live outside the base prompt, or when the user references a workflow by nickname and you need the exact id. \
         Disabled entries are hidden; layering (project vs global) is already merged by the host—treat this output as authoritative for ids. \
         Metadata only—always follow with read_skill to fetch full Markdown and tool hooks. \
         Anti-pattern: inventing ids; anti-pattern: assuming skills exist when count is zero (author with write_skill first). \
         No parameters—pass {}."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: HashMap::new(),
            required: Vec::new(),
        }
    }

    async fn execute(&self, _arguments: &str) -> ToolResult {
        let skills: Vec<serde_json::Value> = self
            .registry
            .list()
            .iter()
            .filter(|s| s.frontmatter.enabled.unwrap_or(true))
            .map(|s| {
                serde_json::json!({
                    "id": s.id,
                    "name": s.name,
                    "description": s.description,
                    "tags": s.frontmatter.tags,
                })
            })
            .collect();

        ToolResult::ok(
            serde_json::json!({
                "skills": skills,
                "count": skills.len(),
            })
            .to_string(),
        )
    }
}

/// Read the full content of a specific skill by ID.
pub struct ReadSkillTool {
    registry: Arc<xiaolin_core::skill::SkillRegistry>,
    usage_store: Option<Arc<xiaolin_core::skill_usage::SkillUsageStore>>,
}

impl ReadSkillTool {
    pub fn new(registry: Arc<xiaolin_core::skill::SkillRegistry>) -> Self {
        Self {
            registry,
            usage_store: None,
        }
    }

    pub fn with_usage_store(mut self, store: Arc<xiaolin_core::skill_usage::SkillUsageStore>) -> Self {
        self.usage_store = Some(store);
        self
    }
}

#[async_trait]
impl Tool for ReadSkillTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }

    fn name(&self) -> &str {
        "read_skill"
    }

    fn description(&self) -> &str {
        "Fetch one skill by id: full Markdown body plus name, description, and declared tools from frontmatter. \
         After list_skills, call read_skill when tags/name/description match the task; obey the Markdown unless the user explicitly contradicts it. \
         Skills are runbooks, not secret storage—do not treat them as permission to leak credentials; still follow safety policy. \
         On skill not found, re-list and copy ids verbatim (often path-like, e.g. 'wps365-skills/drive'). \
         Anti-pattern: guessing ids without list_skills. \
         Example: {\"skill_id\": \"greeting\"}."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "skill_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Registry id copied from list_skills output (field 'id'). Examples: 'greeting', 'wps365-skills/drive'. Case and punctuation must match exactly—ids are not fuzzy-searchable through this tool."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["skill_id".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "read_skill: arguments are not valid JSON: {e}. \
                 Pass exactly one JSON object, e.g. {{\"skill_id\": \"greeting\"}} with double-quoted keys, then retry."
            )),
        };

        let skill_id = match args.get("skill_id").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return ToolResult::err(
                    "read_skill is missing required string field 'skill_id'. \
                 Example: {\"skill_id\": \"greeting\"}. \
                 Call list_skills first if you do not know valid ids."
                        .to_string(),
                )
            }
        };

        match self.registry.get(skill_id) {
            Some(skill) => {
                if let Some(store) = &self.usage_store {
                    let store = store.clone();
                    let sid = skill_id.to_string();
                    tokio::spawn(async move {
                        if let Err(e) = store.record(
                            &sid,
                            xiaolin_core::skill_usage::UsageEventType::Read,
                            None,
                        ).await {
                            tracing::warn!(skill_id = %sid, error = %e, "failed to record skill read event");
                        }
                    });
                }
                ToolResult::ok(
                    serde_json::json!({
                        "id": skill.id,
                        "name": skill.name,
                        "description": skill.description,
                        "content": skill.content,
                        "tools": skill.frontmatter.tools,
                    })
                    .to_string(),
                )
            }
            None => ToolResult::err(format!(
                "read_skill: skill not found for id '{skill_id}'. \
                 What went wrong: no enabled skill matches that exact id string (typo, disabled entry, or not registered yet). \
                 What to do next: run list_skills and paste an 'id' field exactly as returned (case and slashes matter); if the user just added a file, ensure the host registered it or use write_skill then reload per operator docs."
            )),
        }
    }
}

/// Create or update a skill. Writes to agent workspace by default, project, or global directory.
pub struct WriteSkillTool {
    workspace: Arc<AgentWorkspace>,
    workspace_root: Option<std::path::PathBuf>,
}

impl WriteSkillTool {
    pub fn new(workspace: Arc<AgentWorkspace>) -> Self {
        Self {
            workspace,
            workspace_root: None,
        }
    }

    pub fn with_workspace_root(mut self, root: std::path::PathBuf) -> Self {
        self.workspace_root = Some(root);
        self
    }
}

#[async_trait]
impl Tool for WriteSkillTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }

    fn name(&self) -> &str {
        "write_skill"
    }

    fn description(&self) -> &str {
        "Create or overwrite SKILL.md for a skill_id with the full Markdown document (optional YAML frontmatter included). \
         Targets: 'workspace' (default) writes under agent-private tree; 'project' writes to <workspace>/.xiaolin/skills/ (shared within this project, preferred for /skillify); \
         'global' writes to ~/.xiaolin/skills/ (shared across all projects). \
         Whole-file replace—identical contract to write_file: partial bodies delete the rest of the file. \
         Anti-pattern: embedding API keys or tokens—reference env-based configuration instead. \
         Example: {\"skill_id\": \"deploy-checklist\", \"content\": \"---\\nname: Deploy\\ntags: [ops]\\n---\\n# Steps\\n1. ...\", \"target\": \"project\"}."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "skill_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Stable id for the skill folder (e.g. 'my-search-skill'). Becomes the directory name under skills/; only [a-zA-Z0-9._-] allowed."
            }),
        );
        props.insert(
            "content".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Entire SKILL.md file as one JSON string: optional YAML frontmatter (--- blocks) for name, tags, tools, then Markdown instructions. Escape newlines as \\n. This overwrites any existing file for the same skill_id and target."
            }),
        );
        props.insert(
            "target".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["workspace", "project", "global"],
                "description": "'workspace' (default): agent-private; 'project': <workspace>/.xiaolin/skills/ (recommended for skillify); 'global': ~/.xiaolin/skills/."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["skill_id".to_string(), "content".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "write_skill: arguments are not valid JSON: {e}. \
                 Pass {{\"skill_id\": \"my-skill\", \"content\": \"...full SKILL.md...\", \"target\": \"workspace\"}} with double-quoted strings, then retry."
            )),
        };

        let skill_id = match args.get("skill_id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => {
                return ToolResult::err(
                    "write_skill is missing or empty required string field 'skill_id'. \
                 Example: {\"skill_id\": \"my-checklist\", \"content\": \"# My Skill\\n...\"}. \
                 Pick a stable id; it becomes the on-disk folder name under skills/."
                        .to_string(),
                )
            }
        };

        let content = match args.get("content").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return ToolResult::err(
                "write_skill is missing or empty required string field 'content'. \
                 Send the entire SKILL.md body as one JSON string (use \\n for newlines), not a patch or excerpt—empty content would delete instructions and is rejected."
                    .to_string(),
            ),
        };

        let target = args
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or("workspace");

        let result = match target {
            "project" => {
                if let Some(ref root) = self.workspace_root {
                    xiaolin_core::workspace::write_project_skill(root, skill_id, content)
                } else {
                    return ToolResult::err(
                        "write_skill target 'project' requires a detected workspace root, \
                         but none is available. Use target 'workspace' or 'global' instead."
                            .to_string(),
                    );
                }
            }
            "global" => xiaolin_core::workspace::write_global_skill(skill_id, content),
            _ => self.workspace.write_skill(skill_id, content),
        };

        match result {
            Ok(path) => ToolResult::ok(
                serde_json::json!({
                    "status": "ok",
                    "skill_id": skill_id,
                    "target": target,
                    "path": path.display().to_string(),
                })
                .to_string(),
            ),
            Err(e) => ToolResult::err(format!(
                "write_skill could not persist SKILL.md for id '{skill_id}' (target '{target}'): {e}. \
                 What to do next: verify the workspace or global skills directory is writable, disk is not full, paths are not blocked by policy, then retry; if errors mention permission denied, ask the operator to fix ownership or choose target 'workspace' under an agent path you control."
            )),
        }
    }
}

// ─── Unified Skill Tool ──────────────────────────────────────────────

/// Single tool that combines list, read, write, and search skill operations.
pub struct UnifiedSkillTool {
    list: ListSkillsTool,
    read: ReadSkillTool,
    write: Option<WriteSkillTool>,
    search: SearchSkillTool,
    reload_callback: Option<Arc<dyn Fn() -> anyhow::Result<()> + Send + Sync>>,
}

impl UnifiedSkillTool {
    pub fn new(
        registry: Arc<xiaolin_core::skill::SkillRegistry>,
        workspace: Option<Arc<AgentWorkspace>>,
    ) -> Self {
        Self {
            list: ListSkillsTool::new(registry.clone()),
            read: ReadSkillTool::new(registry.clone()),
            search: SearchSkillTool::new(registry.clone()),
            write: workspace.map(WriteSkillTool::new),
            reload_callback: None,
        }
    }

    pub fn with_workspace_root(mut self, root: std::path::PathBuf) -> Self {
        if let Some(ref mut w) = self.write {
            w.workspace_root = Some(root);
        }
        self
    }

    pub fn with_semantic(
        mut self,
        store: Arc<xiaolin_core::skill_embedding::SkillEmbeddingStore>,
        provider: Arc<dyn xiaolin_memory::EmbeddingProvider>,
    ) -> Self {
        self.search = self.search.with_semantic(store, provider);
        self
    }

    pub fn with_usage_store(mut self, store: Arc<xiaolin_core::skill_usage::SkillUsageStore>) -> Self {
        self.read = self.read.with_usage_store(store);
        self
    }

    pub fn readonly(registry: Arc<xiaolin_core::skill::SkillRegistry>) -> Self {
        Self {
            list: ListSkillsTool::new(registry.clone()),
            read: ReadSkillTool::new(registry.clone()),
            search: SearchSkillTool::new(registry.clone()),
            write: None,
            reload_callback: None,
        }
    }

    pub fn with_reload_callback(
        mut self,
        callback: Arc<dyn Fn() -> anyhow::Result<()> + Send + Sync>,
    ) -> Self {
        self.reload_callback = Some(callback);
        self
    }
}

#[async_trait]
impl Tool for UnifiedSkillTool {
    fn name(&self) -> &str {
        "skill"
    }

    fn description(&self) -> &str {
        "Manage agent skills: list available skills, read a skill's full content, search by keyword, or write/update a skill. \
         Actions: 'list' (no params needed), 'read' (requires skill_id), 'search' (requires query), 'write' (requires skill_id + content). \
         Always list or search before read to get valid ids. Skills are runbooks with instructions—obey them unless the user explicitly contradicts."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert("action".to_string(), serde_json::json!({
            "type": "string",
            "enum": ["list", "read", "search", "write"],
            "description": "list: return all enabled skills; read: fetch one skill by id; search: find skills by keyword; write: create/overwrite a SKILL.md."
        }));
        props.insert(
            "skill_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Required for read and write. Copy exactly from list output."
            }),
        );
        props.insert(
            "query".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Required for search. Keywords matched against name, description, tags, content."
            }),
        );
        props.insert(
            "tag".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional for search. Only return skills with this tag."
            }),
        );
        props.insert(
            "content".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Required for write. Full SKILL.md content (use \\n for newlines)."
            }),
        );
        props.insert(
            "target".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["workspace", "project", "global"],
                "description": "For write only. 'workspace' (default), 'project' (<workspace>/.xiaolin/skills/), or 'global'."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["action".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("skill: invalid JSON: {e}")),
        };

        let action = match args.get("action").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => {
                return ToolResult::err(
                    "skill requires 'action': 'list', 'read', 'search', or 'write'.".to_string(),
                )
            }
        };

        match action {
            "list" => self.list.execute("{}").await,
            "read" => {
                let inner = serde_json::json!({
                    "skill_id": args.get("skill_id")
                })
                .to_string();
                self.read.execute(&inner).await
            }
            "search" => {
                let inner = serde_json::json!({
                    "query": args.get("query"),
                    "tag": args.get("tag"),
                })
                .to_string();
                self.search.execute(&inner).await
            }
            "write" => match &self.write {
                Some(w) => {
                    let result = w.execute(arguments).await;
                    if result.success {
                        if let Some(ref cb) = self.reload_callback {
                            if let Err(e) = cb() {
                                tracing::warn!(error = %e, "skill reload callback failed after write");
                            }
                        }
                    }
                    result
                }
                None => {
                    ToolResult::err("skill write is not available in read-only mode.".to_string())
                }
            },
            other => ToolResult::err(format!(
                "skill: unknown action '{other}'. Use 'list', 'read', 'search', or 'write'."
            )),
        }
    }
}

// ── Skillify Prompt ──────────────────────────────────────────────────

/// System prompt injected when the user invokes `/skillify`.
/// Instructs the LLM to analyze recent conversation and generate a SKILL.md.
pub const SKILLIFY_PROMPT: &str = r#"
## Skillify — Convert This Conversation Into a Reusable Skill

You are now in **skillify mode**. Your task is to analyze the conversation above and extract a reusable skill from the patterns you see.

### What to Extract

Look for:
1. **Multi-step procedures** the user asked you to perform (3+ steps)
2. **Tool usage patterns** — which tools were called, in what order, with what parameters
3. **Constraints and rules** — things the user corrected or specified ("always do X", "never do Y")
4. **Project-specific knowledge** — paths, configs, naming conventions unique to this codebase

### Output Format

Generate a complete SKILL.md with YAML frontmatter. The skill MUST follow this structure:

```
---
name: <Descriptive Name>
description: <One-line summary of when and why to use this skill>
tags: [<relevant>, <tags>]
tools: [<tools_used_in_the_procedure>]
paths: [<glob_patterns_if_project_type_specific>]
---

# <Title>

<Clear, actionable instructions that an agent can follow to reproduce the procedure.
Include concrete steps, expected outcomes, and error handling.>
```

### Rules

- `name` and `description` are REQUIRED in frontmatter
- `tools` should list only tools actually used in the procedure (omit if not specific)
- `paths` should include glob patterns if the skill only applies to certain file types (omit if universal)
- The body must contain **concrete, numbered steps** — not vague guidelines
- Include verification steps (how to confirm success)
- Include common failure modes and recovery if observed in the conversation

### Workflow

1. **Analyze** the conversation history above
2. **Draft** the complete SKILL.md content
3. **Present** it to the user in a code block for review
4. **Ask** the user to confirm, edit, or provide a skill_id name
5. **Save** using the `skill` tool: `action: write, skill_id: "<user-chosen-id>", target: "project", content: "<the SKILL.md>"`

IMPORTANT: Do NOT save automatically. Always show the draft first and wait for user confirmation.
"#;

// ── Search Skill Tool ────────────────────────────────────────────────

/// Search skills by keyword with optional tag filtering and semantic search.
pub struct SearchSkillTool {
    registry: Arc<xiaolin_core::skill::SkillRegistry>,
    embedding_store: Option<Arc<xiaolin_core::skill_embedding::SkillEmbeddingStore>>,
    embedding_provider: Option<Arc<dyn xiaolin_memory::EmbeddingProvider>>,
}

impl SearchSkillTool {
    pub fn new(registry: Arc<xiaolin_core::skill::SkillRegistry>) -> Self {
        Self {
            registry,
            embedding_store: None,
            embedding_provider: None,
        }
    }

    pub fn with_semantic(
        mut self,
        store: Arc<xiaolin_core::skill_embedding::SkillEmbeddingStore>,
        provider: Arc<dyn xiaolin_memory::EmbeddingProvider>,
    ) -> Self {
        self.embedding_store = Some(store);
        self.embedding_provider = Some(provider);
        self
    }

    /// Keyword-only search (synchronous).
    fn keyword_search(&self, query: &str, tag_filter: Option<&str>) -> Vec<SkillSearchResult> {
        let query_lower = query.to_lowercase();
        let keywords: Vec<&str> = query_lower.split_whitespace().collect();
        let cache = self.registry.lowercase_cache();

        let mut results: Vec<SkillSearchResult> = self
            .registry
            .list()
            .iter()
            .filter(|s| s.frontmatter.enabled.unwrap_or(true))
            .filter(|s| {
                if let Some(tag) = tag_filter {
                    s.frontmatter
                        .tags
                        .iter()
                        .any(|t| t.eq_ignore_ascii_case(tag))
                } else {
                    true
                }
            })
            .filter_map(|s| {
                let score = compute_relevance(&keywords, s, cache.get(&s.id));
                if score > 0.0 {
                    Some(SkillSearchResult {
                        id: s.id.clone(),
                        name: s.name.clone(),
                        description: s.description.clone().unwrap_or_default(),
                        score,
                    })
                } else {
                    None
                }
            })
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(10);
        results
    }

    /// Hybrid search: combine keyword scores with semantic similarity.
    async fn hybrid_search(&self, query: &str, tag_filter: Option<&str>) -> Vec<SkillSearchResult> {
        let keyword_results = self.keyword_search(query, tag_filter);

        let semantic_scores = self.semantic_search(query, 20).await;
        if semantic_scores.is_empty() {
            return keyword_results;
        }

        let semantic_map: std::collections::HashMap<&str, f32> =
            semantic_scores.iter().map(|(id, s)| (id.as_str(), *s)).collect();

        let registry_entries: std::collections::HashMap<&str, &xiaolin_core::skill::SkillEntry> =
            self.registry.list().iter().map(|s| (s.id.as_str(), *s)).collect();

        let mut combined: std::collections::HashMap<String, SkillSearchResult> =
            std::collections::HashMap::new();

        for r in &keyword_results {
            let sem_boost = semantic_map.get(r.id.as_str()).copied().unwrap_or(0.0);
            combined.insert(
                r.id.clone(),
                SkillSearchResult {
                    id: r.id.clone(),
                    name: r.name.clone(),
                    description: r.description.clone(),
                    score: r.score + (sem_boost as f64) * 5.0,
                },
            );
        }

        for (id, sim) in &semantic_scores {
            if combined.contains_key(id.as_str()) {
                continue;
            }
            if *sim < 0.3 {
                continue;
            }
            if let Some(entry) = registry_entries.get(id.as_str()) {
                if !entry.frontmatter.enabled.unwrap_or(true) {
                    continue;
                }
                if let Some(tag) = tag_filter {
                    if !entry.frontmatter.tags.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
                        continue;
                    }
                }
                combined.insert(
                    id.clone(),
                    SkillSearchResult {
                        id: id.clone(),
                        name: entry.name.clone(),
                        description: entry.description.clone().unwrap_or_default(),
                        score: (*sim as f64) * 5.0,
                    },
                );
            }
        }

        let mut results: Vec<SkillSearchResult> = combined.into_values().collect();
        results.sort_by(|a, b| {
            b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(10);
        results
    }

    /// Vector similarity search via embedding store.
    async fn semantic_search(&self, query: &str, limit: usize) -> Vec<(String, f32)> {
        let (Some(store), Some(provider)) = (&self.embedding_store, &self.embedding_provider) else {
            return Vec::new();
        };
        let query_vec = match provider.embed(query).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "semantic skill search: embed query failed, falling back to keyword-only");
                return Vec::new();
            }
        };
        match store.search_by_vector(&query_vec, limit).await {
            Ok(results) => results,
            Err(e) => {
                tracing::warn!(error = %e, "semantic skill search: vector search failed, falling back to keyword-only");
                Vec::new()
            }
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SkillSearchResult {
    pub id: String,
    pub name: String,
    pub description: String,
    pub score: f64,
}

fn compute_relevance(
    keywords: &[&str],
    skill: &xiaolin_core::skill::SkillEntry,
    cached: Option<&xiaolin_core::skill::CachedLowercase>,
) -> f64 {
    let mut score = 0.0;

    let fallback;
    let lc = match cached {
        Some(c) => c,
        None => {
            fallback = xiaolin_core::skill::CachedLowercase {
                name: skill.name.to_lowercase(),
                description: skill.description.as_deref().unwrap_or("").to_lowercase(),
                when_to_use: skill.frontmatter.when_to_use.as_deref().unwrap_or("").to_lowercase(),
                content: skill.content.to_lowercase(),
            };
            &fallback
        }
    };

    for kw in keywords {
        if lc.name.contains(kw) {
            score += 3.0;
        }
        if lc.description.contains(kw) {
            score += 2.0;
        }
        if !lc.when_to_use.is_empty() && lc.when_to_use.contains(kw) {
            score += 2.0;
        }
        if skill
            .frontmatter
            .tags
            .iter()
            .any(|t: &String| t.to_lowercase().contains(kw))
        {
            score += 2.5;
        }
        if lc.content.contains(kw) {
            score += 1.0;
        }
    }

    score
}

#[async_trait]
impl Tool for SearchSkillTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }

    fn name(&self) -> &str {
        "search_skills"
    }

    fn description(&self) -> &str {
        "Search skills by keyword query with optional tag filter. Returns ranked results \
         with relevance scores. Use before starting any multi-step task to check if a \
         matching skill already exists. Prefer this over list_skills when you have a \
         specific task in mind. Example: {\"query\": \"deploy backend\", \"tag\": \"ops\"}."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "query".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Search query (keywords matched against name, description, tags, content)"
            }),
        );
        props.insert(
            "tag".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional tag filter (only return skills with this tag)"
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["query".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("Invalid JSON: {e}")),
        };

        let query = match args.get("query").and_then(|v| v.as_str()) {
            Some(q) if !q.trim().is_empty() => q,
            _ => {
                return ToolResult::err(
                    "search_skills requires a non-empty 'query' string. \
                 Example: {\"query\": \"deploy\"}"
                        .to_string(),
                )
            }
        };
        let tag = args.get("tag").and_then(|v| v.as_str());

        let results = self.hybrid_search(query, tag).await;
        ToolResult::ok(
            serde_json::json!({
                "results": results,
                "count": results.len(),
            })
            .to_string(),
        )
    }
}

#[cfg(test)]
mod skill_tool_tests {
    use super::*;
    use crate::builtin_tools::{register_skill_tools, register_skill_tools_full};
    use xiaolin_core::skill::{SkillEntry, SkillFrontmatter, SkillLayer, SkillRegistry};
    use xiaolin_core::tool::ToolRegistry;
    use std::path::PathBuf;

    fn build_registry() -> Arc<SkillRegistry> {
        let mut reg = SkillRegistry::new();
        reg.register(SkillEntry {
            id: "greeting".into(),
            name: "Greeting Skill".into(),
            description: Some("Greet the user warmly.".into()),
            content: "Say hello in the user's preferred language.".into(),
            source_path: PathBuf::from("/fake/greeting/SKILL.md"),
            frontmatter: SkillFrontmatter {
                name: Some("Greeting Skill".into()),
                enabled: Some(true),
                tools: vec!["greet_user".into()],
                tags: vec!["social".into()],
                ..Default::default()
            },
            layer: SkillLayer::Project,
            source: None,
        });
        reg.register(SkillEntry {
            id: "calc".into(),
            name: "Calculator Skill".into(),
            description: Some("Perform arithmetic.".into()),
            content: "Use the calculator tool.".into(),
            source_path: PathBuf::from("/fake/calc/SKILL.md"),
            frontmatter: SkillFrontmatter {
                name: Some("Calculator Skill".into()),
                enabled: Some(true),
                tags: vec!["math".into()],
                ..Default::default()
            },
            layer: SkillLayer::Project,
            source: None,
        });
        reg.register(SkillEntry {
            id: "disabled-one".into(),
            name: "Disabled".into(),
            description: Some("Should not appear.".into()),
            content: "Hidden content.".into(),
            source_path: PathBuf::from("/fake/disabled/SKILL.md"),
            frontmatter: SkillFrontmatter {
                enabled: Some(false),
                ..Default::default()
            },
            layer: SkillLayer::Project,
            source: None,
        });
        Arc::new(reg)
    }

    // ── ListSkillsTool ─────────────────────────────────────────────

    #[tokio::test]
    async fn list_skills_returns_enabled_only() {
        let reg = build_registry();
        let tool = ListSkillsTool::new(reg);

        let result = tool.execute("{}").await;
        assert!(result.success);

        let json: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        let count = json["count"].as_u64().unwrap();
        assert_eq!(count, 2);

        let skills = json["skills"].as_array().unwrap();
        let ids: Vec<&str> = skills.iter().map(|s| s["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"greeting"));
        assert!(ids.contains(&"calc"));
        assert!(!ids.contains(&"disabled-one"));
    }

    #[tokio::test]
    async fn list_skills_includes_metadata() {
        let reg = build_registry();
        let tool = ListSkillsTool::new(reg);

        let result = tool.execute("{}").await;
        let json: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        let skills = json["skills"].as_array().unwrap();

        let greeting = skills.iter().find(|s| s["id"] == "greeting").unwrap();
        assert_eq!(greeting["name"], "Greeting Skill");
        assert_eq!(greeting["description"], "Greet the user warmly.");
        assert_eq!(greeting["tags"], serde_json::json!(["social"]));
    }

    #[tokio::test]
    async fn list_skills_empty_registry() {
        let reg = Arc::new(SkillRegistry::new());
        let tool = ListSkillsTool::new(reg);

        let result = tool.execute("{}").await;
        assert!(result.success);

        let json: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(json["count"].as_u64().unwrap(), 0);
        assert!(json["skills"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn list_skills_schema_has_no_required_params() {
        let reg = build_registry();
        let tool = ListSkillsTool::new(reg);

        let schema = tool.parameters_schema();
        assert!(schema.required.is_empty());
        assert!(schema.properties.is_empty());
    }

    // ── ReadSkillTool ──────────────────────────────────────────────

    #[tokio::test]
    async fn read_skill_found() {
        let reg = build_registry();
        let tool = ReadSkillTool::new(reg);

        let result = tool.execute(r#"{"skill_id": "greeting"}"#).await;
        assert!(result.success);

        let json: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(json["id"], "greeting");
        assert_eq!(json["name"], "Greeting Skill");
        assert_eq!(
            json["content"],
            "Say hello in the user's preferred language."
        );
        assert_eq!(json["tools"], serde_json::json!(["greet_user"]));
    }

    #[tokio::test]
    async fn read_skill_not_found() {
        let reg = build_registry();
        let tool = ReadSkillTool::new(reg);

        let result = tool.execute(r#"{"skill_id": "nonexistent"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("skill not found"));
        assert!(result.output.contains("nonexistent"));
    }

    #[tokio::test]
    async fn read_skill_missing_param() {
        let reg = build_registry();
        let tool = ReadSkillTool::new(reg);

        let result = tool.execute(r#"{}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("missing"));
    }

    #[tokio::test]
    async fn read_skill_invalid_json() {
        let reg = build_registry();
        let tool = ReadSkillTool::new(reg);

        let result = tool.execute("not json").await;
        assert!(!result.success);
        assert!(
            result.output.contains("JSON"),
            "expected JSON parse hint, got: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn read_skill_schema_requires_skill_id() {
        let reg = build_registry();
        let tool = ReadSkillTool::new(reg);

        let schema = tool.parameters_schema();
        assert_eq!(schema.required, vec!["skill_id"]);
        assert!(schema.properties.contains_key("skill_id"));
    }

    // ── register_skill_tools ───────────────────────────────────────

    #[test]
    fn register_adds_unified_skill_tool() {
        let reg = build_registry();
        let tool_reg = ToolRegistry::new();

        register_skill_tools(&tool_reg, reg);

        let defs = tool_reg.definitions();
        let names: Vec<&str> = defs.iter().map(|d| d.function.name.as_str()).collect();
        assert!(
            names.contains(&"skill"),
            "should register unified 'skill' tool, got: {:?}",
            names
        );
    }

    #[test]
    fn register_full_adds_unified_skill_tool() {
        let reg = build_registry();
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = Arc::new(xiaolin_core::workspace::AgentWorkspace::new(
            tmp.path(),
            "test-agent",
        ));
        let tool_reg = ToolRegistry::new();

        register_skill_tools_full(&tool_reg, reg, ws, None);

        let defs = tool_reg.definitions();
        let names: Vec<&str> = defs.iter().map(|d| d.function.name.as_str()).collect();
        assert!(
            names.contains(&"skill"),
            "should register unified 'skill' tool, got: {:?}",
            names
        );
    }

    // ── WriteSkillTool ─────────────────────────────────────────────

    #[tokio::test]
    async fn write_skill_to_workspace() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = Arc::new(xiaolin_core::workspace::AgentWorkspace::new(
            tmp.path(),
            "agent-x",
        ));
        let tool = WriteSkillTool::new(ws);

        let args = serde_json::json!({
            "skill_id": "my-skill",
            "content": "# My Skill\n\nDoes something useful."
        })
        .to_string();
        let result = tool.execute(&args).await;

        assert!(result.success, "execute failed: {}", result.output);
        let json: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(json["skill_id"], "my-skill");
        assert_eq!(json["target"], "workspace");
        assert_eq!(json["status"], "ok");

        let written =
            std::fs::read_to_string(tmp.path().join("skills").join("my-skill").join("SKILL.md"))
                .unwrap();
        assert!(written.contains("# My Skill"));
    }

    #[tokio::test]
    async fn write_skill_missing_skill_id() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = Arc::new(xiaolin_core::workspace::AgentWorkspace::new(
            tmp.path(),
            "agent-x",
        ));
        let tool = WriteSkillTool::new(ws);

        let result = tool.execute(r#"{"content": "stuff"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("skill_id"));
    }

    #[tokio::test]
    async fn write_skill_missing_content() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = Arc::new(xiaolin_core::workspace::AgentWorkspace::new(
            tmp.path(),
            "agent-x",
        ));
        let tool = WriteSkillTool::new(ws);

        let result = tool.execute(r#"{"skill_id": "x"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("content"));
    }

    #[tokio::test]
    async fn write_skill_schema_requires_id_and_content() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = Arc::new(xiaolin_core::workspace::AgentWorkspace::new(
            tmp.path(),
            "agent-x",
        ));
        let tool = WriteSkillTool::new(ws);

        let schema = tool.parameters_schema();
        assert!(schema.required.contains(&"skill_id".to_string()));
        assert!(schema.required.contains(&"content".to_string()));
        assert!(schema.properties.contains_key("target"));
    }

    // ── Search Skill Tests ──

    #[test]
    fn compute_relevance_name_scores_highest() {
        let skill = xiaolin_core::skill::SkillEntry {
            id: "deploy".into(),
            name: "Deploy Backend".into(),
            description: Some("Deploy the backend service".into()),
            content: "# Steps\n1. Run deploy script".into(),
            source_path: std::path::PathBuf::from("/tmp/skill.md"),
            frontmatter: xiaolin_core::skill::SkillFrontmatter {
                name: Some("Deploy Backend".into()),
                description: Some("Deploy the backend service".into()),
                tags: vec!["ops".into()],
                tools: Vec::new(),
                enabled: Some(true),
                ..Default::default()
            },
            layer: xiaolin_core::skill::SkillLayer::Project,
            source: None,
        };
        let keywords = vec!["deploy"];
        let score = compute_relevance(&keywords, &skill);
        assert!(
            score >= 6.0,
            "name + desc + content should score high: {score}"
        );
    }

    #[test]
    fn compute_relevance_no_match_returns_zero() {
        let skill = xiaolin_core::skill::SkillEntry {
            id: "greeting".into(),
            name: "Greeting".into(),
            description: Some("Say hello".into()),
            content: "# Hello\nWorld".into(),
            source_path: std::path::PathBuf::from("/tmp/skill.md"),
            frontmatter: xiaolin_core::skill::SkillFrontmatter {
                name: Some("Greeting".into()),
                description: Some("Say hello".into()),
                tags: vec!["social".into()],
                tools: Vec::new(),
                enabled: Some(true),
                ..Default::default()
            },
            layer: xiaolin_core::skill::SkillLayer::Project,
            source: None,
        };
        let keywords = vec!["deploy"];
        let score = compute_relevance(&keywords, &skill);
        assert_eq!(score, 0.0);
    }

    #[tokio::test]
    async fn search_skills_empty_query_returns_error() {
        let registry = Arc::new(xiaolin_core::skill::SkillRegistry::new());
        let tool = SearchSkillTool::new(registry);
        let result = tool.execute(r#"{"query": ""}"#).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn search_skills_with_valid_query() {
        let mut registry = xiaolin_core::skill::SkillRegistry::new();
        registry.register(xiaolin_core::skill::SkillEntry {
            id: "deploy-backend".into(),
            name: "Deploy Backend".into(),
            description: Some("Deploy the backend to production".into()),
            content: "# Deploy\n1. Build\n2. Push\n3. Verify".into(),
            source_path: std::path::PathBuf::from("/tmp/deploy.md"),
            frontmatter: xiaolin_core::skill::SkillFrontmatter {
                name: Some("Deploy Backend".into()),
                description: Some("Deploy the backend to production".into()),
                tags: vec!["ops".into(), "deploy".into()],
                tools: vec!["shell".into()],
                enabled: Some(true),
                ..Default::default()
            },
            layer: xiaolin_core::skill::SkillLayer::Project,
            source: None,
        });
        let tool = SearchSkillTool::new(Arc::new(registry));
        let result = tool.execute(r#"{"query": "deploy"}"#).await;
        assert!(result.success);
        assert!(result.output.contains("deploy-backend"));
    }

    #[tokio::test]
    async fn hybrid_search_boosts_semantic_matches() {
        let mut registry = xiaolin_core::skill::SkillRegistry::new();
        registry.register(xiaolin_core::skill::SkillEntry {
            id: "ci-pipeline".into(),
            name: "CI Pipeline".into(),
            description: Some("Continuous integration setup".into()),
            content: "# CI\nSetup GitHub Actions".into(),
            source_path: std::path::PathBuf::from("/tmp/ci.md"),
            frontmatter: xiaolin_core::skill::SkillFrontmatter {
                enabled: Some(true),
                ..Default::default()
            },
            layer: xiaolin_core::skill::SkillLayer::Project,
            source: None,
        });
        registry.register(xiaolin_core::skill::SkillEntry {
            id: "deploy-app".into(),
            name: "Deploy App".into(),
            description: Some("Deploy application".into()),
            content: "# Deploy\n1. Ship it".into(),
            source_path: std::path::PathBuf::from("/tmp/deploy.md"),
            frontmatter: xiaolin_core::skill::SkillFrontmatter {
                enabled: Some(true),
                ..Default::default()
            },
            layer: xiaolin_core::skill::SkillLayer::Project,
            source: None,
        });

        use std::str::FromStr;
        let opts = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .busy_timeout(std::time::Duration::from_secs(5));
        let pool = sqlx::SqlitePool::connect_with(opts).await.unwrap();
        let store = Arc::new(
            xiaolin_core::skill_embedding::SkillEmbeddingStore::open(pool).await.unwrap(),
        );
        store.upsert("ci-pipeline", "h1", &[0.9, 0.1, 0.0]).await.unwrap();
        store.upsert("deploy-app", "h2", &[0.1, 0.9, 0.0]).await.unwrap();

        struct FakeEmbedder;
        #[async_trait]
        impl xiaolin_memory::EmbeddingProvider for FakeEmbedder {
            async fn embed_batch(&self, _texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
                Ok(vec![vec![0.85, 0.15, 0.0]])
            }
            fn dimensions(&self) -> usize { 3 }
            fn name(&self) -> &str { "fake" }
        }

        let tool = SearchSkillTool::new(Arc::new(registry))
            .with_semantic(store, Arc::new(FakeEmbedder));

        let results = tool.hybrid_search("continuous integration", None).await;
        assert!(!results.is_empty());
        assert_eq!(results[0].id, "ci-pipeline", "semantic match should rank first");
    }
}
