## ADDED Requirements

### Requirement: XiaoLin exposes built-in tools as MCP server

The system SHALL provide an `McpServer` implementation backed by `ToolRegistry`, exposing XiaoLin's built-in tools to external MCP hosts. The server supports stdio transport via `McpServer::run_stdio()` and can be integrated into the Tauri app or used programmatically.

> **Note**: XiaoLin is a desktop application, not a CLI tool. There is no standalone `xiaolin mcp serve` command. The reverse MCP server is an internal API available for future integration (e.g., via Tauri IPC or plugin).

#### Scenario: Server starts and handles initialize

- **WHEN** the MCP server is started (via `run_stdio()` or other transport)
- **THEN** it SHALL handle JSON-RPC messages per the MCP protocol
- **AND** respond to `initialize` with server capabilities including `tools: { listChanged: true }`
- **AND** include `serverInfo: { name: "XiaoLin", version: "<pkg_version>" }`

#### Scenario: tools/list returns built-in tools only

- **WHEN** an MCP host sends `tools/list`
- **THEN** the server SHALL return XiaoLin's built-in tools (shell, file operations, search, etc.)
- **AND** SHALL NOT include tools from connected remote MCP servers (prevents circular calls)

#### Scenario: tools/call executes a built-in tool

- **WHEN** an MCP host sends `tools/call` with a valid tool name and arguments
- **THEN** the server SHALL execute the tool through the existing `ToolRegistry`
- **AND** return the result as `content: [{ type: "text", text: "..." }]`

#### Scenario: Unknown tool returns error

- **WHEN** an MCP host sends `tools/call` with an unrecognized tool name
- **THEN** the server SHALL return a JSON-RPC error with code `-32602` (invalid params)

#### Scenario: Server is not auto-started

- **WHEN** XiaoLin desktop app starts normally
- **THEN** the MCP server SHALL NOT start automatically
- **AND** it SHALL only be activated when explicitly enabled via settings or API
