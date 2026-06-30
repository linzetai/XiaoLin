## MODIFIED Requirements

### Requirement: Turn-level assistant response renderer
The frontend SHALL render chat transcripts from `TurnDisplayNode[]` through Codex App / ChatGPT-like message blocks rather than reconstructing message-specific segment arrays or flattening nodes into a Codex CLI-style log. Completed assistant turns SHALL use the completed-turn process folding policy so final answers are primary and intermediate process is secondary by default.

#### Scenario: Renderer receives display nodes
- **WHEN** a session is live or replayed
- **THEN** the renderer SHALL receive normalized display nodes
- **AND** it SHALL group nodes by turn into user messages and assistant response blocks
- **AND** assistant text, reasoning, tools, approvals, and notices SHALL render in timestamp/timeline order inside the assistant response where they belong while the turn is running

#### Scenario: UI is not a CLI transcript
- **WHEN** tool, reasoning, and text nodes occur in the same assistant turn
- **THEN** tool and reasoning nodes SHALL render as assistant-response activity, not as peer chat messages
- **AND** the final assistant answer SHALL remain the primary narrative

#### Scenario: Completed process is secondary
- **WHEN** an assistant turn completes normally
- **THEN** completed reasoning, completed tool activity, approvals, and process-only activity SHALL fold into a turn-level process summary by default
- **AND** the final assistant answer SHALL remain visible as the primary response body

### Requirement: Timeline-positioned reasoning display
Reasoning SHALL be represented as timeline display nodes and rendered consistently in live and replay as in-place assistant-response activity while a turn is running. Once a turn completes normally, completed reasoning SHALL be represented inside the folded process transcript by default.

#### Scenario: Reasoning is active
- **WHEN** reasoning deltas are streaming
- **THEN** the UI SHALL show an active reasoning segment at its current timeline position with subtle running state
- **AND** it SHALL NOT move all active reasoning into one global top-of-response container

#### Scenario: Reasoning completes
- **WHEN** the turn completes
- **THEN** completed reasoning SHALL be folded into the completed-turn process summary by default
- **AND** replay SHALL use the same completed folding state

#### Scenario: Multiple reasoning segments are separated by activity
- **WHEN** reasoning occurs before and after a tool, approval, assistant text segment, or terminal status
- **THEN** the expanded process transcript SHALL preserve those reasoning segments at their original relative positions
- **AND** it MAY coalesce only consecutive reasoning deltas that are not separated by visible assistant-response activity

### Requirement: Internal iteration boundaries
Iteration boundaries SHALL remain canonical timeline metadata but SHALL NOT render as user-facing labels in the default chat UI or the default expanded process transcript.

#### Scenario: Boundary occurs live
- **WHEN** the agent emits an iteration boundary
- **THEN** the timeline SHALL preserve the boundary at the correct position
- **AND** the default chat UI SHALL NOT display labels such as `iteration 2`

#### Scenario: Boundary appears in replay data
- **WHEN** the same session is replayed
- **THEN** the boundary metadata SHALL remain available for diagnostics, grouping, and tests
- **AND** it SHALL NOT force visible user-facing transcript content

#### Scenario: Completed process expands
- **WHEN** the user expands a completed process transcript
- **THEN** iteration boundary labels SHALL remain hidden unless an explicit diagnostic mode is active

### Requirement: Terminal status display
Terminal turn status SHALL be rendered as a timeline node or notice when the turn does not end as a normal completed response. Abnormal terminal status SHALL remain visible in the default completed view when needed to explain the outcome.

#### Scenario: Tool loop status appears
- **WHEN** a replayed or live turn ends with a tool-loop diagnosis
- **THEN** the transcript SHALL show a terminal status node or notice after any partial assistant text
- **AND** the node SHALL expose enough user-visible text to explain that the turn stopped abnormally
- **AND** the terminal status SHALL NOT be hidden only inside the folded process summary

#### Scenario: Cancellation or error status appears
- **WHEN** a turn is cancelled, aborted, or fails with a runtime error
- **THEN** live and replay SHALL render equivalent terminal status without relying on transient toast-only UI
- **AND** the terminal status SHALL remain visible outside the folded process summary when it is the primary outcome
