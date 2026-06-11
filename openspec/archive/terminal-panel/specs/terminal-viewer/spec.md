## ADDED Requirements

### Requirement: TerminalViewer component (Phase 1/2)
前端 SHALL 提供 `TerminalViewer` React 组件，在 WorkspacePanel 的 Terminal tab 中渲染当前 session 的命令执行记录，支持 ANSI 颜色。

#### Scenario: Render command entries
- **WHEN** 当前 session 有 shell 执行记录
- **THEN** TerminalViewer 按时间顺序渲染所有 `TerminalEntry`
- **AND** 每条记录显示：命令行（`$ command`）、输出内容、exit code（非零时红色显示）
- **AND** Agent 命令和用户命令有不同的视觉标记（Agent 前缀 🤖 图标或左侧彩色边框）

#### Scenario: ANSI color rendering
- **WHEN** 命令输出包含 ANSI 转义序列（如 `\x1b[31m` 红色）
- **THEN** TerminalViewer 使用 `ansi-to-react` 库将 ANSI 转为对应颜色的 React 元素
- **AND** 渲染在 monospace 字体的容器中

#### Scenario: Auto-scroll to bottom
- **WHEN** 新的输出行追加到当前 entry
- **AND** 用户未手动向上滚动
- **THEN** TerminalViewer 自动滚动到底部显示最新输出

#### Scenario: Manual scroll pause
- **WHEN** 用户手动向上滚动查看历史输出
- **THEN** 自动滚动暂停
- **AND** 底部出现 "↓ New output" 按钮，点击恢复自动滚动

### Requirement: Empty state
TerminalViewer SHALL 在当前 session 无任何命令记录时显示空状态提示。

#### Scenario: No commands yet
- **WHEN** 当前 session 没有 shell 执行记录
- **THEN** 显示居中的提示文字："终端输出将在 Agent 执行命令时自动显示"
- **AND** 提示使用 `--text-3` 次要文字色

### Requirement: XTermTerminal component (Phase 3)
Phase 3 SHALL 使用 `XTermTerminal` 组件替换 `TerminalViewer`，基于 xterm.js 提供完整终端体验。

#### Scenario: xterm.js initialization
- **WHEN** 用户打开 Terminal tab 且 session 有活跃 PTY
- **THEN** 渲染 xterm.js Terminal 实例
- **AND** 加载 FitAddon（自适应面板大小）、WebglAddon（GPU 加速，fallback Canvas）、SearchAddon

#### Scenario: PTY output rendering
- **WHEN** Tauri 事件 `terminal:data` 到达
- **THEN** 调用 `term.write(data)` 渲染到 xterm.js
- **AND** 支持完整 VT100/VT220 ANSI 序列（颜色、光标移动、清屏等）

#### Scenario: Terminal resize
- **WHEN** WorkspacePanel 宽度或高度变化
- **THEN** FitAddon 重新计算 cols/rows
- **AND** 调用 `invoke("terminal.resize", { cols, rows })` 通知 PTY

#### Scenario: Agent command decoration (Phase 3)
- **WHEN** Agent 通过 PTY 执行命令（收到 `terminal:agent_cmd` 事件）
- **THEN** xterm.js 在该命令区域显示特殊装饰（左侧彩色边框或顶部提示行 "🤖 Agent: {command}"）

### Requirement: Terminal toolbar
终端组件上方 SHALL 显示工具栏，包含 shell 类型、cwd 路径、Agent 执行状态指示器。

#### Scenario: Toolbar display
- **WHEN** Terminal tab 处于活跃状态
- **THEN** 显示工具栏：`[bash ▾] [~/project/path] [⚡ Agent 执行中]`
- **AND** shell 类型为只读标签
- **AND** cwd 显示为相对于 home 的缩写路径

#### Scenario: Agent activity indicator
- **WHEN** Agent 正在通过终端执行命令（agent_lock 被持有）
- **THEN** 工具栏显示 "⚡ Agent 执行中" 指示器（带动画）
- **WHEN** Agent 命令完成
- **THEN** 指示器消失
