## ADDED Requirements

### Requirement: Backend forwards iteration boundary events
The backend SHALL forward `ToolRoundBoundary` as `IterationBoundary { iteration }` to the WebSocket frontend instead of filtering it.

#### Scenario: Tool round completes
- **WHEN** a tool round finishes and the agent starts the next LLM iteration
- **THEN** an `IterationBoundary { iteration: N }` event SHALL be sent to the frontend

### Requirement: Frontend displays iteration counter
The frontend SHALL display the current iteration number during multi-step agent execution.

#### Scenario: Multi-iteration turn
- **WHEN** iteration_boundary events arrive with iteration > 1
- **THEN** the streaming row SHALL display "Step N" as a separator between tool groups

#### Scenario: Single-iteration turn
- **WHEN** the turn completes with only 1 iteration (no iteration_boundary received)
- **THEN** no "Step" counter SHALL be displayed

### Requirement: Iteration separator in tool groups
When a new iteration starts, tool groups from different iterations SHALL be visually separated.

#### Scenario: Tools across two iterations
- **WHEN** tools from iteration 1 complete and iteration 2 begins new tools
- **THEN** a visual separator with "Step 2" label SHALL appear between the two groups
