## ADDED Requirements

### Requirement: Sessions grouped by workspace
The session list UI SHALL support grouping sessions by their `workDir` (workspace root).

#### Scenario: Group sessions by normalized workDir
- **WHEN** sessions have different `workDir` values
- **THEN** the sidebar SHALL group them under separate workspace headers
- **AND** each header displays the workspace path (abbreviated with `~`)

#### Scenario: Sessions without workDir
- **WHEN** a session has no `workDir` (null)
- **THEN** it SHALL appear under a "未关联项目" group

#### Scenario: Session group display
- **WHEN** rendering the session list
- **THEN** each workspace group shows the project name (last directory component) and session count
- **AND** sessions within a group are sorted by `updatedAt` descending

### Requirement: Workspace context awareness
- **WHEN** the user creates a new session
- **THEN** the session SHALL automatically inherit the current workspace root as its `workDir`
- **AND** the session appears in the correct workspace group immediately
