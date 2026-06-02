## Context

`xiaolin-agent` 当前包含 35+ builtin tools（分布在 `builtin_tools/` 下约 20 个子模块）、LLM provider 管理、AgentRuntime loop、PromptEngine、ToolOrchestrator、SubAgentManager 等。browser 工具约 2.4k LOC、network 工具约 2.1k LOC，这些工具的变更频率和依赖图谱与 agent runtime 核心完全不同。

`xiaolin-path`（~500 LOC，路径解析/绝对化）和 `xiaolin-hardening`（~200 LOC，进程早期安全加固）作为独立 crate 增加了 workspace 复杂度但无独立发布价值。

MCP client 在 `xiaolin-mcp/src/lib.rs` 中使用 `Arc<Mutex<McpClient>>` 包装，每次 `tools/call` 需要获取全局锁，并发调用被串行化。

## Goals / Non-Goals

**Goals:**
- `xiaolin-agent` 编辑任何一个工具时，增量编译不触发 runtime 核心重编译
- 工具 crate 可独立测试，不需要完整的 AgentRuntime 上下文
- 减少 workspace 成员数（合并 2 个过薄 crate）同时增加 4 个工具 crate（净增 2 个）
- MCP 并发调用不再串行

**Non-Goals:**
- 不拆分 `xiaolin-gateway`（仅做模块边界清理，不新增 crate）
- 不改变工具的外部行为和 API
- 不改变 `ToolRegistry` 注册机制的公共接口
- 不引入动态加载（依然是编译时链接）

## Decisions

### D1: 工具 crate 拆分边界

**决定**：按领域拆分为 4 个 crate：

| 新 Crate | 包含的工具模块 | 依赖 |
|----------|---------------|------|
| `xiaolin-tools-fs` | filesystem, shell, shell_readonly, shell_security, shell_path_validation, terminal, worktree, exec_command | xiaolin-core, xiaolin-security, xiaolin-path |
| `xiaolin-tools-network` | network (http_fetch, web_search, web_fetch) | xiaolin-core |
| `xiaolin-tools-browser` | browser (feature-gated) | xiaolin-core |
| `xiaolin-tools-code` | code_intel, lsp_manager, notebook, treesitter bridge | xiaolin-core, xiaolin-treesitter |

**保留在 `xiaolin-agent`**：plan_mode, plan_file, ask_question, confirm, brief, identity, memory, skill, todo, goal, task, session, coordinator, team, worker, workflow, snip, utility, tool_search, request_permissions, screenshot, media — 这些与 agent runtime 紧密耦合或过小不值得独立。

**替代方案**：全部拆出只留 runtime。但很多工具（如 plan_mode、ask_question）需要直接访问 agent 内部状态，拆分反而需要更多 trait 抽象。

### D2: 工具注册机制

**决定**：每个工具 crate 导出一个 `pub fn register(registry: &mut ToolRegistry, config: &AgentConfig)` 函数。`xiaolin-agent` 的 `register_builtin_tools` 调用各 crate 的 register 函数。

**理由**：最小化对现有注册流程的改动，不需要 trait object 或 inventory 宏。

### D3: 合并 path 和 hardening

**决定**：
- `xiaolin-path` → `xiaolin-core::path` 模块（已被 core 和 security 依赖）
- `xiaolin-hardening` → `xiaolin-core::hardening` 模块（仅在进程启动时调用一次）

**理由**：两者都是纯工具代码，无外部状态，合并后减少 Cargo.toml 管理成本。

### D4: MCP 并发改进

**决定**：将 `McpClient` 内部的 stdio 通信改为 request-id → oneshot channel 映射。发送请求时生成唯一 ID 并注册 oneshot，后台读取线程按 ID 分发响应。

**替代方案**：每个调用开新 stdio 进程。但 MCP 规范要求一个 server 一个连接。

**理由**：标准的 multiplexing 模式，不改变 MCP 协议语义。

## Risks / Trade-offs

- **编译依赖图变化** → 需要重新验证 feature flag 组合。缓解：CI 测试所有 feature 组合。
- **工具跨 crate 共享类型** → 部分工具间有隐式依赖（如 shell 工具使用 filesystem 的路径验证）。缓解：共享类型下沉到 `xiaolin-core`。
- **合并 xiaolin-path** → 下游 crate 需要从 `xiaolin_core::path` 导入而非 `xiaolin_path`。一次性 sed 替换。
- **MCP 并发** → 需要处理 server 进程崩溃时的 pending request 清理。缓解：超时 + drop 时自动 cancel。
