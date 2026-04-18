# FastClaw 架构设计文档

> 版本：0.1.0 | 更新日期：2026-04-20

## 1. 项目概述

FastClaw 是一个基于 **Rust** 的高性能 AI Agent 编排引擎，编译为**单一可部署二进制** `fastclaw`。它将网关、路由、记忆、DAG 工作流、多智能体协作与安全扩展统一集成，面向生产环境提供完整的 AI Agent 基础设施。

### 1.1 设计目标

| 目标 | 实现策略 |
|------|---------|
| **高性能** | Rust 零成本抽象 + Tokio 异步运行时 + 单二进制部署 |
| **生产就绪** | 健康探针、Prometheus 指标、优雅关停、热重载 |
| **多渠道接入** | 7 类即时通讯渠道原生扩展（飞书/Telegram/Discord/Slack/WhatsApp/Matrix/Teams） |
| **安全纵深** | 恒定时间密钥校验、HMAC 签名总线、SSRF 防御、注入防护、WASM 沙箱 |
| **可扩展** | WASM 插件、MCP 协议、Tool 注册表、Channel 注册表 |
| **自进化** | 轨迹记录 → 技能提取 → 技能注入 → 退役的完整生命周期 |

### 1.2 技术选型

| 领域 | 技术栈 |
|------|--------|
| 语言/运行时 | Rust 2021 Edition / Tokio |
| HTTP 框架 | Axum 0.7 + Tower 中间件 |
| 数据库 | SQLite (WAL 模式，via sqlx) |
| 序列化 | serde + serde_json + JSON5 |
| 向量搜索 | usearch (可选特性) / hypembed (纯 Rust 嵌入) |
| WASM 宿主 | wasmtime 19 (Component Model) |
| 图算法 | petgraph |
| 加密 | hmac + sha2 + constant_time_eq |
| 可观测性 | tracing + metrics + metrics-exporter-prometheus |
| TUI | ratatui + crossterm |
| CLI | clap 4 |

## 2. 系统架构

### 2.1 四面体架构

FastClaw 采用**四面体架构**，将系统清晰划分为四个正交的关注面，由横切关注点统一包裹：

| 面 | 职责 | 核心 crate |
|----|------|-----------|
| **接入层 (Ingress)** | 外部请求入口：REST API、WebSocket、渠道 Webhook | `fastclaw-gateway`, `extensions/*` |
| **控制面 (Control)** | 请求路由、Agent 调度、安全策略、模型选择 | `fastclaw-core`, `fastclaw-agent`, `fastclaw-security`, `fastclaw-model-router`, `fastclaw-agent-factory` |
| **数据面 (Data)** | 状态持久化：会话、记忆、DAG 检查点、定时任务 | `fastclaw-session`, `fastclaw-memory`, `fastclaw-dag`, `fastclaw-cron` |
| **扩展面 (Extensions)** | 能力扩展：WASM 插件、MCP 互操作、代码智能 | `fastclaw-plugin`, `fastclaw-collab`, `fastclaw-code`, `fastclaw-studio` |
| **横切 (Cross-cutting)** | 可观测性、配置热重载 | `fastclaw-observe`, `fastclaw-core::config` |

### 2.2 Cargo 工作区结构

```
FastClaw/
├── Cargo.toml                    # 工作区根 Manifest
├── crates/
│   ├── fastclaw-cli/             # 二进制入口：TUI、网关守护、MCP 服务
│   ├── fastclaw-core/            # 核心类型：配置、路由、工具注册、消息总线
│   ├── fastclaw-gateway/         # Axum HTTP/WS 网关、Webhook、REST
│   ├── fastclaw-agent/           # Agent 运行时、LLM 提供商、内置工具
│   ├── fastclaw-session/         # SQLite WAL 会话、TTL、压缩钩子
│   ├── fastclaw-memory/          # 工作/情景/语义三层记忆 + 向量 + 图
│   ├── fastclaw-dag/             # DAG 定义、执行器、检查点
│   ├── fastclaw-plugin/          # WASM 宿主、签名校验、热重载
│   ├── fastclaw-evolution/       # 反馈、评估、蒸馏、技能生命周期
│   ├── fastclaw-observe/         # Prometheus 渲染、tracing 辅助
│   ├── fastclaw-security/        # API 密钥、限流、注入防护、SSRF
│   ├── fastclaw-agent-factory/   # Agent 模板 + 意图路由 + 动态路由
│   ├── fastclaw-studio/          # FlowDSL 编译器、Studio WS 协议、版本化
│   ├── fastclaw-collab/          # CollabHub、MCP 客户端/服务端、协作模式
│   ├── fastclaw-model-router/    # 模型路由策略、复杂度分层、预算
│   ├── fastclaw-context/         # 六层上下文拼装、滚动压缩、用户画像
│   ├── fastclaw-self-iter/       # 执行诊断、沙箱校验、自动修复
│   ├── fastclaw-code/            # Tree-sitter、CodeGraph、测试运行器、补丁引擎
│   └── fastclaw-cron/            # Cron 持久化 + 崩溃恢复
├── extensions/
│   ├── feishu/                   # 飞书（最完整的渠道实现）
│   ├── telegram/                 # Telegram
│   ├── discord/                  # Discord
│   ├── slack/                    # Slack
│   ├── whatsapp/                 # WhatsApp
│   ├── matrix/                   # Matrix
│   └── msteams/                  # Microsoft Teams
├── config/                       # JSON5 模板 + Agent 配置
├── docs/                         # Markdown 文档树
├── Dockerfile                    # 多阶段构建
└── docker-compose.yml            # 容器编排
```

### 2.3 Crate 依赖关系

```
fastclaw-cli (二进制入口)
  └─ fastclaw-gateway (HTTP/WS 网关)
       ├─ fastclaw-core (核心抽象)
       │    ├─ 配置、类型、错误
       │    ├─ Tool / ToolRegistry
       │    ├─ ChannelPlugin / ChannelRegistry
       │    ├─ Router / resolve_route
       │    ├─ MessageBus (HMAC)
       │    └─ ComplexityTier / Skill / Hub
       ├─ fastclaw-agent (Agent 运行时)
       │    ├─ LlmProvider (trait)
       │    ├─ AgentRuntime (工具循环)
       │    ├─ SubAgentTool (委托)
       │    └─ builtin_tools (网络/Shell/文件等)
       ├─ fastclaw-session (会话)
       ├─ fastclaw-memory (记忆)
       ├─ fastclaw-dag (工作流)
       ├─ fastclaw-plugin (WASM)
       ├─ fastclaw-evolution (进化)
       ├─ fastclaw-observe (可观测)
       ├─ fastclaw-security (安全)
       ├─ fastclaw-model-router (模型路由)
       ├─ fastclaw-context (上下文)
       ├─ fastclaw-self-iter (自迭代)
       ├─ fastclaw-collab (协作/MCP)
       ├─ fastclaw-code (代码智能)
       ├─ fastclaw-cron (定时任务)
       ├─ fastclaw-agent-factory (Agent 工厂)
       ├─ fastclaw-studio (可视化编排)
       └─ extensions/* (渠道扩展)
```

## 3. 核心抽象与接口

### 3.1 关键 Trait（接口层）

FastClaw 通过 Rust trait + `async_trait` 宏实现面向接口编程，所有核心扩展点均为 trait 对象（`Arc<dyn Trait>`）。

#### Tool — 工具抽象

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> ToolParameterSchema;
    async fn execute(&self, arguments: &str) -> ToolResult;
    fn to_definition(&self) -> ToolDefinition; // OpenAI 兼容格式
}
```

`ToolRegistry` 使用 `HashMap<String, Arc<dyn Tool>>` 管理所有已注册工具。工具来源包括：
- **内置工具**（网络搜索、HTTP 请求、文件操作等）
- **渠道工具**（各渠道插件提供的专属工具）
- **WASM 插件工具**（`PluginTool`）
- **MCP 桥接工具**（`McpBridgedTool`，远程 MCP 服务暴露的工具）
- **子 Agent 工具**（`SubAgentTool`，委托另一个 Agent 执行）

#### LlmProvider — LLM 提供商

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat_completion(&self, params: &CompletionParams<'_>) -> anyhow::Result<ChatResponse>;
    async fn chat_completion_stream(
        &self,
        params: &CompletionParams<'_>,
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<StreamDelta>>>;
}
```

实现者：
- **OpenAI 兼容提供商**（覆盖 OpenAI/DeepSeek/DashScope/Ollama 等）
- **Anthropic 提供商**
- **FallbackProvider**（链式降级，逐个尝试直到成功）

每个提供商支持：**重试退避**、**并发信号量**、**可配超时**。

#### ChannelPlugin — 渠道插件

```rust
#[async_trait]
pub trait ChannelPlugin: Send + Sync {
    fn meta(&self) -> &ChannelMeta;
    fn capabilities(&self) -> ChannelCapabilities;
    async fn verify_webhook(&self, headers: &BTreeMap<String, String>, raw_body: &[u8]) -> anyhow::Result<()>;
    async fn handle_webhook(&self, payload: serde_json::Value) -> anyhow::Result<WebhookResult>;
    async fn send_message(&self, msg: &OutboundMessage) -> anyhow::Result<serde_json::Value>;
    async fn reply_message(&self, message_id: &str, text: &str) -> anyhow::Result<serde_json::Value>;
    async fn reply_streaming_placeholder(&self, message_id: &str, text: &str) -> anyhow::Result<serde_json::Value>;
    async fn update_message(&self, message_id: &str, text: &str) -> anyhow::Result<serde_json::Value>;
    fn tools(&self) -> Vec<Arc<dyn Tool>>;
    async fn start(&self, inbound_tx: mpsc::UnboundedSender<InboundMessage>) -> anyhow::Result<()>;
}
```

支持 **Webhook** 和 **长连接**（如飞书 WebSocket）两种接入模式。

### 3.2 核心数据结构

| 结构 | 位置 | 职责 |
|------|------|------|
| `FastClawConfig` | `fastclaw-core::config` | 全局配置根，JSON5 反序列化 |
| `AgentConfig` | `fastclaw-core::agent_config` | 单个 Agent 的模型、行为、工具列表、MCP 服务器 |
| `ChatRequest` / `ChatResponse` | `fastclaw-core::types` | OpenAI 兼容的请求/响应类型 |
| `AppState` | `fastclaw-gateway::state` | 网关共享状态，持有所有注册表和运行时引用 |
| `AgentRuntime` | `fastclaw-agent::runtime` | Agent 执行运行时（工具循环引擎） |
| `Router` | `fastclaw-core::routing` | ChatRequest → AgentConfig 的解析器 |
| `MessageBus` | `fastclaw-core::bus` | 跨 Agent 消息：直发、广播、主题、请求-应答 |
| `FastClawError` | `fastclaw-core::error` | 统一错误类型（基于 thiserror） |

### 3.3 注册表模式

FastClaw 大量使用**注册表模式**管理动态组件：

| 注册表 | 管理对象 | 存储方式 |
|--------|---------|---------|
| `ToolRegistry` | `Arc<dyn Tool>` | `HashMap<String, Arc<dyn Tool>>` |
| `ChannelRegistry` | `Arc<dyn ChannelPlugin>` | `HashMap<String, Arc<dyn ChannelPlugin>>` |
| `PluginRegistry` | WASM 插件 | 插件目录扫描 + 热重载 |
| `SkillStore` | 进化技能 | 向量相似度检索 |
| `TrajectoryStore` | 执行轨迹 | 持久化存储 |

## 4. 请求处理流程

### 4.1 HTTP/WebSocket Chat 请求

这是 FastClaw 最核心的请求路径，完整流程如下：

**Phase 1 — 接入与安全**

1. 请求到达 Axum 路由栈
2. **CORS 中间件** 校验跨域策略
3. **TraceLayer** 记录请求级 span
4. **Gzip 压缩层** 自动处理
5. **限流中间件** (`RateLimiter`) 检查 IP 维度限流
6. **认证中间件** (`ApiKeyAuth`) 校验 `Authorization: Bearer` 或 `X-API-Key`

**Phase 2 — Chat Pipeline Setup**

7. `setup_chat()` 编排以下步骤：
   - **Router::resolve** — 根据 `agent_id` 解析 `AgentConfig`
   - **Session 解析** — 创建或恢复 SQLite 会话
   - **Context 注入** — `ContextEngine.ingest` 将历史消息合并
   - **模型路由** — `ModelRouter` 根据策略选择最优模型/提供商
   - **预算检查** — 原子预留 token 预算
   - 产出 `ChatSetup` 结构

**Phase 3 — Agent 执行**

8. `AgentRuntime::execute` / `execute_stream` 启动**工具循环**：
   - **技能注入** — 从 `SkillStore` 检索相关技能，注入 system prompt
   - **工具过滤** — 根据 Agent 的 allow/deny 策略过滤工具定义
   - **LLM 调用** — 通过 `LlmProvider` 请求 LLM
   - **工具执行** — 如果 LLM 返回 `tool_calls`，逐个执行
   - **自迭代恢复** — 连续工具失败时，`SelfIterEngine` 注入诊断指导
   - **轨迹记录** — 每一步记入 `TrajectoryStep`
   - 循环直到 LLM 不再请求工具调用，或达到 `max_tool_calls_per_turn` 上限

**Phase 4 — 后处理**

9. 记录预算实际消耗
10. 记录情景记忆 (episodic memory)
11. 写入会话历史
12. 记录轨迹到 `TrajectoryStore`
13. 更新技能使用统计

### 4.2 渠道 Webhook 请求

1. `POST /webhook/:channel_id` 到达网关
2. `ChannelRegistry.get(channel_id)` 查找渠道插件
3. `channel.verify_webhook()` — 平台级签名校验（HMAC/Ed25519 等）
4. `channel.handle_webhook()` — 解析为 `WebhookResult`
5. 对 `WebhookResult::Messages` 中的每条消息，异步派发：
   - `resolve_route()` — 五级优先级绑定匹配（Peer > Channel > Account > ChannelWild > Default）
   - `build_session_key()` — 根据 `DmScope` 生成会话隔离 key
   - 进入与 HTTP Chat 相同的 Agent 执行路径
6. 响应通过 `channel.send_message()` / `channel.reply_message()` 回传

### 4.3 定时任务 (Cron)

1. `CronScheduler` 后台 loop 检查到期任务
2. 通过 `JobTrigger` trait 的三种触发方式：
   - `trigger_agent_chat` — 构造 `ChatRequest`，走完整 Agent 路径
   - `trigger_dag_execute` — 直接执行 DAG 工作流
   - `trigger_webhook` — SSRF 安全的 HTTP 回调

## 5. 子系统详细设计

### 5.1 路由系统

FastClaw 实现了两套互补的路由机制：

**API 路由 (`Router`)**

针对 HTTP/WebSocket 的 `ChatRequest`：
- 优先使用请求中显式指定的 `agent_id`
- 其次使用默认 Agent（`main` 或首个配置项）
- 支持运行时动态注册/注销路由绑定

**渠道路由 (`resolve_route`)**

五级优先级匹配规则：

| 优先级 | 匹配层 (MatchTier) | 说明 |
|--------|-------------------|------|
| 5 (最高) | `Peer` | 精确匹配 DM/群组 ID |
| 4 | `Channel` | 匹配特定渠道 |
| 3 | `AccountId` | 匹配特定账号 |
| 2 | `ChannelWild` | 渠道通配（`accountId: "*"`） |
| 1 (最低) | `Default` | 回退到默认 Agent |

运行时绑定（`RuntimeRouteBinding`）优先于文件绑定，支持 API 动态管理。

### 5.2 Agent 运行时

`AgentRuntime` 是执行 Agent 的核心引擎，实现了**迭代式工具调用循环**：

```
┌─────────────────────────────────────────────────┐
│              AgentRuntime::execute               │
│                                                  │
│  ┌──────┐  ┌──────┐  ┌──────────┐  ┌────────┐  │
│  │ 构建  │→│ 技能  │→│ 工具过滤  │→│  LLM   │──┤
│  │ 消息  │  │ 注入  │  │ allow/   │  │  调用  │  │
│  │      │  │      │  │ deny     │  │       │  │
│  └──────┘  └──────┘  └──────────┘  └───┬────┘  │
│                                         │       │
│            有 tool_calls?               │       │
│            ┌───Yes───┘   └───No──┐      │       │
│            ▼                      ▼     │       │
│     ┌──────────┐          ┌────────┐    │       │
│     │ 执行工具  │          │ 返回   │    │       │
│     │ + 记录轨迹│          │ 最终   │    │       │
│     └─────┬────┘          │ 结果   │    │       │
│           │               └────────┘    │       │
│     连续失败?                            │       │
│     ┌───Yes───┐                         │       │
│     ▼         │                         │       │
│  SelfIter    │                         │       │
│  诊断恢复    └───────→ 回到 LLM ──────→│       │
└─────────────────────────────────────────────────┘
```

关键特性：
- **`max_tool_calls_per_turn`** — 单次执行最大工具调用次数
- **`max_consecutive_errors`** — 连续错误上限，达到后终止
- **流式断点恢复** — SSE 流中断后，将已累积的文本追加到上下文，重新连接（最多 5 次）
- **自迭代恢复** — 连续工具失败时，`SelfIterEngine` 注入诊断指导到 system prompt

### 5.3 LLM 提供商

多提供商架构，通过统一的 `LlmProvider` trait 抽象：

| 提供商 | 协议 | 特殊能力 |
|--------|------|---------|
| OpenAI 兼容 | `/v1/chat/completions` | 覆盖 OpenAI/DeepSeek/DashScope/Ollama |
| Anthropic | `/v1/messages` | 原生 Anthropic API |
| Fallback | 链式组合 | 按序尝试多个后端，首个成功即返回 |

统一的 HTTP 客户端层提供：
- **指数退避重试** — 针对 429/5xx 状态码和连接错误
- **并发信号量** — 每个提供商独立的 `Semaphore` 控制并发
- **可配超时** — 连接超时 + 请求超时独立配置

### 5.4 模型路由

`fastclaw-model-router` 实现五种路由策略：

| 策略 | 说明 |
|------|------|
| `fixed` | 始终使用配置的模型 |
| `cost` | 选择成本最低的可用模型 |
| `quality` | 选择质量最高的模型 |
| `latency` | 选择延迟最低的模型 |
| `fallback` | 按优先级列表逐个降级 |

配合 **ComplexityTier**（Tiny → Frontier）做请求复杂度分级，以及 **Budget** 做原子 token 预算追踪。

### 5.5 三层记忆系统

```
┌────────────────────────────────────────┐
│           Memory Architecture          │
│                                        │
│  ┌──────────────────────────────────┐  │
│  │      Working Memory (LRU)       │  │
│  │  最近对话的短期缓存              │  │
│  └──────────────┬───────────────────┘  │
│                 │ 溢出/沉淀             │
│  ┌──────────────▼───────────────────┐  │
│  │     Episodic Memory (向量)       │  │
│  │  事件记忆：嵌入向量 + 相似检索    │  │
│  │  可选 usearch 后端              │  │
│  └──────────────┬───────────────────┘  │
│                 │ 抽象/关联             │
│  ┌──────────────▼───────────────────┐  │
│  │   Semantic Memory (petgraph)     │  │
│  │  语义图：Fact 节点 + Relationship │  │
│  │  支持 BFS 遍历和关联查询         │  │
│  └──────────────────────────────────┘  │
│                                        │
│  ┌──────────────────────────────────┐  │
│  │     Dreaming Pipeline            │  │
│  │  巩固周期：整理、压缩、归档       │  │
│  └──────────────────────────────────┘  │
└────────────────────────────────────────┘
```

- **WorkingMemory** — LRU 策略的短期记忆，存放最近对话上下文
- **EpisodicMemory** — 向量嵌入的情景记忆，支持语义相似检索，可配 `ForgetPolicy`
- **SemanticMemory** — 基于 petgraph 的语义图，存储 `Fact`（分类节点）和 `Relationship`（边）
- **DreamingPipeline** — 可配周期的记忆巩固管线，生成 `DreamCycleReport`

### 5.6 DAG 工作流引擎

支持 **9 种节点类型**的有向无环图执行引擎：

| 节点类型 | 说明 |
|---------|------|
| `LLM` | 调用 LLM 生成内容 |
| `Tool` | 执行已注册工具 |
| `Condition` | 条件分支判断 |
| `Parallel` | 并行执行多个分支 |
| `Join` | 等待并行分支汇合 |
| `HumanApproval` | 人工审批门控 |
| `Loop` | 循环执行（带 `LoopConfig`） |
| `Reflect` | 自省节点 |
| `Code` | 代码执行节点 |

核心组件：
- **`DagDefinition`** — 从 JSON 反序列化的 DAG 定义
- **`DagGraph`** — 基于 petgraph 构建的执行图
- **`DagExecutor`** — 拓扑排序执行，通过 `NodeHandler` trait 回调节点处理
- **`ExecutionContext`** — 节点间共享数据传递
- **`CheckpointStore`** — 检查点持久化（支持 SQLite 和内存两种后端）
- **`ExecutionEvent`** / `EventSink` — 结构化执行事件发射
- **表达式求值** — JSON Pointer、运算符、索引、`in`、`contains` 等

### 5.7 MCP 协议支持

FastClaw 同时实现了 MCP 的**服务端**和**客户端**：

**服务端 (McpServer)**
- 通过 JSON-RPC 2.0 暴露 FastClaw 的工具给外部 Agent
- 支持 `initialize`、`tools/list`、`tools/call` 方法
- 可通过 **stdio** 传输运行（`fastclaw mcp-server` 命令）

**客户端 (McpClient)**
- 连接外部 MCP 服务器，消费其工具
- 支持两种传输：**subprocess stdio** 和 **HTTP SSE**
- `McpBridgedTool` 将远程工具适配为本地 `Tool` trait 实现
- `register_mcp_tools` 将发现的远程工具以前缀命名注册到 `ToolRegistry`

### 5.8 多智能体协作

`fastclaw-collab` 提供五种协作模式：

| 模式 | 说明 |
|------|------|
| **CollabHub** | 能力注册中心，Agent 注册自己的能力供他人发现 |
| **Delegation** | 委托模式：将子任务委托给其他 Agent（`SubAgentTool`） |
| **Pipeline** | 流水线模式：多个 Agent 串行处理，前者输出作后者输入 |
| **Dialectic** | 辩证模式：两个 Agent 围绕同一问题进行多轮辩论 |
| **Committee** | 委员会模式：多个专家 Agent 各自给出意见后综合 |

底层通过 `MessageBus` 支持：
- 直发（单播）
- 广播
- 主题订阅
- 请求-应答（带超时）
- HMAC-SHA256 签名 + 重放保护

### 5.9 安全体系

FastClaw 实现了纵深防御策略：

| 层 | 机制 | 实现 |
|----|------|------|
| **认证** | API Key（恒定时间比较） | `fastclaw-security::auth` |
| **限流** | IP 维度请求限流 | `fastclaw-security::rate_limit` |
| **输入防护** | Prompt 注入检测 | `fastclaw-security::prompt_guard` |
| **网络安全** | SSRF 防御（私有 IP 阻断 + DNS 检查 + 安全重定向） | `fastclaw-security::ssrf` |
| **总线安全** | HMAC-SHA256 签名 + 时间戳重放保护 + 跳数限制 | `fastclaw-core::bus` |
| **WASM 沙箱** | fuel 限制 + epoch 优雅退出 | `fastclaw-plugin` |
| **代码沙箱** | Shell 禁用 + 大小限制 | `fastclaw-code::sandbox` |
| **Webhook 校验** | 平台级签名验证（Slack/WhatsApp/Feishu 等） | 各渠道扩展 |
| **预算控制** | 原子预留/释放 token 预算 | `fastclaw-model-router::budget` |
| **路径穿越** | 配置包含路径的遍历防护 | `fastclaw-core::config` |

### 5.10 自进化系统

实现了类 Hermes 的技能全生命周期：

```
 ┌──────────┐    ┌──────────┐    ┌──────────┐    ┌──────────┐
 │ 轨迹记录  │ →  │ 技能提取  │ →  │ 技能存储  │ →  │ 技能注入  │
 │Trajectory│    │Extractor │    │SkillStore│    │ Inject   │
 └──────────┘    └──────────┘    └──────────┘    └────┬─────┘
                                                      │
 ┌──────────┐    ┌──────────┐    ┌──────────┐         │
 │ 技能退役  │ ←  │ 评估反馈  │ ←  │ 使用追踪  │ ←───────┘
 │ Retire   │    │Evaluator │    │ Record   │
 └──────────┘    └──────────┘    └──────────┘
```

- **Trajectory** — 成功的 Agent 执行自动记录步骤轨迹
- **SkillExtractor** — 从轨迹中提取可复用的技能模式
- **SkillStore** — 向量相似度索引，检索匹配任务的技能
- **Skill Injection** — 运行时将 Active/Candidate 技能注入 system prompt
- **Feedback / Evaluator** — 收集反馈，评估技能效果
- **Distiller** — 规则 + 可选 LLM 的 prompt 蒸馏
- **状态机** — `Candidate → Active → Retired` 的生命周期管理

### 5.11 上下文引擎

`fastclaw-context` 实现六层上下文预算管理：

| 层 | 内容 | 优先级 |
|----|------|--------|
| System | 系统 prompt | 最高 |
| Skills | 注入的技能指导 | 高 |
| Memory | 记忆检索结果 | 中高 |
| History | 会话历史 | 中 |
| Context | 外部上下文 | 中低 |
| User | 当前用户消息 | 低 |

通过 `ContextBudget` 按 token 预算分配各层额度，支持**滚动压缩**（历史过长时自动摘要）和**用户画像**集成。

### 5.12 代码智能

`fastclaw-code` 提供完整的代码理解与操作能力：

| 组件 | 能力 |
|------|------|
| **Tree-sitter Index** | Rust/JS/TS/Python 的 AST 解析，Go/Java regex 回退 |
| **CodeGraph** | 基于调用关系的 BFS 遍历、影响分析、SCC 环路检测 |
| **TestRunner** | 多语言测试执行器（cargo/pytest/npm/go test） |
| **PatchEngine** | 补丁应用/回滚/校验，支持原子多文件操作 |
| **Refactor** | 跨文件重命名支持 |
| **AutoFix** | 自动修复循环 |
| **Sandbox** | Shell 禁用 + 大小限制的安全代码执行环境 |

## 6. 配置管理

### 6.1 配置加载顺序

```
$FASTCLAW_CONFIG_PATH（显式指定）
  ↓ 不存在
~/.fastclaw/config/default.json
  ↓ 不存在
$OPENCLAW_CONFIG_PATH（兼容遗留）
  ↓ 不存在
~/.openclaw/openclaw.json
  ↓ 不存在
内置默认值（config/default.json）
```

- 格式：**JSON5**（支持注释和尾逗号）
- 合并策略：**deep_merge** 深度合并
- `$include` 指令：递归包含子配置，有深度限制和路径穿越防护
- **热重载**：文件监听 + SIGHUP 信号触发 Agent 配置重载，校验失败时**原子回滚**

### 6.2 运行状态目录

```
~/.fastclaw/          # 默认
~/.fastclaw-dev/      # --dev 模式
~/.fastclaw-<profile>/  # --profile 指定
```

## 7. 部署架构

### 7.1 单二进制部署

```bash
fastclaw serve              # 前台网关
fastclaw gateway start      # 后台守护
fastclaw tui                # 终端 UI
fastclaw mcp-server         # MCP 服务模式
```

### 7.2 Docker 部署

- **多阶段构建**：`rust:1.82-bookworm` → `debian:bookworm-slim`
- **非 root 用户**：`fastclaw` 用户运行
- **健康检查**：`fastclaw health` 内置命令
- **默认端口**：18789

### 7.3 Kubernetes 适配

- `/health` — 存活探针 (Liveness)
- `/ready` — 就绪探针 (Readiness)，检查 Router + Agent 可用性
- `/metrics` — Prometheus 采集端点
- SIGTERM → 30s 优雅关停

## 8. 可观测性

| 维度 | 实现 |
|------|------|
| **指标** | Prometheus text exporter（`/metrics` + `/api/v1/metrics`） |
| **追踪** | 结构化 tracing（JSON 格式可选） |
| **健康** | `/health`（存活）、`/ready`（就绪） |
| **Agent 重载** | `record_agent_reload` 指标 |
| **Chat 请求** | `record_chat_request` 指标（区分流式/非流式） |

## 9. 错误处理

### 9.1 分层错误模型

```
FastClawError (核心层, thiserror)
  ├── Config      — 配置错误
  ├── Session     — 会话错误
  ├── Agent       — Agent 错误
  ├── Plugin      — 插件错误
  ├── Memory      — 记忆错误
  ├── Routing     — 路由错误
  ├── LlmProvider — LLM 错误
  ├── ToolNotFound / ToolExecution — 工具错误
  ├── Bus*        — 消息总线错误（未注册/邮箱关闭/超时/签名无效）
  ├── Io / Json / Json5 — 底层 I/O / 解析
  └── Internal    — anyhow 兜底

AppError (网关层)
  ├── BadRequest  → 400
  ├── Unauthorized → 401
  ├── NotFound    → 404
  ├── RateLimited → 429
  └── ServerError → 500
```

### 9.2 工具执行错误

工具执行使用 `ToolResult { success, output }` 的 stringly-typed 错误传递，让 LLM 能理解失败原因并决定重试策略。

## 10. 测试策略

| 层级 | 覆盖 | 机制 |
|------|------|------|
| **单元测试** | 所有 crate 模块内嵌 `#[cfg(test)]` | `cargo test --workspace` |
| **集成测试** | `crates/fastclaw-gateway/tests/integration.rs` | 真实 TCP + MockProvider |
| **E2E 场景** | `crates/fastclaw-gateway/tests/e2e_scenarios.rs` | ScriptedProvider 确定性流程 |
| **质量门禁** | CI | `cargo fmt` + `clippy -D warnings` + `cargo test` |
| **跨平台** | CI 矩阵 | Ubuntu / macOS / Windows |

当前共 **448** 项工作区测试，CI 要求零 warning。

## 11. 设计模式总结

| 模式 | 应用场景 |
|------|---------|
| **注册表 (Registry)** | ToolRegistry, ChannelRegistry, PluginRegistry |
| **策略 (Strategy)** | WebSearchBackend, ModelRouter 策略, 上下文压缩策略 |
| **责任链/降级 (Chain/Fallback)** | FallbackProvider, 模型降级列表 |
| **桥接/适配器 (Bridge/Adapter)** | McpBridgedTool (远程 MCP → 本地 Tool) |
| **工厂 (Factory)** | AgentFactory, create_provider, StateBuilder 多阶段 |
| **模板方法 (Template Method)** | ChannelPlugin 默认方法, Context Hooks |
| **发布/订阅 (Pub/Sub)** | MessageBus 广播/主题 |
| **请求-应答 (Request-Reply)** | MessageBus::request |
| **装饰器 (Decorator)** | Axum 中间件栈（限流 → 认证 → 压缩 → 追踪 → CORS） |
| **构建器 (Builder)** | StateBuilder 分阶段初始化 |
| **观察者 (Observer)** | DAG EventSink, StreamEvent 推送 |
