## MODIFIED Requirements

### Requirement: Sessions grouped by workspace
The session list UI SHALL support grouping sessions by their associated project (via `projectId`), falling back to `workDir` path extraction for sessions without a project.

#### Scenario: Group sessions by project
- **WHEN** sessions have `projectId` values
- **THEN** the sidebar SHALL group them under their project's `name` with the project's `color` dot
- **AND** each group header displays the project name (from project-store, not path extraction)

#### Scenario: Sessions without projectId but with workDir
- **WHEN** a session has no `projectId` but has a `workDir`
- **THEN** the session SHALL appear under a group derived from the workDir's last path component (legacy behavior)

#### Scenario: Sessions without projectId and without workDir
- **WHEN** a session has no `projectId` and no `workDir`
- **THEN** it SHALL appear under a "未关联项目" group

#### Scenario: Session group display
- **WHEN** rendering the session list
- **THEN** pinned projects SHALL appear first, followed by non-pinned projects sorted by `lastOpenedAt` descending
- **AND** sessions within a group are sorted by `createdAt` descending

### Requirement: Workspace context awareness
- **WHEN** the user creates a new session
- **THEN** the session SHALL automatically inherit the current workspace root as its `workDir`
- **AND** the session SHALL be assigned to the corresponding project via `projectId`
- **AND** the session appears in the correct project group immediately
