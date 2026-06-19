## ADDED Requirements

### Requirement: Evolution promote safety check
The system SHALL check that evolution skills being promoted to static SKILL.md files do not contain potentially unsafe patterns (shell commands, hooks, etc.) in their strategy template. A warning SHALL be returned if unsafe patterns are detected.

#### Scenario: Safe skill promoted successfully
- **WHEN** an evolution skill's strategy template contains no unsafe patterns
- **THEN** the promotion SHALL succeed without warnings

#### Scenario: Unsafe strategy triggers warning
- **WHEN** an evolution skill's strategy template contains patterns like `shell:`, `hooks:`, `shell_exec`, or `execute_command`
- **THEN** the system SHALL include a `warning` field in the promote response indicating which unsafe patterns were detected
- **AND** the promotion SHALL still proceed (warn, not block)

### Requirement: YAML frontmatter validation diagnostic
The system SHALL log a warning when a SKILL.md file contains invalid YAML frontmatter, instead of silently falling back to raw content.

#### Scenario: Invalid YAML detected
- **WHEN** a SKILL.md file has malformed YAML between `---` delimiters
- **THEN** the system SHALL log a `warn!` with the file path and parse error
- **AND** the skill SHALL still be loaded with raw content as fallback

#### Scenario: Valid YAML parsed normally
- **WHEN** a SKILL.md file has valid YAML frontmatter
- **THEN** the skill SHALL be loaded with parsed frontmatter fields
