use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolResult};
use fastclaw_core::workspace::AgentWorkspace;

// --- Skill tools for lazy/compact modes ---

/// List all available skills with their names and descriptions.
pub struct ListSkillsTool {
    registry: Arc<fastclaw_core::skill::SkillRegistry>,
}

impl ListSkillsTool {
    pub fn new(registry: Arc<fastclaw_core::skill::SkillRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for ListSkillsTool {
    fn name(&self) -> &str {
        "list_skills"
    }

    fn description(&self) -> &str {
        "Return JSON listing enabled skills: id, name, short description, and tags from each SKILL.md frontmatter. \
         Call list_skills before read_skill whenever procedures might live outside the base prompt, or when the user references a workflow by nickname and you need the exact id. \
         Disabled entries are hidden; layering (project vs global) is already merged by the host—treat this output as authoritative for ids. \
         Metadata only—always follow with read_skill to fetch full Markdown and tool hooks. \
         Anti-pattern: inventing ids; anti-pattern: assuming skills exist when count is zero (author with write_skill or hub_install first). \
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
    registry: Arc<fastclaw_core::skill::SkillRegistry>,
}

impl ReadSkillTool {
    pub fn new(registry: Arc<fastclaw_core::skill::SkillRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for ReadSkillTool {
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
            None => return ToolResult::err(
                "read_skill is missing required string field 'skill_id'. \
                 Example: {\"skill_id\": \"greeting\"}. \
                 Call list_skills first if you do not know valid ids."
                    .to_string(),
            ),
        };

        match self.registry.get(skill_id) {
            Some(skill) => ToolResult::ok(
                serde_json::json!({
                    "id": skill.id,
                    "name": skill.name,
                    "description": skill.description,
                    "content": skill.content,
                    "tools": skill.frontmatter.tools,
                })
                .to_string(),
            ),
            None => ToolResult::err(format!(
                "read_skill: skill not found for id '{skill_id}'. \
                 What went wrong: no enabled skill matches that exact id string (typo, disabled entry, or not registered yet). \
                 What to do next: run list_skills and paste an 'id' field exactly as returned (case and slashes matter); if the user just added a file, ensure the host registered it or use write_skill then reload per operator docs."
            )),
        }
    }
}

/// Create or update a skill. Writes to agent workspace by default, or global directory.
pub struct WriteSkillTool {
    workspace: Arc<AgentWorkspace>,
}

impl WriteSkillTool {
    pub fn new(workspace: Arc<AgentWorkspace>) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for WriteSkillTool {
    fn name(&self) -> &str {
        "write_skill"
    }

    fn description(&self) -> &str {
        "Create or overwrite SKILL.md for a skill_id with the full Markdown document (optional YAML frontmatter included). \
         Default target 'workspace' writes under the agent-private skills tree; use target 'global' only when the user asked for a shared skill under the operator-wide skills directory visible to other agents. \
         Whole-file replace—identical contract to write_file: partial bodies delete the rest of the file. \
         Hosts may require reload/restart before list_skills reflects changes—check product docs. \
         Anti-pattern: embedding API keys or tokens—reference env-based configuration instead. \
         Example: {\"skill_id\": \"deploy-checklist\", \"content\": \"---\\nname: Deploy\\ntags: [ops]\\n---\\n# Steps\\n1. ...\", \"target\": \"workspace\"}."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "skill_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Stable id for the skill folder (e.g. 'my-search-skill', 'team/onboarding'). Becomes the directory name under skills/; avoid spaces if you want simple shell paths."
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
                "enum": ["workspace", "global"],
                "description": "'workspace' (default): agent-private skills root; 'global': shared ~/.fastclaw/skills/. Omit or null for workspace. Choose global only when the user asked for a team-wide skill."
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
            _ => return ToolResult::err(
                "write_skill is missing or empty required string field 'skill_id'. \
                 Example: {\"skill_id\": \"my-checklist\", \"content\": \"# My Skill\\n...\"}. \
                 Pick a stable id; it becomes the on-disk folder name under skills/."
                    .to_string(),
            ),
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
            "global" => fastclaw_core::workspace::write_global_skill(skill_id, content),
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

#[cfg(test)]
mod skill_tool_tests {
    use super::*;
    use crate::builtin_tools::{register_skill_tools, register_skill_tools_full};
    use fastclaw_core::skill::{SkillEntry, SkillFrontmatter, SkillLayer, SkillRegistry};
    use fastclaw_core::tool::ToolRegistry;
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
    fn register_adds_both_tools() {
        let reg = build_registry();
        let mut tool_reg = ToolRegistry::new();

        register_skill_tools(&mut tool_reg, reg);

        let defs = tool_reg.definitions();
        let names: Vec<&str> = defs.iter().map(|d| d.function.name.as_str()).collect();
        assert!(names.contains(&"list_skills"));
        assert!(names.contains(&"read_skill"));
    }

    #[test]
    fn register_full_adds_all_three_tools() {
        let reg = build_registry();
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = Arc::new(fastclaw_core::workspace::AgentWorkspace::new(
            tmp.path(),
            "test-agent",
        ));
        let mut tool_reg = ToolRegistry::new();

        register_skill_tools_full(&mut tool_reg, reg, ws);

        let defs = tool_reg.definitions();
        let names: Vec<&str> = defs.iter().map(|d| d.function.name.as_str()).collect();
        assert!(names.contains(&"list_skills"));
        assert!(names.contains(&"read_skill"));
        assert!(names.contains(&"write_skill"));
    }

    // ── WriteSkillTool ─────────────────────────────────────────────

    #[tokio::test]
    async fn write_skill_to_workspace() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = Arc::new(fastclaw_core::workspace::AgentWorkspace::new(
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
        let ws = Arc::new(fastclaw_core::workspace::AgentWorkspace::new(
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
        let ws = Arc::new(fastclaw_core::workspace::AgentWorkspace::new(
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
        let ws = Arc::new(fastclaw_core::workspace::AgentWorkspace::new(
            tmp.path(),
            "agent-x",
        ));
        let tool = WriteSkillTool::new(ws);

        let schema = tool.parameters_schema();
        assert!(schema.required.contains(&"skill_id".to_string()));
        assert!(schema.required.contains(&"content".to_string()));
        assert!(schema.properties.contains_key("target"));
    }
}
