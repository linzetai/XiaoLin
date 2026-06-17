## ADDED Requirements

### Requirement: Reduced border weight
StepIndicator card border SHALL use `0.5px solid var(--step-border)` instead of `1px`, or optionally no border at all with only a subtle background hover state.

#### Scenario: Step card at rest
- **WHEN** a completed StepIndicator is rendered without hover
- **THEN** its border is 0.5px or absent, background is transparent

#### Scenario: Step card on hover
- **WHEN** user hovers over a StepIndicator
- **THEN** background changes to `var(--step-hover-bg)` providing visual feedback

### Requirement: Reduced vertical spacing
The gap between consecutive StepIndicator cards (`--step-gap`) SHALL be reduced by 2px from its current value to increase information density.

#### Scenario: Multiple tools render compactly
- **WHEN** three consecutive tool steps render
- **THEN** the vertical gap between them is 2px less than the previous design

### Requirement: Subtle running state
A running StepIndicator SHALL NOT apply a tinted background fill. Instead, only the status dot SHALL animate (spin) to indicate activity.

#### Scenario: Running tool visual treatment
- **WHEN** a tool has status "running"
- **THEN** the card background is transparent (no tint fill), only the 5px status dot spins
