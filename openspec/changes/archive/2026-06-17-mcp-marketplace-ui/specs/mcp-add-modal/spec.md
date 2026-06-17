## ADDED Requirements

### Requirement: Add server modal entry
PluginsView MCP Tab Header SHALL 提供 "+ Add" 按钮，点击后打开 AddServerModal。

#### Scenario: Open modal from header
- **WHEN** 用户点击 MCP Tab Header 的 "+ Add" 按钮
- **THEN** AddServerModal 以 overlay 模态框形式打开
- **AND** 默认选中 Stdio transport 类型

#### Scenario: Close modal
- **WHEN** 用户点击关闭按钮、遮罩层或按 ESC
- **THEN** 模态框关闭，表单状态清空

### Requirement: Transport type selector
AddServerModal SHALL 提供 transport 类型选择器（Stdio / SSE / Streamable HTTP），选择后动态切换表单字段。

#### Scenario: Select Stdio transport
- **WHEN** 用户选择 "Stdio" transport
- **THEN** 表单显示 "Command" 必填输入框和 "Arguments" 可选输入框

#### Scenario: Select SSE transport
- **WHEN** 用户选择 "SSE" transport
- **THEN** 表单显示 "URL" 必填输入框
- **AND** 隐藏 Command / Arguments 字段

#### Scenario: Select Streamable HTTP transport
- **WHEN** 用户选择 "Streamable HTTP" transport
- **THEN** 表单显示 "URL" 必填输入框
- **AND** 隐藏 Command / Arguments 字段

### Requirement: Server ID input
AddServerModal SHALL 提供 "Server ID" 必填输入框，用于唯一标识 MCP Server。

#### Scenario: Valid server ID
- **WHEN** 用户输入 "my-github-server"
- **THEN** 无验证错误

#### Scenario: Invalid server ID with double underscore
- **WHEN** 用户输入包含 "__" 的 ID（如 "my__server"）
- **THEN** 显示行内验证错误提示："ID 不能包含连续双下划线"

#### Scenario: Duplicate server ID
- **WHEN** 用户输入已存在的 server ID
- **THEN** 显示行内提示："该 ID 已存在，将覆盖现有配置"

### Requirement: Environment variables editor
AddServerModal SHALL 提供可选的环境变量键值对编辑器。

#### Scenario: Add environment variable
- **WHEN** 用户点击 "+ Add Variable" 按钮
- **THEN** 新增一行 Key / Value 输入框对

#### Scenario: Remove environment variable
- **WHEN** 用户点击某行的删除按钮
- **THEN** 该行键值对被移除

### Requirement: Submit and connect
AddServerModal SHALL 提供 "Add & Connect" 提交按钮，调用 `addPlugin` 并在成功后关闭模态框。

#### Scenario: Successful submission
- **WHEN** 用户填写完表单并点击 "Add & Connect"
- **THEN** 调用 `mcp.add` API 传入所有表单字段
- **AND** 按钮显示 loading 状态
- **AND** 成功后模态框关闭，Installed 列表自动刷新

#### Scenario: Submission failure
- **WHEN** `mcp.add` 返回错误
- **THEN** 模态框保持打开
- **AND** 显示错误消息

#### Scenario: Validation blocks submission
- **WHEN** 必填字段为空（id 或 command/url）
- **THEN** "Add & Connect" 按钮禁用
- **AND** 必填字段显示验证提示
