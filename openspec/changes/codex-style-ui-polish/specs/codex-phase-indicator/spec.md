## ADDED Requirements

### Requirement: Pulsing dot indicator
PhaseIndicator SHALL display a single 8px circular dot with a CSS pulse animation (scale 0.8 to 1.2, opacity 0.4 to 1, duration 1.5s infinite) instead of the multi-ring OrbitSpinner SVG.

#### Scenario: Thinking phase displays pulsing dot
- **WHEN** PhaseIndicator renders with `phase="thinking"`
- **THEN** an 8px dot with `background: var(--tint)` and pulse animation is visible, no SVG element present

#### Scenario: Connecting phase displays pulsing dot
- **WHEN** PhaseIndicator renders with `phase="connecting"`
- **THEN** the same pulsing dot is shown with appropriate phase label

### Requirement: Elapsed timer display
PhaseIndicator SHALL display an elapsed time counter (in seconds) next to the phase label, starting from when the indicator mounts.

#### Scenario: Timer increments during thinking
- **WHEN** PhaseIndicator has been visible for 5 seconds
- **THEN** the display shows "思考中 5s" (or localized equivalent)

#### Scenario: Timer resets on phase change
- **WHEN** phase changes from "connecting" to "thinking"
- **THEN** the elapsed timer resets to 0

### Requirement: No SVG spinner
The OrbitSpinner SVG component SHALL be removed from PhaseIndicator rendering. All animation SHALL use CSS-only techniques.

#### Scenario: DOM inspection shows no SVG
- **WHEN** PhaseIndicator is rendered
- **THEN** no `<svg>` element exists within its subtree
