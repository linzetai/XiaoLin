## ADDED Requirements

### Requirement: Per-session terminal state management
前端 SHALL 提供 `useTerminalStore` (Zustand store)，管理每个 session 的终端状态，包括命令执行记录（Phase 1/2）和 PTY 连接状态（Phase 3）。

#### Scenario: Store initialization
- **WHEN** 应用启动
- **THEN** `useTerminalStore` 初始化为空状态（无活跃终端）

#### Scenario: Session terminal state creation
- **WHEN** 用户首次打开某 session 的 Terminal tab
- **THEN** store 为该 session 创建 `TerminalState` 条目

### Requirement: Command entry aggregation (Phase 1/2)
Store SHALL 将同一 session 的所有 shell 执行记录聚合为 `TerminalEntry[]`，每条记录包含 `source`（agent/user）、`command`、`output`（逐行累积）、`exitCode`、`startedAt`、`finishedAt`。

#### Scenario: Agent command entry from ToolProgress
- **WHEN** 收到 `tool_executing` 事件（tool_name = "shell_exec"）
- **THEN** store 创建新的 `TerminalEntry`，`source: "agent"`，`command` 从 args 提取
- **AND** 后续 `tool_progress` 的 `partial_output` 追加到该 entry 的 `output`
- **AND** `tool_result` 到达时标记 entry 为完成，记录 `exitCode` 和 `finishedAt`

#### Scenario: User command entry (Phase 2)
- **WHEN** 用户通过 CommandInput 提交命令
- **THEN** store 创建新的 `TerminalEntry`，`source: "user"`
- **AND** WS `terminal.output` 事件的内容追加到该 entry 的 `output`

### Requirement: PTY state management (Phase 3)
Store SHALL 管理 PTY 生命周期状态，包括 `ptyId`、`isAlive`、`shellType`、`cwd`。

#### Scenario: PTY spawned
- **WHEN** PTY 成功创建（invoke "terminal.spawn" 返回）
- **THEN** store 更新该 session 的 `ptyId`、`isAlive: true`、`shellType`

#### Scenario: PTY exited
- **WHEN** 收到 `terminal:exit` Tauri 事件
- **THEN** store 更新 `isAlive: false`，记录 `exitCode`

### Requirement: Active session tracking
Store SHALL 跟踪当前活跃 session（与 `useChatMetaStore.currentSessionId` 同步），Terminal tab 仅显示当前 session 的终端内容。

#### Scenario: Session switch
- **WHEN** 用户切换到另一个 session
- **THEN** Terminal tab 切换显示目标 session 的终端状态
- **AND** 前一个 session 的 PTY（如有）继续后台运行

### Requirement: Terminal state cleanup
Store SHALL 在 session 删除时清理对应的终端状态，包括 kill PTY。

#### Scenario: Session deleted
- **WHEN** session 被删除
- **THEN** store 移除该 session 的 `TerminalState`
- **AND** 如果有活跃 PTY，调用 `invoke("terminal.kill")` 销毁
