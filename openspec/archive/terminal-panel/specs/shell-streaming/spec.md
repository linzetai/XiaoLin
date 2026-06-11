## ADDED Requirements

### Requirement: Streaming shell output via ToolProgress
ShellRuntime SHALL 在执行命令时逐行（或逐块）读取子进程的 stdout/stderr，并通过 `ToolProgress { partial_output }` 事件实时推送到前端，而非等待进程退出后批量返回。

#### Scenario: Long-running command streams output
- **WHEN** Agent 执行 `shell_exec("cargo build --release")` 且编译持续 30 秒
- **THEN** ShellRuntime 在编译过程中持续发射 `ToolProgress` 事件
- **AND** 每个事件的 `partial_output` 包含最新读取的输出行
- **AND** 前端在进程退出前即可看到编译进度

#### Scenario: Final ToolResult contains full output
- **WHEN** 子进程退出
- **THEN** ShellRuntime 发射最终的 `ToolResult` 事件，`output` 字段包含完整的 stdout + stderr
- **AND** `ToolResult.metadata` 包含 `exit_code`、`duration_ms`

### Requirement: Configurable streaming granularity
ShellRuntime SHALL 支持按行或按时间窗口（默认 100ms）聚合输出后发射 `ToolProgress`，避免高频输出时产生过多事件。

#### Scenario: High-frequency output throttling
- **WHEN** 子进程在 1 秒内输出 1000 行
- **THEN** ShellRuntime 将输出按 100ms 时间窗口聚合
- **AND** 每 100ms 发射一次 `ToolProgress`，`partial_output` 包含该窗口内所有累积行

#### Scenario: Low-frequency output immediate delivery
- **WHEN** 子进程每隔 2 秒输出一行
- **THEN** 每行输出后立即发射 `ToolProgress`（不等待聚合窗口）

### Requirement: Backward-compatible piped mode
当 session 没有活跃 PTY 时，ShellRuntime SHALL 仍然支持传统的 piped 模式（`Stdio::piped()` + 等待退出），确保向后兼容。

#### Scenario: No PTY fallback
- **WHEN** session 没有活跃 PTY
- **AND** Agent 执行 shell_exec
- **THEN** ShellRuntime 使用 piped 模式执行，行为与改造前一致
- **AND** 仍然发射 ToolProgress 流式事件（从 piped stdout 逐行读取）

### Requirement: stderr 与 stdout 合并流
ShellRuntime SHALL 将 stdout 和 stderr 合并为单一输出流推送，通过前缀或元数据区分来源。

#### Scenario: stderr interleaved with stdout
- **WHEN** 子进程同时输出 stdout 和 stderr
- **THEN** ToolProgress 的 `partial_output` 按时间顺序交织 stdout 和 stderr 内容
- **AND** stderr 行在 `partial_output` 中可通过可选的 `[stderr]` 前缀区分（或在 metadata 中标记）
