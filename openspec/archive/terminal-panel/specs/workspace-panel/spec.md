## MODIFIED Requirements

### Requirement: Tab bar
WorkspacePanel 标签栏 SHALL 通过 tab 注册系统渲染各贡献方标签，包含：文件图标按钮、已注册标签（如 "Review"、"Terminal"）、"+" 按钮（添加标签，预留）、右侧面板操作按钮组（最大化/切换面板位置）。激活的标签使用 `--bg-active` 背景。

Terminal 面板通过 tab 系统注册到 WorkspacePanel：

```typescript
{ id: "terminal", label: "Terminal", icon: TerminalIcon, component: TerminalTabContent }
```

#### Scenario: Active tab display
- **WHEN** 打开 WorkspacePanel
- **THEN** 默认激活的标签（如 "Review"）显示为激活状态（`--text-1` 颜色 + `--bg-active` 背景）
- **AND** "Terminal" 标签显示为非激活状态（`--text-3` 颜色）

#### Scenario: Switch to Terminal tab
- **WHEN** 用户点击 "Terminal" 标签
- **THEN** "Terminal" 标签变为激活状态（`--text-1` + `--bg-active`）
- **AND** 其他标签变为非激活
- **AND** 内容区切换为终端面板（TerminalViewer 或 XTermTerminal）
- **AND** 底部操作栏切换为终端操作栏（Phase 2 显示 CommandInput，Phase 3 隐藏）

#### Scenario: Switch back to Review tab
- **WHEN** 用户从 Terminal tab 点击 "Review" 标签
- **THEN** 内容区切换回文件变更列表 + diff 视图
- **AND** 底部操作栏切换回 "Revert all" / "Stage all"
- **AND** 终端面板的 PTY（如有）继续后台运行

#### Scenario: Terminal tab with active Agent command
- **WHEN** Agent 正在执行 shell 命令
- **AND** 用户当前不在 Terminal tab
- **THEN** Terminal 标签图标上显示活动指示器（小圆点或脉冲动画）
- **AND** 提示用户终端有新输出

#### Scenario: Cmd+J toggles Terminal tab
- **WHEN** 用户按下 Cmd+J（macOS）或 Ctrl+J（Windows/Linux）
- **THEN** 若 WorkspacePanel 未打开，则打开面板并激活 Terminal tab
- **AND** 若 WorkspacePanel 已打开且当前为 Terminal tab，则关闭 WorkspacePanel
- **AND** 若 WorkspacePanel 已打开但当前为其他 tab，则切换到 Terminal tab（不关闭面板）
- **AND** 快捷键仅作用于 Terminal tab，不影响整个 WorkspacePanel 的显隐（除非当前已是 Terminal tab）

## ADDED Requirements

### Requirement: Tab-specific footer
WorkspacePanel 底部操作栏 SHALL 根据当前激活的 tab 显示不同内容。

#### Scenario: Review tab footer
- **WHEN** Review tab 激活
- **THEN** 底部显示 "Revert all" / "Stage all" 按钮

#### Scenario: Terminal tab footer (Phase 1)
- **WHEN** Terminal tab 激活且处于 Phase 1 模式
- **THEN** 底部不显示操作栏（终端仅为只读查看器）

#### Scenario: Terminal tab footer (Phase 2)
- **WHEN** Terminal tab 激活且处于 Phase 2 模式
- **THEN** 底部显示 CommandInput 输入框

#### Scenario: Terminal tab footer (Phase 3)
- **WHEN** Terminal tab 激活且有活跃 PTY
- **THEN** 底部不显示操作栏（xterm.js 本身处理输入）
