## ADDED Requirements

### Requirement: Agent shell execution via shared PTY (Phase 3)
ShellRuntime SHALL 在 session 有活跃 PTY 时，将命令通过 PTY 执行而非传统 piped 模式。

#### Scenario: Agent command routed to PTY
- **WHEN** Agent 执行 `shell_exec("cargo test")`
- **AND** 当前 session 有活跃 PTY
- **THEN** ShellRuntime 切换到 PTY 模式
- **AND** 获取 `agent_lock`（标记 Agent 正在执行）
- **AND** 写入 PTY: `"cargo test\r\n"`
- **AND** 通知前端 `terminal:agent_cmd { session_id, command: "cargo test", call_id }`

#### Scenario: Agent command without PTY
- **WHEN** Agent 执行 `shell_exec("cargo test")`
- **AND** 当前 session 没有活跃 PTY
- **THEN** ShellRuntime 使用传统 piped 模式（向后兼容）

### Requirement: Agent command output collection
ShellRuntime SHALL 在 PTY 模式下通过 Shell Integration 信号收集命令输出。

#### Scenario: Successful output collection
- **WHEN** Agent 通过 PTY 写入命令
- **AND** Shell Integration 已注入
- **THEN** ShellRuntime 监听 PTY 输出中的 OSC 133;D 序列
- **AND** 收集从命令写入到 OSC 133;D 之间的所有输出
- **AND** 从 OSC 133;D 的参数中提取 exit_code
- **AND** 返回 ToolResult { output, exit_code }

#### Scenario: Shell Integration not available fallback
- **WHEN** Agent 通过 PTY 写入命令
- **AND** Shell Integration 未成功注入
- **THEN** ShellRuntime 回退到超时检测模式
- **AND** 当 PTY stdout 连续 3 秒无新输出时视为命令完成
- **AND** exit_code 设为 None（未知）

#### Scenario: Command timeout
- **WHEN** Agent 通过 PTY 写入命令
- **AND** 超过 `timeout_ms`（默认 120s）仍未收到完成信号
- **THEN** ShellRuntime 不 kill PTY（PTY 属于用户）
- **AND** 返回 ToolResult { success: false, output: "Command timed out" }
- **AND** 释放 agent_lock

### Requirement: Agent lock (cooperative mutex)
PTY 桥接层 SHALL 使用 `agent_lock` 保护 Agent 命令的完整执行周期。

#### Scenario: Agent lock acquired
- **WHEN** Agent 开始执行 PTY 命令
- **THEN** 获取 `agent_lock`
- **AND** 前端收到 Agent 活动指示（用于 toolbar 显示）
- **WHEN** 命令完成或超时
- **THEN** 释放 `agent_lock`

#### Scenario: User input during Agent execution
- **WHEN** `agent_lock` 被持有（Agent 正在执行命令）
- **AND** 用户在 xterm.js 中按键（包括 Ctrl+C）
- **THEN** 用户击键仍然写入 PTY（不被 agent_lock 阻塞）
- **AND** 如果用户按 Ctrl+C，子进程收到 SIGINT

#### Scenario: User Ctrl+C interrupts Agent command
- **WHEN** Agent 正在通过 PTY 执行命令
- **AND** 用户按 Ctrl+C
- **THEN** PTY 子进程的前台进程收到 SIGINT
- **AND** Shell Integration 发射 OSC 133;D with exit_code=130
- **AND** ShellRuntime 收到 exit_code=130，在 ToolResult 中标记 `metadata: { interrupted: true }`
- **AND** 释放 agent_lock

### Requirement: Agent command notification
PTY 桥接层 SHALL 在 Agent 写入命令时通知前端，用于 xterm.js 命令装饰。

#### Scenario: Agent command event emitted
- **WHEN** Agent 通过 PTY 写入命令 "cargo test"
- **THEN** 发射 Tauri 事件 `terminal:agent_cmd { session_id, command: "cargo test", call_id, timestamp }`
- **AND** 前端可用该信息在 xterm.js 中装饰 Agent 命令区域
