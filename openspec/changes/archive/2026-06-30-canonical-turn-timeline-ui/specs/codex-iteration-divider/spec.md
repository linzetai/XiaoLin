## MODIFIED Requirements

### Requirement: Internal iteration metadata
Iteration boundaries SHALL be represented in the canonical timeline but SHALL NOT render as user-facing labels in the default Codex App / ChatGPT-like chat UI.

#### Scenario: Iteration boundary is preserved
- **WHEN** an `iteration_boundary` timeline event is reduced
- **THEN** an `IterationBoundaryNode` or equivalent metadata SHALL remain available at the correct relative position for diagnostics, grouping, and replay equivalence
- **AND** the default chat UI SHALL NOT display labels such as `iteration 2`

#### Scenario: Iteration boundary position matches between live and replay
- **WHEN** the same session is replayed after a live turn
- **THEN** each iteration boundary SHALL retain the same relative metadata position as in the live timeline

### Requirement: Optional diagnostic display
Diagnostic or developer views MAY display iteration boundaries, but the normal chat transcript SHALL keep them hidden or visually silent.

#### Scenario: Diagnostic mode renders boundary
- **WHEN** an `IterationBoundaryNode` is rendered
- **THEN** any visible boundary UI SHALL be gated behind an explicit diagnostic/developer mode
- **AND** it SHALL NOT appear in the default user-facing assistant response stream
