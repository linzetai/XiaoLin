## ADDED Requirements

### Requirement: MCP progress notifications are forwarded to UI

The system SHALL handle `notifications/progress` from MCP servers and forward progress information to the frontend for display during long-running tool calls.

#### Scenario: Progress with total displays percentage

- **WHEN** an MCP server sends `notifications/progress` with `progressToken`, `progress: 5`, `total: 10`
- **THEN** the system SHALL broadcast a `plugins.tool_progress` WebSocket event
- **AND** the frontend SHALL display a progress indicator showing 50% completion

#### Scenario: Progress without total displays indeterminate

- **WHEN** an MCP server sends `notifications/progress` with `progress: 3` but no `total`
- **THEN** the frontend SHALL display an indeterminate progress indicator with the progress value

#### Scenario: Progress message is displayed

- **WHEN** an MCP server sends `notifications/progress` with `message: "Processing file 5 of 10"`
- **THEN** the frontend SHALL display the message text alongside the progress indicator

#### Scenario: Progress token maps to active tool call

- **WHEN** the system sends a `tools/call` with `_meta.progressToken: "tok-123"`
- **AND** the server sends `notifications/progress` with `progressToken: "tok-123"`
- **THEN** the system SHALL associate the progress with the correct tool call UI element
