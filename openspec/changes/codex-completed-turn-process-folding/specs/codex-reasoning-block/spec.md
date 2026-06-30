## MODIFIED Requirements

### Requirement: In-place assistant reasoning activity
Reasoning SHALL be represented as `ReasoningNode` display nodes in the canonical timeline and rendered as subtle in-place assistant-response activity while the turn is active. Completed reasoning SHALL fold into the completed-turn process transcript by default after normal completion.

#### Scenario: Streaming reasoning node uses secondary style
- **WHEN** a `ReasoningNode` is streaming
- **THEN** the node renders at its timeline position with secondary visual weight, no card-like outer container, and a subtle running indicator
- **AND** it SHALL NOT be hoisted into a single global thinking panel

#### Scenario: Completed reasoning folds by default
- **WHEN** a turn completes normally with one or more completed `ReasoningNode` items
- **THEN** those reasoning nodes SHALL be hidden from the default completed transcript
- **AND** they SHALL be available inside the expanded process transcript

### Requirement: Collapse transition animation
Completed `ReasoningNode` content SHALL use the completed-turn process folding policy in live rendering and historical replay.

#### Scenario: Completed reasoning node folds identically
- **WHEN** a `ReasoningNode` completes and the turn completes normally
- **THEN** the node SHALL fold behind the turn-level process summary in both live completed view and replay
- **AND** expanding the process summary SHALL reveal the reasoning content with stable identity

### Requirement: Expand/collapse toggle
The expand/collapse toggle SHALL operate at the turn-level process summary for completed turns and SHALL NOT depend on legacy message reconstruction.

#### Scenario: User toggles completed process
- **WHEN** user expands or collapses a completed turn process summary
- **THEN** the reasoning nodes inside that turn SHALL become visible or hidden according to the summary state
- **AND** text, tool, and boundary nodes SHALL retain stable identity

### Requirement: Reasoning segmentation preserves chronology
Multiple reasoning phases in a turn SHALL preserve their original timeline positions inside the expanded process transcript.

#### Scenario: Reasoning before and after a tool
- **WHEN** reasoning occurs before a tool call and additional reasoning occurs after that tool call
- **THEN** the expanded process transcript SHALL render two reasoning segments around the tool activity in the same relative order
- **AND** consecutive reasoning deltas MAY coalesce only until a visible assistant-response activity boundary occurs
