## MODIFIED Requirements

### Requirement: In-place assistant reasoning activity
Reasoning SHALL be represented as `ReasoningNode` display nodes in the canonical timeline and rendered as subtle in-place assistant-response activity in both live and replay states.

#### Scenario: Streaming reasoning node uses secondary style
- **WHEN** a `ReasoningNode` is streaming
- **THEN** the node renders at its timeline position with secondary visual weight, no card-like outer container, and a subtle running indicator
- **AND** it SHALL NOT be hoisted into a single global thinking panel

### Requirement: Fixed-height streaming panel with auto-scroll
The fixed-height streaming panel and auto-scroll behavior SHALL apply to active `ReasoningNode` content.

#### Scenario: Active reasoning node auto-scrolls
- **WHEN** reasoning deltas are appended to an active `ReasoningNode`
- **THEN** the content area SHALL use the fixed max-height panel with auto-scroll
- **AND** unrelated transcript nodes SHALL retain stable identity

### Requirement: Collapse transition animation
Completed `ReasoningNode` content SHALL use the same collapse transition policy in live rendering and historical replay.

#### Scenario: Completed reasoning node collapses identically
- **WHEN** a `ReasoningNode` completes
- **THEN** the node renders with the dimmed left-line style and animates to collapsed state per the autoCollapse policy
- **AND** replay SHALL render the same completed, collapsed state as the live turn

### Requirement: Expand/collapse toggle
The expand/collapse toggle SHALL operate on canonical `ReasoningNode` state and SHALL NOT depend on legacy message reconstruction.

#### Scenario: User toggles completed reasoning
- **WHEN** user expands or collapses a completed `ReasoningNode`
- **THEN** only that reasoning node's expanded state changes
- **AND** text, tool, and boundary nodes SHALL retain stable identity

### Requirement: Reasoning segmentation preserves chronology
Multiple reasoning phases in a turn SHALL preserve their original timeline positions.

#### Scenario: Reasoning before and after a tool
- **WHEN** reasoning occurs before a tool call and additional reasoning occurs after that tool call
- **THEN** the UI SHALL render two reasoning segments around the tool activity in the same relative order
- **AND** consecutive reasoning deltas MAY coalesce only until a visible assistant-response activity boundary occurs
