## ADDED Requirements

### Requirement: Track skill usage events
The system SHALL record every skill invocation (read_skill, search result click, prompt injection) as a usage event.

#### Scenario: read_skill invocation tracked
- **WHEN** the agent calls `read_skill` with a skill ID
- **THEN** a usage event SHALL be recorded with skill_id, timestamp, and event_type="read"

#### Scenario: Skill prompt injection tracked
- **WHEN** a skill is included in the system prompt (full or compact mode)
- **THEN** a usage event SHALL be recorded with event_type="injected"

### Requirement: Usage-based skill sorting
The system SHALL sort skills by usage frequency when displaying lists or injecting into prompts.

#### Scenario: Frequently used skills prioritized
- **WHEN** generating the skill injection list
- **THEN** within the same layer, skills SHALL be sorted by usage count (descending)
- **AND** this SHALL affect the order of truncation when context budget is exceeded

#### Scenario: Usage count in skill list API
- **WHEN** the frontend requests `skills.list`
- **THEN** each skill SHALL include a `usage_count` field reflecting total invocations in the last 30 days

### Requirement: Usage data stored in SQLite
Usage events SHALL be stored in the existing SQLite database.

#### Scenario: Usage table schema
- **WHEN** the database is initialized
- **THEN** a `skill_usage` table SHALL exist with columns: id, skill_id, event_type, session_id, timestamp

#### Scenario: Usage data cleanup
- **WHEN** usage data older than 90 days exists
- **THEN** it SHALL be automatically pruned during database maintenance
