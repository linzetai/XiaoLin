## MODIFIED Requirements

### Requirement: Project-level skills discovery
The system SHALL discover skills from `<project_root>/.xiaolin/skills/` using the project's `root_path` from the project registry, and merge them with user-level skills.

#### Scenario: Discover project skills via registry
- **WHEN** the active session has a `project_id`
- **THEN** the system SHALL resolve the project's `root_path` from the projects table
- **AND** load skills from `<root_path>/.xiaolin/skills/` with highest priority

#### Scenario: Fallback to workspace root detection
- **WHEN** the active session has no `project_id` but has a `work_dir`
- **THEN** the system SHALL fall back to `detect_workspace_root(work_dir)` to find the project root
- **AND** load skills from the detected root (current behavior)

#### Scenario: Skills dynamic discovery convention
- **WHEN** a directory contains a file named `SKILL.md`
- **THEN** that directory is treated as a skill; no manifest registration is needed

#### Scenario: Backward-compatible skills path
- **WHEN** `<project_root>/skills/` exists (legacy convention)
- **THEN** those skills SHALL also be discovered but with lower priority than `.xiaolin/skills/`

### Requirement: Project-level MCP configuration
The system SHALL load MCP server configurations from the project's root_path (resolved via project registry when available).

#### Scenario: Load project MCP via registry
- **WHEN** the active session has a `project_id` pointing to a project with `root_path`
- **THEN** the system SHALL load `.xiaolin/mcp.json` from that `root_path`
- **AND** merge with user-level MCP configs (project wins)

#### Scenario: Fallback MCP loading
- **WHEN** no project_id is available
- **THEN** the system SHALL use `detect_workspace_root(cwd)` to find the project root (current behavior)

#### Scenario: Disable user-level MCP from project
- **WHEN** a project MCP entry has `"enabled": false` for a server ID that exists at user level
- **THEN** that server SHALL not be started

#### Scenario: MCP format alignment with Cursor
- **WHEN** the system reads `.xiaolin/mcp.json`
- **THEN** the format SHALL be compatible with Cursor's `.cursor/mcp.json` schema: `{ "mcpServers": { "<id>": { "command", "args", "env" } } }`

### Requirement: Project-level rules
The system SHALL discover and load rules from `<project_root>/.xiaolin/rules/*.md`, with `project_root` resolved from the project registry when available.

#### Scenario: Load project rules via registry
- **WHEN** the active session has a `project_id`
- **THEN** the system SHALL resolve the project's `root_path` and load rules from `.xiaolin/rules/`
- **AND** rules with `alwaysApply: true` (or no frontmatter) SHALL be injected into the system prompt

#### Scenario: Load project rules via fallback
- **WHEN** no `project_id` is available but `work_dir` exists
- **THEN** the system SHALL use `detect_workspace_root(work_dir)` for rules discovery (current behavior)

### Requirement: Project config merge order
- **WHEN** both project-level and user-level configs exist
- **THEN** the merge order SHALL be: System defaults < User level < Project level (project wins)

### Requirement: Project config includes project.json
- **WHEN** loading project configuration
- **THEN** the system SHALL also load `.xiaolin/project.json` if it exists
- **AND** its `defaultModel` field SHALL be used as the default model for new sessions in this project (if set)
