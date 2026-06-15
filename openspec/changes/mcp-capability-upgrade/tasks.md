# MCP 能力升级 — 任务清单

> 基于 6 个详细 spec + Codex/Claude Code 交叉验证（2026-06-15 深度三方对比更新）。
> 每个任务标注对应 spec、前置依赖和完成状态。

## 阶段一：修 Bug + 安全基线 (P0)

> 目标：修复阻塞性 Bug、消除安全漏洞、清理死代码。零新功能。
> **进度**：7/10 完成，1/10 部分完成，2/10 未开始

### T1: 删除死代码 ✅

**Spec**: [`plugins-ui/spec.md`](specs/plugins-ui/spec.md) 变更 13
**前置**: 无
**状态**: ✅ 已完成（P1 阶段一并处理）
**文件**:
- `settings/McpManager.tsx` → ✅ 已删除
- `connections/ConnectionsPage.tsx` → ✅ 已删除（P1 迁移后）
- `settings/SkillsTab.tsx` → ✅ 已删除（P1 迁移后）

---

### T2: 工具命名规范化 — Rust 端 ✅

**Spec**: [`naming-pipeline/spec.md`](specs/naming-pipeline/spec.md)
**前置**: 无
**状态**: ✅ 已完成（全链路 `mcp__` 一致）

**已完成**:
- ✅ `xiaolin-mcp/src/naming.rs` — `sanitize_for_api`、`mcp_server_prefix`、`mcp_tool_name`、`parse_mcp_tool_name`、`is_mcp_tool`
- ✅ `xiaolin-mcp/src/lib.rs` — `register_mcp_tools` 使用 `naming::mcp_server_prefix()` + doc comment 更新
- ✅ gateway `state/mod.rs` — 启动/热重载使用 `mcp__` 前缀
- ✅ `xiaolin-gateway/src/chat_pipeline.rs` — `inject_mcp_tools_prompt` 改用 `naming::parse_mcp_tool_name`，system prompt example 改为 `mcp__serverId__toolName`
- ✅ `xiaolin-core/src/tool.rs` — `mcp_definitions()` 前缀匹配改为 `mcp__`
- ✅ `xiaolin-core/src/agent_config.rs` — 权限 glob `mcp_*` → `mcp__*`，测试全部更新
- ✅ `xiaolin-agent/src/subagent.rs` — `starts_with("mcp_")` → `starts_with("mcp__")`
- ✅ `xiaolin-agent/src/runtime/tool_executor.rs` — `starts_with("mcp_")` → `starts_with("mcp__")`，`COMPACTABLE_TOOLS` 更新

**对标 Codex 额外能力（可选）**:
- Codex 有 hash 去重（namespace 冲突追加 SHA1 12字符 suffix）和 64 字节截断
- 当前 XiaoLin 无此需求，但工具数增多后可能需要

**验证**: ✅ `cargo check` + `cargo test -p xiaolin-core --lib agent_config` + `cargo test -p xiaolin-mcp --lib naming` + `npx tsc --noEmit` 全部通过

---

### T3: 工具命名规范化 — 前端 ⚠️ 部分完成

**Spec**: [`naming-pipeline/spec.md`](specs/naming-pipeline/spec.md)
**前置**: T2
**状态**: ⚠️ prefix 字符串更新完成，工具函数待创建

**已完成**（随 T2 一并处理）:
- ✅ `components/message-stream/StepIndicator.tsx` → `getToolCategory` 中 `name.startsWith("mcp_")` → `name.startsWith("mcp__")`
- ✅ `components/message-stream/__tests__/ToolCallCard.test.tsx` → mock 工具名 `mcp_github_list_repos` → `mcp__github__list_repos`

**剩余**:
- ❌ `lib/mcpNaming.ts` → **新建**：`sanitizeForApi`、`mcpServerPrefix`、`parseMcpToolName`、`isMcpTool` 前端工具函数
- ❌ `components/message-stream/ToolCallCard.tsx` → `getMcpMeta` 改用 `parseMcpToolName`（当前仍按第一个 `_` 分割）

**验证**: 前端编译通过 + tool call 卡片正确显示 MCP server 名 + 测试通过

---

### T4: Transport 枚举化 + 统一连接入口 ✅

**Spec**: [`transport-fix/spec.md`](specs/transport-fix/spec.md) 变更 1-2
**前置**: T2 ✅（naming 依赖已满足）
**状态**: ✅ 已完成

**已完成**:
- ✅ `xiaolin-core/src/agent_config.rs` → `McpTransportType` 枚举（`Stdio | Sse | StreamableHttp | Http`），`#[serde(alias = "http")]` 兼容，`effective()` 归一化 Http→StreamableHttp
- ✅ `McpServerConfig::validate()` — id 非空、不含 `__`、stdio 需 command、sse/http 需 url
- ✅ `McpServerConfig.transport` 从 `String` 改为 `McpTransportType`
- ✅ `ProjectMcpServerEntry.transport` 同步更新
- ✅ `xiaolin-mcp/src/lib.rs` → `connect_mcp_server(cfg, registry)` 统一入口，prefix 内部派生
- ✅ 10 个新增单测（serde 往返、http alias、validate 路径、legacy 字符串兼容）

**验证**: ✅ `cargo check` + `cargo clippy -- -D warnings` 零警告 + 32 agent_config 测试通过 + E2E 验证

---

### T5: 消除三套重载逻辑 — 统一调用 `connect_mcp_server` ✅

**Spec**: [`transport-fix/spec.md`](specs/transport-fix/spec.md) 变更 3-5
**前置**: T4 ✅
**状态**: ✅ 已完成

**已完成**:
- ✅ `xiaolin-gateway/src/state/mod.rs` → 启动路径 `register_mcp_and_subagent_tools` 改用 `connect_mcp_server`
- ✅ `xiaolin-gateway/src/state/mod.rs` → 热重载路径 `reload_mcp_servers` 改用 `connect_mcp_server`
- ✅ `xiaolin-gateway/src/mcp_tool.rs` → `do_reload` 改用 `connect_mcp_server`
- ✅ `xiaolin-gateway/src/ws/mcp.rs` → `handle_mcp_add` 支持 transport/url 参数 + `McpServerConfig::validate()`
- ✅ `xiaolin-protocol/src/op.rs` → `McpAddParams` 新增 `transport` 和 `url` 字段

**Bug 修复**: ✅ `do_reload` 中 streamable_http 被错误路由到 stdio → 统一后自动修复
**Bug 修复**: ✅ `handle_mcp_add` 只能添加 stdio server → 支持 transport 参数
**验证**: ✅ E2E 验证三条路径均正确连接 + clippy 零警告 + 34 gateway 测试通过

---

### T6: Notification Dispatch 改造

**Spec**: [`notification-dispatch/spec.md`](specs/notification-dispatch/spec.md) 变更 1-4
**前置**: T4
**文件**:
- `xiaolin-mcp/src/lib.rs`:
  - 新增 `McpNotification` struct + `broadcast::channel`
  - 重构 `stdio_reader_loop`：`serde_json::Value` 先解析，按 `id` 有无区分 Response/Notification
  - 重构 `sse_reader_loop`：同步增加 notification dispatch
  - 新增 `McpClient::subscribe_notifications()` 公开 API
  - 新增 `McpClient::list_tools()` 强制刷新（区别于缓存版 `tools()`）

**验证**: 连接 MCP server → 触发 `tools/list_changed` → notification 不被丢弃 → 日志可见

---

### T7: stderr 捕获 ✅

**Spec**: [`notification-dispatch/spec.md`](specs/notification-dispatch/spec.md) 变更 5
**前置**: 无
**状态**: ✅ 已完成
**文件**:
- ✅ `xiaolin-mcp/src/lib.rs` → `stderr_reader_loop` → `tracing::warn!`

**验证**: ✅ MCP 子进程输出 stderr → gateway 日志中可见

---

### T8: 项目 MCP 审批门 — 后端

**Spec**: [`approval-gate/spec.md`](specs/approval-gate/spec.md)
**前置**: T4, T5
**文件**:
- `xiaolin-core/src/project_mcp_approval.rs` → **新建**：`ProjectMcpApprovals`、`approval_key`、`get_approval`、`set_approval`、`load_approvals`
- `xiaolin-core/src/agent_config.rs` → `ProjectMcpApproval` 枚举（`Pending | Approved | Rejected`）
- `xiaolin-gateway/src/state/builder.rs` → 项目 MCP 加载时检查审批状态（approved → connect，pending/rejected → skip + 推送状态）
- `xiaolin-gateway/src/ws/plugins.rs` → 新增 `handle_approve_project_mcp`、`handle_reject_project_mcp` handler

**验证**: 项目配置中有 MCP server → 首次启动不连接 → 调用 approve API → 连接成功

---

### T9: 升级 MCP 协议版本 ✅

**Spec**: [`transport-fix/spec.md`](specs/transport-fix/spec.md) 变更 6 (隐含)
**前置**: T4
**状态**: ✅ 已完成
**文件**:
- ✅ `xiaolin-mcp/src/lib.rs` → `protocolVersion: "2025-06-18"`

**验证**: ✅ MCP initialize 握手成功

---

### T10: 配置验证

**Spec**: [`transport-fix/spec.md`](specs/transport-fix/spec.md) 变更 6
**前置**: T4
**文件**:
- `xiaolin-core/src/agent_config.rs` → `McpServerConfig::validate()` — stdio 需 command、sse 需 url、id 不含 `__`
- `xiaolin-gateway/src/state/mod.rs` → 连接前调用 validate
- `xiaolin-gateway/src/ws/plugins.rs` → `plugins.add` handler 调用 validate

**验证**: 无效配置（缺 command、id 含 `__`）→ 清晰错误消息

---

## 阶段二：PluginsView 三 Tab 整合 (P1)

> 目标：PluginsView 成为 MCP + Skills + Channels 的统一管理入口。
> **进度**：4/9 完成（骨架 + 迁移完成，MCP CRUD/详情/审批 UI 待做，新增 UI 风格统一任务）

### T10.5: PluginsView UI 风格统一（与主界面对齐）

**Spec**: [`plugins-ui-alignment/spec.md`](specs/plugins-ui-alignment/spec.md)
**前置**: T11 ✅
**状态**: 未开始

8 项设计不一致修复：
1. Icon 尺寸 → 引用 `ICON_SIZE` token
2. 字号收敛 → 去掉 `text-[10px]`，合并到 3-4 档
3. Header 轻量化 → 对齐主界面 flat 风格
4. Tab Bar → 抽取 `<SegmentedControl>` 共享组件
5. Button 样式 → 使用 `BTN_ICON` / `BTN_TEXT_SM` token
6. 内容宽度 → 使用 `--content-max-w` CSS variable
7. 动画 → 删除 `ANIM_CSS` 注入，迁移到 `index.css` 全局 keyframes
8. 国际化 → 新增 `plugins.json` 翻译文件，`useTranslation("plugins")`

**文件**:
- `components/plugins/PluginsView.tsx` — 主体重构
- `lib/ui-tokens.ts` — 新增 `BTN_TEXT_SM` / `BTN_PRIMARY_SM`
- `components/common/SegmentedControl.tsx` — 新增共享组件
- `index.css` — 迁移动画 keyframes
- `locales/{zh,en}/plugins.json` — 新增翻译

**验证**: 零 `size={N}` 硬编码、零 `<style>` 注入、零 inline button style、i18n 100% 覆盖

---

### T11: PluginsView 三 Tab 骨架 ✅

**Spec**: [`plugins-ui/spec.md`](specs/plugins-ui/spec.md) 变更 1
**前置**: T1
**文件**:
- `plugins/PluginsView.tsx` → 重构为 tab 布局：`[MCP Servers] [Skills] [Channels]`
  - 新增 `type PluginsTab = "mcp" | "skills" | "channels"`
  - Header 标题改为 "Plugins" + 副标题 "Extend capabilities with MCP servers, skills & channels"
  - Tab Bar：pill-style 切换器，每个 tab 显示数量 badge
  - Action 按钮区域随 tab 切换（MCP: +Add/↻Reload, Skills: ↑Upload/↻Refresh, Channels: ↻Refresh）
  - 现有 MCP 列表代码移入 `McpTabContent` 组件
  - 现有 `PluginRow` / `StatusDot` / `ScopeBadge` / `DetailRow` / `EmptyState` 保留不动
  - 新增空壳 `SkillsTabContent` 和 `ChannelsTabContent`（T16/T17 填充）
- `lib/stores/ui-store.ts` → 无变更（已有 mainView = "plugins"）

**实施细节**:
1. 保持现有动画 CSS（`ANIM_CSS`）不变
2. Tab 切换时内容区域使用 `pvFadeIn` 动画
3. Header 连接状态 badge 仅在 MCP tab 时显示
4. Footer "Add MCP servers in Settings" 移除

**状态**: ✅ 已完成 + E2E 验证通过

**验证**: ✅ 三个 tab 切换正常，MCP tab（2 servers）、Skills tab（190 skills）、Channels tab（2 channels）

---

### T12: MCP Tab — 添加/删除/重载

**Spec**: [`plugins-ui/spec.md`](specs/plugins-ui/spec.md) 变更 1b, 4, 6, 7
**前置**: T4, T11
**文件**:
- `plugins/PluginsView.tsx` → Header 增加 `+ Add Server` + `↻ Reload All`
- `plugins/PluginsView.tsx` → **新增** `AddPluginModal`（从 ConnectionsPage 迁移 + 增强：transport 选择、env vars）
- `lib/stores/plugin-store.ts` → 新增 `addPlugin`、`removePlugin`、`reloadAll` actions
- `lib/transport.ts` → 新增 `addPlugin()`、`removePlugin()`、`reloadAllPlugins()` API
- `xiaolin-gateway/src/ws/plugins.rs` → 新增 `plugins.add`、`plugins.remove`、`plugins.reload_all` handler

**验证**: 从 PluginsView 添加 stdio/SSE server → 连接成功 → 删除 → 从列表消失

---

### T13: MCP Tab — PluginSummary 扩展 + 分组

**Spec**: [`plugins-ui/spec.md`](specs/plugins-ui/spec.md) 变更 2, 10
**前置**: T11
**文件**:
- `lib/transport.ts` → `PluginSummary` 增加 `transport`、`commandPreview`、`pendingApproval` 字段
- `plugins/PluginsView.tsx` → MCP 列表按 scope（User/Project）分组
- `xiaolin-gateway/src/ws/plugins.rs` → `plugins.list` 响应增加新字段

**验证**: User / Project 分组正确显示，transport 类型可见

---

### T14: MCP Tab — 审批 UI

**Spec**: [`plugins-ui/spec.md`](specs/plugins-ui/spec.md) 变更 3 + [`approval-gate/spec.md`](specs/approval-gate/spec.md)
**前置**: T8, T11, T13
**文件**:
- `plugins/PluginsView.tsx` → **新增** `PendingApprovalSection`（审批卡片：命令预览 + 批准/拒绝按钮）
- `lib/stores/plugin-store.ts` → 新增 `approveProjectMcp`、`rejectProjectMcp` actions
- `lib/transport.ts` → 新增 `approveProjectMcp()`、`rejectProjectMcp()` API

**验证**: 项目级 MCP server → 显示为 pending → 批准后连接 → 拒绝后消失

---

### T15: MCP Tab — PluginDetailModal

**Spec**: [`plugins-ui/spec.md`](specs/plugins-ui/spec.md) 变更 5
**前置**: T11
**文件**:
- `plugins/PluginsView.tsx` → **新增** `PluginDetailModal`（配置预览、工具搜索、错误日志）

**验证**: 点击 plugin → 弹出详情 → 工具列表 + 配置可见

---

### T16: Skills Tab ✅

**Spec**: [`plugins-ui/spec.md`](specs/plugins-ui/spec.md) 变更 11
**前置**: T11
**状态**: ✅ 已完成
**文件**:
- `plugins/PluginsView.tsx` → 填充 `SkillsTabContent`（从 `settings/SkillsTab.tsx` 迁移逻辑）
  - Skills / Tools pill 切换器
  - Global skills 列表（`Globe` 图标 + 分组标题）
  - Agent-specific skills 列表（`User` 图标 + 分组标题）
  - 每个 skill 显示 name, version, description, tags
  - Upload 菜单（Folder + ZIP 选项）→ 调用 `api.uploadSkill()` + `api.refreshSkills()`
  - Refresh 按钮 → 调用 `api.refreshSkills()`
  - 数据源：`api.listSkills()`, `api.listSkills("main")`, `api.listTools()`
- `settings/SettingsPanel.tsx` → 移除 `SkillsTab` lazy import 和渲染（L9, L106）
  - 从 `tabs` 数组移除 `{ id: "skills", ... }`
  - 从 `SettingsTab` type 移除 `"skills"`
- `settings/SkillsTab.tsx` → **删除整个文件**

**注意**:
- 原 SkillsTab 依赖 `SectionTitle` from `SettingsShared.tsx` → 替换为 PluginsView 内联样式
- 原 SkillsTab 依赖 `useGatewayStore` 判断 gateway ready → 保留此逻辑
- 原 SkillsTab 使用 `settings` i18n namespace → 需确保 key 仍可用或改用内联文本

**验证**: ✅ Skills tab 显示 190 skills + 上传/刷新功能正常 + Settings 中不再有 Skills tab + `npx tsc --noEmit` 通过

---

### T17: Channels Tab ✅

**Spec**: [`plugins-ui/spec.md`](specs/plugins-ui/spec.md) 变更 12
**前置**: T11
**状态**: ✅ 已完成
**文件**:
- `plugins/PluginsView.tsx` → ✅ 填充 `ChannelsTabContent`（从 `connections/ConnectionsPage.tsx` 迁移）
  - 迁移 `ChannelCard`（L155-248）：状态、能力标签、连接/断开按钮
  - 迁移 `WechatQrModal`（L252-527）：完整的扫码登录流程（idle → loading → scanning → scanned → verify_code → confirmed）
  - 迁移 `ChannelDetailModal`（L794-1190）：配置查看/编辑、工具列表、连接操作
  - 迁移 `STATUS_CONFIG` + `CAP_LABELS` 常量
  - Channel 数据加载：`api.listChannels()`，事件订阅：`transport.onChannelsChanged()`
  - Channel 操作：connect → WeChat 走 QR 流程，其他走 `api.channelsConnect()`；disconnect → `api.channelsDisconnect()`
- `connections/ConnectionsPage.tsx` → **删除整个文件**

**注意**:
- WechatQrModal 的 `pollRef.current` 需在组件卸载 / tab 切换时正确 cleanup
- ChannelDetailModal 的编辑功能调用 `api.channelsUpdate()` + `api.channelsRestore()`
- ConnectionsPage 使用 `common` i18n namespace → 需确保 key 仍可用
- `EDITABLE_CONFIG_KEYS` 常量需一并迁移

**验证**: ✅ Channels tab 正常显示 2 channels + `npx tsc --noEmit` 通过 + ConnectionsPage.tsx 已删除无引用

---

### T18: EmptyState 更新 ✅

**Spec**: [`plugins-ui/spec.md`](specs/plugins-ui/spec.md) 变更 9, 13
**前置**: T11
**状态**: ✅ 已完成
**文件**:
- ✅ `plugins/PluginsView.tsx` → 每个 tab 独立空状态，Settings 中 Skills tab 已移除

**验证**: ✅ 空列表时显示正确引导文案 + Settings 无 Skills tab

---

## 阶段三：后端能力增强 (P2)

> 目标：连接管理健壮性、性能优化、动态更新。
> **关键洞察**：Codex 的 `tools/list_changed` 也只 log 不处理，XiaoLin 做好 T19 即超越 Codex。

### T19: tools/list_changed 处理

**Spec**: [`notification-dispatch/spec.md`](specs/notification-dispatch/spec.md)（Gateway 订阅部分）
**前置**: T6
**文件**:
- `xiaolin-gateway/src/state/mod.rs` → 连接后 `subscribe_notifications()`，处理 `tools/list_changed`：
  - 调用 `list_tools()` 刷新
  - `unregister_by_prefix(prefix)` + 重新 register
  - 推送 `plugins.status_changed` 事件

**验证**: MCP server 动态添加工具 → XiaoLin 自动感知 + 工具列表更新

---

### T20: 自动重连（仅 SSE/HTTP）

**Spec**: 无独立 spec，对应 D5
**前置**: T4
**文件**:
- `xiaolin-mcp/src/lib.rs` → SSE 连接断开时启动重连：指数退避 `min(1000×2^(n-1), 30000)ms`，最多 5 次
- `xiaolin-gateway/src/state/mod.rs` → disable 时取消 in-flight 重连定时器

**验证**: SSE server 断开 → 自动重连 → 5 次失败后停止

---

### T21: 连接批次限制

**Spec**: 无独立 spec，对应 D7
**前置**: T4
**文件**:
- `xiaolin-gateway/src/state/mod.rs` → 启动/重载时使用 `tokio::sync::Semaphore`：stdio 并发 3，remote 并发 20

**验证**: 10 个 stdio server → 同时最多 3 个在连接

---

### T22: 启动超时

**Spec**: 无独立 spec，对应 D9
**前置**: T4
**文件**:
- `xiaolin-core/src/agent_config.rs` → `McpServerConfig` 增加 `startup_timeout_sec`（默认 30s）
- `xiaolin-mcp/src/lib.rs` → `connect_mcp_server` 中 `tokio::time::timeout` 包裹

**验证**: 超时的 MCP server → failed + 可读错误消息

---

### T23: stale server 清理

**Spec**: 无独立 spec，对应 D7
**前置**: T5
**文件**:
- `xiaolin-gateway/src/state/mod.rs` → `reload_mcp_servers` 中检测 config 移除/变更的 server → `unregister_by_prefix` + kill 进程

**验证**: 从配置删除 MCP server → 重载后工具自动注销

---

### T24: Description 截断保护

**Spec**: 无独立 spec
**前置**: T2
**文件**:
- `xiaolin-mcp/src/lib.rs` → `register_mcp_tools` 中截断 description ≤ 2048 字符
- `xiaolin-gateway/src/chat_pipeline.rs` → `inject_mcp_tools_prompt` 中同步截断

**验证**: 超长 description 的工具 → 截断后注册 + prompt 注入不溢出

---

### T25: Session 级 Schema 缓存

**Spec**: 无独立 spec
**前置**: T19
**文件**:
- `xiaolin-gateway/src/chat_pipeline.rs` → 缓存序列化后的 tool schema JSON 字节
- `tools/list_changed` 时按 prefix 局部 invalidate

**验证**: 重复调用时 schema 不重复序列化

---

### T26: 逐 server 启动状态事件

**Spec**: 无独立 spec，对应 D9
**前置**: T4
**文件**:
- `xiaolin-gateway/src/state/mod.rs` → 并行启动时推送 `McpStartupUpdate { server_name, status }`
- `lib/transport.ts` → 订阅 startup 事件
- `plugins/PluginsView.tsx` → MCP tab 逐行状态动画

**验证**: 启动时 PluginsView 每个 server 逐个从 connecting → connected/failed

---

## 阶段四：智能工具注入 (P3)

> 目标：大量 MCP 工具时的 token 优化。

### T27: MCP 工具接入 deferred 管线

**Spec**: [`deferred-pipeline/spec.md`](specs/deferred-pipeline/spec.md) 变更 1-2
**前置**: T2, T6
**文件**:
- `xiaolin-mcp/src/lib.rs` → `register_mcp_tools` 增加 `deferred: bool` 参数
- `xiaolin-mcp/src/lib.rs` → `McpToolBridge` 增加 `search_hint`（server_id + 原始 tool name）

**验证**: deferred=true 时，工具注册到 deferred set → tool_search 可搜索到

---

### T28: Deferred 时跳过 system prompt 注入

**Spec**: [`deferred-pipeline/spec.md`](specs/deferred-pipeline/spec.md) 变更 4
**前置**: T27
**文件**:
- `xiaolin-gateway/src/chat_pipeline.rs` → `inject_mcp_tools_prompt` 在 deferred 模式下跳过（或仅注入 eager 子集）

**验证**: deferred 模式 → system prompt 中无大量 MCP 工具描述 → token 显著减少

---

### T29: 阈值策略（更新：对标 Claude Code）

**Spec**: [`deferred-pipeline/spec.md`](specs/deferred-pipeline/spec.md) 变更 3
**前置**: T27
**文件**:
- `xiaolin-gateway/src/state/mod.rs` → 阈值判断：
  - 工具总数 > 100（对标 Codex `DIRECT_MCP_TOOL_EXPOSURE_THRESHOLD`）
  - **或** MCP 工具 description token 总量 > context window 的 10%（对标 Claude Code `getAutoToolSearchTokenThreshold`）
  - 考虑增加"默认 defer"模式（Claude Code 的 `tst` 模式，所有 MCP 工具默认 defer）

**验证**: 注册 > 100 个 MCP 工具 → 自动切换到 deferred

---

### T30: alwaysLoad 元数据支持

**Spec**: [`deferred-pipeline/spec.md`](specs/deferred-pipeline/spec.md) 变更 5
**前置**: T27
**文件**:
- `xiaolin-mcp/src/lib.rs` → `McpTool` struct 增加 `meta` 字段（解析 `_meta`）
- `xiaolin-mcp/src/lib.rs` → `register_mcp_tools` 中检查 `_meta.anthropic/alwaysLoad` → 强制 eager

**验证**: 带 `alwaysLoad` 的工具在 deferred 模式下仍然 eager 注册

---

### T31: Schema 完整性

**Spec**: [`deferred-pipeline/spec.md`](specs/deferred-pipeline/spec.md) 变更隐含
**前置**: T27
**文件**:
- `xiaolin-mcp/src/lib.rs` → `McpToolBridge::parameters_schema` 保留完整 JSON Schema（嵌套、oneOf、additionalProperties）

**验证**: 复杂 schema 的 MCP 工具 → parameters_schema 无信息丢失

---

### T32: Server Instructions Delta 注入（新增）

**Spec**: 无独立 spec（对标 Claude Code `getMcpInstructionsDelta`）
**前置**: T6
**文件**:
- `xiaolin-gateway/src/chat_pipeline.rs` → MCP server 的 `InitializeResult.instructions` 以 delta 方式增量注入对话历史，而非每轮重建 system prompt
- 避免 instructions 变化导致 prompt cache 失效

**背景**: Claude Code 通过 `getMcpInstructionsDelta()` 对比已公告的 server 和当前连接的 server，仅发送增量 `addedBlocks`/`removedNames`，保持 system prompt 稳定以最大化 prompt cache 命中率。

**验证**: MCP server 连接/断开后，system prompt 主体不变，instructions 通过 delta message 注入

---

### T33: 配置签名去重（新增）

**Spec**: 无独立 spec（对标 Claude Code `getMcpServerSignature`）
**前置**: T4
**文件**:
- `xiaolin-gateway/src/state/mod.rs` → 配置合并时计算签名：stdio → `stdio:{json(command+args)}`，HTTP → `url:{url}`
- 相同签名的 server 只连接一次（plugin + 手动配置可能重复）

**背景**: Claude Code 的 `getMcpServerSignature` 用命令数组或 URL 生成签名，`dedupPluginMcpServers` 基于签名去重。

**验证**: 手动配置和 project 配置指向同一 command → 仅连接一次

---

## 依赖关系总览

```
✅ T1 (清理)          ─── 完成
✅ T2 (命名 Rust)     ─── 完成（全链路 mcp__）
⚠️ T3 (命名 TS)       ←── T2 ✅，prefix 更新完成，工具函数待建
❌ T4 (Transport)     ←── T2 ✅  ← 【下一步】
❌ T5 (路由修复)      ←── T4
❌ T6 (Notification)  ←── T4
✅ T7 (stderr)        ─── 完成
❌ T8 (审批门)        ←── T4 + T5
✅ T9 (协议版本)      ─── 完成
❌ T10 (配置验证)     ←── T4
✅ T11 (Tab 骨架)     ─── 完成
❌ T12 (MCP 添加)     ←── T4 + T11
❌ T13 (分组)         ←── T11
❌ T14 (审批 UI)      ←── T8 + T11 + T13
❌ T15 (详情)         ←── T11
✅ T16 (Skills)       ─── 完成
✅ T17 (Channels)     ─── 完成
✅ T18 (EmptyState)   ─── 完成
❌ T19-T26 (P2)       ←── 各自前置
❌ T27-T31 (P3)       ←── T2 ✅ + T6
❌ T32 (Instructions) ←── T6（新增）
❌ T33 (签名去重)     ←── T4（新增）
```

## 建议实施顺序（ROI 优先级）

> 基于 2026-06-15 三方对比分析的推荐顺序

1. ~~**T2**（命名全链路）~~ → ✅ 已完成
2. **T4 + T5**（统一连接入口）— 消除三套重载 + 修复 streamable_http/mcp.add bug，后续所有改进的基础 ← **下一步**
3. **T6 + T19**（Notification dispatch + list_changed）— 超越 Codex 的机会（Codex 只 log 不处理）
4. **T8 + T14**（审批门）— 安全必须项（`.xiaolin/mcp.json` 任意 command 当前直接执行）
5. **T10 + T33**（配置验证 + 签名去重）— 防御性编程
6. **T3 剩余 + T12 + T15**（前端工具函数 + Add Modal + Detail Modal）— 用户体验
7. **T20-T22**（重连 + 批次 + 超时）— 连接健壮性
8. **T27-T31**（Deferred 管线）— 最大 token 收益（对标 Claude Code 默认 defer）
9. **T32**（Instructions Delta）— prompt cache 优化（对标 Claude Code 独有能力）

## Spec 覆盖对照

| Spec | 对应任务 | 完成度 |
|------|---------|:---:|
| `naming-pipeline/spec.md` | T2 ✅, T3 ⚠️ | 75% |
| `transport-fix/spec.md` | T4, T5, T9 ✅, T10 | 25% |
| `notification-dispatch/spec.md` | T6, T7 ✅, T19 | 33% |
| `approval-gate/spec.md` | T8, T14 | 0% |
| `deferred-pipeline/spec.md` | T27, T28, T29, T30, T31 | 0% |
| `plugins-ui/spec.md` | T1 ✅, T11-T18 (4✅ 4❌) | 63% |

## 整体进度

- **P0**：4/10 完成（T1 ✅, T2 ✅, T7 ✅, T9 ✅），1/10 部分完成（T3 ⚠️），5/10 未开始
- **P1**：4/8 完成（T11 ✅, T16 ✅, T17 ✅, T18 ✅），4/8 待做
- **P2**：0/8 完成 + 2 新增任务（T32, T33）
- **P3**：0/5 完成 + 2 新增任务（T32, T33 归属 P3）
- **总计**：8/33 完成 + 1 部分完成（~26%），**当前评分 ~50/100**

### 关键阻塞项（按优先级）

1. **T4+T5（统一连接入口）**：所有后续改进的基础，解除 T6/T8/T10/T12 的前置依赖
2. **T6（Notification dispatch）**：解除 T19/T27/T32 的前置依赖
3. **T8（审批门后端）**：安全风险，解除 T14 UI 前置依赖
