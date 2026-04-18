# OpenClaw 运行架构研究报告

> **版本**: 基于 openclaw v2026.4.14 源码分析  
> **日期**: 2026-04-16  
> **仓库**: https://github.com/openclaw/openclaw

---

## 1. 项目概述

**OpenClaw** 是一个开源的个人 AI 助手平台，运行在用户自有设备上。核心理念是"本地优先、通道无关、单一控制面"——通过一个长驻 Gateway 进程连接 25+ 即时通讯平台（WhatsApp、Telegram、Slack、Discord、Signal、WeChat 等），提供统一的 AI 助手体验。

### 关键定位
- **个人 AI 助手**：单用户、本地运行、数据自主
- **多通道聚合**：一个 Gateway 接入所有聊天平台
- **工具增强**：浏览器、Canvas、Cron、MCP 等丰富工具链
- **插件化架构**：101+ 扩展插件覆盖通道/模型/内存/工具

### 技术栈
| 层次 | 技术选型 |
|------|---------|
| 核心语言 | TypeScript 89.9%，Swift 5%（macOS/iOS） |
| 运行时 | Node.js 24（推荐）/ Node.js 22.16+ |
| 包管理 | pnpm workspace（monorepo） |
| 协议验证 | TypeBox（JSON Schema + 类型推导） |
| 配置格式 | JSON5 + 环境变量替换 + 热重载 |
| 构建工具 | tsdown（ESBuild 封装） |
| 测试 | Vitest |
| 部署 | Docker / systemd / launchd |

---

## 2. 整体架构

### 2.1 架构分层

```
┌─────────────────────────────────────────────────────┐
│              客户端层（Clients）                       │
│  macOS App · iOS/Android Node · CLI · WebChat · IDE  │
└─────────────┬───────────────────────────┬───────────┘
              │ WebSocket / HTTP          │
┌─────────────▼───────────────────────────▼───────────┐
│                Gateway 核心进程                       │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌────────┐ │
│  │ HTTP 服务 │ │ WS 服务  │ │ 路由引擎 │ │ Auth   │ │
│  └──────────┘ └──────────┘ └──────────┘ └────────┘ │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌────────┐ │
│  │通道管理器 │ │Agent调度 │ │ 插件注册 │ │ Canvas │ │
│  └──────────┘ └──────────┘ └──────────┘ └────────┘ │
└──┬──────────────────┬──────────────────┬────────────┘
   │                  │                  │
┌──▼──────┐    ┌──────▼──────┐    ┌──────▼──────┐
│ 通道适配 │    │ Agent 执行  │    │ 存储/配置   │
│ 25+ 平台 │    │ Pi Runner   │    │ Session     │
│ Plugin   │    │ Tools Loop  │    │ Config      │
└──────────┘    └─────────────┘    └─────────────┘
```

### 2.2 核心设计原则

1. **单进程架构**：Gateway 是唯一的长驻进程，所有通道、Agent、工具运行在同一 Node.js 进程内
2. **配置驱动**：Agent 不是独立进程，而是配置+会话键+工作区的运行时组合
3. **插件优先**：核心保持精简，能力通过 extensions/ 下的插件注入
4. **本地优先**：默认绑定 loopback，支持 LAN/Tailscale/SSH 隧道远程访问
5. **安全默认**：DM 配对审批、沙箱隔离、危险工具默认拒绝

---

## 3. 核心模块详解

### 3.1 Gateway 服务 (`src/gateway/`)

Gateway 是整个系统的控制面，由 `startGatewayServer()` 启动：

**HTTP 服务** (`server-http.ts`)：
- Control UI 和 WebChat 页面
- OpenAI 兼容 API（`/v1/chat/completions`、`/v1/responses`）
- 工具调用端点（`/tools/invoke`）
- Canvas/A2UI 路径（`/__openclaw__/canvas/`）
- 插件自定义 HTTP 路由
- 健康检查（`/healthz`）

**WebSocket 服务** (`server/ws-connection/`)：
- 控制面协议：`connect` → `req/res` + `event`
- 客户端管理：`Set<GatewayWsClient>` 跟踪连接
- 事件广播：`createGatewayBroadcaster` 推送到所有订阅者
- Node 集成：`createGatewayNodeSessionRuntime` 处理移动端设备

**Gateway 方法** (`server-methods/`)：
- 核心方法 + 通道插件方法合并
- 支持 exec/plugin approval 等额外处理器

### 3.2 通道系统 (`src/channels/`)

**通道插件接口** (`ChannelPlugin`)：

```typescript
interface ChannelPlugin {
  config?: ChannelConfigAdapter;
  gateway?: {
    startAccount(params): Promise<void>;
    gatewayMethods?: Record<string, Handler>;
  };
  outbound?: {
    sendText(params): Promise<void>;
    sendMedia(params): Promise<void>;
  };
  messaging?: MessagingAdapter;
  pairing?: PairingAdapter;
  auth?: AuthAdapter;
  heartbeat?: HeartbeatAdapter;
  directory?: DirectoryAdapter;
}
```

**通道管理器** (`server-channels.ts`)：
- `createChannelManager()` 创建管理实例
- 每个通道/账号独立 `startAccount` 任务
- 自动重连（带退避）+ 健康监控
- 手动停止/重启控制

**内置通道**（25+ 平台）：
WhatsApp、Telegram、Slack、Discord、Google Chat、Signal、iMessage、BlueBubbles、IRC、Microsoft Teams、Matrix、Feishu、LINE、Mattermost、Nextcloud Talk、Nostr、Synology Chat、Tlon、Twitch、Zalo、WeChat、QQ、WebChat 等。

### 3.3 路由系统 (`src/routing/`)

路由是多通道架构的核心桥梁，负责将入站消息映射到正确的 Agent 和 Session。

**分层匹配策略**（按优先级，源自 `resolve-route.ts` 的 tiers 数组）：
1. `binding.peer` — 精确 peer 匹配
2. `binding.peer.parent` — 父 peer 匹配（线程场景）
3. `binding.peer.wildcard` — Peer 通配符匹配
4. `binding.guild+roles` — Guild + 成员角色匹配
5. `binding.guild` — Guild 匹配
6. `binding.team` — Team 匹配
7. `binding.account` — Account 作用域匹配
8. `binding.channel` — Channel 全局匹配
9. 默认 Agent（`resolveDefaultAgentId(cfg)`）

**路由输出**：
```typescript
interface ResolvedAgentRoute {
  agentId: string;
  sessionKey: string;
  mainSessionKey: string;
  lastRoutePolicy: RoutePolicy;
  matchedBy: 'peer' | 'guild' | 'team' | 'account' | 'channel' | 'default';
}
```

### 3.4 Agent 执行引擎 (`src/agents/`)

Agent 不是独立的对象或进程，而是**运行时组装**的执行单元：

**执行管道**：
```
入站消息 → dispatchInboundMessage
  → resolveAgentRoute（路由解析）
  → dispatchReplyFromConfig（分发调度）
    → 去重 · hooks · 发送策略
    → loadSessionStore（会话加载）
    → getReplyFromConfig（获取回复）
      → runPreparedReply（准备执行）
        → Pi Embedded Runtime（模型-工具循环）
          → LLM API 调用
          → 工具执行（带前置钩子）
          → 结果返回 → 继续推理...
        → ReplyPayload 输出
```

**Agent 配置要素**：
- `agentId`：配置标识（默认 `"main"`）
- `sessionKey`：路由生成的会话键
- `workspace`：工作区目录（默认 `~/.openclaw/workspace`）
- `model`：模型提供者 + 模型 ID
- `skills`：注入的技能集合
- `tools`：可用工具列表

### 3.5 会话管理 (`src/sessions/` + `src/config/sessions/`)

**双层设计**：
1. **轻量层**（`src/sessions/`）：打字状态、发送策略、事件总线、会话 ID 工具
2. **持久层**（`src/config/sessions/`）：磁盘存储的 `SessionEntry`，带写锁、缓存、维护剪枝

**会话键格式**：
- `agent:<agentId>:<mainKey>`：Agent 主会话（如 `agent:main:main`）
- `agent:<agentId>:channel:<channelId>:...`：路由生成的通道作用域会话（可含 thread、guild 等段）
- `acp:<uuid>`：ACP 桥接会话
- 长度 1-512 字符，由路由系统根据通道/账号/peer 信息动态构建

### 3.6 工具系统

**工具组装**：
```
createOpenClawCodingTools()
  ├── Pi Coding Agent 基础工具（read/write/edit/exec）
  └── createOpenClawTools()
       ├── 内置工具（canvas/cron/gateway/message/sessions/web_search/TTS/nodes）
       └── resolveOpenClawPluginToolsForOptions()
            └── resolvePluginTools()（插件注册的工具）
```

**工具安全**：
- `beforeToolCallHook`：全局前置钩子可拦截/修改参数
- 危险工具列表：`exec`、`spawn`、`shell`、`fs_write`、`fs_delete`、`fs_move`、`sessions_*`、`cron` 等默认拒绝 HTTP 调用
- 沙箱策略：按 Agent/Session 决定 Docker 隔离级别

### 3.7 插件系统 (`src/plugins/`)

**生命周期**：
```
发现（discover） → 清单解析（manifest） → 激活判定（activate）
  → 注册表构建（PluginRegistry） → 运行时同步（pin/setActive）
```

**插件能力维度**：
| 维度 | 说明 | 示例 |
|------|------|------|
| 通道 | 聊天平台适配器 | telegram、slack、discord |
| 模型提供者 | LLM API 接入 | openai、anthropic、google |
| 内存 | 长期记忆存储（单激活） | memory-* 系列 |
| 工具 | Agent 可调用的工具 | browser、voice-call |
| 钩子 | 事件前后拦截 | session-memory、boot-md |
| HTTP 路由 | 自定义 API 端点 | 插件专属接口 |
| CLI 命令 | 子命令注入 | 插件管理命令 |

**插件数量**：`extensions/` 下包含 **101+** 个 `openclaw.plugin.json` 清单。

### 3.8 MCP 集成 (`src/mcp/`)

两个独立的 MCP 服务面：

**Channel MCP**（`channel-server.ts`）：
- IDE/Claude 通过 stdio 连接 Gateway
- 提供：会话列表、消息读取、消息发送、审批管理
- 客户端通过 `OpenClawChannelBridge` 建立 Gateway WS 连接

**Plugin Tools MCP**（`plugin-tools-serve.ts`）：
- ACP/Claude Code 访问插件注册的 Agent 工具
- ListTools/CallTool 处理器
- 工具前置钩子（`wrapToolWithBeforeToolCallHook`）

### 3.9 Canvas & A2UI (`src/canvas-host/`)

- HTTP 服务提供可视工作区
- WebSocket 实时重载（基于 chokidar 文件监控）
- A2UI 协议：`a2ui_push`、`a2ui_reset`
- Agent 工具：`present`、`hide`、`navigate`、`eval`、`snapshot`

### 3.10 Cron 调度 (`src/cron/`)

- `CronService`：任务 CRUD、定时器循环、心跳唤醒
- 安全防护：最大定时器分片、最小重触发间隔、任务超时
- 隔离 Agent 运行：完整 Agent 上下文（模型/会话/技能快照）

---

## 4. 消息流转完整路径

```
  ┌──────────┐
  │ 用户发送  │ WhatsApp / Telegram / Slack / ...
  │ 一条消息  │
  └─────┬────┘
        │
  ┌─────▼────────────────┐
  │ 通道适配器             │ ChannelPlugin.startAccount 内的消息监听
  │ 构建 MsgContext       │ 解析发送者、内容、附件、线程信息
  └─────┬────────────────┘
        │
  ┌─────▼────────────────┐
  │ 路由解析               │ resolveAgentRoute()
  │ channel + account     │ 分层匹配 → agentId + sessionKey
  │ + peer → route        │
  └─────┬────────────────┘
        │
  ┌─────▼────────────────┐
  │ 消息分发               │ dispatchReplyFromConfig()
  │ 去重 · hooks          │ 检查重复消息、执行前置钩子
  │ 发送策略              │ 判定是否回复、打字状态
  └─────┬────────────────┘
        │
  ┌─────▼────────────────┐
  │ 会话加载               │ loadSessionStore()
  │ 历史上下文恢复         │ resolveSessionStoreEntry()
  └─────┬────────────────┘
        │
  ┌─────▼────────────────┐
  │ Agent Runner          │ runPreparedReply()
  │ ┌─────────────────┐  │ 组装 Prompt（AGENTS.md + SOUL.md + 历史）
  │ │ LLM API 调用    │  │ 调用模型获取回复
  │ └────┬────────────┘  │
  │      │ 需要工具？     │
  │ ┌────▼────────────┐  │
  │ │ 工具执行循环     │  │ beforeToolCall hook → execute → 结果
  │ │ Browser/Canvas  │  │ 结果反馈给模型继续推理
  │ └────┬────────────┘  │
  │      │ 完成          │
  └──────┼───────────────┘
         │
  ┌──────▼───────────────┐
  │ 回复分发               │ ReplyPayload
  │ → 通道 outbound       │ sendText/sendMedia 返回原通道
  │ → WS broadcast        │ 推送给所有 WS 客户端
  │ → hooks 触发          │ 触发后置钩子
  └──────────────────────┘
```

---

## 5. 部署架构

### 5.1 推荐部署

```
┌─────────────────────────────────────────┐
│              用户设备                     │
│                                         │
│  ┌──────────────┐  ┌─────────────────┐ │
│  │ Gateway 进程  │  │ Supervisor      │ │
│  │ :18789       │  │ launchd/systemd │ │
│  └──────────────┘  └─────────────────┘ │
│                                         │
│  ┌──────────────┐  ┌─────────────────┐ │
│  │ ~/.openclaw/  │  │ macOS App       │ │
│  │  config      │  │ (可选)          │ │
│  │  workspace   │  └─────────────────┘ │
│  │  sessions    │                      │
│  └──────────────┘                      │
└─────────────────────────────────────────┘
```

### 5.2 Docker 部署

```yaml
services:
  openclaw-gateway:
    # 主 Gateway 服务
    command: node dist/index.js gateway --bind lan --port 18789
    ports: ["18789:18789", "18790:18790"]
    healthcheck: GET /healthz

  openclaw-cli:
    # CLI 附属容器，共享网络命名空间
    network_mode: "service:openclaw-gateway"
    stdin_open: true
    tty: true
```

### 5.3 远程访问

- **Tailscale**：推荐方案，端到端加密
- **SSH 隧道**：`ssh -L 18789:localhost:18789 user@host`
- **LAN 绑定**：`--bind lan`（需配合 Auth token）

---

## 6. 安全模型

### 6.1 信任层次

| 层次 | 策略 | 说明 |
|------|------|------|
| main 会话 | 完全信任 | 本地用户，工具无限制 |
| 非 main 会话 | 可配沙箱 | `sandbox.mode: "non-main"` → Docker 隔离 |
| 入站 DM | 配对审批 | `dmPolicy="pairing"` → 配对码确认 |
| HTTP 工具调用 | 危险工具拒绝 | exec/shell/fs_* 等默认不可通过 HTTP 调用 |
| 远程访问 | 强制认证 | Token/Password + 设备审批 |

### 6.2 沙箱能力

- **Docker 沙箱**：每 Session 独立容器
- **允许工具**：bash、process、read、write、edit、sessions_*
- **拒绝工具**：browser、canvas、nodes、cron、discord、gateway
- **文件系统隔离**：工作区绑定挂载
- **安全审计**：`openclaw doctor` 检查配置风险

---

## 7. 性能与扩展性分析

### 7.1 架构优势

1. **单进程简洁性**：避免 IPC 开销，内存共享，调试友好
2. **配置驱动的弹性**：无需重启即可调整路由/Agent/通道
3. **插件化低耦合**：核心精简，能力按需加载
4. **本地优先低延迟**：无需云端中转，直连 LLM API

### 7.2 潜在瓶颈

1. **单进程限制**：所有通道共享一个 Node.js 事件循环，高并发场景可能阻塞
2. **内存压力**：101+ 插件全加载时的内存占用
3. **会话存储**：基于文件的 SessionStore 在大规模会话下的 I/O 性能
4. **单 Gateway 限制**：一台机器默认只运行一个 Gateway 实例

### 7.3 扩展路径

- **多 Gateway**：支持不同端口/配置路径运行多个实例
- **远程沙箱**：SSH 远程沙箱后端
- **插件按需加载**：激活规则控制实际加载的插件数
- **配置热重载**：支持 `off`、`hot`、`restart`、`hybrid` 多种模式

---

## 8. 与同类项目对比

| 特性 | OpenClaw | AutoGPT | Claude Desktop |
|------|----------|---------|----------------|
| 部署模式 | 本地 Gateway | 云端/本地 | 桌面应用 |
| 多通道支持 | 25+ 平台 | 无原生支持 | 仅本地 |
| 工具系统 | 内置 + 插件 + MCP | 内置 | MCP |
| 自定义 Agent | 配置驱动 | 代码驱动 | 有限 |
| 插件生态 | 101+ 扩展 | 社区 | MCP 服务器 |
| Voice/TTS | 内置（ElevenLabs 等） | 无 | 无 |
| 移动端支持 | iOS/Android Node | 无 | 无 |
| 安全沙箱 | Docker per-session | 无 | 无 |
| 开源协议 | MIT | MIT | 闭源 |

---

## 9. 关键发现与结论

### 9.1 架构亮点

1. **优雅的通道抽象**：`ChannelPlugin` 接口将 25+ 平台统一为一致的适配器模式，新通道接入成本低
2. **智能路由系统**：分层匹配策略优雅地处理了从个人 DM 到群组频道的所有场景
3. **Agent 即配置**：避免了重量级 Agent 对象的管理开销，配置热重载使得 Agent 调整即时生效
4. **安全默认思维**：配对审批、沙箱隔离、危险工具拒绝等安全机制作为默认行为而非可选项
5. **MCP 双面设计**：Channel MCP 面向 IDE 集成，Plugin Tools MCP 面向 ACP 协议，各司其职

### 9.2 值得关注的设计决策

1. **TypeScript 单体**：选择 TS 而非 Python/Go，牺牲了部分性能换取了极高的可扩展性和社区贡献友好度
2. **文件系统持久化**：会话和配置使用文件而非数据库，简化了部署但限制了规模
3. **单进程模型**：简化了状态管理但可能成为高负载下的瓶颈
4. **内存插件单激活**：避免冲突但限制了混合记忆策略

### 9.3 适用场景

- ✅ 个人 AI 助手：最佳场景，本地运行，数据自主
- ✅ 多平台消息聚合：统一入口管理所有通讯
- ✅ 开发者 AI 工作流：MCP/ACP 集成 IDE
- ⚠️ 团队协作：需要仔细配置多 Agent 路由和安全策略
- ❌ 高并发公共服务：单进程架构不适合大规模公共部署

---

## 10. 代码统计

| 指标 | 数值 |
|------|------|
| 总提交数 | 31,557 |
| 贡献者 | 500+ |
| GitHub Stars | 358k |
| Forks | 72.8k |
| src/ 模块数 | 60+ 目录 |
| 插件数量 | 101+ |
| 支持通道 | 25+ |
| 文档页面 | 481+ |
| 主要语言 | TypeScript (89.9%) |
| 许可证 | MIT |

---

*本报告基于 OpenClaw 公开源码静态分析生成，不涉及运行时测试。*
