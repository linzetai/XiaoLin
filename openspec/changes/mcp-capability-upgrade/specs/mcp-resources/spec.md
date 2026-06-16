## ADDED Requirements

### Requirement: Resources 客户端 API

`McpClient` SHALL 实现以下 MCP Resources 客户端方法：

1. `list_resources()` → 返回服务器支持的资源列表（name, uri, description, mimeType）
2. `read_resource(uri)` → 读取指定 URI 的资源内容
3. `list_resource_templates()` → 返回资源模板列表（uriTemplate, name, description）

这些方法 SHALL 在 `initialize` 成功后，根据服务器声明的 `capabilities.resources` 决定是否可用。

#### Scenario: 服务器声明 resources 能力
- **WHEN** MCP 服务器在 `initialize` 响应中声明 `capabilities.resources`
- **THEN** `list_resources()`、`read_resource()` 和 `list_resource_templates()` SHALL 可用

#### Scenario: 服务器未声明 resources 能力
- **WHEN** MCP 服务器未声明 `capabilities.resources`
- **THEN** 调用 resources 相关方法 SHALL 返回空列表或明确错误，不发送 RPC 请求

### Requirement: Resources 作为 Agent 工具

系统 SHALL 注册以下工具到 `ToolRegistry`，允许 agent 按需发现和读取 MCP 资源：

1. `mcp__list_resources`：列出所有已连接 MCP 服务器的可用资源
2. `mcp__read_resource`：读取指定服务器的指定资源 URI

这两个工具 SHALL 注册为 deferred 工具（`shouldDefer: true`），仅在 agent 需要时通过 tool_search 发现。

#### Scenario: Agent 列出资源
- **WHEN** agent 调用 `mcp__list_resources` 工具
- **THEN** 系统 SHALL 聚合所有声明了 resources 能力的 MCP 服务器的资源列表，每条资源附带 server name

#### Scenario: Agent 读取资源
- **WHEN** agent 调用 `mcp__read_resource`，参数 `server_name` 和 `uri`
- **THEN** 系统 SHALL 调用对应服务器的 `resources/read`，返回资源内容

#### Scenario: 资源内容大小限制
- **WHEN** `resources/read` 返回超过 1MB 的内容
- **THEN** 系统 SHALL 截断到 1MB 并在返回值中标注 `[truncated]`

### Requirement: resources/list_changed 通知处理

当 MCP 服务器的 `capabilities.resources.listChanged` 为 `true` 时，系统 SHALL 监听 `notifications/resources/list_changed` 通知，并在收到通知时重新拉取资源列表。

#### Scenario: 资源列表变更
- **WHEN** MCP 服务器发送 `notifications/resources/list_changed`
- **THEN** 系统 SHALL 重新调用 `resources/list` 更新缓存的资源列表
