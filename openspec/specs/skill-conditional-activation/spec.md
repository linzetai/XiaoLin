## ADDED Requirements

### Requirement: Frontmatter paths field for conditional activation
The `SkillFrontmatter` struct SHALL support an optional `paths` field containing glob patterns. When present, the skill SHALL only be activated when workspace files match at least one pattern.

#### Scenario: Skill activates for matching project
- **WHEN** a skill has `paths: ["*.rs", "Cargo.toml"]` in frontmatter
- **AND** the current workspace contains `.rs` files
- **THEN** the skill SHALL be included in prompt injection

#### Scenario: Skill skipped for non-matching project
- **WHEN** a skill has `paths: ["*.py", "requirements.txt"]` in frontmatter
- **AND** the current workspace contains no `.py` files
- **THEN** the skill SHALL NOT be included in prompt injection

#### Scenario: Skill with no paths field always activates
- **WHEN** a skill has no `paths` field in frontmatter
- **THEN** the skill SHALL always be included (backward compatible behavior)

### Requirement: Workspace file index for path matching
The system SHALL maintain a lightweight file index of the workspace for glob matching.

#### Scenario: File index built on session start
- **WHEN** a new session starts with a workspace root
- **THEN** the system SHALL scan the workspace (respecting .gitignore) and build a file index
- **AND** the index SHALL be used for all conditional activation checks during the session

#### Scenario: File index updated on file change
- **WHEN** a tool creates, modifies, or deletes a file in the workspace
- **THEN** the file index SHALL be updated to reflect the change
- **AND** conditional activation SHALL be re-evaluated

### Requirement: Glob matching uses globset crate
The system SHALL use the `globset` crate for efficient batch glob matching.

#### Scenario: Batch evaluation performance
- **WHEN** 100 skills each have `paths:` patterns
- **THEN** all patterns SHALL be evaluated against the file index in a single pass
- **AND** the total evaluation time SHALL be under 10ms for a typical workspace (10K files)
