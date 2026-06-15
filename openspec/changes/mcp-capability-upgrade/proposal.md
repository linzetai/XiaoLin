# MCP 能力全面升级

## 概要

对标 Codex CLI 和 Claude Code 的 MCP 实现，将 XiaoLin 的 MCP 能力从约 30 分提升到 100 分。

**当前进度**：~60 分（2026-06-15）。10/34 任务完成 + 1 部分完成（新增 T10.5 UI 风格统一）。P1 UI 重构已完成，P0 命名一致性（T2）已完成，基础任务（T1/T7/T9）已完成，Transport 枚举化 + 统一连接入口（T4）和三路径重载统一（T5）已完成并通过 E2E 验证。主要差距集中在：Notification 处理、安全审批门、Deferred 工具注入、前端安装流程。

## 动机

当前 XiaoLin MCP 的主要痛点：

1. ~~**Settings 中的 McpManager 是完全的 mock 数据**~~ → ✅ 已删除
2. **PluginsView 已成为三 Tab 统一入口**（MCP/Skills/Channels），但 MCP Tab 仍缺添加/详情/审批功能
3. **三套 Transport 重载逻辑不一致**：gateway `state/mod.rs` 启动/热重载已支持三种 transport，但 `mcp_tool.rs::do_reload` 缺 streamable_http 分支、WS `mcp.add` handler 硬编码 stdio
4. ~~**工具名管线分裂**~~ → ✅ 全链路 `mcp__` 一致（T2 完成：chat_pipeline、tool.rs、agent_config、subagent、tool_executor + 前端 StepIndicator/ToolCallCard）
5. **无 deferred loading**：MCP 工具全量 eager 注册 + system prompt 双重注入，token 浪费严重
6. **无自动重连**：MCP server 进程崩溃 / 网络断开后无恢复机制
7. **Notification 黑洞**：reader 只解析带 `id` 的 Response，无 `id` 的 Notification 被丢弃
8. **项目 MCP 无审批门**（安全风险）：`.xiaolin/mcp.json` 中任意 command 在打开项目时直接执行

## 对标分析（深度三方对比 2026-06-15）

| 维度 | XiaoLin (~50) | Codex (~85) | Claude Code (~95) |
|------|:---:|:---:|:---:|
| **管理入口** | ✅ Web UI 三 Tab 统一 | TUI `/mcp` + CLI `codex mcp` | TUI `/mcp` + CLI `claude mcp` |
| **Transport 客户端** | stdio/SSE/HTTP | stdio/StreamableHTTP（弃用 SSE） | stdio/SSE/HTTP/WS/SDK/InProcess |
| **Transport 管理面** | ⚠️ state/mod.rs 有，do_reload/mcp.add 缺 | 统一 `AsyncManagedClient` → `make_rmcp_client` | 统一 `connectToServer` [memoized] |
| **工具命名** | ✅ `mcp__` 全链路（T2 完成） | `mcp__` 全链路 + hash 去重 + 64B 截断 | `mcp__` 全链路 |
| **Deferred Loading** | ❌ 全量 eager + 双注入 | 阈值 100 + BM25 search + startup snapshot | **默认 defer** + 10% context window |
| **Notification** | ❌ 丢弃 | ⚠️ 只 log 不处理 | ✅ 完整 list_changed 链路（三类） |
| **自动重连** | ❌ | ⚠️ StreamableHTTP 404 session 恢复 | ✅ remote 指数退避 5 次 + 3 连续错误触发 |
| **批次限制** | ❌ | ✅ JoinSet 并行（无 cap） | ✅ local 3 / remote 20 |
| **启动超时** | ❌ 仅 RPC 30s | ✅ per-server 30s + required 标志 | ✅ 30s |
| **审批门** | ❌ 死代码 | ✅ 4 层 approval + OAuth 自动检测 | ✅ project gate + enterprise deny/allow |
| **配置层数** | 2 (user + project) | 7 层 + plugin + external agent migration | 8 层 + enterprise 独占 |
| **Instructions 注入** | ❌ | ❌ | ✅ delta attachment 避免 cache bust |
| **配置签名去重** | ❌ | ❌ | ✅ `getMcpServerSignature` + `dedupPluginMcpServers` |
| **Session 恢复** | ❌ | ✅ StreamableHTTP 404 自动重新 initialize | ✅ HTTP session 重试 MAX_SESSION_RETRIES=1 |
| **进程清理** | ❌ | ✅ | ✅ SIGINT→SIGTERM→SIGKILL (~600ms) |
| **Channels/Skills** | ✅ 独有 | ❌ | ❌ |

### 关键洞察

1. **Codex 也没处理 `tools/list_changed`**（只 log）→ XiaoLin 做好此项即超越 Codex
2. **Claude Code 默认 defer 所有 MCP 工具**（非阈值触发）→ 更激进但效果显著
3. **Claude Code 的 instructions delta 注入**避免 system prompt 变化导致 prompt cache 失效
4. **Codex 已完全弃用 legacy SSE**，只保留 Stdio + StreamableHTTP → 更符合 MCP 2025-06-18 规范
5. **Claude Code 配置签名去重**（`getMcpServerSignature`）防止同一 server 被 plugin + 手动配置连接两次
6. **Codex 有 startup snapshot cache**：`codex_apps` server 启动时先用磁盘缓存的工具列表，后台完成真正 `list_tools` — 显著减少冷启动延迟
7. **Codex 的 `required` 标志**：标记为 required 的 server 失败会中止整个 session — 适用于关键 MCP 依赖
8. **Codex 的 OAuth 自动检测**：`codex mcp add` 时自动探测 server OAuth 支持并触发 login 流程
9. **Claude Code 的 `needs-auth` 状态**：15 分钟 TTL 缓存避免重复 OAuth 探测 — 好的 auth 模式
10. **Claude Code 的连续错误重连**：3 次连续 ECONNRESET/ETIMEDOUT 后触发完整重连 — 比单次断开检测更健壮

### 三套连接逻辑分析（XiaoLin 特有问题）

XiaoLin 有三条独立的 MCP 连接/重载路径，各自维护 transport 分发，行为不一致：

| 路径 | 位置 | 并行 | stdio | SSE | streamable_http | project MCP |
|------|------|:---:|:---:|:---:|:---:|:---:|
| 启动 `register_mcp_and_subagent_tools` | state/mod.rs | ✅ join_all | ✅ | ✅ | ✅ | ❌ |
| 热重载 `reload_mcp_servers` | state/mod.rs | ❌ 串行 | ✅ | ✅ | ✅ | ✅ |
| Agent Tool `do_reload` | mcp_tool.rs | ❌ 串行 | ✅ | ✅ | **❌ 缺失** | ❌ |
| WS `mcp.add` | ws/mcp.rs | N/A | ✅ | ❌ 硬编码 stdio | ❌ | ❌ |

**Codex 做法**：两层抽象 `McpConnectionManager` → `AsyncManagedClient` → `make_rmcp_client()`，transport enum 分发在最底层且仅有一处。
**Claude Code 做法**：`connectToServer()` + memoize（config hash 作 key），stale 检测 + 清缓存 + 重新 pending。

## 方案

### 阶段一：修 Bug + 清理 + 安全 (P0) — 进行中

| 项目 | 状态 | 说明 |
|------|:---:|------|
| 删除 McpManager.tsx | ✅ | T1，纯 mock 死代码 |
| 捕获 MCP 子进程 stderr | ✅ | T7，`stderr_reader_loop` |
| 升级 protocolVersion 2025-06-18 | ✅ | T9 |
| 命名 Rust 端全链路 | ✅ | T2，chat_pipeline/tool.rs/agent_config/subagent/tool_executor 全部迁移到 `mcp__` |
| 命名前端 prefix 更新 | ✅ | T3 部分，StepIndicator/ToolCallCard.test.tsx 已更新到 `mcp__` |
| 命名前端工具函数 | ⚠️ | T3 剩余，`mcpNaming.ts` 工具函数未创建，ToolCallCard 未统一用 `parseMcpToolName` |
| Transport 枚举 + `connect_mcp_server` | ❌ | T4，消除三套重载的前置 |
| 启动/热重载路由统一 | ⚠️ | `state/mod.rs` 三种 transport ✅；`mcp_tool.rs`/WS `mcp.add` ❌ |
| Notification dispatch | ❌ | T6，reader 只解析 Response，Notification 被丢弃 |
| 项目 MCP 审批门 | ❌ | T8，`project_mcp_approval.rs` 是未接线死代码 |
| 配置验证 | ❌ | T10 |

### 阶段二：PluginsView 三 Tab 统一管理入口 (P1) — ✅ 已完成

PluginsView 已重构为 **MCP + Skills + Channels 的统一管理入口**：

```
┌──────────────────────────────────────────────────────────┐
│ 🧩 Plugins                                               │
│ Extend capabilities with MCP servers, skills & channels  │
│                                                           │
│  [MCP Servers (2)]  [Skills (190)]  [Channels (2)]       │
│                                            [↻ Reload]    │
├──────────────────────────────────────────────────────────┤
│  (当前 tab 内容区域)                                      │
└──────────────────────────────────────────────────────────┘
```

| 任务 | 状态 |
|------|:---:|
| T11: 三 Tab 骨架 | ✅ |
| T16: Skills Tab 迁移 | ✅ |
| T17: Channels Tab 迁移 + 删除 ConnectionsPage | ✅ |
| T18: EmptyState 更新 + Settings Skills 移除 | ✅ |
| T12: MCP 添加/删除 Modal | ❌ 待做 |
| T13: PluginSummary 扩展 + 分组 | ❌ 待做 |
| T14: 审批 UI | ❌ 待做（依赖 T8） |
| T15: PluginDetailModal | ❌ 待做 |

**已清理**：
- ✅ 删除 `settings/McpManager.tsx`
- ✅ 删除 `connections/ConnectionsPage.tsx`
- ✅ 删除 `settings/SkillsTab.tsx`
- ✅ `SettingsPanel.tsx` 移除 Skills tab
- ✅ E2E 验证通过（三 Tab 切换、数据加载、设置页面无残留）

### 阶段三：后端能力补齐 (P2)

- `tools/list_changed` 通知 → diff 更新 ToolRegistry + 局部 schema 缓存失效（**超越 Codex：Codex 只 log 不处理**）
- 自动重连（仅 SSE/HTTP，stdio 不重连，对标 Claude Code）+ 指数退避 `min(1000×2^(n-1), 30000)ms`，最多 5 次
- 连接批次限制：stdio 并发 3、remote 并发 20（对标 Claude Code `MCP_SERVER_CONNECTION_BATCH_SIZE`）
- 启动超时 `startup_timeout_sec` per-server 配置（默认 30s，对标 Codex `DEFAULT_STARTUP_TIMEOUT`）
- 逐 server 启动状态事件推送（Starting → Ready/Failed，对标 Codex `McpStartupUpdateEvent`）
- Session 级 tool schema 缓存（字节级，局部 invalidate）
- MCP 工具 description 截断（最大 2048 字符，对标 Claude Code `MAX_MCP_DESCRIPTION_LENGTH`）
- stale server 检测与清理（对标 Claude Code `excludeStalePluginClients`，config hash 变化检测）
- **新增**：Server instructions delta 注入（对标 Claude Code `getMcpInstructionsDelta`，避免 system prompt cache bust）
- **新增**：配置签名去重（对标 Claude Code `getMcpServerSignature`，防 plugin + 手动配置重复连接）

### 阶段四：智能工具注入 (P3)

- MCP 工具接入现有 deferred 管线（XiaoLin 已有 ToolSearch + register_deferred，无需重写）
- Deferred 时关闭 `inject_mcp_tools_prompt` 全量注入（消除双重 token 浪费）
- 阈值策略更新：**context window 的 10% 或工具数 > 100**（对标 Claude Code `getAutoToolSearchTokenThreshold`，比固定 100 更灵活）
- 支持 `alwaysLoad` 元数据（`tool._meta['anthropic/alwaysLoad']`）
- Schema 完整性：保留完整 JSON Schema 嵌套结构（oneOf、additionalProperties）
- **新增**：考虑默认 defer 模式（Claude Code 默认 `tst` 模式，所有 MCP 工具 defer）

## 不做的事

- Plugin marketplace / registry（当前规模不需要）
- OAuth 认证流程（后续单独提案；Codex 有 `StreamableHttpWithOAuth`，Claude Code 有 `ClaudeAuthProvider`）
- 完整 Elicitation Form/URL UI（后续单独提案；**风险**：部分 MCP server 如 OAuth/GitHub Copilot 依赖 elicitation，当前会静默失败）
- 企业级策略 allowlist/denylist（Claude Code 有 `deniedMcpServers`/`allowedMcpServers`，暂不需要）
- MCP Resources/Prompts client 消费（Claude Code 消费 prompts 作为 commands/skills）
- per-tool approval_mode 细粒度配置（Codex 有 4 层 approval，暂按 per-server 即可）
- WebSocket transport（Codex 不支持，Claude Code 有但非主流）
- 7+ 层配置 stack（Codex 7 层、Claude Code 8 层 + enterprise 独占模式，XiaoLin 2 层足够）

## 影响

- **前端**（已完成）：✅ 删除 McpManager.tsx + ConnectionsPage.tsx + SkillsTab.tsx，PluginsView.tsx 成为三 Tab 统一入口
- **后端**（进行中）：`xiaolin-mcp` 客户端增加 notification dispatch + 重连，gateway 统一连接入口
- **协议**（已完成）：工具名分隔符从 `_` 改为 `__` 全链路一致（T2 完成）
- **配置**：兼容现有格式，Transport 枚举化待做

## 评分路线图

```
~30 分 (初始)  →  ~45 分 (P1 UI 完成)  →  ~50 分 (T2 命名完成)
→  ~52 分 (T3 前端命名工具函数)  →  ★ ~60 分 (T4+T5 统一连接) ← 当前
→  ~67 分 (T6+T19 Notification)  →  ~73 分 (T8 审批门)
→  ~80 分 (T12+T15 MCP UI)  →  ~87 分 (T20-T22 重连/超时)
→  ~95 分 (T27-T31 Deferred)  →  100 分 (T32+T33 Delta+去重)
```
