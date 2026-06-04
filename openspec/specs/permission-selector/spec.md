## ADDED Requirements

### Requirement: Permission selector component
前端 SHALL 提供 `PermissionSelector` React 组件，在 InputBar 工具栏中显示当前权限预设名称，点击弹出下拉菜单。

#### Scenario: Default display
- **WHEN** InputBar 渲染完成
- **THEN** 权限选择器显示 "🔒 {当前预设名称} ▾"
- **AND** 使用 `--text-2` 颜色，hover 时变为 `--text-1`

#### Scenario: Dropdown menu
- **WHEN** 用户点击权限选择器
- **THEN** 弹出下拉菜单，列出所有可用预设
- **AND** 每个选项显示：预设名称 + 一句描述
- **AND** 当前选中的预设有 ✓ 标记
- **AND** 底部有分隔线和 "自定义..." 入口

#### Scenario: Select preset
- **WHEN** 用户点击某个预设选项
- **THEN** 下拉菜单关闭
- **AND** 选择器文字更新为新预设名称
- **AND** 发送 `permissions.set { session_id, preset_id }` WS 消息

#### Scenario: Full-auto confirmation
- **WHEN** 用户选择 "Full auto" 预设
- **THEN** 先弹出确认对话框："此模式将跳过所有安全确认，Agent 可自由执行任何操作。确定继续？"
- **AND** 用户确认后才切换

#### Scenario: Custom entry
- **WHEN** 用户点击 "自定义..."
- **THEN** 打开 Settings 的 SecurityTab 页面

### Requirement: Permission mode visual indicator
当使用 "Full auto" 预设时，InputBar SHALL 显示安全警告视觉提示。

#### Scenario: Full-auto warning indicator
- **WHEN** 当前 session 使用 "Full auto" 预设
- **THEN** 权限选择器使用橙色文字 + 橙色图标
- **AND** InputBar 底部显示一行橙色小字 "⚠ Agent 将自动执行所有操作"

#### Scenario: Plan-only indicator
- **WHEN** 当前 session 使用 "Plan only" 预设
- **THEN** 权限选择器使用蓝色文字
- **AND** InputBar 底部显示一行蓝色小字 "📋 Agent 仅规划不执行"
