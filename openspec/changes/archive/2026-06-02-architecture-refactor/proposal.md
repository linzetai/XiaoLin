## Why

`xiaolin-agent` 承担了过多职责（runtime + 35+ tools + browser 2.4k LOC + network 2.1k LOC），导致编译时间长、变更影响面大、难以独立测试。`xiaolin-gateway` 同样职责过多（HTTP/WS 路由 + chat 管线 + 状态初始化 + MCP 管理 + cron + memory monitor）。同时存在过薄的 crate（`xiaolin-path` ~500 LOC、`xiaolin-hardening` ~200 LOC）增加了不必要的工作区复杂度。MCP client 的全局 Mutex 限制了并发工具调用。这些问题随着功能增长会持续恶化。

## What Changes

- **拆分 `xiaolin-agent`**：将 35+ builtin tools 按领域拆分为独立 crate：
  - `xiaolin-tools-fs`：filesystem、shell、terminal、worktree 工具
  - `xiaolin-tools-network`：http_fetch、web_search、web_fetch 工具
  - `xiaolin-tools-browser`：Chrome/CDP 浏览器自动化（feature-gated）
  - `xiaolin-tools-code`：code_intel、lsp_manager、notebook 工具
  - `xiaolin-agent` 保留：runtime loop、prompt engine、orchestrator、provider 管理
- **合并过薄 crate**：
  - `xiaolin-path` → 合并进 `xiaolin-core`
  - `xiaolin-hardening` → 合并进 `xiaolin-core`（`core::hardening` 模块）
- **改善 MCP 并发**：将 `xiaolin-mcp` client 的全局 Mutex 改为 per-request channel，支持并发工具调用
- **Gateway 模块化**：在 `xiaolin-gateway` 内部按职责划分清晰的模块边界（chat、admin、mcp、cron），不新增 crate 但改善内聚

## Capabilities

### New Capabilities

- `tool-crate-splitting`: 定义工具 crate 拆分的边界、注册机制和依赖规则
- `mcp-concurrent-client`: MCP client 并发调用支持的协议和实现规范

### Modified Capabilities

- `tool-exposure`: 工具注册机制需要适配多 crate 来源的 Tool 实现

## Impact

- **Rust 源码**：`xiaolin-agent` 大幅重构，35+ 工具文件迁移到新 crate
- **Cargo.toml**：新增 4 个 crate，删除 2 个 crate，根 workspace 成员变更
- **编译时间**：拆分后增量编译加速，工具变更不再触发 agent runtime 重编译
- **API 兼容**：对外 API 无变化，工具行为不变
- **测试**：工具测试随代码迁移到各自 crate
