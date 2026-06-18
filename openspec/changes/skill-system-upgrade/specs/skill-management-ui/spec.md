## ADDED Requirements

### Requirement: Skill list with enable/disable toggle
The frontend SHALL display a list of all discovered skills with a toggle to enable or disable each skill.

#### Scenario: View all skills
- **WHEN** the user navigates to the Plugins → Skills tab
- **THEN** all discovered skills SHALL be listed with name, description, source origin (XiaoLin/Cursor/Codex), and layer
- **AND** each skill SHALL have a toggle switch reflecting its enabled/disabled state

#### Scenario: Disable a skill
- **WHEN** the user toggles a skill to disabled
- **THEN** the skill ID SHALL be added to `SkillsConfig.deny` list
- **AND** the skill SHALL be excluded from prompt injection on the next turn
- **AND** the change SHALL persist across app restarts

#### Scenario: Re-enable a skill
- **WHEN** the user toggles a disabled skill back to enabled
- **THEN** the skill ID SHALL be removed from `SkillsConfig.deny` list

### Requirement: Skill detail modal
The frontend SHALL provide a modal to view the full content and metadata of a skill.

#### Scenario: View skill details
- **WHEN** the user clicks on a skill in the list
- **THEN** a modal SHALL display: full SKILL.md content (rendered as markdown), frontmatter fields (name, description, tags, tools), source path, layer, and origin

#### Scenario: Edit skill content
- **WHEN** the user clicks "Edit" on a XiaoLin-owned skill (origin = XiaoLin)
- **THEN** the modal SHALL switch to edit mode with a textarea for the SKILL.md content
- **AND** saving SHALL write the updated content to disk and trigger hot-reload

#### Scenario: Read-only for cross-tool skills
- **WHEN** the user views a Cursor or Codex origin skill
- **THEN** the "Edit" button SHALL be disabled with tooltip "This skill belongs to another tool"

### Requirement: Skill search and filter
The frontend SHALL provide search and filter controls for the skill list.

#### Scenario: Search by name or description
- **WHEN** the user types in the search box
- **THEN** the list SHALL filter to skills whose name or description contains the query (case-insensitive)

#### Scenario: Filter by source
- **WHEN** the user selects a source filter (All / XiaoLin / Cursor / Codex / SharedAgents)
- **THEN** only skills from the selected source SHALL be displayed

#### Scenario: Filter by layer
- **WHEN** the user selects a layer filter (Project / User / Extension)
- **THEN** only skills from the selected layer group SHALL be displayed

### Requirement: Skill CRUD via WebSocket API
The backend SHALL expose skill management operations via the existing WebSocket channel.

#### Scenario: List skills
- **WHEN** the frontend sends `skills.list` request
- **THEN** the backend SHALL return all skills with metadata (id, name, description, source, layer, enabled)

#### Scenario: Read skill
- **WHEN** the frontend sends `skills.read` with a skill ID
- **THEN** the backend SHALL return the full SKILL.md content and parsed frontmatter

#### Scenario: Update skill
- **WHEN** the frontend sends `skills.update` with skill ID and new content
- **THEN** the backend SHALL write the content to disk (only for XiaoLin-owned skills)
- **AND** trigger hot-reload of the registry

#### Scenario: Delete skill
- **WHEN** the frontend sends `skills.delete` with a skill ID
- **THEN** the backend SHALL delete the skill directory (only for XiaoLin-owned skills)
- **AND** trigger hot-reload of the registry
