## ADDED Requirements

### Requirement: Skill injection budget based on context window percentage
The system SHALL limit the total character count of injected skill content to a configurable percentage of the context window size.

#### Scenario: Budget limits skill injection
- **WHEN** the total skill content exceeds the configured budget (default 5% of context window)
- **THEN** skills SHALL be sorted by layer priority (highest first) and injected until the budget is exhausted
- **AND** a truncation warning SHALL be emitted to the session status/UI channel (NOT appended to the system prompt body)

#### Scenario: Budget configuration
- **WHEN** `SkillsConfig.context_budget_percent` is set to a value between 1 and 50
- **THEN** the budget SHALL be calculated as `context_window_tokens × (percent / 100) × 4` characters
- **AND** the default SHALL be 5 (representing 5%)

#### Scenario: Budget disabled
- **WHEN** `SkillsConfig.context_budget_percent` is set to 0
- **THEN** no budget limit SHALL be applied and all enabled skills SHALL be injected

### Requirement: Gradual truncation strategy
When budget is exceeded, the system SHALL apply a multi-stage truncation (aligned with Codex behavior).

#### Scenario: Stage 1 — shorten descriptions
- **WHEN** total content exceeds budget but all skills fit with truncated descriptions
- **THEN** skill descriptions SHALL be truncated to first line only
- **AND** all skills SHALL remain visible in the injection

#### Scenario: Stage 2 — omit low-priority skills
- **WHEN** total content still exceeds budget after description truncation
- **THEN** skills from the lowest priority layer SHALL be omitted first
- **AND** `AgentWorkspace` and `ProjectFastclaw` layer skills SHALL never be omitted before lower layers

### Requirement: Context window propagation
The `inject_skills_prompt` function SHALL receive the model's context window size from the current session configuration.

#### Scenario: Budget calculation for 128K context window
- **WHEN** the model context window is 128,000 tokens and budget is 5%
- **THEN** the token budget SHALL be 6,400 tokens (128000 × 0.05)
- **AND** the character budget SHALL be approximately 25,600 characters (6400 × 4)
