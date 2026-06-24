## ADDED Requirements

### Requirement: Collapsed-state live activity display
SubAgentCard 在折叠状态下 SHALL 显示当前运行的工具（工具名 + 关键参数）、实时计时和工具调用计数，无需展开即可感知 sub-agent 在做什么。

#### Scenario: Running sub-agent shows current tool
- **WHEN** 一个 sub-agent 正在运行且有一个工具处于 running 状态
- **THEN** 折叠行显示该工具名和关键参数（如文件名），并随工具切换更新

#### Scenario: Running sub-agent shows live timer
- **WHEN** 一个 sub-agent 处于 running 状态
- **THEN** 折叠行显示实时跳动的计时（每秒更新）

#### Scenario: Completed sub-agent shows result summary
- **WHEN** 一个 sub-agent 完成且有 result
- **THEN** 折叠行第二行显示 result 首行摘要（截断）

### Requirement: Markdown result rendering
SubAgentCard 展开后的 result 区域 SHALL 在非失败状态下用 Markdown 渲染；失败状态 SHALL 保留纯文本（pre）渲染错误信息。

#### Scenario: Completed result renders as Markdown
- **WHEN** sub-agent 成功完成且 result 为 Markdown 格式
- **THEN** result 以渲染后的 Markdown 展示（标题、列表、代码高亮）

#### Scenario: Failed result renders as plain text
- **WHEN** sub-agent 失败且 result 为错误信息
- **THEN** result 以等宽 pre 文本展示，颜色标红

### Requirement: State transition animation
SubAgentCard 的展开/折叠 SHALL 使用平滑高度动画；状态变化（running→completed/failed）SHALL 有过渡动画；所有动画 MUST 尊重 `prefers-reduced-motion`。

#### Scenario: Expand animates smoothly
- **WHEN** 用户点击折叠的 SubAgentCard
- **THEN** 卡片以高度过渡动画展开，无突变闪烁

#### Scenario: Reduced motion respected
- **WHEN** 用户启用了 prefers-reduced-motion
- **THEN** 所有动画时长被压缩到接近 0，无动效

### Requirement: Shared elapsed timer
SubAgentCard 和 CoordinatorPanel 的运行项 SHALL 复用同一个计时 hook（`useElapsedTimer`），保证计时行为一致。

#### Scenario: Both components show consistent timing
- **WHEN** 同一个 sub-agent 同时在 SubAgentCard 和 CoordinatorPanel 中展示
- **THEN** 两处显示的计时一致
