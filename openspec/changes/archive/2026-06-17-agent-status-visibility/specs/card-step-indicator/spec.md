## MODIFIED Requirements

### Requirement: StepIndicator displays tool execution progress
The StepIndicator component SHALL consume `tool_progress` events (matched by call_id) to display incremental progress for running tools, in addition to the existing elapsed timer.

#### Scenario: Tool emits progress with percentage
- **WHEN** a `tool_progress` event arrives with `progress` field (0.0-1.0)
- **THEN** the StepIndicator SHALL display a progress bar alongside the elapsed timer

#### Scenario: Tool emits progress with message
- **WHEN** a `tool_progress` event arrives with `message` field
- **THEN** the StepIndicator SHALL display the message text below the tool name (replacing the args preview)

#### Scenario: Tool without progress events
- **WHEN** a tool is running and no `tool_progress` events are received
- **THEN** the StepIndicator SHALL display only the spinner and elapsed timer (current behavior unchanged)
