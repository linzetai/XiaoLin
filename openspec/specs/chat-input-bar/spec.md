## MODIFIED Requirements

### Requirement: Inline toolbar
文本输入区域下方 SHALL 显示水平排列的工具栏，包含（从左到右）：附加按钮（+）、权限选择器（🔒 {预设名} ▾）、刷新按钮（↻）、模型选择器（🟢 模型名 ▾）、计算等级（Extra High ▾）。右侧为附件按钮（📎）和发送按钮（圆形，accent 背景色，白色箭头图标）。

#### Scenario: Permission selector in toolbar
- **WHEN** InputBar 渲染完成
- **THEN** 工具栏中的权限选择器显示当前 session 的生效预设名称
- **AND** 位于附加按钮（+）右侧、刷新按钮（↻）左侧

#### Scenario: Model selector
- **WHEN** 用户点击模型选择器
- **THEN** 弹出模型列表下拉菜单（复用现有 ModelSelector 逻辑）

#### Scenario: Send button states
- **WHEN** 输入框有内容
- **THEN** 发送按钮 opacity 为 1，hover 时 scale 1.06 + box-shadow
- **WHEN** 输入框为空
- **THEN** 发送按钮 opacity 为 0.3，不可点击
