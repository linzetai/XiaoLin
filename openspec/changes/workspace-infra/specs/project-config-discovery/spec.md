## ADDED Requirements

### Requirement: Project-level skills discovery
The system SHALL discover skills from `<workspace_root>/.fastclaw/skills/` and merge them with user-level skills.

#### Scenario: Discover project skills
- **WHEN** the workspace root contains `.fastclaw/skills/my-skill/SKILL.md`
- **THEN** the system loads it as a project-level skill with highest priority

#### Scenario: Skills dynamic discovery convention
- **WHEN** a directory contains a file named `SKILL.md`
- **THEN** that directory is treated as a skill; no manifest registration is needed

#### Scenario: Backward-compatible skills path
- **WHEN** `<workspace_root>/skills/` exists (legacy convention)
- **THEN** those skills SHALL also be discovered but with lower priority than `.fastclaw/skills/`

### Requirement: Project-level MCP configuration
The system SHALL load MCP server configurations from `<workspace_root>/.fastclaw/mcp.json` if the file exists.

#### Scenario: Load project MCP
- **WHEN** `.fastclaw/mcp.json` exists and contains a valid `mcpServers` object
- **THEN** those servers SHALL be merged with user-level MCP configs
- **AND** project-level servers with the same ID SHALL take priority over user-level

#### Scenario: Disable user-level MCP from project
- **WHEN** a project MCP entry has `"enabled": false` for a server ID that exists at user level
- **THEN** that server SHALL not be started

#### Scenario: MCP format alignment with Cursor
- **WHEN** the system reads `.fastclaw/mcp.json`
- **THEN** the format SHALL be compatible with Cursor's `.cursor/mcp.json` schema: `{ "mcpServers": { "<id>": { "command", "args", "env" } } }`

### Requirement: Project-level rules
The system SHALL discover and load rules from `<workspace_root>/.fastclaw/rules/*.md`.

#### Scenario: Load project rules
- **WHEN** `.fastclaw/rules/` contains Markdown files
- **THEN** rules with `alwaysApply: true` (or no frontmatter) SHALL be injected into the system prompt
- **AND** rules with `globs` SHALL only be injected when the agent operates on matching files

### Requirement: Project config merge order
- **WHEN** both project-level and user-level configs exist
- **THEN** the merge order SHALL be: System defaults < User level < Project level (project wins)
