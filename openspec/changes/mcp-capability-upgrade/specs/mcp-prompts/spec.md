## ADDED Requirements

### Requirement: Prompts 客户端 API

`McpClient` SHALL 实现以下 MCP Prompts 客户端方法：

1. `list_prompts()` → 返回服务器提供的 prompt 模板列表（name, description, arguments）
2. `get_prompt(name, arguments)` → 获取渲染后的 prompt 内容（messages 数组）

这些方法 SHALL 在 `initialize` 成功后，根据服务器声明的 `capabilities.prompts` 决定是否可用。

#### Scenario: 服务器声明 prompts 能力
- **WHEN** MCP 服务器在 `initialize` 响应中声明 `capabilities.prompts`
- **THEN** `list_prompts()` 和 `get_prompt()` SHALL 可用

#### Scenario: 服务器未声明 prompts 能力
- **WHEN** MCP 服务器未声明 `capabilities.prompts`
- **THEN** 调用 prompts 相关方法 SHALL 返回空列表或明确错误

### Requirement: Prompts 通过 WebSocket API 暴露

系统 SHALL 通过 WebSocket API 暴露 MCP prompts，允许前端获取和使用：

1. `plugins.prompts` 请求：返回所有已连接 MCP 服务器的 prompt 列表
2. `plugins.get_prompt` 请求：获取指定 prompt 的渲染内容

#### Scenario: 前端获取 prompt 列表
- **WHEN** 前端发送 `plugins.prompts` 请求
- **THEN** 系统 SHALL 返回聚合的 prompt 列表，每条包含 `server_name`、`name`、`description`、`arguments`

#### Scenario: 前端使用 prompt
- **WHEN** 前端发送 `plugins.get_prompt` 请求，附带 `server_name`、`prompt_name` 和 `arguments`
- **THEN** 系统 SHALL 调用对应服务器的 `prompts/get`，返回渲染后的 messages 数组

### Requirement: prompts/list_changed 通知处理

当 MCP 服务器的 `capabilities.prompts.listChanged` 为 `true` 时，系统 SHALL 监听 `notifications/prompts/list_changed` 通知。

#### Scenario: Prompt 列表变更
- **WHEN** MCP 服务器发送 `notifications/prompts/list_changed`
- **THEN** 系统 SHALL 重新调用 `prompts/list` 更新缓存，并通过 WebSocket 广播 `plugins.prompts_changed` 事件
