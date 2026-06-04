## ADDED Requirements

### Requirement: Compute level selector component
前端 SHALL 提供 `ComputeLevelSelector` React 组件，在 InputBar 工具栏中显示当前计算等级标签，点击弹出下拉菜单。

#### Scenario: Default display
- **WHEN** InputBar 渲染完成且当前 session 有有效计算等级
- **THEN** 选择器显示「⚡ {当前等级标签} ▾」（如「⚡ High ▾」）
- **AND** 使用 `--text-2` 颜色，hover 时变为 `--text-1`
- **AND** 位于模型选择器右侧

#### Scenario: Dropdown menu with five options
- **WHEN** 用户点击计算等级选择器
- **THEN** 弹出下拉菜单，列出 5 个选项：Low、Medium、High、Extra High、Max
- **AND** 每个选项显示：等级名称 + 一句速度/质量权衡描述
- **AND** 当前选中的等级有 ✓ 标记

#### Scenario: Select compute level
- **WHEN** 用户点击某个非 Max 等级选项
- **THEN** 下拉菜单关闭
- **AND** 选择器文字更新为新等级标签
- **AND** 发送 `compute_level.set { session_id, level }` WS 消息

#### Scenario: Max level confirmation
- **WHEN** 用户选择「Max」等级
- **THEN** 先弹出确认对话框：「最高算力将使用前沿模型，成本与延迟显著增加。确定继续？」
- **AND** 用户确认后才发送 `compute_level.set` 并更新 UI

### Requirement: Compute level visual indicator for Max
当 session 使用 Max 计算等级时，InputBar SHALL 显示成本/算力警告视觉提示。

#### Scenario: Max level warning indicator
- **WHEN** 当前 session 的计算等级为 Max
- **THEN** 选择器使用琥珀色（`--warning` 或等效 token）文字与图标
- **AND** InputBar 底部显示一行琥珀色小字：「⚡ 最高算力 — 成本与延迟显著增加」

#### Scenario: Non-Max levels use neutral styling
- **WHEN** 当前 session 的计算等级为 Low / Medium / High / Extra High
- **THEN** 选择器使用默认 `--text-2` 样式，无底部警告条

### Requirement: Session switch syncs compute level display
前端 SHALL 在切换活跃 session 时同步计算等级选择器的显示状态。

#### Scenario: Switch to session with override
- **WHEN** 用户切换到 session B
- **AND** session B 有计算等级覆盖（如 Extra High）
- **THEN** 选择器立即显示 session B 的有效等级标签
- **AND** 视觉指示器（如 Max 警告）反映 session B 的状态

#### Scenario: Switch to session without override
- **WHEN** 用户切换到 session C且无覆盖
- **THEN** 选择器显示全局默认等级（High）
- **AND** `is_override` 为 false 时不显示覆盖相关 UI 标记

#### Scenario: Initialize on session mount
- **WHEN** 活跃 session 首次加载或切换完成
- **THEN** 前端调用 `compute_level.get { session_id }` 填充 store
- **AND** 选择器在 API 返回前可显示骨架或上次缓存值，返回后必须与后端一致
