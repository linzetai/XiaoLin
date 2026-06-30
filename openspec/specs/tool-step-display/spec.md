## ADDED Requirements

### Requirement: Assistant activity tool rows
Tool calls SHALL render as compact assistant-response activity rows with semantic status and low visual noise.

#### Scenario: Tool starts
- **WHEN** a tool call starts
- **THEN** the UI SHALL render a compact running activity row inside the current assistant response with tool title, status indicator, and primary target metadata when available
- **AND** it SHALL NOT render as a peer chat message or a terminal-style log block

#### Scenario: Tool finishes
- **WHEN** a tool call succeeds or fails
- **THEN** the UI SHALL update the same activity row with final status, duration, and result summary

### Requirement: Semantic tool titles
Tool display nodes SHALL provide human-readable titles derived from tool name and arguments without exposing raw JSON by default.

#### Scenario: Shell command title
- **WHEN** a shell command tool is displayed
- **THEN** the step title SHALL summarize the command or action and expose full arguments only in expanded details

#### Scenario: File or search title
- **WHEN** a file read, file write, or search tool is displayed
- **THEN** the step title SHALL include the path, query, or target in a compact form

### Requirement: Small output inline preview
Tool output that satisfies the display small-output policy SHALL be fully replayable from inline display-node data without an additional detail fetch.

#### Scenario: Small output renders inline
- **WHEN** tool output is <= 8,000 UTF-8 bytes, <= 200 lines, <= 2,000 estimated display tokens, and is not binary
- **THEN** the tool display node SHALL include the complete text output or an equivalent complete structured representation inline
- **AND** any summary or preview SHALL be additional display metadata, not a replacement for the inline small output
- **AND** the default assistant-response UI MAY keep that inline output collapsed so tool output does not overpower the assistant narrative

#### Scenario: Small output does not require extra fetch
- **WHEN** a historical transcript containing only small tool outputs is opened
- **THEN** the UI SHALL NOT need to call a tool detail API merely to show the default assistant response

### Requirement: Large output lazy details
Large or structured tool output SHALL be summarized in the transcript and fetched only when the user expands details.

#### Scenario: Large output summary
- **WHEN** tool output exceeds any small-output threshold
- **THEN** the default activity row SHALL show status, size metadata when available, and a bounded summary
- **AND** it SHALL provide an expansion affordance when full details are available

#### Scenario: Detail expansion
- **WHEN** the user expands a large-output step
- **THEN** the UI SHALL fetch details through an authorized backend API
- **AND** it SHALL render loading, success, empty, expired, and error states

#### Scenario: Detail expansion is paged or sectional
- **WHEN** expanded output exceeds the detail response size limit
- **THEN** the UI SHALL render the returned bounded section and provide available continuation, range, tail, summary, or search affordances
- **AND** it SHALL NOT require loading the entire output blob to keep the transcript usable

### Requirement: Tool grouping
When the product enables tool grouping, consecutive low-value or repetitive tool steps SHALL be grouped into a `ToolGroupNode` while preserving individual details.

#### Scenario: Repetitive steps are grouped
- **WHEN** multiple adjacent tool steps are eligible for grouping
- **THEN** the UI SHALL render a compact activity group summary inside the assistant response
- **AND** expanding the group SHALL reveal the individual step nodes in original order

### Requirement: Visual regression coverage
Tool step display SHALL have regression coverage for running, success, failure, grouped, small-output, large-output, and replay states.

#### Scenario: Tool fixture screenshots
- **WHEN** frontend visual tests run
- **THEN** screenshots or DOM assertions SHALL cover live and replay rendering for representative tool sequences
