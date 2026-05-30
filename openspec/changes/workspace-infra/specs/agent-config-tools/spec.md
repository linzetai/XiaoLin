## ADDED Requirements

### Requirement: Built-in configuration management skill
The system SHALL include a built-in skill (`fastclaw-config-manager`) that teaches the agent about the `.fastclaw/` directory structure, file formats, and conventions.

#### Scenario: Agent helps create a skill
- **WHEN** the user asks "帮我加个 skill" or "create a skill"
- **THEN** the agent SHALL know to create `<root>/.fastclaw/skills/<name>/SKILL.md` with the correct format

#### Scenario: Agent helps add MCP server
- **WHEN** the user asks "添加 MCP" or "add MCP server"
- **THEN** the agent SHALL know to create/update `<root>/.fastclaw/mcp.json`

### Requirement: Project config listing tool
The system SHALL provide a `list_project_config` tool that shows the current project's skills, MCP servers, and rules with their sources.

#### Scenario: List project config
- **WHEN** the tool is invoked
- **THEN** it returns a structured summary of all discovered skills (with source), MCP servers, and rules for the current workspace

### Requirement: Project context injection
- **WHEN** a session starts in a workspace with project-level configuration
- **THEN** a brief summary of the project config (skill count by source, MCP server names, rule count) SHALL be injected into the agent's system prompt context
