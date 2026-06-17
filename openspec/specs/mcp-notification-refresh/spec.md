## ADDED Requirements

### Requirement: resources/list_changed triggers cache refresh

The system SHALL refresh the cached resource list when receiving `notifications/resources/list_changed` from an MCP server, and notify the frontend of the change.

#### Scenario: Resource list refresh on notification

- **WHEN** the MCP server sends `notifications/resources/list_changed`
- **THEN** the system SHALL call `list_resources()` on the affected server
- **AND** update the cached resource list for that server
- **AND** broadcast a `plugins.resources_changed` WebSocket event

#### Scenario: Resource tool re-registration after refresh

- **WHEN** the resource list changes (new resources added or existing removed)
- **THEN** the system SHALL re-register `mcp__list_resources` and `mcp__read_resource` deferred tools with updated descriptions

#### Scenario: Refresh failure is logged but does not crash

- **WHEN** the `list_resources()` call fails after receiving `resources/list_changed`
- **THEN** the system SHALL log a warning with the server ID and error
- **AND** retain the previous cached resource list

### Requirement: prompts/list_changed triggers cache refresh

The system SHALL refresh the cached prompt list when receiving `notifications/prompts/list_changed` from an MCP server.

#### Scenario: Prompt list refresh on notification

- **WHEN** the MCP server sends `notifications/prompts/list_changed`
- **THEN** the system SHALL call `list_prompts()` on the affected server
- **AND** update the cached prompt list for that server
- **AND** broadcast a `plugins.prompts_changed` WebSocket event
