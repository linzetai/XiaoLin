## ADDED Requirements

### Requirement: Node-based transcript renderer
The frontend SHALL render chat transcripts from `TurnDisplayNode[]` rather than reconstructing message-specific segment arrays.

#### Scenario: Renderer receives display nodes
- **WHEN** a session is live or replayed
- **THEN** the renderer SHALL receive normalized display nodes
- **AND** node-specific components SHALL render assistant text, reasoning, tools, approvals, iteration boundaries, and notices

### Requirement: Stable assistant text streaming
Assistant text streaming SHALL update incrementally without rebuilding unrelated transcript nodes or causing avoidable layout shift.

#### Scenario: Text delta arrives
- **WHEN** an assistant text delta is received
- **THEN** the reducer SHALL append or coalesce it into the active assistant text node
- **AND** unrelated tool, reasoning, or previous assistant nodes SHALL retain stable identity

#### Scenario: Markdown-safe streaming
- **WHEN** streamed text contains partial markdown or code fences
- **THEN** the UI SHALL render an acceptable in-progress view
- **AND** the finalized replay SHALL render valid markdown equivalent to the completed live view

#### Scenario: Text node target is stable
- **WHEN** multiple text deltas append to the same assistant text node
- **THEN** the reducer SHALL use explicit target identity or deterministic open-node state rather than rebuilding the entire assistant message
- **AND** replay SHALL produce the same node content and relative ordering after reload

#### Scenario: Text resumes after a tool
- **WHEN** assistant text resumes after a tool step, approval, reasoning block, or iteration boundary
- **THEN** the reducer SHALL append the resumed text to the correct assistant text node according to timeline event metadata
- **AND** it SHALL NOT merge across visible timeline boundaries unless the event explicitly targets the same node

### Requirement: Reasoning display
Reasoning SHALL be represented as timeline display nodes and rendered consistently in live and replay.

#### Scenario: Reasoning is active
- **WHEN** reasoning deltas are streaming
- **THEN** the UI SHALL show an active reasoning node with subtle running state

#### Scenario: Reasoning completes
- **WHEN** the turn completes
- **THEN** completed reasoning SHALL be collapsed or visually secondary by default according to the UI policy
- **AND** replay SHALL use the same completed state

### Requirement: Iteration boundary display
Iteration boundaries SHALL be display nodes in the canonical timeline.

#### Scenario: Boundary appears live
- **WHEN** the agent emits an iteration boundary
- **THEN** the live transcript SHALL render the boundary at the correct position

#### Scenario: Boundary appears in replay
- **WHEN** the same session is replayed
- **THEN** the boundary SHALL appear at the same relative position with equivalent label and metadata

### Requirement: Terminal status display
Terminal turn status SHALL be rendered as a timeline node or notice when the turn does not end as a normal completed response.

#### Scenario: Tool loop status appears
- **WHEN** a replayed or live turn ends with a tool-loop diagnosis
- **THEN** the transcript SHALL show a terminal status node or notice after any partial assistant text
- **AND** the node SHALL expose enough user-visible text to explain that the turn stopped abnormally

#### Scenario: Cancellation or error status appears
- **WHEN** a turn is cancelled, aborted, or fails with a runtime error
- **THEN** live and replay SHALL render equivalent terminal status without relying on transient toast-only UI

### Requirement: Long transcript performance
The transcript UI SHALL remain responsive for long timelines and frequent streaming deltas.

#### Scenario: High-frequency text stream
- **WHEN** many text deltas arrive within a short interval
- **THEN** the UI SHALL batch or coalesce rendering work so input and scrolling remain responsive

#### Scenario: Long replay
- **WHEN** a session contains many turns and tool steps
- **THEN** replay SHALL use virtualization or equivalent bounded rendering work
