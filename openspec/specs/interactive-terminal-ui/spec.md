## ADDED Requirements

### Requirement: Terminal emulator rendering
前端 SHALL 使用 xterm.js 提供完整的 VT100/VT220 终端仿真，支持颜色、光标定位、滚动缓冲区。

#### Scenario: Display command output with colors
- **WHEN** PTY 输出包含 ANSI 颜色序列
- **THEN** xterm.js 正确渲染彩色文本

#### Scenario: Full-screen program support
- **WHEN** 用户在交互式终端中运行 `top` 或 `vim`
- **THEN** xterm.js 正确渲染全屏程序的 UI（光标定位、清屏、反色等）

#### Scenario: Scrollback buffer
- **WHEN** 输出超出可见区域
- **THEN** 用户可通过滚轮或键盘上翻查看历史输出（默认保留 5000 行）

### Requirement: Keyboard input forwarding
前端 SHALL 将用户所有键盘输入转发至 PTY WebSocket，不做本地拦截（除平台快捷键外）。

#### Scenario: Regular text input
- **WHEN** 用户在终端焦点下键入字符
- **THEN** 字符作为 Binary WebSocket 帧发送到后端 PTY

#### Scenario: Special keys
- **WHEN** 用户按下 Ctrl+C、Ctrl+D、Tab、方向键等
- **THEN** 对应的终端控制序列正确发送（如 Ctrl+C → \x03）

### Requirement: Terminal auto-resize
前端 SHALL 在面板尺寸变化时自动调整 xterm.js 终端尺寸，并通知后端 PTY。

#### Scenario: Panel resized by user
- **WHEN** 用户拖动面板边界调整大小
- **THEN** xterm.js fit addon 重新计算 cols/rows，发送 resize 控制消息给后端

#### Scenario: Window maximized
- **WHEN** 应用窗口最大化
- **THEN** 终端自动扩展填充可用空间

### Requirement: Multi-session UI management
前端 SHALL 支持多个终端会话的创建、切换和关闭。

#### Scenario: Create new session
- **WHEN** 用户点击 "+" 按钮
- **THEN** 前端发送 create 请求，新建会话并自动切换到新 tab

#### Scenario: Switch between sessions
- **WHEN** 用户点击不同的会话 tab
- **THEN** 显示对应的 xterm.js 实例（保留滚动位置和内容）

#### Scenario: Close session
- **WHEN** 用户点击会话 tab 的关闭按钮
- **THEN** 发送 close 消息，关闭 PTY，移除 tab

#### Scenario: Session exited indicator
- **WHEN** PTY 子进程自行退出
- **THEN** tab 显示 "[已退出]" 标记，终端变为只读

### Requirement: Terminal tab structure
Terminal tab SHALL 分为 "Output" 和 "Shell" 两个子视图。

#### Scenario: Default view
- **WHEN** 用户首次打开 Terminal tab
- **THEN** 显示 "Output" 子视图（现有 Agent shell 执行记录）

#### Scenario: Switch to Shell view
- **WHEN** 用户点击 "Shell" 子 tab
- **THEN** 显示交互式终端界面，自动创建一个会话（若无活跃会话）

#### Scenario: Badge shows active sessions
- **WHEN** 有活跃的 PTY 会话
- **THEN** "Shell" 子 tab 显示活跃会话数量 badge
