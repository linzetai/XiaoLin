# MCP 能力升级 — 任务清单

> 基于 6 个详细 spec + Codex/Claude Code 交叉验证（2026-06-15 深度三方对比更新）。
> 含 Batch A（T12/T12.5/T15）已完成 + UI 视觉抛光（T34-T39）已整合。
> 每个任务标注对应 spec、前置依赖和完成状态。

## 阶段一：修 Bug + 安全基线 (P0)

> 目标：修复阻塞性 Bug、消除安全漏洞、清理死代码。零新功能。
> **进度**：10/10 完成 ✅

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

### T3: 工具命名规范化 — 前端 ✅

**Spec**: [`naming-pipeline/spec.md`](specs/naming-pipeline/spec.md)
**前置**: T2
**状态**: ✅ 已完成

**已完成**:
- ✅ `components/message-stream/StepIndicator.tsx` → `getToolCategory` 中 `name.startsWith("mcp_")` → `name.startsWith("mcp__")`
- ✅ `components/message-stream/__tests__/ToolCallCard.test.tsx` → mock 工具名 `mcp_github_list_repos` → `mcp__github__list_repos`
- ✅ `lib/mcpNaming.ts` — `sanitizeForApi`、`mcpServerPrefix`、`parseMcpToolName`、`isMcpTool` 前端工具函数
- ✅ `components/message-stream/ToolCallCard.tsx` → `getMcpMeta` 使用 `parseMcpToolName`

**验证**: 前端编译通过 + tool call 卡片正确显示 MCP server 名

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

### T6: Notification Dispatch 改造 ✅

**Spec**: [`notification-dispatch/spec.md`](specs/notification-dispatch/spec.md) 变更 1-4
**前置**: T4 ✅
**状态**: ✅ 已完成

**已完成**:
- ✅ `xiaolin-mcp/src/lib.rs` → 新增 `McpNotification` struct（`method: String` + `params: Option<Value>`，`Debug + Clone`）
- ✅ `McpClient` 新增 `notification_tx: broadcast::Sender<McpNotification>`，缓冲区 64
- ✅ 重构 `stdio_reader_loop`：先解析为 `serde_json::Value`，按 `id` 字段分流 Response/Notification
- ✅ 重构 `sse_reader_loop`：同步改造，与 stdio 相同的分流逻辑
- ✅ 重构 `streamable_http_listener`：同步改造，三条路径行为一致
- ✅ 新增 `McpClient::subscribe_notifications()` → 返回 `broadcast::Receiver<McpNotification>`
- ✅ 新增 `McpClient::refresh_tools()` → 强制重新 `tools/list`（区别于缓存版 `tools()`）
- ✅ 3 个新增单测：notification_dispatch_logic、mcp_notification_clone_and_debug、subscribe_notifications_returns_receiver

**验证**: ✅ `cargo check --workspace` 零错误 + `cargo clippy -- -D warnings` 零警告 + 37 tests passed + E2E 验证 reader loop 新路径生效（`MCP stdio: unparseable JSON` 日志可见）

---

### T7: stderr 捕获 ✅

**Spec**: [`notification-dispatch/spec.md`](specs/notification-dispatch/spec.md) 变更 5
**前置**: 无
**状态**: ✅ 已完成
**文件**:
- ✅ `xiaolin-mcp/src/lib.rs` → `stderr_reader_loop` → `tracing::warn!`

**验证**: ✅ MCP 子进程输出 stderr → gateway 日志中可见

---

### T8: 项目 MCP 审批门 — 后端 ✅

**Spec**: [`approval-gate/spec.md`](specs/approval-gate/spec.md)
**前置**: T4, T5
**状态**: ✅ 已完成 + Code Review 修复 3 个问题

**已完成**:
- ✅ `xiaolin-core/src/project_mcp_approval.rs` — `ProjectMcpApprovals`、`approval_key`、`get_approval`、`set_approval`、`load_approvals`
- ✅ `xiaolin-core/src/types.rs` — `McpStatus::PendingApproval` + `McpServerStatus` 增加 `scope`、`command_preview`
- ✅ `xiaolin-gateway/src/state/mod.rs` — `resolve_project_mcp()` 共享函数（启动+热重载复用）
- ✅ `xiaolin-gateway/src/state/builder.rs` — 启动路径调用 `resolve_project_mcp`
- ✅ `xiaolin-gateway/src/ws/plugins.rs` — `handle_plugins_approve` + `handle_plugins_reject`（reject 含断开+工具注销）
- ✅ `xiaolin-protocol/src/op.rs` — `PluginsApprove` / `PluginsReject` ClientOp

**Code Review 修复**:
- ✅ R3a: 重复审批逻辑抽取为 `resolve_project_mcp` 共享函数
- ✅ R3b: reject 时显式断开 server + unregister tools
- ✅ R2b: 热重载中正确传播 project scope

**验证**: ✅ E2E: 项目 MCP → pending_approval → approve → connected → reject → 消失

---

### T9: 升级 MCP 协议版本 ✅

**Spec**: [`transport-fix/spec.md`](specs/transport-fix/spec.md) 变更 6 (隐含)
**前置**: T4
**状态**: ✅ 已完成
**文件**:
- ✅ `xiaolin-mcp/src/lib.rs` → `protocolVersion: "2025-06-18"`

**验证**: ✅ MCP initialize 握手成功

---

### T10: 配置验证 ✅

**Spec**: [`transport-fix/spec.md`](specs/transport-fix/spec.md) 变更 6
**前置**: T4
**状态**: ✅ 已完成

**已完成**:
- ✅ `xiaolin-core/src/agent_config.rs` → `McpServerConfig::validate()` — stdio 需 command、sse 需 url、id 不含 `__`
- ✅ `xiaolin-mcp/src/lib.rs` → `connect_mcp_server` 入口统一调用 `validate()`（所有路径受益）
- ✅ `xiaolin-gateway/src/mcp_tool.rs` → `do_reload` handler 调用 validate
- ✅ `xiaolin-gateway/src/ws/mcp.rs` → `handle_mcp_add` handler 调用 validate

**验证**: 无效配置（缺 command、id 含 `__`）→ 清晰错误消息

---

## 阶段二：PluginsView 三 Tab 整合 (P1)

> 目标：PluginsView 成为 MCP + Skills + Channels 的统一管理入口。
> **进度**：10/10 完成 ✅

### T10.5: PluginsView UI 风格统一（与主界面对齐）✅

**Spec**: [`plugins-ui-alignment/spec.md`](specs/plugins-ui-alignment/spec.md)
**前置**: T11 ✅
**状态**: ✅ 已完成

**已完成**（8 项设计不一致修复）:
- ✅ Icon 尺寸 → 全部引用 `ICON_SIZE` token（xs/sm/md/lg/xl/2xl）
- ✅ 字号收敛 → 去掉 `text-[10px]`，合并到 11px/12px/13px/14px/16px 五档
- ✅ Header 轻量化 → flat 风格（PuzzlePiece icon + h1 标题），去掉 hero icon block
- ✅ Tab Bar → 抽取 `<SegmentedControl>` 共享组件
- ✅ Button 样式 → 使用 `BTN_TEXT_SM` / `BTN_PRIMARY_SM` token
- ✅ 内容宽度 → 使用 `--content-max-w` CSS variable
- ✅ 动画 → 删除 `ANIM_CSS` 注入，迁移到 `index.css` 全局 keyframes（pv-fade-in, pv-stagger）
- ✅ 国际化 → 新增 `plugins.json` 翻译文件（zh/en），`useTranslation("plugins")`

**文件**:
- ✅ `components/plugins/PluginsView.tsx` — 主体重构
- ✅ `lib/ui-tokens.ts` — 新增 `BTN_TEXT_SM` / `BTN_PRIMARY_SM`
- ✅ `components/common/SegmentedControl.tsx` — 新增共享组件
- ✅ `index.css` — 迁移动画 keyframes + `--content-max-w` CSS variable
- ✅ `locales/{zh,en}/plugins.json` — 新增翻译

**验证**: ✅ 零 `size={N}` 硬编码、零 `<style>` 注入、i18n 覆盖 + E2E 视觉验证通过

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

### T12: MCP Tab — 添加/删除 + AddServerModal（参考 Codex）✅

**Spec**: [`mcp-add-modal/spec.md`](../mcp-marketplace-ui/specs/mcp-add-modal/spec.md) + [`plugin-store/spec.md`](../mcp-marketplace-ui/specs/plugin-store/spec.md) + [`plugin-panel/spec.md`](../mcp-marketplace-ui/specs/plugin-panel/spec.md)
**前置**: T4 ✅, T11 ✅
**设计**: [`mcp-marketplace-ui/design.md`](../mcp-marketplace-ui/design.md) D5-D6
**状态**: ✅ 已完成

**已完成**:
- ✅ 12.1 扩展 `transport.addMcpServer` 签名为对象参数 `AddMcpServerParams`
- ✅ 12.2 在 `plugin-store.ts` 新增 `addPlugin(params)` 和 `removePlugin(id)` actions
- ✅ 12.3 创建 `AddServerModal.tsx`：transport 选择器（Stdio/SSE/StreamableHTTP）+ 动态表单 + ID 验证 + env 键值对编辑器
- ✅ 12.4 PluginsView Header 增加 "+ Add" 按钮
- ✅ 12.5 PluginRow hover 时显示删除图标（确认 → `removePlugin`）
- ✅ 后端 `handle_mcp_add` 修复：正确传递 `params.env`（非 `Default::default()`）
- ✅ `McpAddParams` 增加 `env: HashMap<String, String>` 字段

**验证**: ✅ E2E 验证通过 + tsc 编译零错误

---

### T12.5: MCP Tab — Explore 面板（参考 Codex Plugin Directory）✅

**Spec**: [`mcp-explore/spec.md`](../mcp-marketplace-ui/specs/mcp-explore/spec.md)
**前置**: T12 ✅
**设计**: [`mcp-marketplace-ui/design.md`](../mcp-marketplace-ui/design.md) D1-D2, D4
**状态**: ✅ 已完成

**已完成**:
- ✅ 12.5.1 创建 `mcp-registry.json`：15 个热门 MCP Server
- ✅ 12.5.2 创建 `McpExplorePanel.tsx`：搜索 + 分类筛选 + 卡片列表 + ICON_MAP
- ✅ 12.5.3 一键安装 + loading/成功/失败反馈
- ✅ 12.5.4 已安装检测 + "已安装" badge
- ✅ 12.5.5 Installed/Explore 子切换
- ✅ 12.5.6 空状态 CTA → 切换到 Explore
- ✅ Code Review 修复：`ICON_MAP` 类型修正为 `Icon`、分类标签 i18n 化

**验证**: ✅ E2E 验证通过 + i18n 中英文正确渲染

---

### T13: MCP Tab — PluginSummary 扩展 + 分组 ✅

**Spec**: [`plugins-ui/spec.md`](specs/plugins-ui/spec.md) 变更 2, 10
**前置**: T11
**状态**: ✅ 已完成

**已完成**:
- ✅ `xiaolin-core/src/types.rs` → `McpServerStatus` 增加 `transport: Option<String>` 字段
- ✅ `xiaolin-gateway/src/state/mod.rs` → 启动/热重载/单server添加路径均传递 transport 信息
- ✅ `xiaolin-gateway/src/ws/plugins.rs` → `enrich_status` 输出 transport 字段
- ✅ `lib/transport.ts` → `PluginSummary` 增加 `transport` 字段
- ✅ `plugins/PluginsView.tsx` → MCP 列表按 scope（User/Project）分组（`PluginGroup` 组件）
- ✅ `plugins/PluginsView.tsx` → `PluginRow` 显示非 stdio 的 transport 类型 badge
- ✅ `locales/{zh,en}/plugins.json` → 新增 `group.user` / `group.project` i18n key

**验证**: `cargo clippy -D warnings` 零警告 + `npx tsc --noEmit` 零类型错误

---

### T14: MCP Tab — 审批 UI ✅

**Spec**: [`plugins-ui/spec.md`](specs/plugins-ui/spec.md) 变更 3 + [`approval-gate/spec.md`](specs/approval-gate/spec.md)
**前置**: T8 ✅, T11 ✅
**状态**: ✅ 已完成 + Code Review 修复 3 个问题

**已完成**:
- ✅ `plugins/PluginsView.tsx` — `PendingApprovalSection` + `PendingApprovalCard`（橙色警告面板、命令预览、approve/reject 按钮）
- ✅ `lib/stores/plugin-store.ts` — `approvePlugin` + `rejectPlugin` actions
- ✅ `lib/transport.ts` — `approvePlugin()` + `rejectPlugin()` API + `PluginSummary` 扩展（`pending_approval` status、`commandPreview`、`scope: global`）

**Code Review 修复**:
- ✅ R1a: PendingApprovalCard 使用 `mountedRef` 防止卸载后 setState
- ✅ R1b: `ScopeBadge` 改用 `plugin.scope` 替代硬编码 `"project"`
- ✅ R2a: `broadcast_status_changed` 产出与 `handle_plugins_list` 一致的 JSON（新增 `name`、`enabled`、`lastError` 字段），抽取 `enrich_status()` 共享函数

**验证**: ✅ E2E: 项目 MCP → 橙色 pending 面板 + 命令预览 → approve → 移到正常列表 → reject → 消失

---

### T15: MCP Tab — McpDetailModal（参考 Codex）✅

**Spec**: [`mcp-detail-modal/spec.md`](../mcp-marketplace-ui/specs/mcp-detail-modal/spec.md)
**前置**: T11 ✅, T12 ✅
**状态**: ✅ 已完成

**已完成**:
- ✅ 15.1 创建 `McpDetailModal.tsx`：模态框 + `transport.mcpDetail` 数据加载
- ✅ 15.2 状态展示区（Status badge + connectedAt）
- ✅ 15.3 配置预览区（Command/Args/URL/Transport + env 脱敏 `maskEnvValue`）
- ✅ 15.4 工具列表区（name + description）
- ✅ 15.5 Remove 操作（确认 → `removePlugin` → 成功关闭/失败保留）
- ✅ 15.6 Restart 操作（`restartPlugin` + 刷新 detail）
- ✅ 15.7 PluginRow name 点击 → 打开 McpDetailModal
- ✅ Code Review 修复：async 错误处理、race condition 防护、i18n status labels

**验证**: ✅ E2E 验证通过

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

## 阶段 2.5：UI 视觉抛光 (P1.5)

> 目标：对标 Codex App 的 Plugin Directory 视觉质感，从扁平列表升级为卡片网格+品牌色+沉浸式详情+动画。
> **设计文档**: [`mcp-ui-visual-polish/design.md`](../mcp-ui-visual-polish/design.md)
> **Specs**: [`mcp-ui-visual-polish/specs/`](../mcp-ui-visual-polish/specs/)
> **进度**：23/23 ✅ 全部完成

### T34: Registry 数据扩展 ✅

**Spec**: [`explore-card-grid/spec.md`](../mcp-ui-visual-polish/specs/explore-card-grid/spec.md)
**前置**: T12.5 ✅
**状态**: ✅ 已完成

**已完成**:
- ✅ 34.1 为 15 个 entry 添加 `brandColor`（如 GitHub #24292F、Docker #2496ED）
- ✅ 34.2 添加 `author` 字段（Anthropic/GitHub/Google/Docker/Brave）
- ✅ 34.3 添加 `tags` 数组（每 entry 2-3 个标签）
- ✅ 34.4 `McpRegistryEntry` 接口更新 + `export`

---

### T35: CSS 动画补充 ✅

**Spec**: [`plugin-ui-animation/spec.md`](../mcp-ui-visual-polish/specs/plugin-ui-animation/spec.md)
**前置**: 无
**状态**: ✅ 已完成

**已完成**:
- ✅ 35.1 `@keyframes pv-float` (上下 6px, 3s) + `.pv-float` 工具类
- ✅ 35.2 `@keyframes modal-enter` (scale 0.96→1, 200ms) + `.pv-modal-enter` 工具类

---

### T36: Explore 卡片网格重设计 ✅

**Spec**: [`explore-card-grid/spec.md`](../mcp-ui-visual-polish/specs/explore-card-grid/spec.md)
**前置**: T34 ✅, T35 ✅
**状态**: ✅ 已完成

**已完成**:
- ✅ 36.1 CSS Grid 布局 `auto-fill, minmax(240px, 1fr)`
- ✅ 36.2 竖式卡片：brandColor icon(40x40) → 名称+作者 → category badge → 描述(line-clamp-2) → tags → 安装
- ✅ 36.3 hover: `-translate-y-0.5` + `shadow-md` + 200ms
- ✅ 36.4 stagger 入场 (`pv-stagger` + `--stagger-i`)
- ✅ 36.5 响应式 auto-fill 自动适配
- ✅ 36.6 搜索栏 `rounded-xl` + 更大 padding
- ✅ 额外：tags 也参与搜索过滤

---

### T37: McpDetailModal 沉浸式升级 ✅

**Spec**: [`detail-modal-hero/spec.md`](../mcp-ui-visual-polish/specs/detail-modal-hero/spec.md)
**前置**: T34 ✅, T35 ✅
**状态**: ✅ 已完成

**已完成**:
- ✅ 37.1 `registryMap` useMemo 按 id 查找元数据
- ✅ 37.2 Hero: 48px icon + brandColor 背景 + 18px name + author + description + status badge
- ✅ 37.3 3px 渐变色条 (`linear-gradient 40% → transparent`)
- ✅ 37.4 工具列表折叠/展开 (`CaretDown/CaretRight`)
- ✅ 37.5 toolCount > 5 时搜索 input
- ✅ 37.6 "编辑配置" 按钮 (optional `onEditConfig` prop)
- ✅ 37.7 `pv-modal-enter` 动画

---

### T38: 空状态与已安装列表品牌色 ✅

**Spec**: [`plugin-panel/spec.md`](../mcp-ui-visual-polish/specs/plugin-panel/spec.md)
**前置**: T34 ✅, T35 ✅
**状态**: ✅ 已完成

**已完成**:
- ✅ 38.1 McpEmptyState 图标 `pv-float` 浮动动画
- ✅ 38.2 双 CTA: "浏览服务器目录" (primary) + "手动添加" (ghost)
- ✅ 38.3 `registryMap` useMemo in McpTabContent
- ✅ 38.4 `PluginIcon` 组件: registry icon + brandColor 背景 + status dot overlay
- ✅ 38.5 非 registry → PuzzlePiece + tint fallback

---

### T39: 国际化与验证 ✅

**前置**: T36 ✅, T37 ✅, T38 ✅
**状态**: ✅ 已完成

**已完成**:
- ✅ 39.1 `plugins.json` zh/en 新增 `empty.add_manually`、`detail.edit_config`、`detail.search_tools`、`detail.by_author`
- ✅ 39.2 `npx tsc --noEmit` 零类型错误

---

## 阶段三：后端能力增强 (P2)

> 目标：连接管理健壮性、性能优化、动态更新。
> **进度**：10/10 完成 ✅
> **关键洞察**：Codex 的 `tools/list_changed` 也只 log 不处理，XiaoLin 做好 T19 即超越 Codex。

### T19: tools/list_changed 处理 ✅

**Spec**: [`notification-dispatch/spec.md`](specs/notification-dispatch/spec.md)（Gateway 订阅部分）
**前置**: T6 ✅
**状态**: ✅ 已完成

**已完成**:
- ✅ `xiaolin-mcp/src/lib.rs` — `fetch_tools(&self)` 获取最新工具列表（不需 `&mut self`）
- ✅ `xiaolin-mcp/src/lib.rs` — `re_register_tools()` 公开函数：`unregister_by_prefix` + 重新注册
- ✅ `xiaolin-gateway/src/state/mod.rs` — `spawn_notification_watcher()`：
  - 使用 `Weak<McpClient>` 防止热重载资源泄漏
  - `tools/list_changed` → `fetch_tools` + `re_register_tools` 刷新 ToolRegistry
  - `notifications/message` → 按 level (error/warning/info) 路由到 tracing
  - 区分 `RecvError::Lagged`（warn + continue）和 `RecvError::Closed`（退出）
- ✅ 启动路径 + 热重载路径均已接入 `spawn_notification_watcher`
- ✅ Code review 通过：3 个问题已修复（Weak client、Lagged 处理、ToolListResult 复用）

**验证**: ✅ `cargo check` + `cargo clippy -- -D warnings` 零警告 + 37 测试全通过 + E2E 验证（Plugins 页面正确显示 13 tools）

**超越 Codex**：Codex 的 `tools/list_changed` 仅 `info!` 日志不刷新工具；XiaoLin 完整处理链路（fetch + re-register）

---

### T20: 自动重连（仅 SSE/HTTP）✅

**Spec**: 无独立 spec，对应 D5
**前置**: T4
**状态**: ✅ 已完成 + Code Review 修复 5 个问题

**已完成**:
- ✅ `xiaolin-mcp/src/lib.rs` → `sse_reader_loop` 拆分为 `_inner` + guard 外层，任何退出路径（正常结束/错误）都发送 `xiaolin/transport_disconnected` 通知
- ✅ `xiaolin-gateway/src/state/mod.rs` → `spawn_notification_watcher_with_handles` 处理 `xiaolin/transport_disconnected`：
  - 指数退避 `min(1000×2^(n-1), 30000)ms`，最多 5 次重连
  - 重连通过 `McpClient::connect_sse()` 创建新客户端 + `tokio::time::timeout(30s)` 防 hang
  - 成功后替换 `mcp_handles` + 更新 `mcp_status` 为 Connected
  - 全部失败后更新 `mcp_status` 为 Failed + 错误信息
- ✅ `xiaolin-mcp/src/lib.rs` → `McpClient` 新增 `sse_url: Option<String>` + `sse_url()` getter（供重连使用）
- ✅ `xiaolin-mcp/src/lib.rs` → StreamableHttp `send_request` 对连接/超时错误自动重试（最多 3 次，500ms→1s→2s）

**Code Review 修复**:
- ✅ R1 (P1): 重连成功后更新 `mcp_status` 为 Connected
- ✅ R2 (P1): 重连全部失败后更新 `mcp_status` 为 Failed
- ✅ R3 (P2): 并发常量提取为模块级 `MCP_STDIO_CONCURRENCY`/`MCP_REMOTE_CONCURRENCY`
- ✅ R4 (P2): SSE reader 错误退出也发 disconnect 通知（guard 模式）
- ✅ R5 (P2): 重连时加 `timeout(30s)` 防止 hang

**验证**: ✅ `cargo check` + `cargo clippy -D warnings` 零警告 + 100 测试全通过 + E2E 验证 MCP 连接正常

---

### T21: 连接批次限制 ✅

**Spec**: 无独立 spec，对应 D7
**前置**: T4
**状态**: ✅ 已完成

**已完成**:
- ✅ `xiaolin-gateway/src/state/mod.rs` → 启动路径 `register_mcp_and_subagent_tools` 使用 `Semaphore` 限流（stdio=3, remote=20）
- ✅ `xiaolin-gateway/src/state/mod.rs` → 热重载路径 `reload_mcp_servers` 从串行 `for` 循环改为 semaphore 限流的并行连接

**验证**: ✅ `cargo check` + `cargo clippy -D warnings` 零警告

---

### T22: 启动超时 ✅

**Spec**: 无独立 spec，对应 D9
**前置**: T4
**状态**: ✅ 已完成

**已完成**:
- ✅ `xiaolin-core/src/agent_config.rs` → `McpServerConfig` 增加 `startup_timeout_sec: Option<u32>`（默认 30s）
- ✅ `xiaolin-mcp/src/lib.rs` → `connect_mcp_server` 提取为 `connect_mcp_server_inner` + `tokio::time::timeout` 包裹，超时返回明确错误信息

**验证**: ✅ `cargo check` + `cargo clippy -D warnings` 零警告 + E2E 验证连接正常（everything server ~0.8s 连接成功）

---

### T23: stale server 清理 ✅

**Spec**: 无独立 spec，对应 D7
**前置**: T5
**状态**: ✅ 已完成（随 T5 一并实现）

**已完成**:
- ✅ `xiaolin-gateway/src/state/mod.rs` → `reload_mcp_servers` 中 `to_remove` 逻辑：检测 config 移除的 server → `unregister_by_prefix` + `remove_mcp_instructions` + 从 handles 移除
- ✅ `xiaolin-mcp/src/lib.rs` → `McpClient::drop()` 自动 `start_kill()` 子进程

**验证**: 从配置删除 MCP server → 重载后工具自动注销 + 进程被 kill

---

### T24: Description 截断保护 ✅

**Spec**: 无独立 spec
**前置**: T2
**状态**: ✅ 已完成

**已完成**:
- ✅ `xiaolin-mcp/src/lib.rs` → `McpToolBridge::new` 中截断 description ≤ `MCP_TOOL_DESC_MAX_CHARS`（2048）字符，超长时 warn 日志
- ✅ `xiaolin-gateway/src/chat_pipeline.rs` → `inject_mcp_tools_prompt` 中 system prompt 截断 ≤ 120 字符

**验证**: 超长 description → 截断后注册 + prompt 注入不溢出

---

### T25: Session 级 Schema 缓存 ✅

**Spec**: 无独立 spec
**前置**: T19
**状态**: ✅ 已完成

**已完成**:
- ✅ `xiaolin-core/src/tool.rs` → `ToolRegistry` 新增 `json_sizes_cache` 字段和 `estimated_json_chars()` 方法，版本化缓存每个工具定义的 JSON 字符数
- ✅ `xiaolin-agent/src/runtime/turn_setup.rs` → 使用 `estimated_json_chars()` 替代逐个 `serde_json::to_string` 序列化
- ✅ `xiaolin-agent/src/runtime/llm_call.rs` → 同上，工具定义刷新时使用缓存的 JSON 字符数
- ✅ `xiaolin-gateway/src/chat_pipeline.rs` → `inject_mcp_tools_prompt` 使用 `MCP_TOOLS_PROMPT_CACHE` 静态缓存 MCP 工具 prompt，按 registry version 失效
- ✅ `tools/list_changed` → 自然失效：`re_register_tools` 调用 `unregister_by_prefix` + `register` → `bump_version()` → 缓存失效

**验证**: `cargo clippy -- -D warnings` 零警告，重复调用时 schema 不重复序列化

---

### T26: 逐 server 启动状态事件 ✅

**Spec**: 无独立 spec，对应 D9
**前置**: T4
**状态**: ✅ 已完成

**已完成**:
- ✅ `xiaolin-gateway/src/state/mod.rs` → `reload_mcp_servers` 使用 `FuturesUnordered` 替代 `join_all`，每个 server 先设 "connecting" 状态并广播，完成后逐个推送 connected/failed 状态更新
- ✅ `xiaolin-gateway/src/ws/plugins.rs` → `broadcast_status_changed` 改为 `pub` 并通过 `ws/mod.rs` re-export
- ✅ 前端无需改动 — `onPluginsStatusChanged` + `usePluginStore` 已自动响应 `plugins.status_changed` 事件，实现逐行状态更新

**验证**: `cargo clippy -- -D warnings` 零警告，热重载时 PluginsView 逐个 server 从 connecting → connected/failed

---

## 阶段四：智能工具注入 (P3)

> 目标：大量 MCP 工具时的 token 优化。

### T27: MCP 工具接入 deferred 管线 ✅

**Spec**: [`deferred-pipeline/spec.md`](specs/deferred-pipeline/spec.md) 变更 1-2
**前置**: T2, T6
**状态**: ✅ 已完成

**已完成**:
- ✅ `xiaolin-mcp/src/lib.rs` → `McpToolBridge` 增加 `hint`（search_hint）和 `keep_eager`（force_eager）字段
- ✅ `xiaolin-core/src/tool.rs` → `Tool` trait 增加 `force_eager()` 默认方法
- ✅ `xiaolin-core/src/tool.rs` → `ToolRegistry` 增加 `demote_to_deferred_by_prefix`、`deferred_tool_names`、`eager_mcp_definitions`
- ✅ `xiaolin-core/src/tool.rs` → `unregister_by_prefix` 修复：同步清理 deferred/channel_scoped 集合
- ✅ `xiaolin-core/src/tool.rs` → `register()` 修复：eager 注册时从 deferred 移除同名项
- ✅ `xiaolin-core/src/tool.rs` → `version()` 公共 getter + 单元测试
- ✅ `xiaolin-agent/src/runtime/turn_state.rs` → `tool_defs` 移入 `TurnMutableState`，增加 `registry_version_at_setup` 和 `extra_tool_defs`
- ✅ `xiaolin-agent/src/runtime/llm_call.rs` → registry version 变化时自动刷新 tool_defs，保留 channel 注入工具

**验证**: ✅ `cargo check` + `clippy -D warnings` 零警告 + 22 个 core 测试全通过

---

### T28: Deferred 时跳过 system prompt 注入 ✅

**Spec**: [`deferred-pipeline/spec.md`](specs/deferred-pipeline/spec.md) 变更 4
**前置**: T27
**状态**: ✅ 已完成

**已完成**:
- ✅ `xiaolin-gateway/src/chat_pipeline.rs` → `inject_mcp_tools_prompt` 改用 `eager_mcp_definitions()` 替代 `mcp_definitions()`
- ✅ deferred 工具仅列名称 + `tool_search` 引导提示

**验证**: ✅ deferred 模式下 system prompt 不含完整工具描述，仅工具名列表 + tool_search 引导

---

### T29: 阈值策略（更新：对标 Claude Code）✅

**Spec**: [`deferred-pipeline/spec.md`](specs/deferred-pipeline/spec.md) 变更 3
**前置**: T27
**状态**: ✅ 已完成

**已完成**:
- ✅ `xiaolin-gateway/src/state/mod.rs` → `maybe_defer_mcp_tools()` 阈值 128（对标 Codex 100 + 余量）
- ✅ 在 `register_mcp_and_subagent_tools`（启动）和 `reload_mcp_servers`（热重载）两处调用
- ✅ `spawn_notification_watcher` 中 `tools/list_changed` 后也检查阈值

**验证**: ✅ 使用 `eager_definitions().len()` 正确判断阈值，E2E 确认 13 工具时不触发 deferral

---

### T30: alwaysLoad 元数据支持 ✅

**Spec**: [`deferred-pipeline/spec.md`](specs/deferred-pipeline/spec.md) 变更 5
**前置**: T27
**状态**: ✅ 已完成

**已完成**:
- ✅ `xiaolin-mcp/src/lib.rs` → `McpTool` struct 增加 `meta: Option<serde_json::Value>`（序列化为 `_meta`）
- ✅ `McpTool::always_load()` helper → 检查 `_meta.alwaysLoad`
- ✅ `McpToolBridge::force_eager()` → 反映 `alwaysLoad` 状态
- ✅ `demote_to_deferred_by_prefix` 尊重 `force_eager()` 工具

**验证**: ✅ 带 `alwaysLoad` 的工具在 deferred 模式下仍然保持 eager

---

### T31: Schema 完整性 ✅

**Spec**: [`deferred-pipeline/spec.md`](specs/deferred-pipeline/spec.md) 变更隐含
**前置**: T27
**状态**: ✅ 已完成（已有实现无需修改）

**说明**: `McpToolBridge::parameters_schema` 已保留完整 JSON Schema，本次 deferred 改动不影响 schema 传递链路

**验证**: ✅ 37 个 MCP 测试全通过，schema 传递链路未受影响

---

### T32: Server Instructions Delta 注入（新增） ✅

**Spec**: 无独立 spec（对标 Claude Code `getMcpInstructionsDelta`）
**前置**: T6
**状态**: ✅ 已完成

**已完成**:
- ✅ `xiaolin-mcp/src/lib.rs` — `InitializeResult` 新增 `instructions: Option<String>` 字段
- ✅ `xiaolin-mcp/src/lib.rs` — `McpClient` 新增 `server_instructions` 字段，`initialize()` 时从 `InitializeResult` 捕获
- ✅ `xiaolin-mcp/src/lib.rs` — `McpClient::instructions()` getter 暴露 server instructions
- ✅ `xiaolin-gateway/src/chat_pipeline.rs` — 新增 `inject_mcp_instructions_delta()`，从 `mcp_handles` 异步收集 instructions，以独立 system message 注入（与工具列表分离）
- ✅ 内容按 server ID 确定性排序（BTreeMap），仅在 server 连接/断开时变化 → 最大化 prompt cache 命中率
- ✅ 37 个 MCP 测试全通过，`cargo clippy -- -D warnings` 零警告

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
✅ T3 (命名 TS)       ─── 完成（mcpNaming.ts + ToolCallCard）
✅ T4 (Transport)     ─── 完成
✅ T5 (路由修复)      ─── 完成
✅ T6 (Notification)  ─── 完成
✅ T7 (stderr)        ─── 完成
✅ T8 (审批门)        ─── 完成
✅ T9 (协议版本)      ─── 完成
✅ T10 (配置验证)     ─── 完成（connect_mcp_server 入口统一 validate）
✅ T11 (Tab 骨架)     ─── 完成
✅ T12 (AddServerModal) ─── 完成（Batch A）
✅ T12.5 (Explore)    ─── 完成（Batch A）
✅ T13 (分组)         ─── 完成（scope 分组 + transport badge）
✅ T14 (审批 UI)      ─── 完成
✅ T15 (DetailModal)  ─── 完成（Batch A）
✅ T16 (Skills)       ─── 完成
✅ T17 (Channels)     ─── 完成
✅ T18 (EmptyState)   ─── 完成
✅ T19 (list_changed) ─── 完成
✅ T20 (自动重连)     ─── 完成
✅ T21 (批次限制)     ─── 完成
✅ T22 (启动超时)     ─── 完成
✅ T23 (stale 清理)   ─── 完成（reload_mcp_servers + McpClient::drop）
✅ T24 (截断保护)     ─── 完成（MCP_TOOL_DESC_MAX_CHARS = 2048）
❌ T25 (Schema 缓存)  ←── T19
❌ T26 (启动事件)     ←── T4
✅ T27-T31 (P3)       ─── 完成
✅ T32 (Instructions) ─── 完成（新增）
✅ T33 (签名去重)     ─── 完成（connection_signature 去重）
✅ T34 (Registry 扩展) ─── 完成（UI 抛光）
✅ T35 (CSS 动画)     ─── 完成（UI 抛光）
✅ T36 (Explore 网格)  ─── 完成（UI 抛光）
✅ T37 (Detail Hero)  ─── 完成（UI 抛光）
✅ T38 (品牌色列表)   ─── 完成（UI 抛光）
✅ T39 (验证)         ─── 完成（UI 抛光）
```

## 建议实施顺序（ROI 优先级）

> 基于 2026-06-15 三方对比分析 + Codex UI 参考 + Codex App 视觉对标

1. ~~**T2**（命名全链路）~~ → ✅ 已完成
2. ~~**T4 + T5**（统一连接入口）~~ → ✅ 已完成
3. ~~**T6 + T19**（Notification dispatch + list_changed）~~ → ✅ 已完成，已超越 Codex
4. ~~**T8 + T14**（审批门）~~ → ✅ 已完成
5. ~~**T20-T22**（重连 + 批次 + 超时）~~ → ✅ 已完成
6. ~~**T27-T31**（Deferred 管线）~~ → ✅ 已完成，对标 Claude Code 默认 defer 模式
7. ~~**T12 + T12.5 + T15**（AddServerModal + Explore + DetailModal）~~ → ✅ 已完成（Batch A）
8. ~~**T34-T39**（UI 视觉抛光：卡片网格 + 品牌色 + Hero + 动画）~~ → ✅ 已完成
9. ~~**T10 + T33**（配置验证 + 签名去重）~~ → ✅ 已完成
10. ~~**T3 + T13**（前端命名 + 分组）~~ → ✅ 已完成
11. ~~**T23 + T24**（stale 清理 + 截断保护）~~ → ✅ 已完成
12. **T25 + T26**（Schema 缓存 + 启动事件）— 性能优化（+1 分）

## Spec 覆盖对照

| Spec | 对应任务 | 完成度 |
|------|---------|:---:|
| `naming-pipeline/spec.md` | T2 ✅, T3 ✅ | 100% |
| `transport-fix/spec.md` | T4 ✅, T5 ✅, T9 ✅, T10 ✅ | 100% |
| `notification-dispatch/spec.md` | T6 ✅, T7 ✅, T19 ✅ | 100% |
| `approval-gate/spec.md` | T8 ✅, T14 ✅ | 100% |
| `deferred-pipeline/spec.md` | T27 ✅, T28 ✅, T29 ✅, T30 ✅, T31 ✅ | 100% |
| `plugins-ui/spec.md` | T1 ✅, T11-T18 (9✅) | 100% |
| `mcp-explore/spec.md` | T12.5 ✅ | 100% |
| `mcp-add-modal/spec.md` | T12 ✅ | 100% |
| `mcp-detail-modal/spec.md` | T15 ✅ | 100% |
| `explore-card-grid/spec.md` | T34 ✅, T36 ✅ | 100% |
| `detail-modal-hero/spec.md` | T37 ✅ | 100% |
| `plugin-ui-animation/spec.md` | T35 ✅, T36 ✅, T38 ✅ | 100% |
| `plugin-panel/spec.md` | T38 ✅ | 100% |

---

## P4: 协议完整度补齐（mcp-protocol-completeness）

> 基于三方深度对比评审（XiaoLin 65.5 / Codex 68.5 / Claude Code 79.5），补齐 Auth、协议覆盖、安全防护三大差距。
> Specs: `mcp-security-hardening`、`mcp-oauth`、`mcp-resources`、`mcp-prompts`、`mcp-elicitation`、`plugin-panel`（modified）

### P4-A: 安全加固（mcp-security-hardening）

**Spec**: `specs/mcp-security-hardening/spec.md`

- [x] T42: `xiaolin-mcp/src/sanitize.rs` 新增 `sanitize_unicode(s: &str) -> String` 函数，移除双向控制字符（U+200E—U+200F, U+202A—U+202E, U+2066—U+2069）、零宽字符（U+200B/C/D, U+FEFF）、不可见控制字符（U+0000—U+001F 中除 `\t\n\r`），保留所有可见字符（中文、emoji 等）
- [x] T43: `sanitize.rs` 新增 `sanitize_json_schema_descriptions(schema: &mut serde_json::Value)` 递归清洗 JSON Schema 中所有 `description` 字段
- [x] T44: 在 `tools/list` 返回后对所有 tool name/description 和 inputSchema description 执行 Unicode 清洗；新增单元测试覆盖双向覆盖字符、零宽字符、嵌套 schema description
- [x] T45: `xiaolin-gateway/src/chat_pipeline.rs` `inject_mcp_instructions_delta` 中，对每个 server instructions 先做 `sanitize_unicode` 再做可疑模式检测（正则匹配 `ignore previous|system:|<\||\[INST\]`），命中时跳过该 server instructions 注入并记录 `warn!` 日志，不影响服务器连接
- [x] T46: `xiaolin-mcp/src/lib.rs` 中 4 处 `format!("{server_prefix}{}", tool.name)` 替换为 `naming::mcp_tool_name(server_id, &tool.name)`，确保工具名经过 `sanitize_for_api` 消毒；消毒后名称碰撞时跳过后注册的工具并 warn
- [x] T47: Streamable HTTP RPC 方法新增 session expired 检测（HTTP 404 或 JSON-RPC error code -32001）和恢复逻辑：`send_request_with_session_recovery` 包装器自动检测 `SessionExpired` 错误和 `-32001` 响应，调用 `recover_streamable_http_session` 重新 `initialize` 并重试原操作（最多 1 次）
- [x] T48: HTTP session 恢复的并发保护：`McpClient` 新增 `recovery_lock: Arc<tokio::sync::Mutex<()>>`，recovery 方法先保存旧 session_id、获取锁后对比判断是否已被其他请求恢复，仅执行一次恢复
- [x] T49: `cargo clippy -- -D warnings` 零警告验证 + 新增单元测试（`session_expired_response_detection`、`session_expired_error_type`、`streamable_http_recovers_from_404`、`streamable_http_recovers_from_json_rpc_32001`），全部 50 个测试通过

### P4-B: Bearer Token & HTTP Headers（mcp-oauth P0）

**Spec**: `specs/mcp-oauth/spec.md`
**前置**: T42-T49（安全加固）

- [x] T50: `McpServerConfig` 新增 `bearer_token_env_var: Option<String>` 和 `http_headers: Option<HashMap<String, String>>` 字段（serde `skip_serializing_if` 避免空 map 输出）
- [x] T51: `validate()` 中新增校验：`bearer_token_env_var` 仅在 HTTP 传输时有效（stdio 报错）；环境变量名不允许空字符串
- [x] T52: `connect_mcp_server` 的 SSE 和 Streamable HTTP 路径中，通过 `resolve_mcp_http_headers()` 解析 `bearer_token_env_var`，注入 `Authorization: Bearer <value>` header；`reqwest::ClientBuilder::default_headers()` 确保所有请求自动携带；变量不存在时 bail 错误
- [x] T53: `connect_mcp_server` 中解析 `http_headers`：值以 `$` 开头的视为环境变量引用（`$MY_VAR` → 读 `MY_VAR`），其他直接使用；环境变量不存在时跳过该 header 并 warn；`McpClient` 新增 `extra_headers` 字段用于 SSE 重连复用
- [x] T54: 前端 `AddServerModal` 新增 `bearer_token_env_var`（单行输入 + 提示文字）和 `http_headers`（key-value 编辑器，支持 `$ENV_VAR` 引用），仅 HTTP 传输时显示
- [x] T55: `cargo clippy -- -D warnings` 零警告验证 + 16 个单元测试（12 agent_config + 4 resolve_headers）全部通过

### P4-C: OAuth 2.0 PKCE 流程（mcp-oauth P1） ✅

**Spec**: `specs/mcp-oauth/spec.md`
**前置**: T50-T55（Bearer Token）
**进度**: 12/12 完成 ✅

- [x] T56: 新增 `xiaolin-mcp/src/oauth.rs` 模块：`McpOAuthClient` 结构体 + `OAuthMetadata` / `TokenResponse` / `StoredToken` / `PkceChallenge` + metadata discovery
- [x] T57: 实现 PKCE 授权码生成：`code_verifier`（64 随机字节 base64url）+ `code_challenge`（S256 hash）
- [x] T58: 实现本地回调 HTTP 服务器（`127.0.0.1:随机端口`），监听 OAuth 授权码回调，含 HTML 成功/失败页
- [x] T59: 实现 token exchange：`exchange_code()` 用授权码 + code_verifier 换取 access_token + refresh_token
- [x] T60: 实现 token 持久化存储：`~/.xiaolin/mcp-tokens/<server_id>.json` 文件存储 + `load/save/remove_stored_token`
- [x] T61: 实现 token 自动刷新：`try_oauth_recovery()` 在 HTTP 401 时加载 stored token → refresh → 重连，失败则 NeedsOAuth
- [x] T62: `McpStatus` 新增 `NeedsAuth` 变体（序列化 `needs_auth`），`mcp_tool.rs` match 补全
- [x] T63: `connect_mcp_server` 集成 OAuth：401 → `try_oauth_recovery` → `mcp_status_for_error` 映射 NeedsOAuth→NeedsAuth（4 处 McpStatus::Failed 改用 helper）
- [x] T64: `plugins.rs` 新增 `handle_plugins_oauth_login`：metadata discovery → PKCE → 本地回调 → 返回 auth_url → 后台等待回调 → 换 token → 重连
- [x] T65: 前端 `PluginRow` / `PluginIcon` / `StatusDot` 支持 `needs_auth` 状态（黄色脉冲 + Key 图标 + "登录" 按钮），`oauthLoginPlugin` store action + `window.open(auth_url)`
- [x] T66: `NEEDS_AUTH_CACHE` 15 分钟 TTL 静态缓存，`connect_mcp_server` 入口检查，`clear_needs_auth_cache()` 在 oauth_login 时清除
- [x] T67: `cargo clippy --workspace -- -D warnings` 零警告 + 7 个 OAuth 单元测试通过（pkce_format/s256/uniqueness, metadata_deserialize, stored_token_roundtrip, callback_server, build_auth_url）

**P4-C Review 修复（5 项）**:
- [x] [P1] 回调服务器未关闭：`AtomicBool` done 标记 + listener loop break，收到 code 后立即停止 accept
- [x] [P1] 后台 OAuth 失败无通知：spawn block 改为 `Result` 流式处理，失败时 broadcast `plugins.oauth_failed` 事件
- [x] [P2] URL 参数未 decode：改用 `reqwest::Url::parse` + `query_pairs()` 自动 percent-decode
- [x] [P2] `build_authorization_url` 缺 client_id：新增 `client_id: Option<&str>` 参数，默认回退到 `server_url`
- [x] [P2] Config 查找过窄：`handle_plugins_oauth_login` 同时查找 user config 和 project-level config

### P4-D: Resources 客户端（mcp-resources） ✅

**Spec**: `specs/mcp-resources/spec.md`
**前置**: T42-T44（Unicode 清洗）
**进度**: 9/9 完成 ✅ + Review 修复 2 项

- [x] T68: `McpClient` 新增 `list_resources()` 方法：发送 `resources/list` RPC，返回 `Vec<McpResource>`（含 has_resources() 守卫）
- [x] T69: `McpClient` 新增 `read_resource(uri: &str)` 方法：发送 `resources/read` RPC，返回资源内容（含 1MB 截断 + `[truncated]` 标记）
- [x] T70: `McpClient` 新增 `list_resource_templates()` 方法：发送 `resources/templates/list` RPC，新增 `McpResourceTemplate` 类型
- [x] T71: `initialize()` 中解析 `ServerCapabilities` 并存入 `server_capabilities: RwLock<ServerCapabilities>`，记录 has_resources/has_prompts 日志
- [x] T72: `xiaolin-gateway` 注册 `mcp__list_resources` deferred agent 工具：聚合所有有 resources 能力的服务器资源列表，附带 server name
- [x] T73: `xiaolin-gateway` 注册 `mcp__read_resource` deferred agent 工具：按 `server_name` + `uri` 参数读取指定资源
- [x] T74: notification watcher 中监听 `notifications/resources/list_changed` 和 `notifications/prompts/list_changed`
- [x] T75: `list_resources()`、`read_resource()`、`list_resource_templates()` 中均调用 `sanitize::sanitize_unicode` 清洗 name/uri/description
- [x] T76: `cargo clippy -- -D warnings` 零警告 + 12 个 resource 单元测试通过 + `McpResource`/`McpResourceContent` 加 `rename_all = "camelCase"` 确保协议兼容

**P4-D Review 修复（2 项）**:
- [x] [P2] `McpListResourcesTool::execute` 持锁调 RPC：重构为先 collect 有 resources 能力的 clients（block scope 内 lock + clone + collect），scope 结束后 lock 自动释放，再遍历做网络请求。避免长时间持有 mutex 阻塞其他工具
- [x] [P3] `read_resource` 截断 UTF-8 边界 panic：`text.truncate(MAX)` 改为 `text.truncate(text.floor_char_boundary(MAX))`，确保截断点落在字符边界上。同步修复测试中的截断逻辑

### P4-E: Prompts 客户端（mcp-prompts） ✅

**Spec**: `specs/mcp-prompts/spec.md`
**前置**: T42-T44（Unicode 清洗）
**进度**: 9/9 完成 ✅ + Review 修复 1 项

- [x] T77: `McpClient` 新增 `list_prompts()` 方法：发送 `prompts/list` RPC，返回 `Vec<McpPrompt>`，含 `has_prompts()` 守卫 + Unicode 清洗
- [x] T78: `McpClient` 新增 `get_prompt(name, arguments)` 方法：发送 `prompts/get` RPC，返回 `Vec<McpPromptMessage>`；新增 `McpPromptMessage`、`McpPromptContent`（Text/Image/Resource）枚举
- [x] T79: `has_prompts()` + `ServerCapabilities.prompts` 已在 P4-D 中实现，无需额外改动
- [x] T80: `plugins.prompts` WS handler：先 collect 有 prompts 能力的 clients（block scope lock），再遍历调用聚合
- [x] T81: `plugins.get_prompt` WS handler：按 `server_name` 查找 client（drop lock），调用 `get_prompt(prompt_name, arguments)`
- [x] T82: notification watcher 中 `prompts/list_changed` → `ws_broadcast` 发送 `plugins.prompts_changed` 事件（新增 `ws_broadcast: Option` 参数）
- [x] T83: `list_prompts()` 中对 name/description/argument.name/argument.description 做 `sanitize_unicode`
- [x] T84: 前端 `McpDetailModal` 新增 Prompts 可折叠区域：显示 prompt name/description/arguments badges（含 required 标记）；`transport.ts` 新增 `mcpPrompts()` + `mcpGetPrompt()` API；i18n `detail.prompts_title` 中英文
- [x] T85: `cargo clippy --workspace -- -D warnings` 零警告 + 73 个 mcp 测试全通过（含 5 个新增 prompt 测试） + `npx tsc --noEmit` 零错误

**P4-E Review 修复（1 项）**:
- [x] [P2] `McpPromptContent::Image` 的 `mime_type` 字段未加 serde rename — MCP 协议使用 `mimeType`（camelCase），已修复为 `#[serde(alias = "mime_type", rename = "mimeType")]`

### P4-F: Elicitation 处理（mcp-elicitation） ✅

**Spec**: `specs/mcp-elicitation/spec.md`
**前置**: T42-T49（安全加固）
**进度**: 8/8 完成 ✅ + Review 零问题

- [x] T86: `xiaolin-mcp` 的 `initialize` 请求中声明 `elicitation: {}` 客户端能力（含 session recovery 路径）
- [x] T87: `McpClient` 新增 `McpServerRequest` 结构体 + `server_request_tx` broadcast channel + `dispatch_incoming()` 三路分发（server request/response/notification）+ `subscribe_server_requests()` API
- [x] T88: `xiaolin-gateway` 新增 `spawn_server_request_watcher`：监听 `elicitation/create` → 生成唯一 ID → 存入 `pending_elicitations` DashMap → broadcast `mcp.elicitation.request` WS 事件
- [x] T89: `xiaolin-gateway` 处理前端回复：`plugins.elicitation_reply` WS handler → 从 DashMap remove → `oneshot::Sender` 发送 `ElicitationReply` → `send_response` 回传 MCP server
- [x] T90: Elicitation 超时处理：`tokio::time::timeout(300s)` 等待 `reply_rx`，超时自动 broadcast `mcp.elicitation.timeout` 事件 + 回复 `{ action: "decline" }`
- [x] T91: 前端 `ElicitationDialog` 组件：根据 `requestedSchema.properties` 动态渲染表单（string→文本框、number→数字框、boolean→复选框、enum/oneOf→下拉框），`wsClient.on("mcp.elicitation.request")` 订阅 + i18n
- [x] T92: 前端 elicitation 取消逻辑：点击"取消"按钮或点击遮罩 → `handleDecline()` → `transport.mcpElicitationReply(id, "decline")` → dialog 关闭
- [x] T93: `cargo clippy -- -D warnings` 零警告 + 5 个新增单元测试（dispatch_incoming 三路分发 + server_request clone/debug + elicitation_response 序列化） + 2 个 protocol 测试（parse_elicitation_reply accept/decline）

### P4-G: Plugin Panel 扩展（plugin-panel modified）

**Spec**: `specs/plugin-panel/spec.md`（MODIFIED）
**前置**: T62（NeedsAuth 变体）, T68-T76（Resources）, T77-T85（Prompts）

- [ ] T94: `PluginRow` 新增 `needs_auth` 状态样式：黄色认证图标 + "登录"按钮
- [ ] T95: 插件详情展开区域新增 Resources 子标签：请求 `plugins.resources` 接口显示资源列表
- [ ] T96: `xiaolin-gateway/src/ws/plugins.rs` 新增 `plugins.resources` 请求处理
- [ ] T97: 无 resources/prompts 能力的服务器不显示对应标签
- [ ] T98: `enrich_status` 函数新增 `capabilities` 字段输出（resources/prompts/tools 能力标记）
- [ ] T99: 前端类型定义更新：`McpServerStatus` 新增 `capabilities` 和 `needs_auth` 相关字段
- [ ] T100: i18n 更新：`plugins.json` 新增 needs_auth、login、resources、prompts 相关翻译 key

---

## Spec 覆盖矩阵

| Spec | 任务 | 完成度 |
|------|------|--------|
| `naming-pipeline/spec.md` | T2 ✅, T3 ✅ | 100% |
| `transport-fix/spec.md` | T4 ✅, T5 ✅, T7 ✅, T9 ✅ | 100% |
| `notification-dispatch/spec.md` | T6 ✅, T19 ✅ | 100% |
| `approval-gate/spec.md` | T8 ✅, T14 ✅ | 100% |
| `deferred-pipeline/spec.md` | T27 ✅, T28 ✅, T29 ✅, T30 ✅, T31 ✅ | 100% |
| `plugins-ui/spec.md` | T1 ✅, T11-T18 (9✅) | 100% |
| `mcp-explore/spec.md` | T12.5 ✅ | 100% |
| `mcp-add-modal/spec.md` | T12 ✅ | 100% |
| `mcp-detail-modal/spec.md` | T15 ✅ | 100% |
| `explore-card-grid/spec.md` | T34 ✅, T36 ✅ | 100% |
| `detail-modal-hero/spec.md` | T37 ✅ | 100% |
| `plugin-ui-animation/spec.md` | T35 ✅, T36 ✅, T38 ✅ | 100% |
| `plugin-panel/spec.md` | T38 ✅, T94-T100 | P0-P3: 100%, P4-G: 0% |
| `mcp-security-hardening/spec.md` | T42-T49 ✅ | 100% |
| `mcp-oauth/spec.md` | T50-T67 ✅ | 100% |
| `mcp-resources/spec.md` | T68-T76 ✅ | 100% |
| `mcp-prompts/spec.md` | T77-T85 ✅ | 100% |
| `mcp-elicitation/spec.md` | T86-T93 ✅ | 100% |
| `plugins-ui-alignment/spec.md` | T39 ✅ | 100% |

## 整体进度

- **P0**：10/10 完成（T1-T10 ✅）
- **P1**：10/10 完成（T10.5 ✅, T11 ✅, T12 ✅, T12.5 ✅, T13 ✅, T14 ✅, T15 ✅, T16 ✅, T17 ✅, T18 ✅）
- **P1.5 (UI 抛光)**：6/6 组完成（T34-T39 ✅），23 个子任务全部完成
- **P2**：10/10 完成 ✅（T19-T26 ✅, T32 ✅, T33 ✅）
- **P3**：5/5 完成（T27-T31 ✅ Deferred Pipeline 全部完成）
- **P4 (协议完整度)**：52/59 完成
  - P4-A 安全加固：8/8（T42-T49）✅
  - P4-B Bearer Token & Headers：6/6（T50-T55）✅
  - P4-C OAuth PKCE：12/12（T56-T67）✅ + Review 修复 5 项
  - P4-D Resources 客户端：9/9（T68-T76）✅ + Review 修复 2 项
  - P4-E Prompts 客户端：9/9（T77-T85）✅ + Review 修复 1 项
  - P4-F Elicitation：8/8（T86-T93）✅ + Review 零问题
  - P4-G Plugin Panel 扩展：0/7（T94-T100）
- **总计**：93/100 完成（93%）

### 关键路径

```
T42-T49（安全加固）→ T50-T55（Bearer Token）→ T56-T67（OAuth PKCE）
                   ↘ T68-T76（Resources）→ T94-T100（Plugin Panel）
                   ↘ T77-T85（Prompts）  ↗
                   ↘ T86-T93（Elicitation）
```
