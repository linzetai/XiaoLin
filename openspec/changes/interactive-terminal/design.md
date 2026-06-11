## Context

XiaoLin 当前的 Terminal Panel（`TerminalPanel.tsx`）通过 Zustand store 接收 Agent 的 `shell_exec` 工具事件（`tool_executing` → `tool_progress` → `tool_result`），将命令输出渲染为只读 ANSI 文本。用户无法在面板中直接输入命令。

后端 `xiaolin-gateway` 已有 WebSocket 基础设施（`/api/ws` 用于 Agent 通信），可扩展新端点。前端使用 React 19 + Zustand 5 + Vite 构建。

## Goals / Non-Goals

**Goals:**
- 用户可在 Terminal Panel 内打开持久化 shell 会话并直接输入命令
- 支持完整终端仿真（readline、光标移动、全屏程序如 top/vim）
- 支持终端 resize，自适应面板尺寸变化
- 支持多个并发 PTY 会话，独立创建/切换/关闭
- 跨平台兼容（Linux、macOS、Windows）

**Non-Goals:**
- 不改变 Agent `shell_exec` 的 oneshot 执行模式
- 不实现 Agent 向 PTY 会话注入命令的功能（解耦设计）
- 不实现断线重连/会话持久化（应用重启后会话丢失）
- 不实现远程 PTY（仅本地 shell）
- 不实现终端复用器功能（如 tmux split-pane）

## Decisions

### 1. PTY 库：`portable-pty`

**选择**: `portable-pty` crate（wezterm 作者维护）

**替代方案**:
- `tokio-pty-process`: 已废弃，最后更新 2019
- 自己 fork/exec + pipe: 不支持 SIGWINCH，无法运行全屏程序
- `nix::pty`: 仅 Unix，Windows 不兼容

**理由**: `portable-pty` 是唯一活跃维护的跨平台 Rust PTY 库，Linux 使用 Unix98 PTY，Windows 使用 ConPTY，macOS 使用 posix_openpt。API 简洁（openpty → spawn → read/write/resize）。

### 2. 前端终端：`@xterm/xterm` v5

**选择**: xterm.js v5（`@xterm/xterm` 包名）

**替代方案**:
- 自定义 canvas/DOM 渲染: 工作量巨大，无法支持完整 VT100
- `terminal-kit`: Node.js 库，不适合浏览器
- 保持现有 `<pre>` + ansi-to-html: 无法交互

**理由**: xterm.js 是 VS Code、Hyper、Theia 等项目使用的标准方案，社区活跃，支持 WebGL 加速渲染，有 fit addon 自适应容器尺寸。

### 3. 通信协议：Gateway WS 新端点 + 混合帧

**选择**: 在现有 gateway 上新增 `/api/pty/{session_id}` WebSocket 端点，使用 Binary 帧传输 I/O 数据，Text 帧传输控制消息

**替代方案**:
- Tauri IPC: 无法利用现有 gateway 基础设施，且 IPC 不适合高频二进制流
- 独立 TCP 服务: 增加端口管理复杂度
- 在现有 `/api/ws` 上复用: 会干扰 Agent 消息流

**理由**: 独立端点隔离性好，Binary 帧零拷贝传输 PTY 字节流性能最优，Text 帧用于低频控制消息（resize/create/close）便于调试。

### 4. 会话生命周期：随应用 + idle timeout

**选择**: PTY 会话随 gateway 进程生命周期存在，配合 idle timeout（默认 30 分钟无活动自动关闭）

**替代方案**:
- 无 timeout（永不关闭）: 泄漏 shell 进程
- 随 WebSocket 断开立即关闭: 面板切换/刷新会丢失会话

**理由**: 平衡资源占用和用户体验，30 分钟 idle timeout 足够覆盖正常使用场景，避免僵尸进程。

### 5. 新 crate 结构

**选择**: 新建 `xiaolin-pty` crate，作为 `xiaolin-gateway` 的依赖

**理由**: PTY 管理逻辑独立于 Agent/LLM 流程，单独的 crate 便于测试和复用。

## Risks / Trade-offs

- **[Windows ConPTY 兼容性]** → ConPTY 在某些 Windows 版本上行为不一致；缓解：初期仅保证 Linux/macOS 完整支持，Windows 标记为 experimental
- **[xterm.js 包体积 ~200KB gzip]** → 对 Tauri 桌面应用影响可忽略（不是 web 应用）
- **[PTY 进程泄漏]** → child 进程可能变成僵尸；缓解：定期检查 child status + SIGTERM + SIGKILL 清理链
- **[高频 I/O 渲染性能]** → cat 大文件时 xterm.js 可能卡顿；缓解：启用 WebGL addon，或在后端做输出节流（throttle 1MB/s 以上时分批发送）
