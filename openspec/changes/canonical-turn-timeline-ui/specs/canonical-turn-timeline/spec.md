## ADDED Requirements

### Requirement: Canonical UI timeline
The system SHALL use a canonical ordered turn timeline as the only source of truth for UI-visible chat transcript state.

#### Scenario: Live rendering uses the timeline
- **WHEN** a chat turn is running
- **THEN** the frontend SHALL render transcript updates from timeline-compatible events
- **AND** it SHALL NOT maintain a separate live-only transcript model with different semantics

#### Scenario: History replay uses the timeline
- **WHEN** a session is opened from history
- **THEN** the frontend SHALL render the transcript from timeline events or display nodes materialized from timeline events
- **AND** it SHALL NOT reconstruct visible transcript state from legacy `messages`, `toolCallsJson`, or `segmentOrder`

### Requirement: Live and replay display equivalence
For a completed turn, reducing the live event sequence and loading the persisted replay SHALL produce equivalent `TurnDisplayNode` content, ordering, status, and metadata.

#### Scenario: Completed live turn is reloaded
- **WHEN** a user completes a turn, closes the session, and opens the same session again
- **THEN** the visible assistant text, reasoning blocks, tool steps, approvals, iteration boundaries, and system notices SHALL match the completed live transcript

#### Scenario: Golden fixture equivalence
- **WHEN** a test fixture feeds timeline events through the live reducer and the replay materializer
- **THEN** both paths SHALL produce the same normalized `TurnDisplayNode[]`

### Requirement: Text node boundary determinism
Assistant text events SHALL carry enough content and target identity for replay to reconstruct the same text nodes and relative positions as the completed live transcript.

#### Scenario: Text is interrupted by a tool step
- **WHEN** assistant text is streamed before a tool call and more assistant text is streamed after the tool call
- **THEN** the timeline SHALL contain text payloads and tool events that preserve the text-tool-text order
- **AND** replay SHALL NOT infer missing text positions from a final assistant message body

#### Scenario: Buffered text flushes before visible non-text events
- **WHEN** the runtime has buffered assistant text and emits a visible non-text event
- **THEN** it SHALL append the pending text event before appending the non-text event
- **AND** the non-text event SHALL appear after that text in both live and replay

#### Scenario: Empty deltas do not create empty transcript nodes
- **WHEN** an assistant text or reasoning delta has no content and no intentional metadata update
- **THEN** the reducer SHALL ignore it for visible node creation

### Requirement: Stable event ordering
Timeline events SHALL have stable per-session sequence ordering and idempotent event identity.

#### Scenario: Events are ordered by sequence
- **WHEN** timeline events are loaded for a session
- **THEN** the system SHALL order them by monotonically increasing session sequence

#### Scenario: Duplicate append is idempotent
- **WHEN** the same timeline event id is appended more than once
- **THEN** the store SHALL retain a single event and SHALL NOT duplicate visible transcript nodes

### Requirement: Append-before-broadcast canonical delivery
Timeline WebSocket events SHALL be broadcast only after the canonical timeline store has assigned their durable sequence and persisted them successfully.

#### Scenario: Live event is broadcast with durable sequence
- **WHEN** the runtime emits a UI-visible timeline event
- **THEN** the append path SHALL persist it and assign its session sequence before broadcasting it
- **AND** the broadcast payload SHALL include the same event id and sequence that replay APIs return

#### Scenario: Timeline append fails
- **WHEN** persisting a timeline event fails
- **THEN** the system SHALL NOT broadcast that event as canonical timeline state
- **AND** it MAY broadcast a separate non-timeline error notice so the user is not left without feedback

### Requirement: Atomic sequence allocation
Per-session sequence numbers SHALL be allocated atomically by the persistence layer so that committed timeline events have a strictly increasing durable order, and any missing sequence range is detectable by reconnect recovery.

#### Scenario: Concurrent emitters do not collide
- **WHEN** two runtime paths emit timeline events for the same session at the same time
- **THEN** the store SHALL assign distinct, strictly increasing sequence numbers to each
- **AND** neither emitter SHALL observe a sequence number reused by another

#### Scenario: Sequence is allocated with the durable write
- **WHEN** an event is appended
- **THEN** sequence allocation and row insertion SHALL occur in a single atomic store operation
- **AND** a sequence number SHALL NOT be visible to a reader before its event row is durable

#### Scenario: Persisted gap after a failed write
- **WHEN** an event is emitted live but its durable append fails after a sequence was tentatively used
- **THEN** the durable store SHALL NOT contain that event
- **AND** the missing sequence SHALL be treated as a possible gap by reconnect recovery rather than silently skipped

#### Scenario: Live emit ordering is best-effort, durable ordering is authoritative
- **WHEN** live events are delivered to the client out of durable order under concurrency
- **THEN** the frontend reducer SHALL be order-tolerant for live delivery
- **AND** the durable sequence SHALL remain the authoritative ordering for replay and reconnect catch-up

### Requirement: Reconnect recovery
The frontend SHALL recover from WebSocket reconnects by loading timeline events after the last seen sequence and applying them through the same reducer.

#### Scenario: Reconnect catches up
- **WHEN** a client reconnects with a valid last seen sequence
- **THEN** the backend SHALL return all later events in order
- **AND** the frontend SHALL apply them without resetting completed nodes

#### Scenario: Reconnect gap reloads display nodes
- **WHEN** the backend cannot provide a complete incremental event range
- **THEN** the frontend SHALL reload materialized display nodes for the affected session
- **AND** the final transcript SHALL remain equivalent to replay

### Requirement: Terminal turn status
The canonical timeline SHALL represent non-successful, cancelled, interrupted, or diagnostically important turn endings as visible terminal status data.

#### Scenario: Tool loop ends a turn
- **WHEN** a turn ends with a `tool_loop` or equivalent terminal diagnosis
- **THEN** replay SHALL show any partial assistant text before the terminal status
- **AND** it SHALL show a visible status node or notice containing the end reason and diagnosis metadata
- **AND** it SHALL NOT render the partial assistant text as if the turn completed normally

#### Scenario: Runtime error or cancellation ends a turn
- **WHEN** a turn ends because of runtime error, cancellation, abort, or budget exhaustion
- **THEN** the timeline SHALL persist a terminal status event with user-visible status and available diagnostic metadata
- **AND** live and replay SHALL render equivalent terminal state

### Requirement: Event coverage classification
Every currently UI-visible live event type SHALL be explicitly represented by the timeline model, attached as metadata to a timeline node, or documented as excluded from transcript replay.

#### Scenario: UI-visible event type is added or audited
- **WHEN** the implementation maps runtime events into timeline events
- **THEN** assistant text, reasoning, tools, approvals, iteration boundaries, compact notices, terminal diagnostics, context warnings, brief messages, suggestions, mode changes, memory notices, and sub-agent activity SHALL each have an explicit mapping or exclusion rationale
- **AND** excluded event types SHALL NOT depend on legacy message reconstruction for replay

### Requirement: Model context isolation
Canonical timeline payloads SHALL NOT be automatically included in model-visible context.

#### Scenario: LLM context is built
- **WHEN** the backend projects session state into messages or history for the model
- **THEN** it SHALL use the existing context projection path or an explicit new projection rule
- **AND** it SHALL NOT include timeline display payloads, large output detail payloads, or display-only metadata merely because they are persisted for UI replay

### Requirement: No legacy UI migration
The change SHALL NOT require migrating pre-change development sessions into the new UI timeline.

#### Scenario: Old session is encountered
- **WHEN** a session has no timeline data because it was created before this change
- **THEN** the app MAY hide it, show an unsupported-history notice, or require development data reset
- **AND** it SHALL NOT reintroduce legacy message reconstruction as the normal replay path
