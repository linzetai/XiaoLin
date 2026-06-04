## ADDED Requirements

### Requirement: Project config file structure
The system SHALL support a `.xiaolin/project.json` configuration file at the project root with the following optional fields: `name` (string), `description` (string), `defaultModel` (string), `defaultAgent` (string).

#### Scenario: Read project config
- **WHEN** a project's root_path contains `.xiaolin/project.json` with valid JSON
- **THEN** the system SHALL parse and return a `ProjectConfig` struct with the file's contents

#### Scenario: Missing project config
- **WHEN** a project's root_path does not contain `.xiaolin/project.json`
- **THEN** the system SHALL return a default empty `ProjectConfig`
- **AND** the project SHALL still function normally

#### Scenario: Invalid project config
- **WHEN** `.xiaolin/project.json` contains invalid JSON
- **THEN** the system SHALL log a warning and return a default empty `ProjectConfig`
- **AND** NOT crash or prevent the project from being used

### Requirement: Project config name takes precedence
When `.xiaolin/project.json` specifies a `name` field, it SHALL take precedence over the auto-detected name (last path component) stored in the SQLite `projects` table.

#### Scenario: Config name overrides DB name
- **WHEN** a project's SQLite row has `name = "my-app"`
- **AND** `.xiaolin/project.json` contains `{"name": "My Application"}`
- **THEN** the effective project name returned to the frontend SHALL be `"My Application"`

#### Scenario: No config name falls back to DB name
- **WHEN** `.xiaolin/project.json` does not contain a `name` field
- **THEN** the effective project name SHALL be the value from the SQLite `projects.name` column

### Requirement: Project config integration with existing configs
The `.xiaolin/project.json` file SHALL complement (not replace) the existing `.xiaolin/mcp.json`, `.xiaolin/rules/`, and `.xiaolin/skills/` configurations.

#### Scenario: All configs loaded together
- **WHEN** a project has `.xiaolin/project.json`, `.xiaolin/mcp.json`, and `.xiaolin/rules/`
- **THEN** the system SHALL load all three independently
- **AND** `project.json` SHALL NOT contain MCP or rules definitions (those stay in their own files)

### Requirement: Write project config
The system SHALL support writing/updating `.xiaolin/project.json` when the user modifies project properties that belong to the project config scope.

#### Scenario: Create project config on first edit
- **WHEN** the user sets a project description via the API
- **AND** `.xiaolin/project.json` does not exist
- **THEN** the system SHALL create `.xiaolin/project.json` with the provided values
- **AND** create the `.xiaolin/` directory if it does not exist
