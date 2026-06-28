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

### Requirement: Long transcript performance
The transcript UI SHALL remain responsive for long timelines and frequent streaming deltas.

#### Scenario: High-frequency text stream
- **WHEN** many text deltas arrive within a short interval
- **THEN** the UI SHALL batch or coalesce rendering work so input and scrolling remain responsive

#### Scenario: Long replay
- **WHEN** a session contains many turns and tool steps
- **THEN** replay SHALL use virtualization or equivalent bounded rendering work
