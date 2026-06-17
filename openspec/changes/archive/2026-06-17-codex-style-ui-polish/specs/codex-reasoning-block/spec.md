## ADDED Requirements

### Requirement: Left-line visual style
ReasoningBlock SHALL render with a 2px left border using `var(--tint)` color and no outer border/background card styling. A 6px pulsing dot SHALL appear at the top-left corner while streaming is active.

#### Scenario: Streaming reasoning displays left-line style
- **WHEN** reasoning content is streaming (`isStreaming=true`)
- **THEN** the block renders with `border-left: 2px solid var(--tint)`, no outer border, no background color, and a pulsing 6px dot at the top-left

#### Scenario: Completed reasoning displays left-line style
- **WHEN** reasoning is complete (`isStreaming=false`)
- **THEN** the block renders with `border-left: 2px solid var(--fill-quaternary)` (dimmed), no pulsing dot

### Requirement: Fixed-height streaming panel with auto-scroll
During streaming, the reasoning content area SHALL have a fixed max-height of 200px with `overflow-y: auto`. The panel SHALL automatically scroll to the bottom as new content arrives.

#### Scenario: Streaming content exceeds max height
- **WHEN** reasoning content length exceeds the 200px panel height during streaming
- **THEN** the panel scrolls to bottom automatically, showing latest content

#### Scenario: User manually scrolls up during streaming
- **WHEN** user scrolls up in the reasoning panel while streaming
- **THEN** auto-scroll pauses until user scrolls back to bottom

### Requirement: Collapse transition animation
When reasoning completes and `autoCollapse` triggers, the block SHALL animate its height to collapsed state using a CSS transition (max-height 300ms ease-out).

#### Scenario: Reasoning completes with following content
- **WHEN** reasoning finishes and text/tool segments follow (`autoCollapse=true`)
- **THEN** the panel height animates from current to collapsed (header only) over 300ms

### Requirement: Expand/collapse toggle
Clicking the header area SHALL toggle between collapsed (header only) and expanded (full content with max-height removed) states.

#### Scenario: User clicks collapsed reasoning header
- **WHEN** user clicks the reasoning block header while collapsed
- **THEN** the block expands to show full content without max-height constraint
