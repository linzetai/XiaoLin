## MODIFIED Requirements

### Requirement: Assistant activity tool rows
Tool step display SHALL render `ToolStepNode` items as assistant-response activity while a turn is running, and SHALL fold completed tool rows into the completed-turn process summary by default after normal completion.

#### Scenario: Running tool row is visible
- **WHEN** a `ToolStepNode` has status `running`
- **THEN** the assistant response SHALL show a compact running activity row at the tool node's timeline position
- **AND** the row SHALL include a semantic title, status, and target metadata when available

#### Scenario: Completed tool row folds
- **WHEN** a turn completes normally with completed `ToolStepNode` items
- **THEN** those completed tool rows SHALL fold behind the turn-level process summary by default
- **AND** expanding the process summary SHALL reveal the completed tool rows in chronological order

### Requirement: Semantic tool titles
Tool activity SHALL use semantic, user-facing titles in default summaries and expanded process rows; raw tool names SHALL NOT be the primary visible label unless no semantic title can be derived.

#### Scenario: Shell command is summarized
- **WHEN** a shell command tool node is rendered in a default process summary
- **THEN** the summary SHALL use a concise phrase such as `已运行 3 条命令` or `正在运行 1 条命令`
- **AND** it SHALL NOT expose raw command titles as separate first-class default rows after completion

#### Scenario: Raw tool name remains inspectable
- **WHEN** the user expands a process detail row for a tool
- **THEN** the raw tool name or command MAY be shown as secondary detail
- **AND** the semantic title SHALL remain the primary label

### Requirement: Tool grouping
Adjacent repetitive tool steps SHALL be groupable in the completed process transcript while preserving individual detail order.

#### Scenario: Completed adjacent tools group
- **WHEN** multiple adjacent completed tool steps share a semantic activity family
- **THEN** the expanded process transcript MAY group them into one activity group
- **AND** each individual tool detail SHALL remain inspectable in canonical order

#### Scenario: Distinct activity families stay separate
- **WHEN** diff inspection and sub-agent review activity occur adjacent to each other
- **THEN** the UI SHALL keep them as separate semantic groups
- **AND** the default completed view SHALL still show only one turn-level process summary row unless abnormal status requires otherwise
