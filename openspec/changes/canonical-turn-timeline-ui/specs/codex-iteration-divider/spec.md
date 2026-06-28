## MODIFIED Requirements

### Requirement: Three-dot centered divider
Iteration boundaries SHALL be `IterationBoundaryNode` display nodes in the canonical timeline, rendered at the timeline position where the boundary event occurred using the three-dot centered divider visual style.

#### Scenario: Iteration boundary node renders as dots
- **WHEN** an `iteration_boundary` timeline event is reduced
- **THEN** an `IterationBoundaryNode` SHALL render at the correct relative position as three 4px centered dots with no labels

#### Scenario: Iteration boundary position matches between live and replay
- **WHEN** the same session is replayed after a live turn
- **THEN** each `IterationBoundaryNode` SHALL appear at the same relative position with equivalent metadata as in the live transcript

### Requirement: Minimal vertical space
Iteration boundary nodes SHALL preserve the minimal vertical spacing policy.

#### Scenario: Iteration boundary height remains compact
- **WHEN** an `IterationBoundaryNode` is rendered
- **THEN** it SHALL use `my-3` spacing
- **AND** the node total height including margins SHALL be at most 32px
