## ADDED Requirements

### Requirement: Context budget percent defaults to 2%
The system SHALL use 2% of the model's context window as the skill injection character budget. The `context_budget_percent` config field SHALL default to `2`.

#### Scenario: Default budget calculation
- **WHEN** no custom `context_budget_percent` is configured and model context window is 128,000 tokens
- **THEN** the skill injection character budget SHALL be `128000 * 0.02 * 4 = 10240` characters

#### Scenario: Custom budget override
- **WHEN** user sets `context_budget_percent: 5` in config
- **THEN** the system SHALL use 5% as the budget, not the default 2%

### Requirement: Accurate injection usage recording
The system SHALL record skill usage injection events only for skills whose content was actually included in the prompt after budget truncation, not for all enabled skills in the registry.

#### Scenario: Skills truncated by budget
- **WHEN** 100 skills are in the registry but budget truncation reduces the injected set to 50
- **THEN** only 50 injection events SHALL be recorded in `skill_usage` table

#### Scenario: No truncation
- **WHEN** all skills fit within the budget
- **THEN** all enabled skills SHALL have injection events recorded

### Requirement: Format returns injected skill IDs
The `format_with_budget_ordered` function SHALL return the list of skill IDs that were actually included in the formatted output, in addition to the formatted string and truncation info.

#### Scenario: Return type includes IDs
- **WHEN** `format_with_budget_ordered` is called with skills exceeding the budget
- **THEN** the return value SHALL include the formatted string, the existing `Option<SkillTruncationInfo>`, and a `Vec<String>` of included skill IDs

### Requirement: Extension nested skill discovery
The system SHALL scan `extensions/*/skills/` subdirectories for SKILL.md files, not just the top-level `extensions/` directory.

#### Scenario: Nested extension skills loaded
- **WHEN** a skill exists at `extensions/feishu/skills/my-skill/SKILL.md`
- **THEN** the system SHALL discover and load it with `SkillLayer::Extension`

#### Scenario: Top-level extension skills still loaded
- **WHEN** a skill exists at `extensions/my-skill/SKILL.md`
- **THEN** the system SHALL continue to discover and load it as before
