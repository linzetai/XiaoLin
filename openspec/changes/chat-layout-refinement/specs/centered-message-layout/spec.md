## ADDED Requirements

### Requirement: Messages are horizontally centered
Message rows SHALL be horizontally centered within the scroll container, with equal whitespace on both sides.

#### Scenario: Message displayed in standard layout
- **WHEN** a message is rendered in the chat area (standard or wide layout tier)
- **THEN** the message content block is centered horizontally with equal padding on both sides

### Requirement: Elastic side padding
The scroll container SHALL use elastic padding that scales with container width, using `clamp(24px, 5%, 80px)` for left and right padding.

#### Scenario: Narrow container
- **WHEN** the chat container width is below 600px
- **THEN** side padding is 24px (minimum)

#### Scenario: Wide container
- **WHEN** the chat container width exceeds 1600px
- **THEN** side padding is 80px (maximum)

### Requirement: Content max-width reduction
The `--content-max-w` token SHALL be reduced to provide more breathing room:
- standard tier: 660px (from 720px)
- wide tier: 760px (from 860px)
- compact tier: unchanged (`calc(100% - 32px)`)

#### Scenario: Standard layout content width
- **WHEN** layout tier is "standard"
- **THEN** message content maxWidth is 660px, centered within the available space

#### Scenario: Wide layout content width
- **WHEN** layout tier is "wide"
- **THEN** message content maxWidth is 760px, centered within the available space

### Requirement: Message content uses margin auto for centering
Message content containers (`.ai-body`, user input, etc.) SHALL use `margin: 0 auto` with `maxWidth: var(--content-max-w)` for centering.

#### Scenario: AI message body centered
- **WHEN** an AI message is rendered
- **THEN** the `.ai-body` div has `margin: 0 auto` and `maxWidth: var(--content-max-w)`
