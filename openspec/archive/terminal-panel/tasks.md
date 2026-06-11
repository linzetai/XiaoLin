## 1. Phase 1 — Shell 流式化后端

- [ ] 1.1 改造 `ShellRuntime::execute()`：将 `child.wait_with_output()` 替换为逐行异步读取 stdout/stderr（使用 `tokio::io::BufReader::lines()`）
- [ ] 1.2 在逐行读取循环中发射 `ToolProgress { partial_output }` 事件，实现 100ms 聚合窗口
- [ ] 1.3 合并 stdout 和 stderr 为单一输出流（使用 `tokio::select!` 交替读取）
- [ ] 1.4 保留最终 `ToolResult` 包含完整输出，确保向后兼容
- [ ] 1.5 验证无 PTY 场景下的 piped 流式模式正常工作

## 2. Phase 1 — 前端终端 Store

- [ ] 2.1 创建 `useTerminalStore` (Zustand)，定义 `TerminalState` 和 `TerminalEntry` 类型
- [ ] 2.2 实现 `tool_executing` → 创建 TerminalEntry 的逻辑（从 args 提取 command）
- [ ] 2.3 实现 `tool_progress` → 追加 partial_output 到活跃 entry 的逻辑
- [ ] 2.4 实现 `tool_result` → 标记 entry 完成、记录 exitCode 的逻辑
- [ ] 2.5 实现 active session 追踪（与 `useChatMetaStore.currentSessionId` 同步）
- [ ] 2.6 实现 session 删除时的 cleanup 逻辑

## 3. Phase 1 — TerminalViewer 组件

- [ ] 3.1 安装 `ansi-to-react` 依赖
- [ ] 3.2 创建 `TerminalViewer` React 组件，渲染 `TerminalEntry[]` 列表
- [ ] 3.3 实现 Agent 命令 vs 用户命令的视觉区分（图标 + 左侧边框颜色）
- [ ] 3.4 实现 monospace 字体容器 + ANSI 颜色渲染
- [ ] 3.5 实现自动滚底 + 手动滚动暂停 + "New output" 恢复按钮
- [ ] 3.6 实现空状态提示（"终端输出将在 Agent 执行命令时自动显示"）

## 4. Phase 1 — WorkspacePanel Terminal Tab 集成

- [ ] 4.1 通过 tab 注册系统在 WorkspacePanel 注册 Terminal 标签：`{ id: "terminal", label: "Terminal", icon: TerminalIcon, component: TerminalTabContent }`
- [ ] 4.2 实现 Tab 切换逻辑（Review ↔ Terminal），保持各 tab 状态独立
- [ ] 4.3 实现 Terminal tab 内容区渲染 TerminalViewer
- [ ] 4.4 实现 Agent 活动指示器（Terminal tab 图标上的小圆点/脉冲，当有 shell 命令执行时显示）
- [ ] 4.6 实现 Cmd+J 快捷键：专门切换 Terminal tab（非 Terminal tab 时切换过去，已是 Terminal tab 时关闭面板）
- [ ] 4.5 实现 tab-specific footer（Review tab 显示 Stage/Revert，Terminal tab Phase 1 无 footer）

## 5. Phase 1 — Chat 消息流集成

- [ ] 5.1 修改 shell_exec 工具卡片：正在执行时显示简化状态（"⚡ 正在执行"）
- [ ] 5.2 修改 shell_exec 工具卡片：完成后显示摘要 + "在终端中查看 →" 链接
- [ ] 5.3 实现"在终端中查看"链接点击：切换到 Terminal tab 并滚动到对应 entry

## 6. Phase 2 — 用户命令输入

- [ ] 6.1 创建 `CommandInput` React 组件，包含输入框 + Run 按钮
- [ ] 6.2 实现命令历史导航（↑↓ 键切换历史命令）
- [ ] 6.3 后端新增 `terminal.exec` WS handler，接收 `{ session_id, command, cwd }` 并流式执行
- [ ] 6.4 后端新增 `terminal.output` WS 事件，流式返回命令输出
- [ ] 6.5 后端新增 `terminal.exec_done` WS 事件，返回 exit_code + duration
- [ ] 6.6 后端新增 `terminal.history` WS handler，返回 session 命令历史
- [ ] 6.7 前端 terminal-store 集成用户命令 entry 创建和 WS 事件处理
- [ ] 6.8 Terminal tab footer 切换为渲染 CommandInput
- [ ] 6.9 处理并发冲突：Agent 执行中用户提交命令 → 排队等待

## 7. Phase 3 — PTY 后端管理

- [ ] 7.1 添加 `tauri-plugin-pty` 依赖到 `src-tauri/Cargo.toml`，在 Tauri Builder 中注册插件
- [ ] 7.2 添加 `pty:default` 权限到 `capabilities/default.json`
- [ ] 7.3 创建 `TerminalManager` 结构体，维护 per-session PTY 注册表（`HashMap<SessionId, TerminalInstance>`）
- [ ] 7.4 实现 `terminal.spawn` Tauri command：创建 PTY，设置 TERM/COLORTERM 环境变量，注入 Shell Integration
- [ ] 7.5 实现 `terminal.kill` Tauri command：SIGHUP → 500ms → SIGKILL
- [ ] 7.6 实现 `terminal.resize` Tauri command：调用 `pty.resize(PtySize)`
- [ ] 7.7 实现 `terminal.write` Tauri command：写入 PTY master writer
- [ ] 7.8 实现 PTY stdout 读取循环 → 发射 Tauri 事件 `terminal:data`
- [ ] 7.9 实现 PTY child exit 检测 → 发射 `terminal:exit` 事件
- [ ] 7.10 实现 `terminal.list` Tauri command：查询所有活跃 PTY
- [ ] 7.11 实现最大 PTY 并发限制（默认 10）
- [ ] 7.12 实现空闲 PTY 超时回收（30 分钟无 I/O → kill）

## 8. Phase 3 — Shell Integration

- [ ] 8.1 编写 bash Shell Integration 脚本（OSC 133 序列注入，使用 PROMPT_COMMAND + trap DEBUG）
- [ ] 8.2 编写 zsh Shell Integration 脚本（OSC 133 序列注入，使用 precmd + preexec hooks）
- [ ] 8.3 将脚本打包为 Tauri 资源文件
- [ ] 8.4 在 `terminal.spawn` 中根据 shell 类型加载对应脚本（bash: `--init-file`，zsh: `ZDOTDIR`）
- [ ] 8.5 添加重复注入检测：检查 `__XIAOLIN_SHELL_INTEGRATION` 环境变量
- [ ] 8.6 实现 OSC 133 解析器：从 PTY 输出流中提取命令边界事件

## 9. Phase 3 — Agent PTY 桥接

- [ ] 9.1 创建 `agent_lock` (tokio::sync::Mutex) per-session，保护 Agent 命令执行周期
- [ ] 9.2 修改 `ShellRuntime::execute()`：检测 session 是否有活跃 PTY，有则切换 PTY 模式
- [ ] 9.3 PTY 模式：获取 agent_lock → 写入命令 → 发射 `terminal:agent_cmd` 事件
- [ ] 9.4 PTY 模式：监听 OSC 133;D 信号收集输出 → 返回 ToolResult
- [ ] 9.5 实现 Shell Integration 不可用时的超时回退（3s 无输出视为完成）
- [ ] 9.6 实现用户 Ctrl+C 中断检测：exit_code=130 → metadata.interrupted=true
- [ ] 9.7 实现命令超时保护：超过 timeout_ms 不 kill PTY，返回超时 ToolResult 并释放 lock

## 10. Phase 3 — xterm.js 前端

- [ ] 10.1 安装 `@xterm/xterm`、`@xterm/addon-fit`、`@xterm/addon-webgl`、`@xterm/addon-search`、`@xterm/addon-web-links`、`tauri-pty` 依赖
- [ ] 10.2 创建 `XTermTerminal` React 组件，初始化 xterm.js Terminal + addons
- [ ] 10.3 实现 `usePtyBridge` hook — Effect 1: Tauri 事件 `terminal:data` → `term.write(data)`
- [ ] 10.4 实现 `usePtyBridge` hook — Effect 2: `term.onData()` → `invoke("terminal.write")`
- [ ] 10.5 实现 `usePtyBridge` hook — Effect 3: ResizeObserver + FitAddon.fit() → `invoke("terminal.resize")`
- [ ] 10.6 实现 `usePtyBridge` hook — Effect 4: `terminal:agent_cmd` → Agent 命令装饰
- [ ] 10.7 实现终端工具栏（shell 类型、cwd、Agent 活动指示器）
- [ ] 10.8 更新 `useTerminalStore` 为 Phase 3 PTY 模型（ptyId、isAlive、shellType）
- [ ] 10.9 实现 Terminal tab 打开时按需 spawn PTY（惰性创建）
- [ ] 10.10 实现 session 切换时的终端视图切换（PTY 后台保持，xterm 实例切换）
- [ ] 10.11 Phase 2 CommandInput 在 PTY 模式下隐藏
- [ ] 10.12 移除 Phase 1/2 的 TerminalViewer + ansi-to-react 依赖（清理死代码）

## 11. 清理与验证

- [ ] 11.1 重构或移除旧 `PtySessionManager`（exec_command.rs），替换为基于 tauri-plugin-pty 的实现
- [ ] 11.2 清理或移除 `terminal_capture` 工具（已无写入者）
- [ ] 11.3 运行 `cargo clippy -- -D warnings` 确认零警告
- [ ] 11.4 验证无 PTY 场景（Phase 1 模式）仍然正常工作
- [ ] 11.5 验证 Agent shell_exec 在 piped 模式和 PTY 模式下的 ToolResult 一致性
