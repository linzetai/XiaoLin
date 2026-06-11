## Why

XiaoLin 当前的 Terminal Panel 只能被动显示 Agent `shell_exec` 的只读输出，用户无法在应用内直接与 shell 交互。用户如果要执行临时命令、调试环境变量、运行 REPL 或管理进程，必须切出到外部终端工具，打断工作流。增加可交互式终端让用户在同一窗口中完成所有命令行操作。

## What Changes

- 新增 PTY (伪终端) 后端服务，支持持久化 shell 会话
- 新增 WebSocket 端点 `/api/pty/:session_id`，用于双向二进制流传输
- 前端集成 xterm.js 作为完整终端仿真器
- Terminal tab 拆分为两个子视图："Output"（现有 Agent 只读输出）和 "Shell"（交互式终端）
- 支持多会话管理（创建、切换、关闭）
- 支持终端 resize（SIGWINCH）
- Agent 的 `shell_exec` 工具保持 oneshot 模式不变，两系统完全解耦

## Capabilities

### New Capabilities
- `pty-backend`: PTY 会话管理后端服务（创建、读写、resize、生命周期管理）
- `interactive-terminal-ui`: xterm.js 前端终端组件（渲染、输入、会话切换、resize）

### Modified Capabilities
- `workspace-panel`: Terminal tab 新增 "Shell" 子视图入口

## Impact

- **新增 crate**: `xiaolin-pty`（依赖 `portable-pty`）
- **Gateway 路由**: 新增 `/api/pty` WebSocket 端点
- **前端依赖**: `@xterm/xterm`, `@xterm/addon-fit`, `@xterm/addon-webgl`（可选）
- **UI 变更**: WorkspacePanel Terminal tab 结构调整
- **平台兼容**: Linux/macOS 使用 Unix PTY，Windows 使用 ConPTY（通过 portable-pty 抽象）
