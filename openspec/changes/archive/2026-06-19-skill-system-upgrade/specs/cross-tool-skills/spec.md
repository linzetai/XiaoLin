## MODIFIED Requirements

### Requirement: Write-only to own directory
- **WHEN** the agent creates or modifies a skill via the unified `skill` tool (`action: "write"`) or `write_skill` alias
- **THEN** it SHALL only write under XiaoLin-owned paths:
  - `target: "project"` (default): `<workspace_root>/.xiaolin/skills/<id>/SKILL.md`
  - `target: "global"`: `~/.xiaolin/skills/<id>/SKILL.md`
  - `target: "workspace"`: `<agent_workspace>/skills/<id>/SKILL.md` (agent-private overlay)
- **AND** it SHALL NOT write to `.cursor/`, `.codex/`, or `.agents/` directories
- **AND** it SHALL trigger `AppState::reload_skills()` after successful write via an injected callback

#### Scenario: write_skill triggers hot-reload
- **WHEN** the agent calls `skill` tool with `action: "write"` to create a new skill
- **THEN** the skill SHALL be immediately available in the registry without manual refresh
- **AND** subsequent `skill` tool with `action: "list"` SHALL include the newly created skill

#### Scenario: Hot-reload preserves builtin skills
- **WHEN** `reload_skills()` is triggered (by write_skill, upload, or manual refresh)
- **THEN** builtin skills (e.g., `xiaolin-config-manager`) SHALL be preserved
- **AND** extension skills SHALL be preserved
- **AND** legacy project skills SHALL be preserved

### Requirement: Full skill scan path order
The complete loading chain (lowest to highest priority):
1. Extension/builtin (e.g., `xiaolin-config-manager`)
2. Legacy `{state_dir}/skills/` (via `resolve_skills_dir`)
3. `~/.agents/skills/` (SharedAgents)
4. `~/.codex/skills/` (UserCodex)
5. `~/.cursor/skills/` (UserCursor)
6. `~/.cursor/skills-cursor/` (Cursor built-in skills)
7. `~/.xiaolin/skills/` (Global / UserFastclaw)
8. `<root>/.cursor/skills/` (ProjectCursor, read-only)
9. `<root>/.xiaolin/skills/` (ProjectFastclaw)
10. `<agent_workspace>/skills/` (AgentWorkspace, per-agent highest)

#### Scenario: Upload skill path alignment
- **WHEN** a user uploads a skill via the Tauri `upload_skill` command
- **THEN** the skill SHALL be written to `{state_dir}/skills/<skill-id>/SKILL.md`
- **AND** the path SHALL match the scan path used by `resolve_global_skills_dir()`

#### Scenario: Migrate existing config/skills data
- **WHEN** the system detects skills in `{state_dir}/config/skills/` but not in `{state_dir}/skills/`
- **THEN** it SHALL migrate them automatically on startup

## ADDED Requirements

### Requirement: Unified skill tool registered in all prompt modes
The `UnifiedSkillTool` (tool name `skill`) with actions `list`, `read`, `write`, `search` SHALL be registered regardless of prompt_mode setting.

#### Scenario: Full mode includes skill tool
- **WHEN** prompt_mode is `full`
- **THEN** the unified `skill` tool SHALL be registered with all four actions
- **AND** the agent MAY use `search` even though skill content is already in the prompt

#### Scenario: Compact mode includes skill tool
- **WHEN** prompt_mode is `compact`
- **THEN** the unified `skill` tool SHALL be registered with all four actions
- **AND** `read` action SHALL be the primary way to access full skill content

### Requirement: Search action in UnifiedSkillTool
The `UnifiedSkillTool` SHALL support `action: "search"` that accepts a query string and returns matching skills ranked by relevance.

#### Scenario: Search skills by keyword
- **WHEN** the agent calls `skill` tool with `action: "search"` and `query: "rust testing"`
- **THEN** matching skills SHALL be returned ranked by keyword relevance in name/description/tags/content

#### Scenario: Search with no results
- **WHEN** the agent searches for a query with no matching skills
- **THEN** an empty result list SHALL be returned with a helpful message

### Requirement: Default prompt_mode SHALL be compact
The default value for `SkillsConfig.prompt_mode` SHALL be `Compact` instead of `Full`.

#### Scenario: New installation uses compact mode
- **WHEN** a user starts XiaoLin without explicit skill configuration
- **THEN** skills SHALL be injected using compact format (name + one-line description)
- **AND** full skill content SHALL only be loaded on demand via `skill` tool (`action: read`)

### Requirement: Extension skills SHALL be loaded
The system SHALL load extension plugin skills into `ext_registry` during gateway initialization by scanning the extensions directory.

#### Scenario: Extension skills appear in registry
- **WHEN** the gateway initializes with `resolve_extensions_dir()` containing skill directories
- **THEN** those skills SHALL be merged into the skill registry at `Extension` layer priority

### Requirement: Clean up SKILL_AUTHORING_PROMPT
The `SKILL_AUTHORING_PROMPT` constant SHALL be corrected to remove claims about unimplemented features.

#### Scenario: No false capability claims
- **WHEN** the SKILL_AUTHORING_PROMPT is injected into context
- **THEN** it SHALL NOT reference semantic search, usage tracking, or other capabilities that are not yet implemented
