## Overview

Enable concurrent MCP tool calls by replacing the global Mutex in `McpClient` with request-id multiplexing.

## Requirements

- Multiple concurrent `tools/call` requests can be in-flight to the same MCP server
- Each request gets a unique JSON-RPC `id` and a dedicated response channel
- Background reader task dispatches responses by `id`
- Server process crash cleans up all pending requests with an error
- Request timeout (configurable, default 30s) cancels pending requests
- No change to MCP protocol semantics
- Existing `McpServer` (stdio) implementation unaffected
