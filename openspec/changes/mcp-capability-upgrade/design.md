# MCP 能力升级 — 设计文档

> **Review 状态**：已对照 Codex CLI 和 Claude Code 源码完成交叉验证。

## 架构决策

### D1: MCP 管理统一到 PluginsView

> 详细方案见 [`specs/plugins-ui/spec.md`](specs/plugins-ui/spec.md)

**决策**：PluginsView 成为所有扩展能力（MCP + Skills + Channels）的唯一管理入口，以三 Tab 布局统一展示。删除分散的旧页面。

**理由**：
- MCP、Skills、Channels 分散在 PluginsView / SettingsPanel / ConnectionsPage 三处，用户体验碎片化
- McpManager.tsx 完全是 mock 数据，无调用方
- ConnectionsPage 无引用入口（疑似废弃），其功能可合并
- SkillsTab 藏在 Settings 面板里，与扩展能力管理强相关，应提升为一级入口

**P0 变更**（13 项）：

1. **三 Tab 布局** — `[MCP Servers (n)] [Skills (m)] [Channels (k)]`，每个 tab 有独立操作按钮
2. **MCP Tab - Header 操作** — `+ Add Server` + `↻ Reload All`
3. **MCP Tab - PluginSummary 扩展** — 增加 `transport`、`commandPreview`、`pendingApproval` 字段
4. **MCP Tab - PendingApprovalSection** — 项目级 MCP 待审批卡片（批准/拒绝按钮）
5. **MCP Tab - AddPluginModal 增强** — 支持 stdio/SSE transport 切换、env vars 编辑
6. **MCP Tab - PluginDetailModal 迁移** — 从 ConnectionsPage 迁移，增加配置预览和错误日志
7. **MCP Tab - 列表分组** — 按 scope（User/Project）分组，pending 项目级显示在顶部
8. **Skills Tab** — 从 SettingsPanel 迁移 SkillsTab 功能：Skills/Tools 切换、Upload、Refresh
9. **Channels Tab** — 从 ConnectionsPage 迁移 Channel 管理：ChannelCard、WeChat 扫码流程
10. **plugin-store 扩展** — 新增 addPlugin/removePlugin/approveProjectMcp/rejectProjectMcp/reloadAll
11. **transport.ts 新增 API** — plugins.add/remove/approve/reject/reload_all
12. **统一 EmptyState** — 每个 tab 有独立的空状态引导文案
13. **清理** — 删除 `McpManager.tsx` + `ConnectionsPage.tsx` + SettingsPanel 中移除 Skills tab

### D2: 工具命名规范化管线

> 详细方案见 [`specs/naming-pipeline/spec.md`](specs/naming-pipeline/spec.md)

**关键 Bug**：当前 `mcp_{id}_{tool}` 单下划线分隔，当 server_id 含 `_`（如 `chrome_devtools`）时前后端解析全部失败。

**决策**：升级为完整的命名规范化管线，涉及 11 个代码点（5 Rust + 2 TS + 配置/过滤/测试）。

**新格式**：`mcp__{sanitized_server_id}__{sanitized_tool_name}` — 与 Claude Code / Codex 完全一致

**P0 变更**：

1. **新增 `xiaolin-mcp/src/naming.rs`** — 集中定义 `sanitize_for_api`、`mcp_server_prefix`、`mcp_tool_name`、`parse_mcp_tool_name`、`is_mcp_tool`
2. **新增 `lib/mcpNaming.ts`** — 前端对应函数
3. **Sanitize 规则** — `[^a-zA-Z0-9_-]` → `_`（对标 Claude Code / Codex）
4. **解析** — `split("__")` 替代 `indexOf("_")`（修复歧义 Bug）
5. **11 个代码点全部更新** — prefix 构造、过滤、解析、权限 glob

**P2 扩展**（暂不实现）：

- Hash 去重：sanitize 后碰撞时追加 SHA1 12-char suffix
- 64 字节长度限制：超长时截断 + hash

**Server 名校验**：添加 server 时禁止 id 包含 `__`

### D3: 传输层修复与扩展

> 详细方案见 [`specs/transport-fix/spec.md`](specs/transport-fix/spec.md)

**Bug**：三条 MCP 连接路径中，两条（启动 + 热重载）**始终 stdio**，忽略 `transport` 字段。只有 `mcp_tool.rs` 的 `do_reload` 有 transport 判断。SSE 配置在启动/热重载时静默失败。

**P0 变更**（6 项）：

1. **Transport 枚举化** — `transport: String` → `McpTransportType` 枚举（`Stdio | Sse`），未知值反序列化失败
2. **`connect_mcp_server` 统一入口** — 新增函数，根据 transport 自动路由，三条路径全部调用
3. **启动路径修复** — `state/mod.rs:915` 改用 `connect_mcp_server`
4. **热重载路径修复** — `state/mod.rs:378` 改用 `connect_mcp_server`
5. **消除重复** — `mcp_tool.rs:259` 改用 `connect_mcp_server`
6. **配置验证** — `McpServerConfig::validate()` — stdio 需 command、sse 需 url、id 不含 `__`

**P2 路线**：Streamable HTTP（MCP 2025-06-18 推荐，替代 SSE）

**协议版本**：升级 `protocolVersion` 从 `2024-11-05` 到 `2025-06-18`（或最新 stable）。

### D4: 添加 MCP 服务器的 UI 增强

> 详细方案见 [`specs/plugins-ui/spec.md`](specs/plugins-ui/spec.md) — 变更 4 (AddPluginModal)

在 PluginsView 中实现，支持：
- 传输类型选择：stdio（默认）/ SSE
- stdio：command + args（空格分隔）+ 环境变量
- SSE：URL 输入
- 快速模板：预置常用 MCP server（P2）
- Server 名校验：`[a-zA-Z0-9_-]`，禁止 `__`

### D5: 重连策略（区分传输类型）

**关键发现**：Claude Code 对 stdio **不自动重连**（进程死了 → failed），仅对远程传输（SSE/HTTP/WS）重连。Codex 也无 stdio 重连。

**决策**：

| 传输 | 策略 |
|------|------|
| stdio | 进程退出 → `failed` + 展示 stderr → 用户手动「重启」 |
| SSE/HTTP | 断连 → 自动重连，指数退避 `min(1000 × 2^(n-1), 30000)ms`，最多 5 次 |
| HTTP 404 | session 过期 → reinitialize（而非 full reconnect） |

前端通过 `plugins.status_changed` 事件收到状态更新。Disable server 时取消 in-flight 重连定时器。

### D6: Notification Dispatch + tools/list_changed

> 详细方案见 [`specs/notification-dispatch/spec.md`](specs/notification-dispatch/spec.md)

**Critical 缺陷**：`stdio_reader_loop`（第 699-739 行）只解析 `JsonRpcResponse`（有 `id`），所有 Notification（无 `id`、有 `method`）**静默丢弃**。SSE reader 同样。

**额外缺陷**：stderr 被 piped 但无 reader，可能导致子进程缓冲区满后阻塞。

**方案**（6 项变更）：

1. **`McpNotification` + `broadcast::channel`** — 新增结构体 + 多订阅者 notification channel
2. **reader_loop 改造** — 先解析为 `serde_json::Value`，按 `id` 有无区分 Response / Notification
3. **SSE reader 同步改造** — 同样增加 notification dispatch
4. **`subscribe_notifications()` 公开 API** — gateway / 前端可各自订阅
5. **stderr reader** — `tokio::spawn` 独立任务读取 stderr → tracing
6. **`list_tools()` 强制刷新** — 区别于现有 `tools()`（返回缓存）

**Gateway 订阅**：连接后 `subscribe_notifications()`，处理 `tools/list_changed` → 重新 `list_tools()` → `unregister_by_prefix` + 重新 register

**对比**：Codex 的 handler 仅打日志；Claude Code 完整实现 diff + refresh。XiaoLin 实现此方案将超越 Codex。

### D7: 热重载逻辑统一

抽取共用函数 `connect_mcp_server(cfg, registry, prefix) -> Result`：
- 根据 transport 路由
- **连接批次限制**（对标 Claude Code）：stdio 并发 3，SSE/HTTP 并发 10-20
- 启动和 `reload_mcp_servers` 共用此函数
- stale server 检测：config 变更/移除的 server → `unregister_by_prefix` + 断开进程

### D8: 项目 MCP 审批门（安全）

> 详细方案见 [`specs/approval-gate/spec.md`](specs/approval-gate/spec.md)

**Critical 安全问题**：当前 `builder.rs` 第 395-416 行直接 push + connect 项目配置中的 MCP server，零审批 = RCE 向量。

**威胁**：恶意仓库通过 `.xiaolin/mcp.json` 注入 `{"command": "curl ... | bash"}` → 用户 clone 并打开 → 自动执行。

**三态模型**：`Pending → Approved / Rejected`

**核心决策**：

1. **用户级存储**（非项目级）— 审批状态存在 `~/.config/xiaolin/project_mcp_approvals.json`，防止仓库自批准
2. **Workspace 绑定** — key = `{workspace_root}::{server_id}`，跨项目不传播
3. **命令预览** — 用户审批前可见完整 command + args
4. **默认拒绝** — 未知 = pending = 不连接
5. **可撤销** — PluginsView 随时禁用/拒绝

**P0 范围**：逐个审批，不做内容哈希检测
**P2 可选**：command hash 变更检测（command 被修改后重新审批）、批量信任开关

### D9: 启动超时 + 逐 server 状态事件

**启动超时**：`startup_timeout_sec` per-server 配置，默认 30s（对标 Codex）。超时 → failed + 提示修改 timeout。

**逐 server 状态事件**：并行启动时推送 `McpStartupUpdate { server_name, status: Starting | Ready | Failed }`，PluginsView 逐行更新动画。

### D10: MCP 工具接入 deferred 管线

> 详细方案见 [`specs/deferred-pipeline/spec.md`](specs/deferred-pipeline/spec.md)

**关键发现**：XiaoLin 已有完整的 deferred loading 基础设施：

- `ToolRegistry::register_deferred()` — 注册到 deferred set
- `ToolRegistry::activate_deferred()` — 提升为 eager
- `ToolRegistry::search_deferred()` — BM25 搜索（name + description + search_hint）
- `ToolSearchTool` — LLM 可用的搜索/激活入口
- `PromptEngine` system_section — 自动注入 `<deferred_tools>` 标签和 tool_search 指引

但 MCP 工具全部走了 `register()`（eager）。同时 `inject_mcp_tools_prompt` 在 system prompt 中**再注入一遍**所有 MCP 工具描述 → **双重 token 浪费**。

**决策**：5 项变更

1. **`register_mcp_tools` 增加 `deferred` 参数** — 调用方按阈值决定 eager/deferred
2. **`McpToolBridge` 增加 `search_hint`** — 包含 server_id + 原始 tool name，提升 BM25 搜索质量
3. **Gateway 阈值决策** — MCP 工具总数 > 100 时触发 deferred（`alwaysLoad` 工具除外）
4. **`inject_mcp_tools_prompt` 条件化** — deferred 模式下仅注入 eager 子集（或完全跳过）
5. **`McpTool` 扩展 `_meta` 字段** — 支持 `anthropic/alwaysLoad` 元数据（当前 struct 缺失此字段）

## 数据流

### 添加 MCP 服务器流程

```
用户点击「添加」
  → PluginsView AddModal
    → api.addMcpServer(id, config)    // WS: mcp.add
      → Gateway: server 名校验（[a-zA-Z0-9_-]，禁止 __）
      → persist_config_key("mcpServers", ...)
      → connect_mcp_server(cfg, registry, prefix)
        → normalize name → McpClient::connect_{stdio|sse}
        → initialize (protocolVersion: 2025-06-18) + tools/list
        → McpToolBridge → ToolRegistry (register / register_deferred)
      → mcp_status 更新
      → plugins.status_changed 事件推送
    → PluginsView 刷新列表
```

### 项目级配置加载流程（增加审批门）

```
Gateway 启动 / reload_mcp_servers()
  → detect_workspace_root(cwd)
  → load_project_mcp_config(ws_root)
    → .xiaolin/mcp.json (优先) / .cursor/mcp.json (兼容)
  → to_mcp_server_configs()
  → 对每个 project server 检查审批状态
    → approved → 合并到 all_mcp_servers
    → pending → 跳过，推送 pending 状态到前端
    → rejected → 跳过
  → connect_mcp_server (批次限制: stdio 3, remote 20)
```

## 文件影响清单

| 文件 | 变更 |
|------|------|
| `crates/xiaolin-app/src/components/settings/McpManager.tsx` | **删除** |
| `crates/xiaolin-app/src/components/settings/SkillsTab.tsx` | **删除**（迁移到 PluginsView Skills tab） |
| `crates/xiaolin-app/src/components/settings/SettingsPanel.tsx` | 移除 Skills tab 引用 |
| `crates/xiaolin-app/src/components/plugins/PluginsView.tsx` | 三 Tab 布局：MCP/Skills/Channels + 添加/详情/重载/审批/上传 |
| `crates/xiaolin-app/src/components/connections/ConnectionsPage.tsx` | **删除**（功能合并到 PluginsView） |
| `crates/xiaolin-app/src/components/message-stream/StepIndicator.tsx` | 修复 getMcpMeta 解析（split `__`） |
| `crates/xiaolin-app/src/lib/stores/plugin-store.ts` | 新增 addPlugin/removePlugin/approveProject |
| `crates/xiaolin-mcp/src/lib.rs` | prefix `__`、normalize、notification dispatch、reconnect、deferred 注册 |
| `crates/xiaolin-gateway/src/state/mod.rs` | 启动路径 SSE/HTTP、批次连接、审批检查 |
| `crates/xiaolin-gateway/src/state/builder.rs` | 同上 |
| `crates/xiaolin-gateway/src/ws/mcp.rs` | add/remove/reload 改用统一 connect 函数 |
| `crates/xiaolin-gateway/src/ws/plugins.rs` | 新增 approve_project_mcp / reject_project_mcp handler |
| `crates/xiaolin-gateway/src/mcp_tool.rs` | 复用统一 connect 函数 |
| `crates/xiaolin-gateway/src/chat_pipeline.rs` | 工具名前缀更新、deferred 时跳过 inject |
| `crates/xiaolin-core/src/tool.rs` | mcp_definitions() 前缀更新 |
| `crates/xiaolin-core/src/agent_config.rs` | 权限 glob `mcp__*`、审批状态配置 |

## Open Questions

1. **Streamable HTTP 时机**：P2 还是 P1？新版 MCP server（GitHub Copilot MCP 等）可能只支持 Streamable HTTP。
2. **Elicitation 最小路径**：是否在 P2 补一个「空 schema 审批 fallback」？可覆盖 80% MCP tool approval 场景，代价很小。
3. **per-server enabled_tools/disabled_tools**：Codex 有完整实现，是否在 P2 加入？
