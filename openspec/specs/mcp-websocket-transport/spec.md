## ADDED Requirements

### Requirement: MCP client supports WebSocket transport

The system SHALL support connecting to MCP servers over WebSocket (`ws://` or `wss://`) as a transport option alongside stdio, SSE, and Streamable HTTP.

#### Scenario: WebSocket connection with subprotocol

- **WHEN** a server config has `transport: "ws"` and `url: "wss://example.com/mcp"`
- **THEN** the client SHALL open a WebSocket connection with subprotocol `mcp`
- **AND** send/receive JSON-RPC messages as text frames

#### Scenario: Initialize handshake over WebSocket

- **WHEN** the WebSocket connection is established
- **THEN** the client SHALL send the MCP `initialize` request
- **AND** wait for `InitializeResult` before sending `notifications/initialized`

#### Scenario: Notifications received via WebSocket

- **WHEN** the MCP server sends a JSON-RPC notification over the WebSocket
- **THEN** the client SHALL dispatch it through the existing notification broadcast channel

#### Scenario: Disconnect triggers reconnect

- **WHEN** the WebSocket connection drops unexpectedly
- **THEN** the system SHALL attempt reconnection with exponential backoff (up to 5 attempts)
- **AND** re-initialize the MCP session on successful reconnect

#### Scenario: McpTransportType enum includes WebSocket

- **WHEN** a server config specifies `transport: "ws"` or `transport: "websocket"`
- **THEN** the system SHALL route to the WebSocket connection path
- **AND** validate that `url` is present and starts with `ws://` or `wss://`
