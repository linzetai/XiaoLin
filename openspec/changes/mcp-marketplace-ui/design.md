## Context

XiaoLin 的 MCP 管理目前是"已安装列表"视图（PluginsView MCP Tab），用户添加 Server 只能通过：
1. 手动编辑 `~/.xiaolin/config/default.json` 的 `mcpServers` 数组
2. Agent 内置工具 `manage_mcp_server`（需要对话中触发）
3. 项目级 `.xiaolin/mcp.json`（需要手写 JSON）

后端 `mcp.add` / `mcp.remove` WebSocket API 已完备，前端 `transport.addMcpServer` 仅暴露了 `(id, command, args)` 三参数。Codex 已提供 Plugin Directory + 一键安装体验。

## Goals / Non-Goals

**Goals:**
- 用户可在 MCP Tab 内浏览热门 MCP Server 目录并一键安装（无需编辑 JSON）
- 用户可通过 GUI 表单添加自定义 MCP Server（支持所有 transport 类型）
- 用户可查看已安装 Server 的完整配置、工具列表并执行删除操作
- 前端 API 层与后端已有能力完全对齐（transport/url/env 透传）

**Non-Goals:**
- 在线 Marketplace 服务（Phase 1 仅本地 JSON 注册表）
- 自动安装 npm/pip 依赖（注册表提供 installHint 文案指引，用户自行安装运行时）
- MCP Server 版本管理或自动更新
- OAuth 认证流程（后续 T23+ 覆盖）

## Decisions

### D1: 数据源 — 本地 JSON 注册表 vs 远程 API

**选择**: 本地 JSON（`src/data/mcp-registry.json`），随 App 打包分发

**理由**:
- 无需搭建后端服务，零运维成本
- 用户离线也能浏览目录
- 注册表数据量小（~15 条），JSON 足矣
- 未来扩展为远程 API 只需替换数据源，UI 层不变

**备选**: 远程 API + CDN 缓存 → 需要服务端，Phase 1 不必要

### D2: UI 架构 — Installed/Explore 子切换 vs 独立 Tab

**选择**: MCP Tab 内部 Installed/Explore 子切换（pill toggle），不新增顶级 Tab

**理由**:
- 保持 MCP/Skills/Channels 三 Tab 简洁层级
- Explore 是 MCP 的发现功能，归属 MCP Tab 语义正确
- 参考 Codex: Plugin Directory 也是在 Plugins 入口内切换

**备选**: 独立 "Marketplace" Tab → 增加顶级导航项，过重

### D3: 组件拆分策略

将 PluginsView.tsx 中新增的大块功能拆分为独立文件：
- `McpExplorePanel.tsx` — Explore 视图（搜索 + 分类 + 卡片网格）
- `AddServerModal.tsx` — 添加模态框（transport 表单 + env 编辑器）
- `McpDetailModal.tsx` — 详情模态框（配置 + 工具 + 删除）

PluginsView.tsx 只负责子视图切换和 Header 编排。

### D4: 注册表条目结构

```typescript
interface McpRegistryEntry {
  id: string;
  name: string;
  description: string;
  category: "development" | "productivity" | "data" | "communication";
  icon: string;        // Phosphor icon 组件名
  transport: "stdio" | "sse" | "streamable_http";
  command?: string;
  args?: string[];
  url?: string;
  env?: Record<string, string>;
  installHint?: string;
  homepage?: string;
}
```

`icon` 用 Phosphor 图标名字符串，运行时映射到组件。避免在 JSON 中嵌入 SVG。

### D5: AddServerModal transport 动态表单

根据选择的 transport 类型动态显示不同字段：
- **Stdio**: command (必填) + args (可选，逗号/空格分隔)
- **SSE / Streamable HTTP**: url (必填)
- **通用**: id (必填) + env 键值对编辑器 (可选)

### D6: transport.addMcpServer 签名扩展

从 `addMcpServer(id, command, args)` 改为对象参数形式：

```typescript
addMcpServer(params: {
  id: string;
  command?: string;
  args?: string[];
  transport?: string;
  url?: string;
  env?: Record<string, string>;
}): Promise<{ ok: boolean; id: string; status?: McpServerStatus }>
```

后端 `handle_mcp_add` 已支持全部参数，仅需前端 WebSocket 层透传。保持向后兼容：旧调用方式不存在（内部 API，无外部消费者）。

## Risks / Trade-offs

- **[注册表过时]** → 内置 JSON 无法自动更新 → 缓解：保持 `installHint` 引导用户确认版本；后续版本可添加远程更新检查
- **[installHint 不够] 用户不知道如何安装 npm 全局包** → 缓解：installHint 包含完整的 `npx` 或 `pip` 命令，Explore 卡片显示 "需先运行" 提示
- **[transport.addMcpServer 签名变更]** → 仅内部 API，无外部消费者 → 风险极低
- **[注册表条目的 command 路径不同平台不同]** → 缓解：使用 `npx` 方式（自动查找 node_modules/.bin），避免硬编码绝对路径
