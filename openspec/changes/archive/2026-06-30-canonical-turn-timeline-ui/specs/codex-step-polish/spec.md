## MODIFIED Requirements

### Requirement: Reduced border weight
StepIndicator visual treatment SHALL apply to `ToolStepNode` rendering from the canonical timeline, using `0.5px solid var(--step-border)` or no border with only a subtle background hover state.

#### Scenario: Tool step node at rest
- **WHEN** a completed `ToolStepNode` is rendered without hover
- **THEN** its border is 0.5px or absent and its background is transparent

#### Scenario: Tool step node on hover
- **WHEN** user hovers over a `ToolStepNode`
- **THEN** background changes to `var(--step-hover-bg)` providing visual feedback

### Requirement: Reduced vertical spacing
The reduced gap between consecutive step indicators SHALL apply to canonical `ToolStepNode` rendering, including when steps are rendered inside an expanded `ToolGroupNode`.

#### Scenario: Multiple tool step nodes render compactly
- **WHEN** three consecutive `ToolStepNode` items render
- **THEN** the vertical gap between them is 2px less than the previous design

### Requirement: Subtle running state
A running `ToolStepNode` SHALL NOT apply a tinted background fill. Instead, only the status dot SHALL animate to indicate activity.

#### Scenario: Running tool step visual treatment
- **WHEN** a `ToolStepNode` has status "running"
- **THEN** the card background is transparent with no tint fill
- **AND** only the status dot animates
