## ADDED Requirements

### Requirement: Workspace root auto-detection
The system SHALL automatically detect the workspace root by walking up from the current working directory, checking for project markers in priority order.

#### Scenario: Detect by .fastclaw directory
- **WHEN** the current directory or any ancestor contains a `.fastclaw/` directory
- **THEN** that directory is returned as the workspace root (highest priority)

#### Scenario: Detect by .git directory
- **WHEN** no `.fastclaw/` is found but an ancestor contains `.git/`
- **THEN** that directory is returned as the workspace root

#### Scenario: Detect by language project markers
- **WHEN** no `.fastclaw/` or `.git/` is found but an ancestor contains a language marker file (`Cargo.toml`, `package.json`, `pyproject.toml`, `go.mod`, `build.gradle`, `pom.xml`)
- **THEN** that directory is returned as the workspace root

#### Scenario: Fallback to current directory
- **WHEN** no markers are found after traversing to the filesystem root
- **THEN** the original current working directory is returned

### Requirement: Workspace root used for session creation
- **WHEN** a new session is created without an explicit `work_dir`
- **THEN** the detected workspace root SHALL be used as the session's `work_dir`, NOT the agent workspace path (`~/.fastclaw/workspace/`)

### Requirement: Workspace root used for project config discovery
- **WHEN** the gateway starts or reloads configuration
- **THEN** project-level configs SHALL be loaded from `<workspace_root>/.fastclaw/` if it exists
