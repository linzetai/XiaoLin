## ADDED Requirements

### Requirement: Float breathing animation for empty states
A CSS keyframe animation `pv-float` SHALL be defined that gently translates an element up and down (6px range) over a 3-second cycle, creating a breathing/floating effect for empty state icons.

#### Scenario: Empty state icon floats
- **WHEN** the MCP empty state is displayed
- **THEN** the puzzle piece icon container SHALL have the `pv-float` class applied, creating a continuous gentle vertical floating motion

### Requirement: Modal entrance and exit animation
Modal overlays (McpDetailModal, AddServerModal) SHALL animate in with a scale-up + fade-in effect, and the backdrop SHALL fade in.

#### Scenario: Modal opens with scale animation
- **WHEN** a modal transitions from closed to open
- **THEN** the modal content SHALL animate from scale(0.96) + opacity(0) to scale(1) + opacity(1) over 200ms with ease-out timing

#### Scenario: Modal backdrop fades in
- **WHEN** a modal opens
- **THEN** the backdrop overlay SHALL fade from opacity(0) to the target opacity over 150ms

### Requirement: Card stagger entrance in Explore grid
When the Explore panel renders or re-filters its card grid, cards SHALL appear with a staggered fade-slide-up animation, each card delayed by 30ms from the previous.

#### Scenario: Cards stagger on initial load
- **WHEN** the Explore panel first renders its card grid
- **THEN** each card SHALL animate in with `fade-slide-up` keyframe, with card[i] delayed by `i * 30ms`

#### Scenario: Cards stagger after filter change
- **WHEN** the user changes the category filter
- **THEN** the filtered cards SHALL re-animate with the same stagger pattern

### Requirement: Sub-view transition between Installed and Explore
Switching between the Installed and Explore sub-views SHALL use a cross-fade transition rather than an instant swap.

#### Scenario: Switch from Installed to Explore
- **WHEN** user clicks the "Explore" toggle
- **THEN** the Installed content SHALL fade out and the Explore content SHALL fade in over 200ms
