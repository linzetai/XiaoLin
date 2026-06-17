# PluginsView UI 整合 — 详细技术方案

## 现状分析

### 当前架构

扩展能力管理分散在四处：

| 位置 | 功能 | 入口 | 状态 |
|------|------|------|------|
| **PluginsView** (`plugins/PluginsView.tsx`) | MCP 列表、启用/禁用/重启、工具详情 | 主视图 tab (chat/automations/**plugins**) | ✅ 接真实 API，但只显示 MCP |
| **SkillsTab** (`settings/SkillsTab.tsx`) | Skills + Tools 列表、上传/刷新 | Settings 面板 → Skills tab | ✅ 功能完整 |
| **ConnectionsPage** (`connections/ConnectionsPage.tsx`) | MCP 添加/删除/详情 + Channel (WeChat) 管理 | 未被引用（疑似废弃） | ⚠️ MCP 部分与 PluginsView 重复 |
| **McpManager** (`settings/McpManager.tsx`) | 完全 mock 数据 | 无引用 | ❌ 死代码 |

### 问题

1. **MCP、Skills、Channels 三类扩展能力分散在不同页面**，用户无法在一处统一管理
2. **添加 MCP server** — 只能在 ConnectionsPage（且无引用入口）或 chat 中的 ManageMcpServerTool
3. **PluginsView footer** 说 "Add MCP servers in Settings" — 但 Settings 中的 McpManager 是死代码
4. **Skills 藏在 Settings 面板里**，不够直观，应与 MCP 同级展示
5. **Channels**（WeChat、Feishu）也是扩展能力的一种，应统一到 Plugins 页面
6. **无审批 UI** — 项目级 MCP server 没有 pending/approve/reject 显示

### PluginSummary 类型

```typescript
interface PluginSummary {
  id: string;
  name: string;
  scope: "user" | "project";
  enabled: boolean;
  status: "connected" | "connecting" | "failed" | "disabled";
  toolCount: number;
  lastError?: string | null;
  connectedAt?: string | null;
}
```

当前缺少：`pendingApproval`、`transport`、`command`、`url`（配置信息用于详情展示和审批预览）。

## 设计方案

### 目标

**PluginsView 成为所有扩展能力的唯一管理入口**：MCP Servers + Skills + Channels，统一在一个页面内，按类别分 tab 展示。

### 变更 1: 三 Tab 布局

```
┌──────────────────────────────────────────────────────────┐
│ 🧩 Plugins                                               │
│ Extend capabilities with MCP servers, skills & channels  │
│                                                           │
│  [MCP Servers (5)]  [Skills (12)]  [Channels (2)]        │
│                                      [+ Add]  [↻ Reload] │
├──────────────────────────────────────────────────────────┤
│                                                           │
│  (当前 tab 内容区域)                                      │
│                                                           │
└──────────────────────────────────────────────────────────┘
```

```tsx
type PluginsTab = "mcp" | "skills" | "channels";
const [activeTab, setActiveTab] = useState<PluginsTab>("mcp");

// Header
<div className="flex items-center gap-1 rounded-lg p-0.5" style={{ background: "var(--bg-tertiary)" }}>
  {(["mcp", "skills", "channels"] as const).map((tab) => (
    <button key={tab} onClick={() => setActiveTab(tab)} ...>
      {tab === "mcp" ? `MCP Servers (${mcpCount})` : tab === "skills" ? `Skills (${skillCount})` : `Channels (${channelCount})`}
    </button>
  ))}
</div>

// Action buttons (context-sensitive)
<div className="flex items-center gap-2">
  {activeTab === "mcp" && <button onClick={() => setShowAddMcpModal(true)}>+ Add</button>}
  {activeTab === "skills" && <SkillUploadMenu />}
  <button onClick={handleReload}>↻</button>
</div>
```

### 变更 1b: Header 操作按钮

每个 tab 有不同的操作：
- **MCP Servers**: `+ Add Server` + `↻ Reload All`
- **Skills**: `↑ Upload` (folder / zip) + `↻ Refresh`
- **Channels**: `↻ Refresh`

### 变更 2: PluginSummary 类型扩展

```typescript
interface PluginSummary {
  id: string;
  name: string;
  scope: "user" | "project";
  enabled: boolean;
  status: "connected" | "connecting" | "failed" | "disabled" | "pending_approval";
  toolCount: number;
  lastError?: string | null;
  connectedAt?: string | null;
  // 新增
  transport: "stdio" | "sse";
  commandPreview?: string;     // "npx @mcp/server arg1 arg2" or SSE URL
  pendingApproval?: boolean;   // 项目级审批状态
}
```

### 变更 3: Pending Approval 卡片

项目级 MCP server 首次发现时显示为审批卡片：

```
┌──────────────────────────────────────────────────────────┐
│ ⚠️  待审批的项目级 MCP 服务器                              │
│                                                            │
│  ○ chrome-devtools                          [PROJECT]      │
│    npx @anthropic/chrome-devtools-mcp                      │
│    ⓘ 此服务器来自项目配置 (.xiaolin/mcp.json)              │
│                                                            │
│    [✓ 批准并连接]   [✗ 拒绝]                               │
│                                                            │
│  ○ custom-tools                             [PROJECT]      │
│    npx custom-tools-server --port 3000                     │
│    ⓘ 此服务器来自项目配置 (.cursor/mcp.json)               │
│                                                            │
│    [✓ 批准并连接]   [✗ 拒绝]                               │
└──────────────────────────────────────────────────────────┘
```

```tsx
function PendingApprovalSection({ pendingPlugins, onApprove, onReject }) {
  if (pendingPlugins.length === 0) return null;
  return (
    <div className="mb-4 rounded-lg border border-orange-200/30 bg-orange-50/5 p-4">
      <div className="flex items-center gap-2 mb-3">
        <ShieldWarning size={16} style={{ color: "var(--orange)" }} />
        <span className="text-[13px] font-semibold" style={{ color: "var(--orange)" }}>
          待审批的项目级 MCP 服务器
        </span>
      </div>
      {pendingPlugins.map((p) => (
        <PendingApprovalCard key={p.id} plugin={p} onApprove={onApprove} onReject={onReject} />
      ))}
    </div>
  );
}
```

### 变更 4: AddPluginModal（增强版）

从 ConnectionsPage 迁移 `AddMcpModal`，增强为支持多种 transport：

```
┌─────────────────────────────────────────┐
│ Add MCP Server                           │
│                                          │
│ Server ID:                               │
│ ┌──────────────────────────────────────┐ │
│ │ my-mcp-server                        │ │
│ └──────────────────────────────────────┘ │
│                                          │
│ Transport:                               │
│  ◉ stdio    ○ SSE                        │
│                                          │
│ ┌── stdio 模式 ─────────────────────┐   │
│ │ Command:                           │   │
│ │ ┌────────────────────────────────┐ │   │
│ │ │ npx                            │ │   │
│ │ └────────────────────────────────┘ │   │
│ │ Arguments:                         │   │
│ │ ┌────────────────────────────────┐ │   │
│ │ │ -y @anthropic/mcp-server       │ │   │
│ │ └────────────────────────────────┘ │   │
│ │ Environment Variables (optional):  │   │
│ │ ┌────────────────────────────────┐ │   │
│ │ │ KEY=value, API_KEY=xxx         │ │   │
│ │ └────────────────────────────────┘ │   │
│ └────────────────────────────────────┘   │
│                                          │
│ ┌── SSE 模式（切换后显示）──────────┐   │
│ │ URL:                               │   │
│ │ ┌────────────────────────────────┐ │   │
│ │ │ http://localhost:3000/sse      │ │   │
│ │ └────────────────────────────────┘ │   │
│ └────────────────────────────────────┘   │
│                                          │
│        [Cancel]    [Add Server]           │
└─────────────────────────────────────────┘
```

### 变更 5: PluginDetailModal（从 ConnectionsPage 迁移）

点击 PluginRow 展开时可选择弹出详情模态框：

- 显示 server 配置信息（command, args, env, transport, url）
- 工具列表（可搜索）
- 操作按钮（重启、删除、禁用）
- 连接日志/错误信息

### 变更 6: Plugin Store 扩展

```typescript
interface PluginStoreState {
  // ... existing ...
  addPlugin: (id: string, transport: string, config: AddPluginConfig) => Promise<boolean>;
  removePlugin: (id: string) => Promise<boolean>;
  approveProjectMcp: (id: string) => Promise<boolean>;
  rejectProjectMcp: (id: string) => Promise<boolean>;
  reloadAll: () => Promise<void>;
}

interface AddPluginConfig {
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  url?: string;
}
```

### 变更 7: Transport API 扩展

```typescript
// transport.ts

export async function addPlugin(
  id: string, transport: string, config: AddPluginConfig,
): Promise<boolean> {
  const resp = await wsClient.send("plugins.add", { id, transport, ...config });
  return resp?.data?.ok ?? false;
}

export async function removePlugin(id: string): Promise<boolean> {
  const resp = await wsClient.send("plugins.remove", { id });
  return resp?.data?.ok ?? false;
}

export async function approveProjectMcp(id: string): Promise<boolean> {
  const resp = await wsClient.send("plugins.approve_project_mcp", { id });
  return resp?.data?.ok ?? false;
}

export async function rejectProjectMcp(id: string): Promise<boolean> {
  const resp = await wsClient.send("plugins.reject_project_mcp", { id });
  return resp?.data?.ok ?? false;
}

export async function reloadAllPlugins(): Promise<boolean> {
  const resp = await wsClient.send("plugins.reload_all");
  return resp?.data?.ok ?? false;
}
```

### 变更 8: ConnectionsPage 删除

由于 MCP 和 Channels 都迁移到了 PluginsView，`ConnectionsPage.tsx` 整个文件删除：

- MCP 相关（McpCard、McpDetailModal、AddMcpModal）→ 已迁移到 MCP tab
- Channel 相关（ChannelCard、ChannelDetailModal、WechatQrModal）→ 已迁移到 Channels tab
- 文件当前无任何外部引用，可安全删除

### 变更 9: EmptyState 更新

```tsx
function EmptyState() {
  return (
    <div className="flex flex-col items-center justify-center gap-5 py-24">
      <PuzzlePiece size={32} style={{ color: "var(--tint)", opacity: 0.8 }} />
      <div className="text-center">
        <p className="text-[17px] font-bold">No plugins installed</p>
        <p className="mt-2 text-[13px] leading-relaxed" style={{ color: "var(--fill-quaternary)" }}>
          Click &quot;Add&quot; above to connect your first MCP server.
        </p>
      </div>
    </div>
  );
}
```

删除 "Manage in Settings" 按钮和 footer 中的 "Add MCP servers in Settings" 文案。

### 变更 10: 列表分组

按 scope 分组显示：

```
── User ──────────────────
  ● chrome-devtools       [connected] 5 tools
  ● github                [connected] 12 tools
  ○ disabled-server       [disabled]

── Project ───────────────
  ⚠ pending-server        [pending_approval]
  ● project-tool          [connected] 3 tools
```

```tsx
const userPlugins = plugins.filter((p) => p.scope === "user");
const projectPlugins = plugins.filter((p) => p.scope === "project");
const pendingPlugins = plugins.filter((p) => p.pendingApproval);

return (
  <>
    <PendingApprovalSection pendingPlugins={pendingPlugins} ... />
    {userPlugins.length > 0 && <ScopeGroup label="User" plugins={userPlugins} ... />}
    {projectPlugins.length > 0 && <ScopeGroup label="Project" plugins={projectPlugins} ... />}
  </>
);
```

### 变更 11: Skills Tab（从 SkillsTab 迁移）

从 `settings/SkillsTab.tsx` 迁移 Skills 和 Tools 管理功能到 PluginsView 的 Skills tab。

```
┌──────────────────────────────────────────────────────────┐
│  [Skills (12)]  [Tools (28)]          [↑ Upload] [↻]     │
├──────────────────────────────────────────────────────────┤
│                                                           │
│ ── Global Skills ──────────────────                       │
│   📄 code-review          Review code changes             │
│   📄 brainstorming        Explore ideas before coding     │
│   📄 architecture-diagram Create architecture diagrams    │
│                                                           │
│ ── Agent: main ────────────────────                       │
│   📄 agent-specific-skill  ...                            │
│                                                           │
└──────────────────────────────────────────────────────────┘
```

保留 SkillsTab 的核心逻辑：
- **双列表模式**：Skills / Tools 切换（复用现有的 filter toggle）
- **Global vs Agent skills 分组**
- **Upload 功能**：文件夹 / ZIP 上传
- **Refresh 功能**：从磁盘重新加载

数据源：直接复用 `api.listSkills()` / `api.listTools()` / `api.refreshSkills()` / `api.uploadSkill()`

### 变更 12: Channels Tab（从 ConnectionsPage 迁移）

从 `ConnectionsPage` 迁移 Channel 管理功能。

```
┌──────────────────────────────────────────────────────────┐
│                                                           │
│  💬 WeChat                              [Connected ●]     │
│     Personal account linked                               │
│                                                           │
│  💬 Feishu                              [Configure →]     │
│     Enterprise messaging integration                      │
│                                                           │
│  💬 DingTalk                            [Available]       │
│     Not configured                                        │
│                                                           │
└──────────────────────────────────────────────────────────┘
```

迁移组件：
- `ChannelCard` — 显示 channel 状态、连接/断开按钮
- `ChannelDetailModal` — 配置编辑、工具列表
- `WechatQrModal` — 微信扫码登录流程

数据源：直接复用 `api.channelsList()` / `api.channelsConnect()` / `api.channelsDisconnect()` / `api.channelsDetail()`

### 变更 13: 统一 EmptyState

每个 tab 有独立的空状态：

- **MCP**: "No MCP servers — Click '+ Add Server' to connect your first MCP server"
- **Skills**: "No skills installed — Upload a skill folder or ZIP to extend capabilities"
- **Channels**: "No channels configured — Configure messaging integrations"

## StatusDot 扩展

```tsx
function StatusDot({ status }: { status: string }) {
  const color =
    status === "connected" ? "var(--green, #38A169)" :
    status === "failed" ? "var(--red, #E53E3E)" :
    status === "connecting" ? "var(--orange, #ED8936)" :
    status === "pending_approval" ? "var(--yellow, #D69E2E)" :
    "var(--fill-quaternary)";

  const animate = status === "connecting" || status === "pending_approval";
  // ...
}
```

## 数据流

```
PluginsView
  ├── Tab Bar: [MCP Servers] [Skills] [Channels]
  │
  ├── MCP Tab
  │     ├── Header: [+ Add Server] [↻ Reload All]
  │     │     ├── + Add → AddPluginModal → plugins.add WS → gateway → connect + register
  │     │     └── ↻ Reload All → plugins.reload_all WS → gateway → reload_mcp_servers
  │     │
  │     ├── PendingApprovalSection（项目级待审批）
  │     │     ├── 批准 → plugins.approve_project_mcp WS → set_approval(Approved) → connect
  │     │     └── 拒绝 → plugins.reject_project_mcp WS → set_approval(Rejected)
  │     │
  │     ├── User Scope Group
  │     │     └── PluginRow → toggle / restart / expand (tools)
  │     │
  │     └── Project Scope Group
  │           └── PluginRow → toggle / restart / expand (tools)
  │
  ├── Skills Tab
  │     ├── Header: [↑ Upload] [↻ Refresh]
  │     ├── Filter Toggle: [Skills (n)] [Tools (m)]
  │     ├── Skills view → api.listSkills() → Global / Agent 分组
  │     └── Tools view → api.listTools() → 扁平列表
  │
  └── Channels Tab
        ├── Header: [↻ Refresh]
        ├── ChannelCard × N → connect / disconnect / configure
        └── WeChat → WechatQrModal 扫码流程

Event Flow:
  gateway → plugins.status_changed event → plugin-store → re-render (MCP tab)
  gateway → channels.changed event → re-fetch channels (Channels tab)
```

## 影响的文件

| 文件 | 变更 |
|------|------|
| `plugins/PluginsView.tsx` | 三 Tab 布局、Header 按钮、AddPluginModal、审批 UI、分组、EmptyState、Skills/Channels 内容 |
| `lib/stores/plugin-store.ts` | 新增 addPlugin、removePlugin、approveProjectMcp、rejectProjectMcp、reloadAll + skills/channels state |
| `lib/transport.ts` | PluginSummary 扩展 + 新增 plugin API 函数 |
| `connections/ConnectionsPage.tsx` | **删除整个文件**（MCP → PluginsView MCP tab，Channels → PluginsView Channels tab） |
| `settings/McpManager.tsx` | **删除整个文件** |
| `settings/SkillsTab.tsx` | **删除整个文件**（迁移到 PluginsView Skills tab） |
| `settings/SettingsPanel.tsx` | 移除 Skills tab 引用 |
| `xiaolin-gateway/src/ws/plugins.rs` | 新增 add、remove、approve、reject、reload_all handler |

## 代码分析结论（2026-06-15 Explore）

### 可复用组件清单

| 源文件 | 组件 | 迁移目标 Tab | 复用程度 |
|--------|------|-------------|---------|
| `ConnectionsPage.tsx` L54-137 | `McpCard` | MCP (可选，当前 PluginRow 已足够) | 参考 |
| `ConnectionsPage.tsx` L540-790 | `McpDetailModal` | MCP | 直接迁移 |
| `ConnectionsPage.tsx` L1207-1324 | `AddMcpModal` | MCP | 增强后迁移（需加 transport 选择 + env 编辑） |
| `ConnectionsPage.tsx` L155-248 | `ChannelCard` | Channels | 直接迁移 |
| `ConnectionsPage.tsx` L252-527 | `WechatQrModal` | Channels | 直接迁移 |
| `ConnectionsPage.tsx` L794-1190 | `ChannelDetailModal` | Channels | 直接迁移 |
| `SkillsTab.tsx` 全文 | `SkillsTab` | Skills | 整体迁移（移除 Settings 依赖） |

### API 层就绪状态

| 功能 | API 函数 | 文件 | 状态 |
|------|---------|------|------|
| MCP 列表 | `listPlugins()` | `transport.ts` | ✅ |
| MCP 启用/禁用 | `enablePlugin()/disablePlugin()` | `transport.ts` | ✅ |
| MCP 重启 | `restartPlugin()` | `transport.ts` | ✅ |
| MCP 工具列表 | `getPluginTools()` | `transport.ts` | ✅ |
| MCP 添加 | `addMcpServer()` | `transport.ts` via `api.ts` | ✅ |
| MCP 删除 | `removeMcpServer()` | `transport.ts` via `api.ts` | ✅ |
| MCP 详情 | `mcpDetail()` | `transport.ts` | ✅ |
| MCP 状态事件 | `onPluginsStatusChanged()` | `transport.ts` | ✅ |
| Skills 列表 | `listSkills()` | `api.ts` | ✅ |
| Skills 刷新 | `refreshSkills()` | `api.ts` | ✅ |
| Skills 上传 | `uploadSkill()` | `api.ts` | ✅ |
| Tools 列表 | `listTools()` | `api.ts` | ✅ |
| Channels 列表 | `channelsList()` | `transport.ts` via `api.ts` | ✅ |
| Channels 连接/断开 | `channelsConnect()/channelsDisconnect()` | `transport.ts` | ✅ |
| Channels 详情 | `channelsDetail()` | `transport.ts` | ✅ |
| WeChat 登录 | `channelsWechatLogin()/Poll/Verify` | `transport.ts` | ✅ |
| Channels 更新 | `channelsUpdate()` | `transport.ts` | ✅ |
| Channels 恢复 | `channelsRestore()` | `transport.ts` | ✅ |
| Channels 事件 | `onChannelsChanged()` | `transport.ts` | ✅ |

### 外部引用检查

| 文件 | 被引用次数 | 安全删除 |
|------|----------|---------|
| `settings/McpManager.tsx` | 0 | ✅ |
| `connections/ConnectionsPage.tsx` | 0 | ✅ |
| `settings/SkillsTab.tsx` | 1（SettingsPanel.tsx L9, L106） | ✅（需同步移除引用） |

### 风险与注意事项

1. **SkillsTab 依赖 `SectionTitle`**：从 `settings/SettingsShared.tsx` 导入，迁移时需替换为 PluginsView 自有的样式
2. **SkillsTab 依赖 `useGatewayStore`**：判断 gateway 是否就绪，PluginsView 中需保留此逻辑
3. **i18n key 分布**：SkillsTab 用 `settings` namespace，ConnectionsPage 用 `common` namespace，迁移时需确保两组 key 都可用
4. **ChannelCard 的 `STATUS_CONFIG` + `CAP_LABELS`**：硬编码在 ConnectionsPage 中，迁移时需提取为共享常量
5. **WechatQrModal 的 poll 间隔**：1500ms，需确保在 PluginsView 卸载时正确 cleanup

## 测试计划

### MCP Tab
1. **E2E**：从 MCP tab 添加 stdio MCP server → 连接成功 → 工具列表显示
2. **E2E**：从 MCP tab 添加 SSE MCP server → 连接成功
3. **E2E**：从 MCP tab 删除 MCP server → 断开连接 → 从列表消失
4. **E2E**：项目配置中的 MCP server 显示为 pending → 批准后连接
5. **E2E**：Reload All → 所有 server 重连

### Skills Tab
6. **E2E**：Skills tab 显示 global 和 agent skills 分组
7. **E2E**：Skills/Tools 切换正常
8. **E2E**：Upload skill folder → 刷新后新 skill 出现
9. **E2E**：Refresh 按钮触发 skills 重新加载

### Channels Tab
10. **E2E**：Channels tab 显示所有可用 channel 及其状态
11. **E2E**：连接 channel → 状态更新为 connected
12. **E2E**：WeChat 扫码流程正常工作

### 清理验证
13. **编译**：Settings 面板中不再有 Skills tab
14. **编译**：McpManager.tsx 已删除，无引用
15. **编译**：ConnectionsPage.tsx 已删除，无引用
16. **单测**：plugin-store 的 addPlugin 调用正确的 transport API
