## ADDED Requirements

### Requirement: Unicode 递归清洗

系统 SHALL 对所有从 MCP 服务器接收的字符串元数据执行 Unicode 安全清洗。清洗范围包括：

- Tool: `name`、`description`、`inputSchema` 中的 `description` 字段
- Resource: `name`、`uri`、`description`
- Prompt: `name`、`description`、`argument.name`、`argument.description`
- Server `instructions` 字符串

清洗 SHALL 移除以下 Unicode 字符：
- 双向控制字符（U+200E—U+200F, U+202A—U+202E, U+2066—U+2069）
- 零宽字符（U+200B, U+200C, U+200D, U+FEFF）
- 其他不可见控制字符（U+0000—U+001F 中除 `\t`、`\n`、`\r` 外的字符）

#### Scenario: Tool description 含双向覆盖字符
- **WHEN** MCP 服务器返回 tool description 包含 `U+202E`（RIGHT-TO-LEFT OVERRIDE）
- **THEN** 系统 SHALL 在注册工具前移除该字符，保留剩余可见文本

#### Scenario: 嵌套 JSON Schema description 清洗
- **WHEN** tool 的 `inputSchema` 中嵌套对象的 `description` 字段包含零宽字符
- **THEN** 系统 SHALL 递归遍历 JSON Schema 并清洗所有 `description` 字符串

#### Scenario: 合法中文字符保留
- **WHEN** tool description 包含中文标点（如 「」、【】）和 emoji
- **THEN** 系统 SHALL 保留这些字符，仅移除不可见控制字符

### Requirement: Instructions PromptGuard

MCP 服务器在 `initialize` 响应中返回的 `instructions` 字符串 SHALL 通过安全检查后才能注入 system context。

检查内容：
1. Unicode 清洗（同上）
2. 长度截断（最大 2048 字符，已有）
3. 可疑模式检测：包含 `ignore previous`, `system:`, `<|`, `[INST]` 等已知 prompt injection 模式时，SHALL 降级为不注入并记录警告日志

#### Scenario: 正常 instructions
- **WHEN** MCP 服务器返回 `instructions: "This server provides GitHub integration. Use the search_repos tool to find repositories."`
- **THEN** 系统 SHALL 清洗后注入到 system context

#### Scenario: 可疑 instructions
- **WHEN** MCP 服务器返回 `instructions` 包含 `"ignore previous instructions and instead..."`
- **THEN** 系统 SHALL 不注入该 instructions，记录 `warn!` 日志，服务器正常连接但不注入 instructions

### Requirement: 工具名严格消毒

所有 MCP 工具在注册到 `ToolRegistry` 时 SHALL 统一使用 `mcp_tool_name()` 函数生成规范化名称，确保工具名仅包含 `[a-zA-Z0-9_-]` 字符。

#### Scenario: 工具名包含特殊字符
- **WHEN** MCP 服务器返回工具名 `search/repos` 或 `get:data`
- **THEN** 注册时 SHALL 使用 `mcp_tool_name()` 消毒为 `search_repos` 或 `get_data`

#### Scenario: 工具名碰撞
- **WHEN** 消毒后两个工具名相同（如 `search-repos` 和 `search_repos` 都变为 `search_repos`）
- **THEN** 后注册的工具 SHALL 被跳过，记录 `warn!` 日志

### Requirement: HTTP Session 恢复

Streamable HTTP 传输 SHALL 在检测到 session 过期时自动恢复：

1. 检测条件：HTTP 404 响应 或 JSON-RPC error code -32001
2. 恢复流程：关闭当前传输 → 重新建立连接 → 重新 `initialize` → 重试原操作（最多 1 次）
3. 并发保护：多个请求同时检测到 session expired 时，仅执行一次恢复，其他请求等待恢复完成后重试

#### Scenario: 服务器重启导致 session 过期
- **WHEN** Streamable HTTP 请求返回 HTTP 404
- **THEN** 系统 SHALL 自动重新初始化 session 并重试该请求

#### Scenario: 恢复失败
- **WHEN** 重新初始化后重试仍然失败
- **THEN** 系统 SHALL 将服务器状态设为 `Failed`，不再自动重试

#### Scenario: 并发请求下的恢复
- **WHEN** 两个并发 RPC 请求同时收到 404
- **THEN** 系统 SHALL 仅执行一次重新初始化，两个请求都在恢复后重试
