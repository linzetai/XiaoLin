## ADDED Requirements

### Requirement: Turn-level assistant response renderer
The frontend SHALL render chat transcripts from `TurnDisplayNode[]` through Codex App / ChatGPT-like message blocks rather than reconstructing message-specific segment arrays or flattening nodes into a Codex CLI-style log.

#### Scenario: Renderer receives display nodes
- **WHEN** a session is live or replayed
- **THEN** the renderer SHALL receive normalized display nodes
- **AND** it SHALL group nodes by turn into user messages and assistant response blocks
- **AND** assistant text, reasoning, tools, approvals, and notices SHALL render in timestamp/timeline order inside the assistant response where they belong

#### Scenario: UI is not a CLI transcript
- **WHEN** tool, reasoning, and text nodes occur in the same assistant turn
- **THEN** tool and reasoning nodes SHALL render as assistant-response activity, not as peer chat messages
- **AND** the final assistant answer SHALL remain the primary narrative

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
- **AND** the assistant response UI SHALL preserve the relative order `text -> activity -> resumed text`

### Requirement: Timeline-positioned reasoning display
Reasoning SHALL be represented as timeline display nodes and rendered consistently in live and replay as in-place assistant-response activity.

#### Scenario: Reasoning is active
- **WHEN** reasoning deltas are streaming
- **THEN** the UI SHALL show an active reasoning segment at its current timeline position with subtle running state
- **AND** it SHALL NOT move all active reasoning into one global top-of-response container

#### Scenario: Reasoning completes
- **WHEN** the turn completes
- **THEN** completed reasoning SHALL be collapsed or visually secondary by default according to the UI policy
- **AND** replay SHALL use the same completed state

#### Scenario: Multiple reasoning segments are separated by activity
- **WHEN** reasoning occurs before and after a tool, approval, assistant text segment, or terminal status
- **THEN** the UI SHALL preserve those reasoning segments at their original relative positions
- **AND** it MAY coalesce only consecutive reasoning deltas that are not separated by visible assistant-response activity

### Requirement: Internal iteration boundaries
Iteration boundaries SHALL remain canonical timeline metadata but SHALL NOT render as user-facing labels in the default chat UI.

#### Scenario: Boundary occurs live
- **WHEN** the agent emits an iteration boundary
- **THEN** the timeline SHALL preserve the boundary at the correct position
- **AND** the default chat UI SHALL NOT display labels such as `iteration 2`

#### Scenario: Boundary appears in replay data
- **WHEN** the same session is replayed
- **THEN** the boundary metadata SHALL remain available for diagnostics, grouping, and tests
- **AND** it SHALL NOT force visible user-facing transcript content

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
