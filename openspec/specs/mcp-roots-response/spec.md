## ADDED Requirements

### Requirement: Client responds to server-initiated roots/list requests

The system SHALL respond to `roots/list` requests from MCP servers with the current workspace root path.

#### Scenario: roots/list returns workspace URI

- **WHEN** an MCP server sends a `roots/list` request
- **THEN** the client SHALL respond with a JSON array containing `{ uri: "file:///path/to/workspace" }` using the current session's workspace path

#### Scenario: Client declares roots capability on initialize

- **WHEN** the MCP client sends the `initialize` request
- **THEN** the `capabilities` object SHALL include `"roots": {}`

#### Scenario: Multiple workspaces listed when available

- **WHEN** the session has multiple open workspace paths
- **THEN** the `roots/list` response SHALL include all workspace paths as separate root entries
