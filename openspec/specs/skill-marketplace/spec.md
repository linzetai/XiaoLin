## ADDED Requirements

### Requirement: Marketplace browse UI
The frontend SHALL provide a marketplace panel within the Plugins section for browsing and installing community skills.

#### Scenario: Browse marketplace
- **WHEN** the user opens the Skills marketplace tab
- **THEN** the system SHALL display a categorized list of available skills from the curated directory
- **AND** each skill SHALL show name, description, author, download count, and install status

#### Scenario: Search marketplace
- **WHEN** the user types in the marketplace search box
- **THEN** results SHALL filter to skills matching the query in name, description, or tags

### Requirement: Curated skill directory
The marketplace SHALL use a JSON index file hosted on GitHub as the source of available skills.

#### Scenario: Fetch directory index
- **WHEN** the marketplace panel opens
- **THEN** the system SHALL fetch the index from a configurable GitHub raw URL
- **AND** cache the index locally with a TTL of 1 hour

#### Scenario: Directory entry format
- **WHEN** parsing a directory entry
- **THEN** each entry SHALL contain: id, name, description, author, repo_url, skill_path, tags, version

### Requirement: One-click skill installation
The marketplace SHALL support installing a skill from the directory with a single click.

#### Scenario: Install a skill
- **WHEN** the user clicks "Install" on a marketplace skill
- **THEN** the system SHALL download the SKILL.md from the specified repo_url + skill_path
- **AND** save it to `~/.xiaolin/skills/<skill-id>/SKILL.md`
- **AND** trigger hot-reload of the skill registry
- **AND** show a success notification

#### Scenario: Update an installed skill
- **WHEN** the user clicks "Update" on an installed skill with a newer version available
- **THEN** the system SHALL download and overwrite the existing SKILL.md
- **AND** trigger hot-reload

#### Scenario: Uninstall a skill
- **WHEN** the user clicks "Uninstall" on a marketplace-installed skill
- **THEN** the system SHALL delete the skill directory from `~/.xiaolin/skills/`
- **AND** trigger hot-reload

### Requirement: Skill preview before installation
The marketplace SHALL show a preview of the skill content before installation.

#### Scenario: Preview skill content
- **WHEN** the user clicks on a marketplace skill entry
- **THEN** a detail panel SHALL show the full SKILL.md content rendered as markdown
- **AND** a "Install" button SHALL be available in the preview
