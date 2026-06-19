## ADDED Requirements

### Requirement: Sub-agent model override
When a `SubAgentDef` specifies a non-null `model` field, the sub-agent runtime SHALL use that model configuration instead of inheriting the parent agent's model.

#### Scenario: Def with explicit model
- **WHEN** a sub-agent is spawned with a `SubAgentDef` whose `model` field contains a valid `AgentModelConfig`
- **THEN** the sub-agent's LLM calls SHALL use the model specified in the def, not the parent agent's model

#### Scenario: Def without model override
- **WHEN** a sub-agent is spawned with a `SubAgentDef` whose `model` field is `None`
- **THEN** the sub-agent SHALL inherit the parent agent's model configuration (current behavior preserved)

### Requirement: Configurable result truncation limit
The `SubAgentDef` struct SHALL support a `max_result_chars` field to control the maximum character count of sub-agent result text returned to the parent.

#### Scenario: Custom max_result_chars
- **WHEN** a sub-agent completes with a `SubAgentDef` that has `max_result_chars` set to 65536
- **THEN** the result text SHALL be truncated to at most 65536 characters before being returned to the parent

#### Scenario: Default max_result_chars
- **WHEN** a sub-agent completes with a `SubAgentDef` that has `max_result_chars` as `None`
- **THEN** the result text SHALL be truncated using the global default of 32768 characters

#### Scenario: max_result_chars upper bound
- **WHEN** a `SubAgentDef` specifies `max_result_chars` greater than 131072
- **THEN** the system SHALL clamp the value to 131072 to prevent context window exhaustion

### Requirement: Session cleanup completeness
`SubAgentManager::cleanup_session()` SHALL remove all in-memory resources associated with a session, including `session_event_senders` entries.

#### Scenario: Cleanup removes event sender
- **WHEN** `cleanup_session("session-123")` is called
- **THEN** `session_event_senders` SHALL NOT contain an entry for "session-123"

#### Scenario: Cleanup preserves other sessions
- **WHEN** `cleanup_session("session-123")` is called while "session-456" has active runs
- **THEN** resources for "session-456" SHALL remain intact
