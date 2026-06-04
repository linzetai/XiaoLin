## MODIFIED Requirements

### Requirement: Inline toolbar
文本输入区域下方 SHALL 显示水平排列的工具栏，包含（从左到右）：附加按钮（+）、权限选择器（🔒 {预设名} ▾）、刷新按钮（↻）、模型选择器（🟢 模型名 ▾）、**计算等级选择器（⚡ {等级名} ▾）**。右侧为附件按钮（📎）和发送按钮（圆形，accent 背景色，白色箭头图标）。

#### Scenario: Compute level selector placement
- **WHEN** InputBar 渲染完成
- **THEN** 计算等级选择器显示当前 session 的有效等级标签（如「Extra High」）
- **AND** 位于模型选择器右侧、附件按钮（📎）左侧
- **AND** 与原型 `docs/prototype-codex-layout.html` 工具栏 chip 顺序一致

#### Scenario: Model selector unchanged
- **WHEN** 用户点击模型选择器
- **THEN** 弹出模型列表下拉菜单（复用现有 `ModelSelector` 逻辑）
- **AND** 模型选择与计算等级选择器独立运作

#### Scenario: Send button states
- **WHEN** 输入框有内容
- **THEN** 发送按钮 opacity 为 1，hover 时 scale 1.06 + box-shadow
- **WHEN** 输入框为空
- **THEN** 发送按钮 opacity 为 0.3，不可点击

#### Scenario: Toolbar chips on narrow viewport
- **WHEN** 视口宽度不足以显示全部工具栏 chip
- **THEN** 计算等级选择器与模型选择器优先保留可见（可截断标签文字）
- **AND** 附加按钮与刷新按钮可折叠或收入 overflow 菜单（与 `layout-overhaul` 响应式策略一致）
