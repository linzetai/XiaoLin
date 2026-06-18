## ADDED Requirements

### Requirement: MCP tools default deferred
MCP 工具注册时 SHALL 默认使用 `ToolExposure::Deferred`，仅当工具声明 `_meta.alwaysLoad` 或 `_meta["anthropic/alwaysLoad"]` 为 true 时才注册为 eager。

#### Scenario: Standard MCP tool registration
- **WHEN** MCP server 连接成功并注册工具
- **THEN** 工具 SHALL 被注册到 deferred set
- **AND** 不出现在每轮的 `tool_defs` 中

#### Scenario: alwaysLoad MCP tool registration
- **WHEN** MCP 工具的 `_meta.alwaysLoad` 为 true
- **THEN** 工具 SHALL 被注册为 eager
- **AND** 出现在每轮的 `tool_defs` 中

### Requirement: Eliminate MCP dual injection
`inject_mcp_tools_prompt()` SHALL 不再为已 eager 的 MCP 工具重复描述完整 schema，仅保留工具名列表。

#### Scenario: Eager MCP tool in system prompt
- **WHEN** MCP 工具已注册为 eager
- **THEN** system prompt 中仅列出工具名，不重复描述 parameters schema

#### Scenario: Deferred MCP tool in system prompt
- **WHEN** MCP 工具处于 deferred 状态
- **THEN** system prompt 中列出工具名 + 简短描述 + tool_search 引导

### Requirement: tool_search select returns full schema
`tool_search` 工具在 `select:` 模式下 SHALL 返回完整的工具 parameters schema，使模型可在同轮立即调用该工具。

#### Scenario: Select single tool
- **WHEN** 调用 `tool_search(query: "select:mcp__github__search_repos")`
- **THEN** 返回该工具的完整 name + description + parameters schema
- **AND** 工具从 deferred 移入 eager（`activate_deferred`）

#### Scenario: Select multiple tools
- **WHEN** 调用 `tool_search(query: "select:mcp__github__search_repos,mcp__github__create_issue")`
- **THEN** 返回两个工具的完整 schema
- **AND** 两个工具均激活为 eager
