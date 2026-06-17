## MODIFIED Requirements

### Requirement: Plugin list display
`PluginPanel` SHALL 展示已配置 MCP 插件列表；每项包含：显示名称（id）、状态徽章、scope 标签（user / project）、启用/禁用 toggle。MCP Tab 内部 SHALL 提供 Installed / Explore 子视图切换。

#### Scenario: List shows configured plugins in Installed view
- **WHEN** `plugins.list` 返回至少一条插件且子视图为 Installed
- **THEN** 列表渲染所有插件行，按名称排序
- **AND** 每行显示 id 作为标题、scope 标签、状态徽章、enable toggle

#### Scenario: Switch to Explore view
- **WHEN** 用户点击 "Explore" 子切换按钮
- **THEN** 隐藏 Installed 列表，显示 McpExplorePanel

#### Scenario: Switch back to Installed view
- **WHEN** 用户从 Explore 切换回 "Installed"
- **THEN** 显示已安装插件列表，隐藏 McpExplorePanel

## ADDED Requirements

### Requirement: Header add button
MCP Tab Header SHALL 在 Installed 子视图时提供 "+ Add" 按钮打开 AddServerModal。

#### Scenario: Add button visible in Installed view
- **WHEN** MCP Tab 子视图为 Installed
- **THEN** Header 区域显示 "+ Add" 按钮

#### Scenario: Add button hidden in Explore view
- **WHEN** MCP Tab 子视图为 Explore
- **THEN** Header 区域不显示 "+ Add" 按钮（Explore 自带安装功能）

### Requirement: Remove action in plugin row
已安装 server 行 SHALL 在 hover 时显示删除按钮或在展开详情中提供删除入口。

#### Scenario: Remove from row hover
- **WHEN** 用户 hover 某已安装 server 行
- **THEN** 行右侧显示删除图标按钮（与 Restart 按钮并列）

#### Scenario: Remove triggers confirmation
- **WHEN** 用户点击删除图标
- **THEN** 显示确认提示
- **AND** 确认后调用 `removePlugin(id)`
- **AND** 成功后该行从列表消失

### Requirement: Empty state with explore CTA
MCP Tab 无已安装插件时的空状态 SHALL 提供跳转到 Explore 视图的 CTA 按钮。

#### Scenario: Empty state shows explore button
- **WHEN** 已安装插件列表为空
- **THEN** 空状态文案中包含 "Browse MCP Servers" 按钮
- **AND** 点击后切换到 Explore 子视图
