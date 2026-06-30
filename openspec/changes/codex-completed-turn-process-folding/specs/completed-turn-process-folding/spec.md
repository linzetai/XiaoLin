## ADDED Requirements

### Requirement: Running turn process visibility
The chat UI SHALL keep reasoning and tool activity visible in chronological order while an assistant turn is still running.

#### Scenario: Active reasoning is visible
- **WHEN** reasoning deltas are streaming for an active turn
- **THEN** the UI SHALL show the active reasoning segment at its timeline position
- **AND** the segment SHALL indicate that thinking is still in progress

#### Scenario: Running tool is visible
- **WHEN** a tool call is running for an active turn
- **THEN** the UI SHALL show a semantic running activity row at its timeline position
- **AND** the row SHALL include enough status to show that work is ongoing

### Requirement: Completed turn process folding
After a turn completes normally, the default chat view SHALL fold completed reasoning, completed tool activity, approvals, and process-only activity into a single expandable process summary row.

#### Scenario: Normal completion folds process
- **WHEN** a turn contains reasoning, completed tools, and a final assistant answer
- **AND** the turn has completed normally
- **THEN** the default transcript SHALL show the final assistant answer as primary content
- **AND** it SHALL show one process summary row for the folded intermediate process
- **AND** it SHALL NOT show each completed reasoning or tool row as first-class default content

#### Scenario: Summary row reports elapsed processing
- **WHEN** folded process activity has duration metadata
- **THEN** the summary row SHALL show a concise processed duration such as `已处理 28s`
- **AND** the summary row SHALL expose an expand affordance

### Requirement: Expanded process transcript
The completed-turn process summary SHALL expand to reveal the folded process transcript in canonical timeline order.

#### Scenario: User expands completed process
- **WHEN** the user expands the completed process summary row
- **THEN** the UI SHALL show reasoning, tool activity, approvals, and relevant process statuses in their original relative order
- **AND** adjacent repetitive tool activity MAY be semantically grouped only if individual detail order remains inspectable

#### Scenario: User collapses completed process
- **WHEN** the user collapses the expanded process transcript
- **THEN** the UI SHALL return to the default completed view with one process summary row
- **AND** the final assistant answer SHALL remain visible

### Requirement: Abnormal terminal visibility
Abnormal turn endings SHALL keep user-visible terminal context outside the default folded process when that context is necessary to explain the outcome.

#### Scenario: Runtime error is visible
- **WHEN** a turn ends with a runtime error
- **THEN** the default transcript SHALL show an error or terminal status notice outside the folded process summary
- **AND** completed process activity MAY remain folded behind the process summary

#### Scenario: Tool loop is visible
- **WHEN** a turn ends with a tool-loop or budget terminal diagnosis
- **THEN** the default transcript SHALL show terminal context explaining that the turn did not complete normally
- **AND** it SHALL NOT present partial assistant text as a normal final answer

### Requirement: Replay uses the same folding policy
Live completed turns and replayed completed turns SHALL use the same process folding policy.

#### Scenario: Completed turn is reloaded
- **WHEN** a completed session is reopened from history
- **THEN** the default transcript SHALL fold the same process activity that was folded after live completion
- **AND** expanding the summary SHALL reveal equivalent process chronology

#### Scenario: Live transition matches replay
- **WHEN** an active turn transitions to normal completion
- **THEN** the UI SHALL replace the live process rows with the completed process summary
- **AND** the resulting DOM SHALL be equivalent to replay for the same canonical timeline nodes
