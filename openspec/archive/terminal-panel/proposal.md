## Why

当前 Agent 通过 `shell_exec` 执行命令的输出仅以 `<pre>` 块散落在消息流中，用户无法实时观察长时间运行命令（如编译、测试）的进度，也无法直接在界面中输入自己的命令。终端体验是 Codex 风格桌面应用的核心交互之一（"Terminal and actions — Run commands in each thread"），缺少嵌入终端面板会使应用在日常开发场景中严重受限。

## What Changes

- **新增嵌入终端面板**：在 WorkspacePanel 注册 "Terminal" 标签页，提供 per-session 的终端视图
- **Shell 执行流式化**：改造 `ShellRuntime` 从批量返回改为逐行流式推送 `ToolProgress`，Agent 命令的输出实时可见
- **终端数据聚合**：新增前端 Zustand store 汇聚同一 session 的所有 shell 执行记录
- **用户命令输入**：允许用户在终端面板直接输入命令执行，走独立的 `terminal.*` WS API
- **完整 PTY 集成**：引入 `tauri-plugin-pty`（基于 `portable-pty`）提供真实伪终端，支持 ANSI 渲染、交互式程序、终端 resize
- **xterm.js 前端渲染**：使用 xterm.js + WebGL addon 替代 `<pre>` 块，提供 GPU 加速的终端渲染
- **Agent/用户共享终端**：Agent 的 shell 命令和用户手动输入共享同一个 PTY 会话，统一可见
- **Shell Integration**：注入 OSC 133 序列实现命令边界检测，让 Agent 能精确捕获命令输出

## Capabilities

### New Capabilities
- `shell-streaming`: ShellRuntime 流式化改造，逐行推送 ToolProgress 事件
- `terminal-store`: 前端 Zustand store，管理 per-session 终端状态、命令记录、PTY 生命周期
- `terminal-viewer`: 终端查看器组件，从 `<pre>` + ansi-to-react 到 xterm.js 的渐进实现
- `terminal-user-input`: 用户命令输入能力，含 WS API (`terminal.exec`)、命令历史、基础补全
- `pty-manager`: 后端 PTY 管理器，基于 tauri-plugin-pty 的 per-session PTY 生命周期管理
- `pty-agent-bridge`: Agent ShellRuntime 与 PTY 的桥接层，共享 PTY 写入、Shell Integration 命令完成检测
- `shell-integration`: Shell Integration (OSC 133) 注入和解析，实现命令边界检测和 exit code 捕获

### Modified Capabilities
- `workspace-panel`: WorkspacePanel 通过 tab 系统接纳 Terminal 标签，终端面板与 Review 等内容并列
- `chat-input-bar`: InputBar 移除或弱化内联 shell 输出展示（Agent shell 结果主要在终端面板可见）

## Impact

- **后端 crates**：
  - `xiaolin-agent/src/runtime/runtimes/shell.rs`：流式化改造 + PTY 桥接
  - `xiaolin-tools-fs/src/exec_command.rs`：PtySessionManager 重构为基于 portable-pty
  - `xiaolin-gateway/src/ws/`：新增 `terminal.rs` handler
  - `Cargo.toml`：新增 `tauri-plugin-pty` 依赖
- **前端**：
  - 新增 `useTerminalStore` + `XTermTerminal` 组件
  - `package.json`：新增 `@xterm/xterm`、`@xterm/addon-fit`、`@xterm/addon-webgl`、`tauri-pty` 依赖
  - WorkspacePanel 组件通过 tab 注册系统集成 Terminal 标签
- **Tauri 配置**：`capabilities/default.json` 增加 `pty:default` 权限
- **协议**：复用 `ToolProgress` 事件 + 新增 `terminal.*` WS API
