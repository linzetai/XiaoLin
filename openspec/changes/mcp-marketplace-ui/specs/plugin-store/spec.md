## MODIFIED Requirements

### Requirement: usePluginStore structure
前端 SHALL 提供 `usePluginStore`（Zustand），管理插件面板 UI 状态与 MCP 插件数据。

#### Scenario: Store initial state
- **WHEN** 应用启动且 store 未初始化
- **THEN** `plugins` 为空数组、`loading` 为 false、`error` 为 null

#### Scenario: Store holds plugin list fields
- **WHEN** list 数据已加载
- **THEN** 每条 `PluginEntry` 包含：`id`、`scope`（user|project）、`enabled`、`status`、`toolCount`、`lastError?`、`connectedAt?`

## ADDED Requirements

### Requirement: Add plugin action
Store SHALL 提供 `addPlugin(params)` action，调用扩展后的 `mcp.add` API 并在成功后刷新插件列表。

#### Scenario: Add stdio server
- **WHEN** 调用 `addPlugin({ id: "fs", command: "npx", args: ["@mcp/fs"], transport: "stdio" })`
- **THEN** 发送 `mcp.add` WebSocket 请求，传入所有参数
- **AND** 成功后自动调用 `fetchPlugins()` 刷新列表
- **AND** 返回 `true`

#### Scenario: Add HTTP server
- **WHEN** 调用 `addPlugin({ id: "remote", transport: "streamable_http", url: "https://..." })`
- **THEN** 发送 `mcp.add` WebSocket 请求，包含 transport 和 url
- **AND** 成功后刷新列表

#### Scenario: Add failure
- **WHEN** `mcp.add` 返回错误
- **THEN** `error` 设为错误消息
- **AND** 返回 `false`

### Requirement: Remove plugin action
Store SHALL 提供 `removePlugin(id)` action，调用 `mcp.remove` API 并在成功后刷新插件列表。

#### Scenario: Remove existing server
- **WHEN** 调用 `removePlugin("fs")`
- **THEN** 发送 `mcp.remove { id: "fs" }` WebSocket 请求
- **AND** 成功后自动调用 `fetchPlugins()` 刷新列表
- **AND** 返回 `true`

#### Scenario: Remove failure
- **WHEN** `mcp.remove` 返回错误
- **THEN** `error` 设为错误消息
- **AND** 返回 `false`
