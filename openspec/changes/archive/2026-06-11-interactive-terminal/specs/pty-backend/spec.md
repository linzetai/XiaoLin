## ADDED Requirements

### Requirement: Create PTY session
系统 SHALL 支持通过 WebSocket 控制消息创建新的 PTY 会话，启动用户默认 shell（或指定 shell），工作目录默认为用户 home 或指定路径。

#### Scenario: Create default shell session
- **WHEN** 客户端发送 `{"type":"create"}` 控制消息
- **THEN** 系统创建新 PTY 会话，启动用户默认 shell，返回 `{"type":"created","session_id":"<uuid>"}`

#### Scenario: Create session with options
- **WHEN** 客户端发送 `{"type":"create","shell":"zsh","cwd":"/tmp","cols":120,"rows":40}` 控制消息
- **THEN** 系统创建 PTY 会话使用 zsh，工作目录为 /tmp，初始尺寸为 120x40

#### Scenario: Max sessions limit
- **WHEN** 已有会话数达到上限（默认 8）且客户端请求创建新会话
- **THEN** 系统返回错误 `{"type":"error","message":"max sessions reached"}`

### Requirement: PTY I/O streaming
系统 SHALL 通过 WebSocket Binary 帧双向传输 PTY 的 stdin/stdout 字节流，延迟不超过 16ms（单帧往返）。

#### Scenario: User input forwarded to PTY
- **WHEN** 客户端通过 WebSocket 发送 Binary 帧包含 `ls\n` 字节
- **THEN** 系统将这些字节写入 PTY master fd，shell 执行 `ls` 命令

#### Scenario: PTY output streamed to client
- **WHEN** PTY 子进程产生 stdout 输出
- **THEN** 系统立即通过 WebSocket Binary 帧将输出发送给客户端

#### Scenario: ANSI escape sequences preserved
- **WHEN** PTY 输出包含 ANSI 转义序列（颜色、光标移动等）
- **THEN** 系统 SHALL 原样传输这些字节，不做任何过滤或转换

### Requirement: PTY resize
系统 SHALL 支持动态调整 PTY 终端尺寸，并发送 SIGWINCH 信号给子进程。

#### Scenario: Client resizes terminal
- **WHEN** 客户端发送 `{"type":"resize","cols":100,"rows":30}` 控制消息
- **THEN** 系统调用 PTY resize API 更新尺寸，子进程收到 SIGWINCH 信号

### Requirement: PTY session lifecycle
系统 SHALL 管理 PTY 会话的完整生命周期：创建、运行、关闭。

#### Scenario: Client closes session
- **WHEN** 客户端发送 `{"type":"close"}` 控制消息或 WebSocket 断开
- **THEN** 系统向子进程发送 SIGHUP，等待 3 秒后 SIGKILL，释放 PTY 资源

#### Scenario: Shell process exits
- **WHEN** PTY 子进程自行退出（exit code N）
- **THEN** 系统发送 `{"type":"exited","code":N}` 并关闭 WebSocket 连接

#### Scenario: Idle timeout
- **WHEN** PTY 会话超过 30 分钟无任何 I/O 活动
- **THEN** 系统自动关闭会话并释放资源

### Requirement: WebSocket endpoint routing
Gateway SHALL 在 `/api/pty/{session_id}` 路径提供 WebSocket 端点。

#### Scenario: Connect to existing session
- **WHEN** 客户端连接 `ws://host:port/api/pty/{session_id}`
- **THEN** 系统建立 WebSocket 连接并关联到指定的 PTY 会话

#### Scenario: Connect to non-existent session
- **WHEN** 客户端连接的 session_id 不存在
- **THEN** 系统返回 404 错误并拒绝 WebSocket 升级
