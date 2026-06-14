## ADDED Requirements

### Requirement: Three-dot centered divider
Iteration boundary segments SHALL render as three horizontally-aligned 4px circular dots with 6px gap between them, vertically centered with 12px top/bottom margin. No horizontal lines or text labels SHALL be displayed.

#### Scenario: Iteration boundary renders as dots
- **WHEN** an `iteration_boundary` segment is encountered in the stream
- **THEN** three 4px dots (color `var(--fill-quaternary)`) render centered horizontally with `my-3` vertical spacing

#### Scenario: No iteration number displayed
- **WHEN** an iteration boundary with iteration=3 is rendered
- **THEN** no numeric label "Step 3" or similar text appears in the output

### Requirement: Minimal vertical space
The three-dot divider SHALL occupy no more than 32px total vertical space (including margins).

#### Scenario: Divider height measurement
- **WHEN** the iteration divider element is measured
- **THEN** its total height including margins is at most 32px
