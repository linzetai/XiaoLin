## ADDED Requirements

### Requirement: Durable timeline event store
The backend SHALL persist UI timeline events in an append-only store with session id, turn id, event id, sequence, event type, schema version, payload, and creation time.

#### Scenario: Event is persisted
- **WHEN** the runtime emits a UI-visible timeline event
- **THEN** the session store SHALL persist the event before it is considered durable for replay

#### Scenario: Event has schema version
- **WHEN** a timeline event is stored
- **THEN** it SHALL include a schema version so future materializers can handle payload evolution explicitly

### Requirement: Timeline query API
The backend SHALL expose an API for loading timeline events by session and sequence range.

#### Scenario: Load events after sequence
- **WHEN** the client requests events after a known sequence
- **THEN** the API SHALL return matching events in ascending sequence order with pagination metadata

#### Scenario: Empty range
- **WHEN** no events exist after the requested sequence
- **THEN** the API SHALL return an empty event list and the current high-water sequence

### Requirement: Display node API
The backend SHALL expose an API for loading materialized `TurnDisplayNode` data derived from the canonical timeline.

#### Scenario: Load display nodes for session
- **WHEN** the frontend opens a session
- **THEN** it SHALL be able to request display nodes without reading legacy chat messages

#### Scenario: Display node source traceability
- **WHEN** a display node is materialized
- **THEN** it SHALL retain enough source event ids or sequence range metadata to debug live/replay mismatches

### Requirement: Tool detail API
The backend SHALL expose a UI-authorized, read-only detail API that serves bounded content or structured detail sections for a `ToolOutputHandle` referenced by a tool display node. The API SHALL be a view over the existing tool output asset store from the `tool-output-assets` capability and SHALL NOT introduce a parallel output backend or handle scheme.

#### Scenario: Small output remains inline
- **WHEN** a tool output satisfies the display small-output policy
- **THEN** the display node SHALL include complete inline output data sufficient for replay without a detail API request

#### Scenario: Large output uses an existing handle reference
- **WHEN** a tool output exceeds any small-output threshold
- **THEN** the display node MAY include a bounded summary and SHALL reference an existing `ToolOutputHandle` when expanded output is available
- **AND** the detail reference SHALL be resolvable by the UI detail API under the same session-scoped ownership rules as the agent-facing recall tools

### Requirement: Timeline storage tests
Timeline persistence SHALL have tests for ordering, idempotency, pagination, reconnect recovery, and display-node materialization.

#### Scenario: Storage test suite runs
- **WHEN** timeline persistence code changes
- **THEN** tests SHALL verify sequence ordering, duplicate event handling, after-sequence loading, empty ranges, and materialized display output
