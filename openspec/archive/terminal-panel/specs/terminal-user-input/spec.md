## ADDED Requirements

### Requirement: Command input box (Phase 2)
Terminal tab 底部 SHALL 提供命令输入框 `CommandInput`，用户可以直接输入并执行 shell 命令。

#### Scenario: Submit command
- **WHEN** 用户在 CommandInput 中输入 "ls -la" 并按 Enter
- **THEN** 发送 WS 消息 `terminal.exec { session_id, command: "ls -la", cwd }`
- **AND** 命令输出通过 `terminal.output` WS 事件实时流入 TerminalViewer
- **AND** CommandInput 清空，ready 状态恢复

#### Scenario: Command history navigation
- **WHEN** 用户在 CommandInput 中按 ↑ 键
- **THEN** 显示上一条用户执行过的命令
- **WHEN** 按 ↓ 键
- **THEN** 显示下一条命令或清空（到达最新时）

#### Scenario: Empty input ignored
- **WHEN** 用户在空输入状态按 Enter
- **THEN** 不发送任何命令

### Requirement: terminal.exec WebSocket API
后端 SHALL 提供 `terminal.exec` WS handler，接收用户命令并执行。

#### Scenario: Successful execution
- **WHEN** 收到 `terminal.exec { session_id, command, cwd }` WS 消息
- **THEN** 后端在指定 cwd 下执行命令
- **AND** 输出通过 `terminal.output { session_id, data, stream: "stdout"|"stderr" }` WS 事件流式返回
- **AND** 命令完成时发送 `terminal.exec_done { session_id, exit_code, duration_ms }`

#### Scenario: Execution while Agent is running
- **WHEN** 用户提交命令但 Agent 正在该 session 中执行命令
- **THEN** 用户命令排队等待 Agent 命令完成后执行
- **OR** 在 Phase 3 PTY 模式下，用户输入直接写入 PTY（与 Agent 命令并行）

### Requirement: terminal.history WebSocket API
后端 SHALL 提供 `terminal.history` WS handler，返回指定 session 的命令执行历史。

#### Scenario: Fetch history on tab open
- **WHEN** 用户打开 Terminal tab
- **THEN** 前端发送 `terminal.history { session_id, limit: 50 }`
- **AND** 后端返回最近 50 条命令记录（含输出摘要）

### Requirement: Phase 3 direct PTY input
Phase 3 中，用户击键 SHALL 直接写入 PTY，无需 CommandInput 组件（xterm.js 处理全部键盘输入）。

#### Scenario: Keystroke forwarding
- **WHEN** 用户在 xterm.js 终端中按键
- **THEN** xterm.js `onData` 回调触发
- **AND** 调用 `invoke("terminal.write", { session_id, data })` 将击键数据发送到 PTY

#### Scenario: CommandInput hidden in Phase 3
- **WHEN** session 有活跃 PTY（Phase 3 模式）
- **THEN** CommandInput 组件不渲染（xterm.js 本身处理输入）
