## ADDED Requirements

### Requirement: Detail modal entry
PluginsView MCP Tab 的已安装 server 行 SHALL 可点击打开 McpDetailModal（替代或增强当前 inline expand）。

#### Scenario: Open detail from plugin row
- **WHEN** 用户点击已安装 server 行
- **THEN** McpDetailModal 以 overlay 模态框形式打开
- **AND** 加载该 server 的详细信息

### Requirement: Connection config display
McpDetailModal SHALL 展示 server 的连接配置信息。

#### Scenario: Stdio server config
- **WHEN** 打开一个 stdio server 的详情
- **THEN** 显示 Command、Arguments、Transport 类型
- **AND** 环境变量值已脱敏（显示为 ••••）

#### Scenario: HTTP server config
- **WHEN** 打开一个 streamable_http server 的详情
- **THEN** 显示 URL、Transport 类型

### Requirement: Connection status display
McpDetailModal SHALL 展示 server 的连接状态信息。

#### Scenario: Connected server
- **WHEN** server 状态为 connected
- **THEN** 显示绿色状态标记、连接时间、工具数量

#### Scenario: Failed server
- **WHEN** server 状态为 failed 且有 lastError
- **THEN** 显示红色状态标记和错误信息

### Requirement: Tool list display
McpDetailModal SHALL 展示该 server 注册的工具列表。

#### Scenario: Server with tools
- **WHEN** server 注册了 5 个工具
- **THEN** 显示工具列表，每个工具显示 name 和 description
- **AND** 工具数量超过 5 个时提供搜索框

#### Scenario: Server with no tools
- **WHEN** server 未注册任何工具
- **THEN** 显示 "No tools available" 提示

### Requirement: Remove server action
McpDetailModal SHALL 提供 "Remove" 按钮，调用 `removePlugin` 后关闭模态框。

#### Scenario: Remove server
- **WHEN** 用户点击 "Remove" 按钮
- **THEN** 显示确认提示
- **AND** 确认后调用 `mcp.remove` API
- **AND** 成功后模态框关闭，Installed 列表自动刷新

#### Scenario: Remove failure
- **WHEN** `mcp.remove` 返回错误
- **THEN** 显示错误消息，模态框保持打开

### Requirement: Restart action in detail
McpDetailModal SHALL 提供 "Restart" 按钮。

#### Scenario: Restart from detail
- **WHEN** 用户点击 "Restart" 按钮
- **THEN** 调用 `plugins.restart` API
- **AND** 状态更新为 connecting 直至事件推送更新
