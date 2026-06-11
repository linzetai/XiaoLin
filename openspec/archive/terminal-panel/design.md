## Context

当前 XiaoLin 的 shell 执行能力：

- **ShellRuntime** (`xiaolin-agent/src/runtime/runtimes/shell.rs`)：`tokio::process::Command` + `Stdio::piped()`，等待进程退出后批量返回 stdout/stderr
- **PtySessionManager** (`xiaolin-tools-fs/src/exec_command.rs`)：名为 PTY 实为 piped stdin/stdout 的 `std::process::Command`，Agent 通过 `exec_command`/`write_stdin` 交互，无流式输出
- **ToolProgress 协议**：`xiaolin-protocol` 已定义 `ToolProgress { partial_output }` 事件，但 shell 工具未接入
- **前端展示**：`StepIndicator` / `ToolCallCard` 中以 `<pre>` 块展示完整 shell 结果，16 行截断
- **layout-overhaul** 已规划 WorkspacePanel 但仅有 Review tab 的 spec

依赖关系：
- 依赖 `layout-overhaul` 的 WorkspacePanel tab 切换框架
- 不强依赖 `project-model`，但 PTY 的 cwd 可从 project.root_path 获取
- 不依赖 `git-integration`，与其并行

## Goals / Non-Goals

**Goals:**
- 在 WorkspacePanel 提供 per-session 的终端面板，Agent 和用户共享统一视图
- 渐进式实现：Phase 1 流式查看 → Phase 2 用户输入 → Phase 3 完整 PTY
- Phase 3 使用 `tauri-plugin-pty` + xterm.js 提供真实终端体验
- Agent shell_exec 的输出实时流式推送到终端面板
- Agent 和用户可在同一 PTY 中操作，Agent 命令有视觉区分
- 终端面板支持 ANSI 颜色、resize、搜索

**Non-Goals:**
- 不做多 tab 终端（tmux 风格）——每个 session 一个终端即可
- 不做 SSH 远程终端——仅本地 PTY
- 不做终端分屏（split pane）——WorkspacePanel 空间有限
- 不替换 Agent 的 sandbox 机制——PTY 模式下 Agent 命令仍受 sandbox 约束
- 不做终端录制/回放

## Decisions

### D1: 三阶段渐进实现

**决策**：分三个 Phase 实现，每个 Phase 可独立交付价值。

| Phase | 交付物 | 核心技术 |
|-------|--------|----------|
| Phase 1 | Agent 命令流式查看器 | ToolProgress 流式推送 + `<pre>` + ansi-to-react |
| Phase 2 | + 用户命令输入 | terminal.* WS API + CommandInput 组件 |
| Phase 3 | + 完整 PTY | tauri-plugin-pty + xterm.js + Shell Integration |

**理由**：Phase 1 可与 layout-overhaul 同步交付，复杂度最低（★★）。Phase 3 需要引入新的 Tauri 插件和前端大依赖（xterm.js），适合作为独立迭代。
**替代方案**：一步到位实现 Phase 3。否决，因为 xterm.js + PTY 引入的复杂度会拖慢 layout-overhaul 的交付。

### D2: PTY 实现选择 tauri-plugin-pty

**决策**：Phase 3 使用 `tauri-plugin-pty` (v0.2, 基于 portable-pty 0.9)。

**理由**：
- Tauri 生态官方插件，IPC 层开箱即用
- 前端 API 简洁：`spawn() → onData / write / resize`
- 对于辅助面板终端（非主产品），IPC 序列化的性能开销可接受

**替代方案**：
- 手动 `portable-pty` + 内部 WebSocket 数据面（OxideTerm 方案）：性能更优但架构复杂度高，预留为 Phase 3+ 升级路径
- `alacritty_terminal` VTE 引擎（AITerm 方案）：Rust 侧完整缓冲，过重

### D3: Agent/用户共享 PTY 模型

**决策**：Agent 和用户共享同一个 per-session PTY，Agent 通过 `agent_lock` 互斥写入。

**工作方式**：
- Agent 执行 shell_exec → 检查 session 是否有活跃 PTY
  - 有 PTY → 获取 `agent_lock`，写入 PTY stdin，监听 Shell Integration 信号收集输出
  - 无 PTY → 走传统 piped 路径（向后兼容）
- 用户击键 → 始终可写入 PTY（包括 Ctrl+C 中断 Agent 命令）
- `agent_lock` 是"协作锁"而非"独占锁"——仅保护 Agent 输出收集，不阻塞用户输入

**理由**：Codex 模式（"Run commands in each thread"）。统一视图让用户看到 Agent 的完整操作过程。
**替代方案**：Agent 和用户分离 PTY。否决，因为 Agent 在用户的工作目录操作，分离会导致文件状态不一致的困惑。

### D4: 命令完成检测使用 Shell Integration (OSC 133)

**决策**：在 PTY 启动时注入 Shell Integration 脚本，通过 OSC 133 序列检测命令边界。

**协议**：
- `\e]133;A\a` — Prompt 开始
- `\e]133;C\a` — 命令执行开始
- `\e]133;D;{exit_code}\a` — 命令执行结束 + exit code

**注入方式**：PTY spawn 时通过环境变量 + init 脚本注入到 bash/zsh。

**回退**：如果 Shell Integration 注入失败（非标准 shell），回退到"超时 + 空闲检测"（stdout 静默 3s 视为完成）。

**理由**：VSCode、iTerm2、WezTerm 使用同样的标准。xterm.js 原生支持 OSC 133 装饰。
**替代方案**：自定义 PS1 标记。可行但侵入性更强，且不被 xterm.js 原生识别。

### D5: ANSI 渲染策略

**决策**：
- Phase 1/2：`ansi-to-react` 库将 ANSI 转 React 元素渲染在 `<pre>` 中
- Phase 3：xterm.js 原生 ANSI 渲染（完整 VT100/VT220 支持）

**理由**：渐进过渡。`ansi-to-react` 体积小（~5KB），满足 Phase 1 需求。Phase 3 升级 xterm.js 后自然替换。

### D6: 终端数据流架构

**Phase 1 数据流**：
```
ShellRuntime → 逐行读 stdout → ToolProgress { partial_output } → WS broadcast
                                                                    → 前端 terminal-store 追加
                                                                    → TerminalViewer 渲染
```

**Phase 2 数据流（用户命令）**：
```
用户输入 → terminal.exec { session_id, command } → WS handler
         → ShellRuntime.execute_streaming() → ToolProgress 流式返回
         → terminal.output WS event → 前端追加渲染
```

**Phase 3 数据流（完整 PTY）**：
```
用户击键 → invoke("terminal.write", bytes) → Tauri IPC → PTY master.write()
PTY master.read() → Tauri event "terminal:data" → xterm.js term.write(bytes)

Agent shell_exec → PTY master.write("cmd\r\n") → 共享 PTY
                  ← OSC 133;D → 收集输出 → ToolResult
```

### D7: 前端 store 设计——统一接口，渐进升级

**决策**：`useTerminalStore` 从 Phase 1 开始设计稳定接口，Phase 3 仅替换内部实现。

```typescript
// Phase 1/2: 基于命令记录的模型
interface TerminalState {
  entries: TerminalEntry[];     // 命令执行记录
  isStreaming: boolean;
  activeEntryId: string | null;
}

// Phase 3: 基于 PTY handle 的模型
interface TerminalState {
  ptyId: string | null;         // PTY handle
  isAlive: boolean;
  shellType: string;
  agentCommands: AgentCommand[];  // Agent 命令标记（用于 xterm.js 装饰）
}
```

**理由**：Phase 1→2 是增量改动，Phase 2→3 是接口变更但组件整体替换（TerminalViewer → XTermTerminal），不会出现中间态。

### D8: PTY 创建时机——惰性创建

**决策**：
- 用户首次打开 Terminal tab 时创建 PTY
- Agent 首次执行 shell_exec 时，如果 session 已有 PTY 则复用，否则不主动创建
- Session 关闭时 kill PTY

**理由**：大多数 session 可能不需要终端（纯对话），惰性创建节省系统资源。

## Risks / Trade-offs

**[R1] Agent PTY 输出收集不可靠** → Shell Integration 注入失败时回退到超时检测；设 5s 超时上限；记录失败日志供诊断

**[R2] 用户 Ctrl+C 中断 Agent 命令后状态混乱** → Agent 的 ShellRuntime 检测非零 exit code（130 = SIGINT），在 ToolResult 中标记"用户中断"而非"执行失败"

**[R3] xterm.js 包体积大（~200KB gzip）** → 仅 Phase 3 引入，且使用 dynamic import 按需加载；Phase 1/2 用轻量的 ansi-to-react

**[R4] 共享 PTY 并发写入** → agent_lock 保护 Agent 命令的完整性；用户写入始终不阻塞；PTY 本身是进程级安全的

**[R5] tauri-plugin-pty 在 Linux 上的兼容性** → portable-pty 在 Linux 上使用 `/dev/ptmx` 原生 PTY，成熟度高（WezTerm 生产使用）；WebKitGTK WebView 与 Tauri IPC 的兼容性已在现有功能中验证

**[R6] Phase 过渡期的代码冗余** → Phase 2→3 时 TerminalViewer + CommandInput 被 XTermTerminal 完全替换（删除而非共存）；useTerminalStore 接口变更但组件是原子替换

## Open Questions

- **Q1**: Phase 3 是否需要 output buffer（用于 session 切换时的回放）？xterm.js 的 `Terminal.buffer` 可以持有历史，但 session 切换时 xterm 实例可能被销毁。候选方案：(a) per-session RingBuffer 64KB 在 Rust 侧缓存 (b) 依赖 xterm.js serialize addon
- **Q2**: Agent 在 PTY 模式下是否仍然需要 sandbox？如果 PTY shell 不经过 bwrap，Agent 写入的命令不受 sandbox 保护。候选方案：(a) PTY shell 本身在 sandbox 内启动 (b) Agent 命令走独立 piped 路径而非 PTY
