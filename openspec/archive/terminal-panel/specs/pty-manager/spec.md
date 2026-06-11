## ADDED Requirements

### Requirement: PTY lifecycle management (Phase 3)
后端 SHALL 提供 `TerminalManager`，管理 per-session 的 PTY 实例生命周期，基于 `tauri-plugin-pty`。

#### Scenario: Spawn PTY
- **WHEN** 调用 `terminal.spawn { session_id, cwd, shell?, cols, rows }`
- **THEN** 通过 `tauri-plugin-pty` 创建 PTY 实例
- **AND** 使用用户默认 shell（`$SHELL` 或 fallback "bash"）
- **AND** 设置 `TERM=xterm-256color`、`COLORTERM=truecolor` 环境变量
- **AND** 注入 Shell Integration 初始化脚本
- **AND** 返回 `{ pty_id, shell_type }`

#### Scenario: Spawn with custom working directory
- **WHEN** `cwd` 参数指定了目录
- **THEN** PTY 子进程的工作目录设为该路径
- **WHEN** `cwd` 为空
- **THEN** 使用 session 的 `work_dir`，若也为空则使用用户 home 目录

#### Scenario: Kill PTY
- **WHEN** 调用 `terminal.kill { session_id }`
- **THEN** 向 PTY 子进程发送 SIGHUP
- **AND** 等待 500ms 后若未退出则 SIGKILL
- **AND** 清理 TerminalManager 中的注册信息
- **AND** 发射 `terminal:exit { session_id, exit_code }` Tauri 事件

#### Scenario: PTY child process self-exit
- **WHEN** PTY 子进程自行退出（如 `exit` 命令或 shell 崩溃）
- **THEN** TerminalManager 检测到退出
- **AND** 发射 `terminal:exit` 事件
- **AND** 标记该 PTY 为 inactive

### Requirement: PTY resize
TerminalManager SHALL 支持动态调整 PTY 终端大小。

#### Scenario: Resize PTY
- **WHEN** 调用 `terminal.resize { session_id, cols, rows }`
- **THEN** 调用 PTY 的 `resize(PtySize { cols, rows })` 更新终端尺寸
- **AND** PTY 子进程收到 SIGWINCH 信号

### Requirement: PTY data forwarding
TerminalManager SHALL 将 PTY 的 stdout 数据通过 Tauri 事件转发到前端。

#### Scenario: PTY output to frontend
- **WHEN** PTY 子进程产生输出
- **THEN** TerminalManager 读取 master.reader() 的字节
- **AND** 通过 Tauri 事件 `terminal:data { session_id, data: Vec<u8> }` 发送到前端

#### Scenario: Frontend input to PTY
- **WHEN** 调用 `terminal.write { session_id, data: Vec<u8> }`
- **THEN** TerminalManager 将 data 写入 PTY master.writer()

### Requirement: PTY registry and limits
TerminalManager SHALL 维护活跃 PTY 注册表，限制最大并发 PTY 数量。

#### Scenario: Maximum PTY limit
- **WHEN** 活跃 PTY 数量达到上限（默认 10）
- **AND** 请求创建新 PTY
- **THEN** 返回错误 "Maximum terminal limit reached"

#### Scenario: Idle PTY cleanup
- **WHEN** PTY 超过 30 分钟无任何 I/O 活动
- **THEN** TerminalManager 自动 kill 该 PTY
- **AND** 发射 `terminal:exit` 事件，exit_code 标记为 "timeout"

### Requirement: PTY list query
TerminalManager SHALL 支持查询当前所有活跃 PTY 的状态。

#### Scenario: List PTYs
- **WHEN** 调用 `terminal.list`
- **THEN** 返回所有活跃 PTY 的 `[{ session_id, pty_id, shell_type, cwd, is_alive, created_at }]`
