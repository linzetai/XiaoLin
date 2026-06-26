## ADDED Requirements

### Requirement: Chat turn request carries intended runtime context
Every WebSocket chat request SHALL be able to carry the user's intended execution mode as part of the chat turn request. The request-level execution mode SHALL be optional for backward compatibility.

#### Scenario: Plan mode sent with chat request
- **WHEN** the frontend sends a WebSocket chat request while the composer is in Plan mode
- **THEN** the request payload MUST include `executionMode = "plan"`

#### Scenario: Missing execution mode preserves compatibility
- **WHEN** an older client sends a WebSocket chat request without `executionMode`
- **THEN** the gateway MUST use the current backend session mode if one exists
- **THEN** the gateway MUST default to Agent mode for a session with no existing mode state

### Requirement: Gateway applies runtime context after session resolution
The gateway SHALL resolve the authoritative backend session id before applying request-level runtime context. The mode used by the runtime SHALL be stored against the resolved backend session id, not a client-local placeholder id.

#### Scenario: New local chat id resolves to backend session
- **WHEN** the client sends a chat request with local session id `new-*` and `executionMode = "plan"`
- **AND** setup resolves the turn to backend session id `S`
- **THEN** the gateway MUST transition `SessionModeRegistry[S]` to Plan before submitting the runtime turn

#### Scenario: Existing session keeps explicit request mode
- **WHEN** the client sends a chat request for existing backend session id `S` with `executionMode = "plan"`
- **THEN** the gateway MUST transition `SessionModeRegistry[S]` to Plan before submitting the runtime turn

### Requirement: Turn start announces effective runtime context
The `turn_start` event SHALL include the resolved session id and effective execution mode used for the runtime turn. The frontend SHALL treat this event as authoritative for chat id migration and visible execution mode.

#### Scenario: Turn start after id migration
- **WHEN** a chat request with local id `new-*` resolves to backend session id `S`
- **THEN** `turn_start` MUST include `session_id = "S"`
- **THEN** `turn_start` MUST include the effective `executionMode`

#### Scenario: Frontend reconciles mode from turn start
- **WHEN** the frontend receives `turn_start` with `executionMode = "plan"`
- **THEN** it MUST update the active chat metadata to Plan mode for the resolved session id

### Requirement: Goal mode precedence is explicit
When `goalMode = true`, the effective runtime execution mode SHALL be Agent for that turn even if the frontend previously displayed Plan mode. The gateway SHALL announce the effective mode in `turn_start`.

#### Scenario: Goal turn requested from Plan UI
- **WHEN** the client sends a chat request with `goalMode = true`
- **AND** the frontend chat metadata currently has `executionMode = "plan"`
- **THEN** the gateway MUST run the turn in Agent mode
- **THEN** `turn_start` MUST include `executionMode = "agent"`

### Requirement: Turn end carries terminal diagnosis
The `turn_end` event SHALL include a machine-readable terminal reason and, when available, runtime diagnosis metadata suitable for user-visible abnormal-ending UI.

#### Scenario: Tool loop force stop reaches UI
- **WHEN** the runtime terminates a turn because repeated or no-progress tool looping reached a hard stop
- **THEN** `turn_end` MUST include `endReason = "tool_loop"` or `diagnosisCode = "tool_loop"`
- **THEN** `turn_end` MUST include `severity = "error"`

#### Scenario: Normal completion remains explicit
- **WHEN** the runtime finishes naturally without abnormal diagnosis
- **THEN** `turn_end` MUST include `endReason = "completed"`
- **THEN** `severity` MUST be absent or equal to `"info"`

### Requirement: Frontend renders abnormal terminal state
The frontend SHALL render a concise visible status when a turn ends abnormally. The message SHALL explain that the turn ended due to runtime protection and whether expected artifacts were produced.

#### Scenario: Tool loop renders visible failure
- **WHEN** the frontend receives `turn_end` with `diagnosisCode = "tool_loop"` and `severity = "error"`
- **THEN** it MUST render a visible assistant-side status indicating that the turn was stopped due to a tool loop

#### Scenario: Usage remains recorded after abnormal end
- **WHEN** an abnormal `turn_end` includes usage and elapsed time summary
- **THEN** the frontend MUST still record usage and elapsed time for the resolved session id
