## ADDED Requirements

### Requirement: Persist per-turn runtime quality summaries
The system SHALL persist a derived runtime quality summary for each agent turn that reaches a normal, aborted, or error terminal state.

#### Scenario: Normal turn summary persisted
- **WHEN** an agent turn emits a normal `turn_end`
- **THEN** the system SHALL write one `turn_quality_summary` record keyed by the session id and turn id
- **AND** the record SHALL include elapsed time, iteration count, tool call count, token/cache fields when available, context fields when available, and diagnosis fields

#### Scenario: Aborted turn summary persisted
- **WHEN** an agent turn emits `turn_aborted`
- **THEN** the system SHALL write or update one `turn_quality_summary` record for that turn
- **AND** the record SHALL set `diagnosis_code` to `aborted` unless a more specific terminal error code is explicitly available

#### Scenario: Error turn summary persisted
- **WHEN** an agent turn terminates with a fatal runtime or stream error before normal completion
- **THEN** the system SHALL write or update one `turn_quality_summary` record for that turn
- **AND** the record SHALL set `diagnosis_code` to `error`

### Requirement: Summary records are derived and privacy-safe
The system SHALL treat runtime quality summaries as derived diagnostic records and MUST NOT store full prompts, full model responses, full tool outputs, user message bodies, secrets, or duplicate raw event streams in the summary record.

#### Scenario: Tool result summarized without output body
- **WHEN** a tool result contains a large output or sensitive-looking text
- **THEN** the `turn_quality_summary` record SHALL store numeric tool metrics and the tool name only
- **AND** it SHALL NOT store the full tool output

#### Scenario: Event log remains the source of truth
- **WHEN** a developer needs raw turn replay data
- **THEN** the system SHALL rely on `event_log` for raw events
- **AND** `turn_quality_summary` SHALL provide only derived aggregate fields

### Requirement: Capture turn timing milestones
The system SHALL record timing milestones for each summarized turn using runtime-measured elapsed durations rather than deriving all timings from SQLite write timestamps.

#### Scenario: First content timing
- **WHEN** a turn receives its first content delta
- **THEN** the summary SHALL record `first_content_ms` as elapsed milliseconds from turn start

#### Scenario: First reasoning timing
- **WHEN** a turn receives its first reasoning delta
- **THEN** the summary SHALL record `first_reasoning_ms` as elapsed milliseconds from turn start

#### Scenario: First tool timing
- **WHEN** a turn starts its first tool call
- **THEN** the summary SHALL record `first_tool_ms` as elapsed milliseconds from turn start

#### Scenario: No milestone observed
- **WHEN** a turn completes without a content, reasoning, or tool milestone
- **THEN** the corresponding first timing field SHALL be null or omitted in the persisted summary

### Requirement: Capture tool quality statistics
The system SHALL aggregate tool execution statistics into each turn quality summary.

#### Scenario: Tool duration aggregation
- **WHEN** a turn executes multiple tools
- **THEN** the summary SHALL record total tool time, total tool calls, failed tool calls, slowest tool name, and slowest tool duration

#### Scenario: Tool repetition aggregation
- **WHEN** runtime repetition detection emits warning or force-stop outcomes during a turn
- **THEN** the summary SHALL record repetition warning and force-stop counts for that turn

#### Scenario: Turn without tools
- **WHEN** a turn completes without executing tools
- **THEN** the summary SHALL record zero tool calls and SHALL leave slowest tool fields null or omitted

### Requirement: Capture token, cache, cost, and context statistics
The system SHALL include token usage, prompt-cache usage, estimated cost, and context pressure fields in the turn quality summary when those values are available.

#### Scenario: Usage available
- **WHEN** provider usage is available for a turn
- **THEN** the summary SHALL record input tokens, output tokens, cache read tokens, cache creation tokens, cache hit percentage, and estimated cost when cost can be estimated

#### Scenario: Usage unavailable
- **WHEN** provider usage is unavailable for a turn
- **THEN** the summary SHALL persist the turn quality record with usage fields null or zero according to the shared data model

#### Scenario: Context usage update observed
- **WHEN** a turn emits context usage updates or compaction boundaries
- **THEN** the summary SHALL record final context tokens, context window, context usage percentage, whether compression occurred, tokens saved, and compaction count

### Requirement: Use deterministic diagnosis classification
The system SHALL compute `diagnosis_code`, `severity`, and `evidence_json` using deterministic rules and MUST NOT call an LLM for diagnosis generation.

#### Scenario: Normal diagnosis
- **WHEN** a turn completes without threshold violations, failures, aborts, errors, cache issues, or context pressure
- **THEN** the summary SHALL set `diagnosis_code` to `normal`

#### Scenario: Slow provider diagnosis
- **WHEN** a turn has high first-delta latency and tool execution does not dominate elapsed time
- **THEN** the summary SHALL set or be eligible to set `diagnosis_code` to `provider_slow`
- **AND** `evidence_json` SHALL include the latency values used by the rule

#### Scenario: Slow tool diagnosis
- **WHEN** tool execution time dominates the turn elapsed time or one tool exceeds the configured slow-tool threshold
- **THEN** the summary SHALL set or be eligible to set `diagnosis_code` to `tool_slow`
- **AND** `evidence_json` SHALL include the slowest tool name and duration

#### Scenario: Cache miss diagnosis
- **WHEN** prompt-cache hit percentage is below the configured cache threshold and the miss is not marked expected
- **THEN** the summary SHALL set or be eligible to set `diagnosis_code` to `cache_miss`
- **AND** `evidence_json` SHALL include cache read, cache creation, and hit percentage values

#### Scenario: Context pressure diagnosis
- **WHEN** final context usage exceeds the configured pressure threshold or compaction occurs
- **THEN** the summary SHALL set or be eligible to set `diagnosis_code` to `context_pressure`
- **AND** `evidence_json` SHALL include context usage and compaction values

### Requirement: Developer analysis access
The system SHALL provide backend access for developers to query or export runtime quality summaries without requiring a frontend diagnostics UI.

#### Scenario: Query recent summaries
- **WHEN** a developer requests recent runtime quality summaries through the backend query/export affordance
- **THEN** the system SHALL return summaries ordered by most recent start or end time

#### Scenario: Query session summaries
- **WHEN** a developer requests runtime quality summaries for a specific session id
- **THEN** the system SHALL return only summaries for that session

#### Scenario: Frontend not required
- **WHEN** this change is implemented
- **THEN** the system SHALL NOT require a new frontend panel or MessageStream UI to satisfy the runtime quality observability capability
