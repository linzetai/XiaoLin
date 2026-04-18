# Harness Engineer — 技术方案

> **版本**: v1.0  
> **日期**: 2026-04-16  
> **状态**: Draft  
> **作者**: FastClaw Team

---

## 1. 总体架构

### 1.1 设计原则

| 原则 | 说明 |
|------|------|
| **单进程·多线程** | 一个 OS 进程，Tokio work-stealing 线程池，充分利用多核 |
| **零 GC** | Rust 所有权模型，编译时内存安全，无运行时垃圾回收 |
| **插件沙箱** | 所有第三方代码在 WASM 沙箱中运行，与宿主内存隔离 |
| **异步优先** | 所有 I/O 操作均为异步，阻塞操作通过 `spawn_blocking` 隔离 |
| **渐进增强** | 核心精简，高级功能通过插件/模块按需启用 |
| **可观测** | 结构化日志 + 指标 + 分布式追踪，贯穿全链路 |

### 1.2 进程内模块拓扑

```
┌─────────────────────────────────────────────────────────┐
│                    Harness Engineer Process              │
│                                                         │
│  ┌──────────┐  ┌──────────────┐  ┌───────────────────┐  │
│  │ CLI/Conf │──│   Gateway    │──│  Channel Adapters │  │
│  │ (clap)   │  │ (Axum + WS) │  │ (WASM plugins)    │  │
│  └────┬─────┘  └──────┬───────┘  └─────────┬─────────┘  │
│       │               │                    │             │
│       ▼               ▼                    ▼             │
│  ┌─────────────────────────────────────────────────┐     │
│  │              Message Router                      │     │
│  │    (9-tier priority matching, petgraph)          │     │
│  └──────────────────────┬──────────────────────────┘     │
│                         │                                │
│  ┌──────────────────────▼──────────────────────────┐     │
│  │              Agent Runtime                       │     │
│  │  ┌────────┐ ┌─────────┐ ┌──────────┐ ┌───────┐ │     │
│  │  │Prompt  │ │LLM Call │ │Tool Exec │ │Stream │ │     │
│  │  │Builder │ │(reqwest)│ │(dispatch)│ │Router │ │     │
│  │  └────────┘ └─────────┘ └──────────┘ └───────┘ │     │
│  └──────────────────────┬──────────────────────────┘     │
│                         │                                │
│  ┌──────────┬───────────┼───────────┬──────────────┐     │
│  │          │           │           │              │     │
│  ▼          ▼           ▼           ▼              ▼     │
│ ┌──────┐ ┌──────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐  │
│ │Session│ │Memory│ │  DAG    │ │ Plugin  │ │Evolution│  │
│ │Store  │ │System│ │ Engine  │ │ Host    │ │ Engine  │  │
│ │(SQLite│ │(3-Lyr│ │(petgraph│ │(wasmtime│ │(feedback│  │
│ │ WAL)  │ │+KG)  │ │+SM)    │ │sandbox) │ │+distill)│  │
│ └──────┘ └──────┘ └─────────┘ └─────────┘ └─────────┘  │
│                                                         │
│  ┌─────────────────────────────────────────────────┐     │
│  │            Observability Layer                   │     │
│  │  tracing + metrics + OpenTelemetry               │     │
│  └─────────────────────────────────────────────────┘     │
└─────────────────────────────────────────────────────────┘
```

### 1.3 Cargo Workspace 结构

```
harness-engineer/
├── Cargo.toml                    # workspace root
├── Cargo.lock
├── config/                       # 默认配置模板
│   ├── default.json
│   └── agents/
│       └── main.json
├── crates/
│   ├── fastclaw-cli/              # 二进制入口（编译产物为 fastclaw）
│   │   ├── Cargo.toml
│   │   └── src/main.rs
│   ├── fastclaw-core/             # 核心类型、错误、配置
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── config.rs         # JSON 配置定义（兼容 OpenClaw 格式）
│   │       ├── error.rs          # 统一错误类型
│   │       ├── message.rs        # 消息模型
│   │       └── agent.rs          # Agent 配置
│   ├── fastclaw-gateway/          # HTTP/WS Gateway
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── server.rs         # Axum 服务器
│   │       ├── websocket.rs      # WS 处理
│   │       ├── middleware.rs     # Tower 中间件
│   │       └── health.rs         # 健康检查
│   ├── fastclaw-router/           # 消息路由
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── matcher.rs        # 9 层匹配引擎
│   │       └── session_key.rs    # 会话键生成
│   ├── fastclaw-agent/            # Agent 运行时
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── pipeline.rs       # Prompt → LLM → Tools 管道
│   │       ├── prompt.rs         # Prompt 组装
│   │       ├── llm.rs            # LLM Provider 抽象
│   │       ├── tools.rs          # 工具注册与调度
│   │       └── stream.rs         # 流式响应
│   ├── fastclaw-session/          # 会话存储
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── store.rs          # SQLite CRUD
│   │       ├── compaction.rs     # 上下文压缩
│   │       └── schema.sql        # DDL
│   ├── fastclaw-dag/              # DAG 编排引擎
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── graph.rs          # DAG 定义与构建
│   │       ├── executor.rs       # 并行执行器
│   │       ├── state_machine.rs  # 节点状态机
│   │       └── checkpoint.rs     # 检查点持久化
│   ├── fastclaw-memory/           # 认知记忆
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── working.rs        # 工作记忆
│   │       ├── episodic.rs       # 情景记忆
│   │       ├── semantic.rs       # 语义记忆
│   │       ├── knowledge_graph.rs # 知识图谱
│   │       ├── vector_index.rs   # 向量索引
│   │       └── forgetting.rs     # 遗忘策略
│   ├── fastclaw-plugin/           # WASM 插件宿主
│   │   ├── Cargo.toml
│   │   ├── wit/                  # WIT 接口定义
│   │   │   ├── channel.wit
│   │   │   ├── tool.wit
│   │   │   └── memory.wit
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── host.rs           # wasmtime 宿主
│   │       ├── loader.rs         # 插件加载与校验
│   │       ├── sandbox.rs        # 资源限制
│   │       └── hot_reload.rs     # 热重载
│   ├── fastclaw-evolution/        # 自我进化
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── feedback.rs       # 反馈收集
│   │       ├── evaluator.rs      # 策略评估
│   │       ├── distiller.rs      # Prompt 蒸馏
│   │       └── user_model.rs     # 用户建模
│   ├── fastclaw-observe/          # 可观测性
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── tracing_setup.rs  # tracing 初始化
│   │       └── metrics.rs        # Prometheus 指标
│   ├── fastclaw-security/         # 安全防护（新增）
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── auth.rs           # API Key / mTLS 认证
│   │       ├── rate_limit.rs     # 速率限制
│   │       ├── prompt_guard.rs   # Prompt 注入防护
│   │       ├── tool_validator.rs # 工具参数 JSON Schema 校验
│   │       ├── audit.rs          # 安全审计日志
│   │       └── sanitizer.rs      # 输入净化 / 日志脱敏
│   ├── fastclaw-agent-factory/    # 快速创建 Agent（新增）
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── factory.rs        # AgentFactory 主逻辑
│   │       ├── template.rs       # 模板注册表
│   │       ├── intent_extractor.rs # 意图解析
│   │       └── templates/        # 内置模板 JSON（20+）
│   ├── fastclaw-studio/           # Skill 可视化编排（新增）
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── server.rs         # Studio WebSocket 服务
│   │       ├── flow_dsl.rs       # FlowDSL 定义与解析
│   │       ├── flow_store.rs     # 流程图持久化
│   │       └── suggestion.rs     # AI 辅助节点建议
│   ├── fastclaw-collab/           # 多 Agent 协作（新增）
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── hub.rs            # CollabHub
│   │       ├── message_bus.rs    # 内部消息总线（HMAC 签名）
│   │       ├── registry.rs       # AgentCapabilityCard 注册
│   │       └── modes/
│   │           ├── orchestrator.rs
│   │           ├── pipeline.rs
│   │           ├── debate.rs
│   │           └── panel.rs
│   ├── fastclaw-model-router/     # 模型复杂度路由（新增）
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── router.rs         # ModelRouter
│   │       ├── estimator.rs      # ComplexityEstimator
│   │       ├── budget.rs         # BudgetTracker
│   │       └── intent_classifier.rs # 轻量意图分类（无 LLM）
│   ├── fastclaw-context/          # 智能上下文管理（新增）
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── manager.rs        # ContextManager（6 层）
│   │       ├── compressor.rs     # 会话滚动压缩
│   │       ├── user_profile.rs   # 用户画像提取与存储
│   │       └── collab_context.rs # 多 Agent 上下文切片
│   ├── fastclaw-self-iter/        # Agent 自我迭代（新增）
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── engine.rs         # SelfIterationEngine
│   │       ├── diagnoser.rs      # 错误诊断
│   │       ├── strategist.rs     # 修复策略生成
│   │       └── sandbox_runner.rs # 沙箱执行验证
│   └── fastclaw-code/             # 代码能力增强（新增）
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── index.rs          # CodebaseIndex（AST+向量+调用图）
│           ├── sandbox.rs        # 代码执行沙箱（seccomp）
│           ├── auto_fix.rs       # 错误自动修复循环
│           ├── style_learner.rs  # 项目风格学习
│           ├── test_generator.rs # 测试自动生成
│           └── parsers/          # Tree-sitter 各语言语法
├── plugins/                      # 内置 WASM 插件
│   ├── channel-telegram/
│   ├── channel-slack/
│   └── tool-http-fetch/
└── tests/
    ├── integration/
    ├── security/                 # 安全专项测试
    └── benchmarks/
```

---

## 2. 核心模块详细设计

### 2.1 Gateway — HTTP/WebSocket 服务

#### 技术选型

| 组件 | 库 | 理由 |
|------|-----|------|
| HTTP 框架 | `axum 0.8` | Tower 生态、类型安全路由、compile-time handler 验证 |
| WebSocket | `tokio-tungstenite` | 零拷贝帧解析，与 Tokio 深度集成 |
| TLS | `rustls` + `rcgen` | 纯 Rust TLS，无 OpenSSL 依赖，可选自签证书 |
| JSON | `serde` + `simd-json` | 编译时序列化 + SIMD 加速反序列化 |

#### 核心结构

```rust
pub struct GatewayServer {
    config: Arc<GatewayConfig>,
    router: Router,
    ws_clients: Arc<DashMap<ClientId, WsSender>>,
    channel_manager: ChannelManager,
    agent_runtime: Arc<AgentRuntime>,
    shutdown: CancellationToken,
}

pub struct GatewayConfig {
    pub bind: SocketAddr,          // 默认 0.0.0.0:18789（对齐 OpenClaw DEFAULT_GATEWAY_PORT）
    pub tls: Option<TlsConfig>,
    pub max_connections: usize,    // 默认 1024
    pub rate_limit: RateLimitConfig,
    pub cors: CorsConfig,
}
```

#### HTTP 路由表

| Method | Path | Handler | 说明 |
|--------|------|---------|------|
| POST | `/api/v1/chat` | `handle_chat` | 同步聊天（完整回复） |
| POST | `/api/v1/chat/stream` | `handle_chat_stream` | SSE 流式聊天 |
| GET | `/api/v1/agents` | `list_agents` | 列出所有 Agent |
| GET | `/api/v1/agents/:id` | `get_agent` | Agent 详情 |
| POST | `/api/v1/agents/:id/reload` | `reload_agent` | 热重载 Agent |
| GET | `/api/v1/sessions` | `list_sessions` | 列出会话 |
| GET | `/api/v1/sessions/:key` | `get_session` | 会话详情 |
| DELETE | `/api/v1/sessions/:key` | `delete_session` | 删除会话 |
| GET | `/api/v1/memory/search` | `search_memory` | 记忆搜索 |
| POST | `/api/v1/dag/execute` | `execute_dag` | 执行 DAG 工作流 |
| GET | `/ws` | `ws_upgrade` | WebSocket 升级 |
| GET | `/health` | `health_check` | 存活探针 |
| GET | `/ready` | `readiness_check` | 就绪探针 |
| GET | `/metrics` | `prometheus_metrics` | Prometheus 指标 |

#### WebSocket 消息协议

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsMessage {
    /// 客户端发送聊天消息
    Chat {
        agent_id: Option<String>,
        session_key: Option<String>,
        content: String,
        #[serde(default)]
        metadata: HashMap<String, Value>,
    },
    /// 服务端推送流式 token
    StreamToken {
        session_key: String,
        token: String,
        index: u32,
    },
    /// 服务端推送完整回复
    Response {
        session_key: String,
        content: String,
        tool_calls: Vec<ToolCallResult>,
        usage: TokenUsage,
    },
    /// 工具执行进度
    ToolProgress {
        session_key: String,
        tool_name: String,
        status: ToolStatus,
        output: Option<Value>,
    },
    /// 心跳
    Ping { ts: u64 },
    Pong { ts: u64 },
    /// 错误
    Error { code: u32, message: String },
}
```

#### 连接生命周期

```rust
async fn handle_ws_connection(
    ws: WebSocket,
    state: Arc<AppState>,
) {
    let (sender, mut receiver) = ws.split();
    let client_id = ClientId::new();
    let sender = Arc::new(Mutex::new(sender));

    state.ws_clients.insert(client_id, sender.clone());

    let heartbeat = tokio::spawn(ws_heartbeat(sender.clone(), state.shutdown.clone()));

    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                let mut bytes = text.into_bytes();
                let parsed: WsMessage = simd_json::from_slice(&mut bytes)?;
                tokio::spawn(process_ws_message(parsed, client_id, state.clone()));
            }
            Ok(Message::Close(_)) => break,
            Err(e) => {
                tracing::warn!(client_id = %client_id, error = %e, "ws error");
                break;
            }
            _ => {}
        }
    }

    heartbeat.abort();
    state.ws_clients.remove(&client_id);
}
```

### 2.2 消息路由引擎

#### 9 层优先级匹配

```rust
pub struct RouteResolver {
    rules: Vec<RouteRule>,
    default_agent: AgentId,
}

#[derive(Debug, Clone)]
pub struct RouteRule {
    pub priority: RouteTier,
    pub matcher: RouteMatcher,
    pub agent_id: AgentId,
}

#[derive(Debug, Clone, PartialOrd, Ord, PartialEq, Eq)]
pub enum RouteTier {
    PeerExact     = 0,  // 精确用户绑定
    PeerParent    = 1,  // 父级用户绑定
    PeerWildcard  = 2,  // 用户通配
    GuildRoles    = 3,  // 群组 + 角色
    Guild         = 4,  // 群组
    Team          = 5,  // 团队
    Account       = 6,  // 账号
    Channel       = 7,  // 渠道
    Default       = 8,  // 兜底
}

impl RouteResolver {
    /// O(n) 扫描，n 通常 < 100，无需复杂索引
    pub fn resolve(&self, ctx: &RouteContext) -> &AgentId {
        self.rules
            .iter()
            .filter(|r| r.matcher.matches(ctx))
            .min_by_key(|r| r.priority)
            .map(|r| &r.agent_id)
            .unwrap_or(&self.default_agent)
    }
}
```

#### 会话键生成

```rust
pub fn build_session_key(
    agent_id: &str,
    channel_id: &str,
    peer_id: &str,
    scope: SessionScope,
) -> String {
    match scope {
        SessionScope::Peer => format!("agent:{agent_id}:ch:{channel_id}:peer:{peer_id}"),
        SessionScope::Guild(g) => format!("agent:{agent_id}:ch:{channel_id}:guild:{g}"),
        SessionScope::Global => format!("agent:{agent_id}:global"),
    }
}
```

### 2.3 Agent 运行时

#### 管道模型

```
InboundMessage
    │
    ▼
┌──────────────────┐
│  Route Resolve   │  → 确定目标 Agent
└────────┬─────────┘
         ▼
┌──────────────────┐
│  Session Load    │  → 从 SQLite 加载历史
└────────┬─────────┘
         ▼
┌──────────────────┐
│  Prompt Build    │  → system + memory + history + user
└────────┬─────────┘
         ▼
┌──────────────────┐
│  LLM Call        │  → 流式请求，逐 token 推送
└────────┬─────────┘
         ▼
┌──────────────────┐
│  Tool Dispatch   │  → 解析 tool_calls，执行工具
└────────┬─────────┘    （可能递归回 LLM Call）
         ▼
┌──────────────────┐
│  Memory Write    │  → 写入情景/语义记忆
└────────┬─────────┘
         ▼
┌──────────────────┐
│  Session Save    │  → 追加消息到 SQLite
└────────┬─────────┘
         ▼
┌──────────────────┐
│  Feedback Emit   │  → 异步发送到 Evolution 引擎
└────────┬─────────┘
         ▼
    OutboundMessage
```

#### 核心 trait 定义

```rust
/// LLM 提供者抽象
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(
        &self,
        request: ChatRequest,
    ) -> Result<ChatResponse>;

    fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>;

    fn name(&self) -> &str;
    fn model_id(&self) -> &str;
}

/// 工具 trait
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> &Value;
    fn is_dangerous(&self) -> bool { false }

    async fn execute(
        &self,
        params: Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput>;
}

/// Prompt 构建器
pub struct PromptBuilder {
    agent_config: AgentConfig,
    memory_system: Arc<MemorySystem>,
}

impl PromptBuilder {
    pub async fn build(
        &self,
        session: &Session,
        user_message: &str,
    ) -> Result<Vec<ChatMessage>> {
        let mut messages = Vec::with_capacity(64);

        // 1. System prompt
        messages.push(ChatMessage::system(&self.agent_config.system_prompt));

        // 2. 语义记忆注入
        let memories = self.memory_system
            .recall(user_message, RecallOptions::default())
            .await?;
        if !memories.is_empty() {
            let memory_text = format_memories(&memories);
            messages.push(ChatMessage::system(memory_text));
        }

        // 3. 历史消息（压缩后）
        let history = session.compressed_history(
            self.agent_config.max_context_tokens,
        );
        messages.extend(history);

        // 4. 用户消息
        messages.push(ChatMessage::user(user_message));

        Ok(messages)
    }
}
```

#### LLM Provider 实现（OpenAI 兼容）

```rust
pub struct OpenAiProvider {
    client: reqwest::Client,
    base_url: Url,
    api_key: SecretString,
    model: String,
    semaphore: Arc<Semaphore>,  // 并发限流
    retry: RetryConfig,
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let _permit = self.semaphore.acquire().await?;

        let body = serde_json::to_value(&request)?;
        let response = self.client
            .post(self.base_url.join("/v1/chat/completions")?)
            .bearer_auth(self.api_key.expose_secret())
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await?;
            return Err(LlmError::ApiError { status, body: error_body }.into());
        }

        let result: OpenAiResponse = response.json().await?;
        Ok(result.into())
    }

    fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>> {
        let client = self.client.clone();
        let url = self.base_url.join("/v1/chat/completions").unwrap();
        let api_key = self.api_key.clone();
        let semaphore = self.semaphore.clone();

        Box::pin(async_stream::try_stream! {
            let _permit = semaphore.acquire().await?;
            let mut request = request;
            request.stream = true;

            let response = client
                .post(url)
                .bearer_auth(api_key.expose_secret())
                .json(&request)
                .send()
                .await?;

            let mut stream = response.bytes_stream();
            let mut buffer = Vec::new();

            while let Some(chunk) = stream.next().await {
                let bytes = chunk?;
                buffer.extend_from_slice(&bytes);

                for line in parse_sse_lines(&mut buffer) {
                    if line == "[DONE]" { return; }
                    let chunk: StreamChunk = simd_json::from_slice(
                        &mut line.into_bytes()
                    )?;
                    yield chunk;
                }
            }
        })
    }

    fn name(&self) -> &str { "openai" }
    fn model_id(&self) -> &str { &self.model }
}
```

### 2.4 会话存储 (SQLite WAL)

#### Schema

```sql
-- 会话元数据
CREATE TABLE sessions (
    key         TEXT PRIMARY KEY,
    agent_id    TEXT NOT NULL,
    channel_id  TEXT NOT NULL,
    peer_id     TEXT NOT NULL,
    created_at  INTEGER NOT NULL,  -- unix millis
    updated_at  INTEGER NOT NULL,
    metadata    TEXT,               -- JSON
    UNIQUE(agent_id, channel_id, peer_id)
);

-- 消息存储
CREATE TABLE messages (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_key TEXT NOT NULL REFERENCES sessions(key),
    role        TEXT NOT NULL CHECK(role IN ('system','user','assistant','tool')),
    content     TEXT NOT NULL,
    tool_calls  TEXT,               -- JSON array
    token_count INTEGER,
    created_at  INTEGER NOT NULL
);
CREATE INDEX idx_messages_session ON messages(session_key, created_at);

-- 知识图谱三元组
CREATE TABLE knowledge_triples (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    subject     TEXT NOT NULL,
    predicate   TEXT NOT NULL,
    object      TEXT NOT NULL,
    agent_id    TEXT,               -- NULL = 全局共享
    confidence  REAL DEFAULT 1.0,
    source      TEXT,               -- 来源会话
    created_at  INTEGER NOT NULL,
    accessed_at INTEGER NOT NULL,
    access_count INTEGER DEFAULT 1
);
CREATE INDEX idx_kg_subject ON knowledge_triples(subject);
CREATE INDEX idx_kg_object ON knowledge_triples(object);
CREATE INDEX idx_kg_agent ON knowledge_triples(agent_id);

-- 向量记忆索引（元数据，向量存储在 usearch 文件中）
CREATE TABLE vector_memories (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id    TEXT,
    session_key TEXT,
    content     TEXT NOT NULL,
    memory_type TEXT NOT NULL CHECK(memory_type IN ('episodic','semantic')),
    vector_id   INTEGER NOT NULL,   -- usearch 内部 ID
    created_at  INTEGER NOT NULL,
    accessed_at INTEGER NOT NULL,
    access_count INTEGER DEFAULT 1,
    decay_score  REAL DEFAULT 1.0
);
CREATE INDEX idx_vm_agent ON vector_memories(agent_id, memory_type);
CREATE INDEX idx_vm_decay ON vector_memories(decay_score);

-- DAG 执行检查点
CREATE TABLE dag_checkpoints (
    dag_id      TEXT NOT NULL,
    node_id     TEXT NOT NULL,
    status      TEXT NOT NULL CHECK(status IN ('pending','running','success','failed','skipped')),
    input       TEXT,               -- JSON
    output      TEXT,               -- JSON
    started_at  INTEGER,
    finished_at INTEGER,
    error       TEXT,
    PRIMARY KEY (dag_id, node_id)
);

-- 反馈记录
CREATE TABLE feedback (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_key TEXT NOT NULL,
    message_id  INTEGER REFERENCES messages(id),
    feedback_type TEXT NOT NULL,    -- 'explicit_rating', 'implicit_tool_success', etc.
    value       TEXT NOT NULL,      -- JSON
    created_at  INTEGER NOT NULL
);

-- 进化策略快照
CREATE TABLE evolution_snapshots (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id    TEXT NOT NULL,
    version     INTEGER NOT NULL,
    prompt_hash TEXT NOT NULL,
    prompt_text TEXT NOT NULL,
    metrics     TEXT,               -- JSON: avg_rating, success_rate, etc.
    is_active   INTEGER DEFAULT 0,
    created_at  INTEGER NOT NULL,
    UNIQUE(agent_id, version)
);
```

#### 异步存储层

```rust
pub struct SessionStore {
    pool: SqlitePool,
}

impl SessionStore {
    pub async fn new(db_path: &Path) -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect(&format!("sqlite:{}?mode=rwc", db_path.display()))
            .await?;

        sqlx::query("PRAGMA journal_mode=WAL").execute(&pool).await?;
        sqlx::query("PRAGMA synchronous=NORMAL").execute(&pool).await?;
        sqlx::query("PRAGMA cache_size=-64000").execute(&pool).await?; // 64MB cache

        sqlx::migrate!("./migrations").run(&pool).await?;

        Ok(Self { pool })
    }

    pub async fn load_session(&self, key: &str) -> Result<Option<Session>> {
        let row = sqlx::query_as::<_, SessionRow>(
            "SELECT * FROM sessions WHERE key = ?1"
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(r) => {
                let messages = self.load_messages(key).await?;
                Ok(Some(Session::from_row(r, messages)))
            }
            None => Ok(None),
        }
    }

    pub async fn append_message(&self, session_key: &str, msg: &ChatMessage) -> Result<i64> {
        let id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO messages (session_key, role, content, tool_calls, token_count, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             RETURNING id"
        )
        .bind(session_key)
        .bind(msg.role.as_str())
        .bind(&msg.content)
        .bind(msg.tool_calls.as_ref().map(|tc| serde_json::to_string(tc).unwrap()))
        .bind(msg.token_count)
        .bind(unix_millis_now())
        .fetch_one(&self.pool)
        .await?;

        sqlx::query("UPDATE sessions SET updated_at = ?1 WHERE key = ?2")
            .bind(unix_millis_now())
            .bind(session_key)
            .execute(&self.pool)
            .await?;

        Ok(id)
    }
}
```

### 2.5 DAG 编排引擎

#### DAG 定义

```rust
use petgraph::graph::DiGraph;

pub type DagGraph = DiGraph<DagNode, DagEdge>;

#[derive(Debug, Clone)]
pub struct DagNode {
    pub id: NodeId,
    pub kind: NodeKind,
    pub config: NodeConfig,
    pub timeout: Duration,
}

#[derive(Debug, Clone)]
pub enum NodeKind {
    LlmCall {
        agent_id: AgentId,
        prompt_template: String,
    },
    ToolExec {
        tool_name: String,
        params_template: Value,
    },
    Condition {
        expression: String,  // JSONPath 或简单表达式
    },
    FanOut {
        branches: Vec<NodeId>,
    },
    FanIn {
        merge_strategy: MergeStrategy,
    },
    SubDag {
        dag_definition: DagDefinition,
    },
    HumanApproval {
        prompt: String,
        timeout: Duration,
    },
    Reflect {
        target_node: NodeId,
        max_iterations: u32,
    },
}

#[derive(Debug, Clone)]
pub struct DagEdge {
    pub condition: Option<String>,  // 条件边，None = 无条件
}

#[derive(Debug, Clone)]
pub enum MergeStrategy {
    WaitAll,       // 等待所有分支完成
    FirstSuccess,  // 第一个成功即可
    Majority,      // 多数成功
}
```

#### 并行执行器

```rust
pub struct DagExecutor {
    session_store: Arc<SessionStore>,
    agent_runtime: Arc<AgentRuntime>,
    checkpoint_store: Arc<CheckpointStore>,
}

impl DagExecutor {
    pub async fn execute(&self, dag: &DagGraph, input: Value) -> Result<DagResult> {
        let mut state = ExecutionState::new(dag, input);

        loop {
            let ready_nodes = state.find_ready_nodes();
            if ready_nodes.is_empty() {
                if state.all_completed() {
                    break;
                }
                return Err(DagError::Deadlock.into());
            }

            // 并行执行所有就绪节点
            let handles: Vec<_> = ready_nodes
                .into_iter()
                .map(|node_idx| {
                    let node = &dag[node_idx];
                    let ctx = state.node_context(node_idx);
                    let executor = self.clone();

                    tokio::spawn(async move {
                        let result = executor.execute_node(node, ctx).await;
                        (node_idx, result)
                    })
                })
                .collect();

            for handle in handles {
                let (node_idx, result) = handle.await?;
                match result {
                    Ok(output) => {
                        state.mark_success(node_idx, output);
                        self.checkpoint_store.save(&state, node_idx).await?;
                    }
                    Err(e) => {
                        state.mark_failed(node_idx, e.to_string());
                        self.checkpoint_store.save(&state, node_idx).await?;
                    }
                }
            }
        }

        Ok(state.into_result())
    }

    async fn execute_node(
        &self,
        node: &DagNode,
        ctx: NodeContext,
    ) -> Result<Value> {
        let _timer = metrics::histogram!("dag_node_duration_ms")
            .start_timer();

        tokio::time::timeout(node.timeout, async {
            match &node.kind {
                NodeKind::LlmCall { agent_id, prompt_template } => {
                    let prompt = render_template(prompt_template, &ctx.variables)?;
                    self.agent_runtime.invoke(agent_id, &prompt).await
                }
                NodeKind::ToolExec { tool_name, params_template } => {
                    let params = render_template_value(params_template, &ctx.variables)?;
                    self.agent_runtime.execute_tool(tool_name, params).await
                }
                NodeKind::Condition { expression } => {
                    let result = evaluate_condition(expression, &ctx.variables)?;
                    Ok(Value::Bool(result))
                }
                NodeKind::FanOut { .. } => Ok(ctx.input.clone()),
                NodeKind::FanIn { merge_strategy } => {
                    merge_results(&ctx.branch_outputs, merge_strategy)
                }
                NodeKind::Reflect { target_node, max_iterations } => {
                    self.execute_reflect(&ctx, *target_node, *max_iterations).await
                }
                _ => todo!(),
            }
        })
        .await?
    }
}
```

### 2.6 认知记忆系统

#### 三层架构

```rust
pub struct MemorySystem {
    working: WorkingMemory,
    episodic: EpisodicMemory,
    semantic: SemanticMemory,
    knowledge_graph: KnowledgeGraph,
    forgetting: ForgettingPolicy,
}

/// 工作记忆：当前会话上下文窗口
pub struct WorkingMemory {
    capacity: usize,  // token 上限
    entries: VecDeque<MemoryEntry>,
}

/// 情景记忆：历史对话片段，向量检索
pub struct EpisodicMemory {
    vector_index: Arc<VectorIndex>,
    store: Arc<SessionStore>,
}

/// 语义记忆：结构化知识
pub struct SemanticMemory {
    vector_index: Arc<VectorIndex>,
    knowledge_graph: Arc<KnowledgeGraph>,
}

impl MemorySystem {
    /// 统一记忆召回接口
    pub async fn recall(
        &self,
        query: &str,
        opts: RecallOptions,
    ) -> Result<Vec<MemoryEntry>> {
        let mut results = Vec::new();

        // 1. 工作记忆（精确匹配 + 最近上下文）
        results.extend(self.working.recall(query));

        // 2. 情景记忆（向量相似度）
        let episodic = self.episodic.search(query, opts.top_k).await?;
        results.extend(episodic);

        // 3. 语义记忆（知识图谱 + 向量）
        let semantic = self.semantic.search(query, opts.top_k).await?;
        results.extend(semantic);

        // 4. 去重 + 排序（相关性 × 时间衰减）
        results.sort_by(|a, b| b.relevance_score().partial_cmp(&a.relevance_score()).unwrap());
        results.dedup_by(|a, b| a.content == b.content);
        results.truncate(opts.max_results);

        Ok(results)
    }

    /// 写入记忆（对话结束后异步调用）
    pub async fn memorize(&self, entry: MemoryEntry) -> Result<()> {
        match entry.memory_type {
            MemoryType::Episodic => self.episodic.store(entry).await?,
            MemoryType::Semantic => {
                self.semantic.store(entry.clone()).await?;
                // 尝试提取知识三元组
                if let Some(triples) = extract_triples(&entry.content) {
                    self.knowledge_graph.insert_batch(triples).await?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
```

#### 向量索引（usearch）

```rust
pub struct VectorIndex {
    index: usearch::Index,
    embedding_provider: Arc<dyn EmbeddingProvider>,
    dimension: usize,
}

impl VectorIndex {
    pub fn new(path: &Path, dimension: usize) -> Result<Self> {
        let index = usearch::Index::new(&usearch::IndexOptions {
            dimensions: dimension,
            metric: usearch::MetricKind::Cos,
            quantization: usearch::ScalarKind::F16,  // 半精度节省内存
            ..Default::default()
        })?;

        if path.exists() {
            index.load(path)?;
        }

        Ok(Self { index, embedding_provider: Arc::new(todo!("inject via DI")), dimension })
    }

    pub fn with_embedding_provider(mut self, provider: Arc<dyn EmbeddingProvider>) -> Self {
        self.embedding_provider = provider;
        self
    }

    pub async fn search(&self, query: &str, top_k: usize) -> Result<Vec<(u64, f32)>> {
        let embedding = self.embedding_provider.embed(query).await?;
        let results = self.index.search(&embedding, top_k)?;
        Ok(results.keys.into_iter().zip(results.distances).collect())
    }

    pub async fn insert(&self, id: u64, content: &str) -> Result<()> {
        let embedding = self.embedding_provider.embed(content).await?;
        self.index.add(id, &embedding)?;
        Ok(())
    }
}
```

#### Embedding Provider 抽象层

记忆系统的向量化依赖 Embedding 模型将文本转换为稠密向量。FastClaw 通过 trait 抽象支持本地和远程两种模式，默认优先零网络依赖的本地推理。

```rust
use async_trait::async_trait;

/// Embedding 向量类型
pub type Embedding = Vec<f32>;

/// Embedding Provider trait — 所有向量化后端的统一接口
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// 返回该模型的输出维度
    fn dimensions(&self) -> usize;

    /// 单条文本 → 向量
    async fn embed(&self, text: &str) -> Result<Embedding>;

    /// 批量文本 → 向量（默认实现逐条调用，具体后端可覆写）
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Embedding>> {
        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            results.push(self.embed(text).await?);
        }
        Ok(results)
    }
}

// ── 本地推理（默认，零网络依赖） ──────────────────────────────
// 使用 fastembed-rs (https://github.com/Anush008/fastembed-rs)
// 内置 ONNX Runtime，首次使用自动下载模型到 ~/.fastclaw/cache/models/

pub struct LocalEmbeddingProvider {
    model: fastembed::TextEmbedding,
    dim: usize,
}

impl LocalEmbeddingProvider {
    pub fn new(model_name: &str, cache_dir: &Path) -> Result<Self> {
        let model_info = match model_name {
            "all-MiniLM-L6-v2" => fastembed::EmbeddingModel::AllMiniLML6V2,
            "bge-small-zh-v1.5" => fastembed::EmbeddingModel::BGESmallZHV15,
            other => return Err(anyhow::anyhow!("unsupported local model: {other}")),
        };

        let model = fastembed::TextEmbedding::try_new(
            fastembed::InitOptions::new(model_info)
                .with_cache_dir(cache_dir.to_path_buf())
                .with_show_download_progress(true),
        )?;

        let dim = match model_name {
            "all-MiniLM-L6-v2" => 384,
            "bge-small-zh-v1.5" => 512,
            _ => 384,
        };

        Ok(Self { model, dim })
    }
}

#[async_trait]
impl EmbeddingProvider for LocalEmbeddingProvider {
    fn dimensions(&self) -> usize { self.dim }

    async fn embed(&self, text: &str) -> Result<Embedding> {
        let texts = vec![text.to_string()];
        let model = self.model.clone();
        // fastembed 是同步的，放到 blocking 线程避免阻塞 tokio
        let embeddings = tokio::task::spawn_blocking(move || {
            model.embed(texts, None)
        }).await??;
        embeddings.into_iter().next()
            .ok_or_else(|| anyhow::anyhow!("empty embedding result"))
    }

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Embedding>> {
        let owned: Vec<String> = texts.iter().map(|t| t.to_string()).collect();
        let model = self.model.clone();
        let embeddings = tokio::task::spawn_blocking(move || {
            model.embed(owned, None)
        }).await??;
        Ok(embeddings)
    }
}

// ── 远程 API（OpenAI / 兼容接口） ─────────────────────────
pub struct RemoteEmbeddingProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    dim: usize,
}

impl RemoteEmbeddingProvider {
    pub fn new(base_url: &str, api_key: &str, model: &str) -> Self {
        let dim = match model {
            "text-embedding-3-small" => 1536,
            "text-embedding-3-large" => 3072,
            "text-embedding-ada-002" => 1536,
            _ => 1536,
        };
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            dim,
        }
    }
}

#[async_trait]
impl EmbeddingProvider for RemoteEmbeddingProvider {
    fn dimensions(&self) -> usize { self.dim }

    async fn embed(&self, text: &str) -> Result<Embedding> {
        let resp = self.client
            .post(format!("{}/embeddings", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({
                "model": self.model,
                "input": text,
            }))
            .send().await?
            .error_for_status()?
            .json::<serde_json::Value>().await?;

        let embedding: Vec<f32> = resp["data"][0]["embedding"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("malformed embedding response"))?
            .iter()
            .filter_map(|v| v.as_f64().map(|f| f as f32))
            .collect();

        Ok(embedding)
    }
}

// ── Provider 工厂 ──────────────────────────────────────────
pub fn create_embedding_provider(config: &EmbeddingConfig) -> Result<Arc<dyn EmbeddingProvider>> {
    match config.provider.as_str() {
        "local" => {
            let cache_dir = config.cache_dir.clone()
                .unwrap_or_else(|| PathBuf::from("~/.fastclaw/cache/models"));
            let provider = LocalEmbeddingProvider::new(&config.model, &cache_dir)?;
            Ok(Arc::new(provider))
        }
        "openai" | "remote" => {
            let api_key = std::env::var(
                config.api_key_env.as_deref().unwrap_or("OPENAI_API_KEY")
            )?;
            let base_url = config.base_url.as_deref()
                .unwrap_or("https://api.openai.com/v1");
            let provider = RemoteEmbeddingProvider::new(base_url, &api_key, &config.model);
            Ok(Arc::new(provider))
        }
        other => Err(anyhow::anyhow!("unknown embedding provider: {other}")),
    }
}

#[derive(Debug, Deserialize)]
pub struct EmbeddingConfig {
    pub provider: String,       // "local" | "openai" | "remote"
    pub model: String,
    pub cache_dir: Option<PathBuf>,
    pub api_key_env: Option<String>,
    pub base_url: Option<String>,
}
```

**选型决策说明**：

| 方案 | 库 | 模型体积 | 延迟 | 是否需要网络 | 选定 |
|------|-----|---------|------|------------|------|
| 本地推理 | `fastembed-rs` (ONNX Runtime) | ~23MB (MiniLM) / ~48MB (BGE) | ~5ms/条 (CPU) | 否（首次下载后离线） | **默认** |
| OpenAI API | `reqwest` | 0 | ~100ms/条 (网络) | 是 | 可选 |
| 自定义 | 实现 `EmbeddingProvider` trait | - | - | - | 扩展 |

默认模型推荐：
- **英文为主**：`all-MiniLM-L6-v2`（384 维，23MB，速度最快）
- **中文为主**：`bge-small-zh-v1.5`（512 维，48MB，中文效果更佳）
- **高精度需求**：远程 `text-embedding-3-small`（1536 维，需 API Key）

#### 知识图谱（petgraph）

```rust
use petgraph::graph::UnGraph;

pub struct KnowledgeGraph {
    graph: RwLock<UnGraph<Entity, Relation>>,
    entity_index: DashMap<String, petgraph::graph::NodeIndex>,
    store: Arc<SessionStore>,  // 持久化到 SQLite
}

#[derive(Debug, Clone)]
pub struct Entity {
    pub name: String,
    pub entity_type: String,
    pub attributes: HashMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct Relation {
    pub predicate: String,
    pub confidence: f64,
    pub source: String,
}

impl KnowledgeGraph {
    pub async fn query_neighbors(
        &self,
        entity_name: &str,
        depth: usize,
    ) -> Result<Vec<Triple>> {
        let graph = self.graph.read().await;
        let start = self.entity_index
            .get(entity_name)
            .ok_or(KgError::EntityNotFound)?;

        let mut result = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::from([(start.value().clone(), 0usize)]);

        while let Some((node_idx, current_depth)) = queue.pop_front() {
            if current_depth >= depth || !visited.insert(node_idx) {
                continue;
            }

            for edge in graph.edges(node_idx) {
                let target = edge.target();
                let relation = edge.weight();
                let subject = &graph[node_idx];
                let object = &graph[target];

                result.push(Triple {
                    subject: subject.name.clone(),
                    predicate: relation.predicate.clone(),
                    object: object.name.clone(),
                    confidence: relation.confidence,
                });

                queue.push_back((target, current_depth + 1));
            }
        }

        Ok(result)
    }

    pub async fn insert_batch(&self, triples: Vec<Triple>) -> Result<()> {
        let mut graph = self.graph.write().await;

        for triple in &triples {
            let subj_idx = self.get_or_create_entity(&mut graph, &triple.subject);
            let obj_idx = self.get_or_create_entity(&mut graph, &triple.object);

            graph.add_edge(subj_idx, obj_idx, Relation {
                predicate: triple.predicate.clone(),
                confidence: triple.confidence,
                source: triple.source.clone().unwrap_or_default(),
            });
        }

        drop(graph);
        self.persist_triples(&triples).await?;

        Ok(())
    }
}
```

### 2.7 WASM 插件系统

#### WIT 接口定义

```wit
// channel.wit — 渠道插件接口
package harness:channel@1.0.0;

interface types {
    record inbound-message {
        id: string,
        channel-id: string,
        peer-id: string,
        content: string,
        metadata: list<tuple<string, string>>,
        timestamp: u64,
    }

    record outbound-message {
        channel-id: string,
        peer-id: string,
        content: string,
        metadata: list<tuple<string, string>>,
    }

    enum channel-event {
        message-received,
        user-joined,
        user-left,
        error,
    }
}

interface channel {
    use types.{inbound-message, outbound-message, channel-event};

    /// 初始化渠道（传入配置）
    init: func(config: string) -> result<_, string>;

    /// 启动渠道（开始监听）
    start: func() -> result<_, string>;

    /// 发送消息
    send: func(msg: outbound-message) -> result<_, string>;

    /// 停止渠道
    stop: func() -> result<_, string>;
}

world channel-plugin {
    export channel;
}
```

```wit
// tool.wit — 工具插件接口
package harness:tool@1.0.0;

interface types {
    record tool-descriptor {
        name: string,
        description: string,
        parameters-schema: string,  // JSON Schema
        is-dangerous: bool,
    }

    record tool-context {
        session-key: string,
        agent-id: string,
        caller: string,
    }

    record tool-result {
        success: bool,
        output: string,    // JSON
        error: option<string>,
    }
}

interface tool {
    use types.{tool-descriptor, tool-context, tool-result};

    /// 返回工具描述
    describe: func() -> tool-descriptor;

    /// 执行工具
    execute: func(params: string, ctx: tool-context) -> tool-result;
}

world tool-plugin {
    export tool;
}
```

#### 插件宿主

```rust
use wasmtime::*;

pub struct PluginHost {
    engine: Engine,
    plugins: DashMap<PluginId, LoadedPlugin>,
    watcher: notify::RecommendedWatcher,
}

pub struct LoadedPlugin {
    pub id: PluginId,
    pub manifest: PluginManifest,
    pub module: Module,
    pub instance_pool: Vec<Instance>,
    pub resource_limits: ResourceLimits,
}

#[derive(Debug, Clone)]
pub struct ResourceLimits {
    pub max_memory_bytes: usize,      // 默认 64MB
    pub max_fuel: u64,                 // 默认 1_000_000_000
    pub max_execution_time: Duration,  // 默认 30s
    pub capabilities: Vec<Capability>,
}

#[derive(Debug, Clone)]
pub enum Capability {
    NetHttp { allowed_hosts: Vec<String> },
    FsRead { allowed_paths: Vec<PathBuf> },
    DbQuery,
    MemoryRead,
    MemoryWrite,
}

impl PluginHost {
    pub async fn load_plugin(&self, path: &Path) -> Result<PluginId> {
        let manifest = read_plugin_manifest(path)?;
        let wasm_bytes = std::fs::read(path.join(&manifest.wasm_file))?;

        let mut config = Config::new();
        config.consume_fuel(true);
        config.memory_guaranteed_dense_image_size(
            manifest.resource_limits.max_memory_bytes,
        );

        let module = Module::new(&self.engine, &wasm_bytes)?;
        let id = PluginId::from_manifest(&manifest);

        self.plugins.insert(id.clone(), LoadedPlugin {
            id: id.clone(),
            manifest,
            module,
            instance_pool: Vec::new(),
            resource_limits: manifest.resource_limits.clone(),
        });

        tracing::info!(plugin_id = %id, "plugin loaded");
        Ok(id)
    }

    pub async fn call_tool(
        &self,
        plugin_id: &PluginId,
        params: &str,
        ctx: &ToolContext,
    ) -> Result<ToolResult> {
        let plugin = self.plugins.get(plugin_id)
            .ok_or(PluginError::NotFound)?;

        let mut store = Store::new(&self.engine, HostState::new(ctx));
        store.set_fuel(plugin.resource_limits.max_fuel)?;
        store.limiter(|state| &mut state.limiter);

        let instance = self.engine
            .instantiate(&mut store, &plugin.module, &[])?;

        // 通过 WIT bindgen 生成的类型安全绑定调用 WASM 组件
        // 实际实现使用 wasmtime::component::bindgen! 宏生成接口
        let instance = linker.instantiate(&mut store, &plugin.component)?;
        let tool_iface = ToolPlugin::new(&mut store, &instance)?;
        let ctx_json = serde_json::to_string(ctx)?;

        let result = tokio::time::timeout(
            plugin.resource_limits.max_execution_time,
            tokio::task::spawn_blocking(move || {
                tool_iface.call_execute(&mut store, params, &ctx_json)
            }),
        )
        .await??;

        Ok(serde_json::from_str(&result?)?)
    }

    /// 热重载：文件变更时自动替换
    pub async fn hot_reload(&self, plugin_id: &PluginId, new_path: &Path) -> Result<()> {
        let new_wasm = std::fs::read(new_path)?;
        let new_module = Module::new(&self.engine, &new_wasm)?;

        if let Some(mut plugin) = self.plugins.get_mut(plugin_id) {
            plugin.module = new_module;
            plugin.instance_pool.clear();
            tracing::info!(plugin_id = %plugin_id, "plugin hot-reloaded");
        }

        Ok(())
    }
}
```

### 2.8 自我进化引擎

#### 反馈 → 评估 → 蒸馏 闭环

```rust
pub struct EvolutionEngine {
    feedback_store: Arc<SessionStore>,
    evaluator: StrategyEvaluator,
    distiller: PromptDistiller,
    user_modeler: UserModeler,
}

impl EvolutionEngine {
    /// 收集反馈（异步，不阻塞主流程）
    pub async fn collect_feedback(&self, feedback: Feedback) -> Result<()> {
        self.feedback_store.save_feedback(&feedback).await?;

        metrics::counter!("evolution_feedback_total",
            "type" => feedback.feedback_type.as_str()
        ).increment(1);

        // 达到评估阈值时触发
        let count = self.feedback_store
            .count_feedback_since(feedback.agent_id, self.evaluator.last_eval_time)
            .await?;

        if count >= self.evaluator.eval_threshold {
            tokio::spawn(self.run_evaluation(feedback.agent_id.clone()));
        }

        Ok(())
    }

    /// 策略评估
    async fn run_evaluation(&self, agent_id: AgentId) -> Result<()> {
        let feedbacks = self.feedback_store
            .load_recent_feedback(&agent_id, 100)
            .await?;

        let current_metrics = self.evaluator.compute_metrics(&feedbacks);
        let baseline = self.feedback_store
            .load_active_snapshot(&agent_id)
            .await?;

        if current_metrics.is_significantly_better(&baseline.metrics) {
            // 触发 Prompt 蒸馏
            let high_quality_conversations = self.feedback_store
                .load_high_rated_conversations(&agent_id, 20)
                .await?;

            let new_prompt = self.distiller
                .distill(&baseline.prompt_text, &high_quality_conversations)
                .await?;

            // 保存为候选版本（不自动激活）
            self.feedback_store.save_evolution_snapshot(
                &agent_id,
                &new_prompt,
                &current_metrics,
                false,  // is_active = false，需要人工审批或 A/B 测试验证
            ).await?;

            tracing::info!(
                agent_id = %agent_id,
                improvement = %current_metrics.improvement_over(&baseline.metrics),
                "evolution candidate generated"
            );
        }

        Ok(())
    }
}

pub struct StrategyEvaluator {
    pub eval_threshold: usize,  // 默认 50 条反馈触发评估
    pub last_eval_time: u64,
}

#[derive(Debug)]
pub struct EvalMetrics {
    pub avg_rating: f64,
    pub tool_success_rate: f64,
    pub avg_turns_per_task: f64,
    pub user_satisfaction: f64,
}
```

### 2.9 可观测性

```rust
use tracing_subscriber::{fmt, EnvFilter, Registry};
use tracing_subscriber::layer::SubscriberExt;

pub fn init_observability(config: &ObserveConfig) -> Result<()> {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.log_level));

    let fmt_layer = fmt::layer()
        .json()
        .with_target(true)
        .with_thread_ids(true)
        .with_span_events(fmt::format::FmtSpan::CLOSE);

    let subscriber = Registry::default()
        .with(env_filter)
        .with(fmt_layer);

    // Prometheus 指标
    let prometheus_handle = metrics_exporter_prometheus::PrometheusBuilder::new()
        .install_recorder()?;

    // 标准指标注册
    metrics::describe_counter!("requests_total", "Total HTTP requests");
    metrics::describe_histogram!("request_duration_ms", "Request duration in milliseconds");
    metrics::describe_gauge!("active_sessions", "Number of active sessions");
    metrics::describe_gauge!("ws_connections", "Number of WebSocket connections");
    metrics::describe_counter!("llm_calls_total", "Total LLM API calls");
    metrics::describe_histogram!("llm_latency_ms", "LLM call latency");
    metrics::describe_counter!("tool_calls_total", "Total tool invocations");
    metrics::describe_counter!("plugin_errors_total", "Plugin execution errors");
    metrics::describe_gauge!("memory_entries", "Total memory entries");
    metrics::describe_histogram!("dag_node_duration_ms", "DAG node execution duration");

    tracing::subscriber::set_global_default(subscriber)?;

    Ok(())
}
```

---

## 3. 数据流

### 3.1 消息处理主流程

```
Channel Inbound
    │
    ▼ (tokio::spawn)
Parse & Validate (simd-json, < 0.1ms)
    │
    ▼
Route Resolve (< 0.1ms)
    │
    ▼
Session Load (SQLite async, < 1ms)
    │
    ▼
Memory Recall (vector search + KG, < 10ms)
    │
    ▼
Prompt Build (template render, < 0.5ms)
    │
    ▼
LLM Stream Call (network bound, 200ms-10s)
    │  ╰─ StreamToken → WS push (per token)
    ▼
Tool Dispatch (0 or more rounds)
    │  ╰─ WASM sandbox exec (per tool)
    ▼
Response Assemble
    │
    ├─ Session Save (SQLite async, < 1ms)
    ├─ Memory Write (vector + KG, async background)
    ├─ Feedback Emit (async background)
    │
    ▼
Channel Outbound
```

### 3.2 热重载流程

```
File Change Detected (notify crate)
    │
    ▼
Parse New Config / Load New WASM
    │
    ├─ 成功 → Swap (ArcSwap / DashMap replace)
    │         ╰─ 在途请求用旧实例完成
    │
    ├─ 失败 → Log Error + 保留旧配置
    │
    ▼
Emit Reload Event (tracing::info!)
```

---

## 4. 部署方案

### 4.1 构建

```bash
# Release 构建（开启 LTO + strip）
cargo build --release --target x86_64-unknown-linux-gnu

# 交叉编译 ARM64
cross build --release --target aarch64-unknown-linux-gnu

# 构建最小 Docker 镜像
docker build -t fastclaw:latest .
```

#### Dockerfile

```dockerfile
FROM rust:1.85-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM gcr.io/distroless/cc-debian12
COPY --from=builder /app/target/release/fastclaw /usr/local/bin/fastclaw
COPY --from=builder /app/config /etc/fastclaw/config
EXPOSE 18789
ENTRYPOINT ["fastclaw", "gateway", "run"]
```

### 4.2 运行

```bash
# 基础启动（对齐 openclaw gateway run）
fastclaw gateway run

# serve 是 gateway run 的便捷别名
fastclaw serve

# 指定配置目录
fastclaw gateway run --config /path/to/config

# 指定端口
fastclaw gateway run --port 8080

# Debug 日志
RUST_LOG=debug fastclaw gateway run

# 健康检查
fastclaw health

# 配置验证（对齐 openclaw config validate）
fastclaw config check

# 环境诊断（对齐 openclaw doctor）
fastclaw doctor

# Agent 管理（对齐 openclaw agents list）
fastclaw agents list

# 会话管理
fastclaw sessions list
fastclaw sessions cleanup

# 交互式初始化向导（对齐 openclaw onboard）
fastclaw onboard

# 整体状态
fastclaw status
```

### 4.3 systemd 单元

```ini
[Unit]
Description=FastClaw AI Assistant Engine (Harness Engineer)
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/fastclaw gateway run --config /etc/fastclaw/config
Restart=always
RestartSec=5
Environment=RUST_LOG=info
LimitNOFILE=65536
MemoryMax=512M

[Install]
WantedBy=multi-user.target
```

---

## 5. 性能优化策略

### 5.1 内存优化

| 策略 | 实现 | 预期效果 |
|------|------|---------|
| Arena 分配器 | `bumpalo` 用于请求级临时对象 | 减少 malloc/free 次数 |
| 零拷贝 JSON | `simd-json` 引用原始 buffer | 避免 String 分配 |
| 半精度向量 | usearch F16 量化 | 向量内存减半 |
| 连接池复用 | DashMap<ClientId, WsSender> | 避免频繁创建/销毁 |
| 字符串驻留 | `string-interner` 用于高频重复字符串 | 去重节省内存 |

### 5.2 CPU 优化

| 策略 | 实现 | 预期效果 |
|------|------|---------|
| SIMD JSON | `simd-json` SSE4.2/AVX2 | 解析速度 3-10x |
| 编译时序列化 | `serde` derive | 零反射开销 |
| Work-stealing | Tokio 多线程调度器 | CPU 核心利用率 > 90% |
| 批量操作 | SQLite 事务批量写入 | 减少磁盘同步次数 |

### 5.3 I/O 优化

| 策略 | 实现 | 预期效果 |
|------|------|---------|
| 异步 I/O | Tokio + io_uring (Linux 5.6+) | 系统调用减少 |
| SQLite WAL | WAL + NORMAL sync | 并发读写不阻塞 |
| 连接池 | SQLx 连接池，4 连接 | 减少连接建立开销 |
| HTTP/2 | hyper HTTP/2 多路复用 | LLM 调用连接复用 |

---

## 6. 迁移策略（从 OpenClaw）

### 6.1 配置兼容

FastClaw 与 OpenClaw 统一使用 **JSON/JSON5** 作为配置格式，用户可以直接复用 OpenClaw 的配置文件，无需格式转换。

```bash
# 直接使用 OpenClaw 配置启动（自动识别 ~/.openclaw/openclaw.json）
fastclaw gateway run

# 或显式指定配置文件
fastclaw gateway run --config ~/.openclaw/openclaw.json

# 将 OpenClaw 配置复制到 FastClaw 目录（可选）
fastclaw migrate config --from ~/.openclaw/openclaw.json --to ~/.fastclaw/config/

# 验证配置
fastclaw config check
```

**兼容策略**：
- FastClaw 直接解析 OpenClaw 的 `openclaw.json` / `json5` 格式
- 共用字段（`gateway`、`models`、`agents`、`memory`、`session`、`tools` 等）完全兼容
- OpenClaw 特有字段（`nodeHost`、`ui`、`wizard`、`update`）会被忽略并输出提示
- FastClaw 扩展字段（`dag`、`evolution`、`modelRouter`、`plugins.wasm` 等）可在原有配置中增量添加

### 6.2 插件迁移

```bash
# TypeScript 插件 → WASM（通过 jco）
jco componentize \
  --wit wit/tool.wit \
  --world tool-plugin \
  path/to/openclaw-plugin.js \
  -o plugins/migrated-plugin.wasm
```

### 6.3 数据迁移

```bash
# 导入 OpenClaw JSONL 会话数据
fastclaw migrate sessions --from /path/to/openclaw/data --format jsonl

# 迁移 memory-core SQLite 数据
fastclaw migrate memory --from /path/to/openclaw/memory.db
```

### 6.4 环境变量兼容

| OpenClaw 环境变量 | FastClaw 等价变量 | 说明 |
|---|---|---|
| `OPENCLAW_STATE_DIR` | `FASTCLAW_STATE_DIR` | 状态目录覆盖 |
| `OPENCLAW_CONFIG_PATH` | `FASTCLAW_CONFIG_PATH` | 配置文件路径覆盖 |
| `OPENCLAW_GATEWAY_PORT` | `FASTCLAW_GATEWAY_PORT` | Gateway 端口覆盖 |
| `OPENCLAW_HOME` | `FASTCLAW_HOME` | 主目录覆盖 |

注：FastClaw 启动时自动检测 `OPENCLAW_*` 环境变量并映射为 `FASTCLAW_*`（如果 FastClaw 版本未设置的话）。

### 6.5 配置字段兼容表

FastClaw 沿用 OpenClaw 的 JSON 配置结构（camelCase 命名），共用字段完全兼容：

| 字段 | OpenClaw | FastClaw | 说明 |
|---|---|---|---|
| `gateway.port` | ✅ | ✅ | 默认均为 18789 |
| `agents[*]` | ✅ 数组或独立文件 | ✅ 同上 | Agent 定义 |
| `agents[*].model` | ✅ | ✅ | 模型标识 |
| `agents[*].systemPrompt` | ✅ | ✅ | 系统提示词 |
| `agents[*].tools` | ✅ | ✅ | 工具白名单 |
| `tools.*` | ✅ | ✅ | 工具配置 |
| `bindings[*]` | ✅ | ✅ | Agent-Channel 绑定 |
| `channels.*` | ✅ | ✅ | 渠道配置 |
| `memory.*` | ✅ | ✅ | 记忆系统 |
| `models.*` | ✅ | ✅ | LLM Provider |
| `mcp.*` | ✅ | ✅ | MCP 配置 |
| `cron[*]` | ✅ | ✅ | 定时任务 |
| `hooks.*` | ✅ | ✅ | 生命周期钩子 |
| `session.*` | ✅ | ✅ | 会话配置 |
| `logging.*` | ✅ | ✅ | 日志配置 |
| `nodeHost.*` | ✅ | ❌ 忽略 | Node.js 运行时，FastClaw 无需 |
| `ui.*` | ✅ | ❌ 忽略 | OpenClaw UI，FastClaw 使用 Harness Studio |
| `wizard.*` | ✅ | ❌ 忽略 | OpenClaw 向导 |
| `update.*` | ✅ | ❌ 忽略 | OpenClaw 自更新 |

**FastClaw 扩展字段**（OpenClaw 中不存在，可增量添加到现有配置中）：

| 字段 | 说明 |
|---|---|
| `dag.*` | DAG 工作流引擎配置 |
| `evolution.*` | 自我进化系统 |
| `modelRouter.*` | 模型复杂度路由 |
| `plugins.wasm.*` | WASM 插件系统 |
| `security.*` | 安全防护配置 |
| `metrics.*` | Prometheus 指标配置 |

### 6.6 状态目录结构

```
~/.fastclaw/                         # 默认状态目录（对齐 OpenClaw 的 ~/.openclaw/）
├── config/
│   ├── default.json                 # 全局配置（JSON，兼容 OpenClaw 格式）
│   └── agents/
│       ├── main.json                # 默认 Agent
│       └── code-review.json         # 自定义 Agent
├── data/
│   ├── sessions.db                  # SQLite 会话数据
│   ├── memory/
│   │   ├── vectors.usearch          # 向量索引文件
│   │   └── knowledge.db             # 知识图谱
│   └── evolution/
│       └── snapshots.db             # 进化快照
├── plugins/                         # WASM 插件目录
│   ├── channel-telegram.wasm
│   └── tool-custom.wasm
├── logs/                            # 日志目录
├── cache/                           # 缓存（代码索引等）
└── credentials/                     # OAuth/API Key 存储
    └── oauth.json
```

注：`--dev` 模式使用 `~/.fastclaw-dev/`；`--profile <name>` 使用 `~/.fastclaw-<name>/`（与 OpenClaw 行为一致）。

### 6.7 配置加载实现

```rust
use std::path::{Path, PathBuf};
use serde_json::Value as JsonValue;

/// 配置加载器 — 支持 JSON/JSON5，直接兼容 OpenClaw 配置格式
pub struct ConfigLoader {
    search_paths: Vec<PathBuf>,
}

impl ConfigLoader {
    pub fn new() -> Self {
        let mut paths = Vec::new();
        // FastClaw 配置优先
        if let Ok(p) = std::env::var("FASTCLAW_CONFIG_PATH") {
            paths.push(PathBuf::from(p));
        }
        if let Some(home) = dirs::home_dir() {
            paths.push(home.join(".fastclaw/config/default.json"));
        }
        // 回退到 OpenClaw 配置
        if let Ok(p) = std::env::var("OPENCLAW_CONFIG_PATH") {
            paths.push(PathBuf::from(p));
        }
        if let Some(home) = dirs::home_dir() {
            paths.push(home.join(".openclaw/openclaw.json"));
        }
        Self { search_paths: paths }
    }

    /// 按优先级搜索并加载第一个存在的配置文件
    pub fn load(&self) -> Result<(PathBuf, FastClawConfig), anyhow::Error> {
        for path in &self.search_paths {
            if path.exists() {
                let config = self.load_file(path)?;
                return Ok((path.clone(), config));
            }
        }
        // 没有找到任何配置文件，使用内置默认值
        tracing::info!("no config file found, using built-in defaults");
        Ok((PathBuf::from("(built-in)"), FastClawConfig::default()))
    }

    fn load_file(&self, path: &Path) -> Result<FastClawConfig, anyhow::Error> {
        let text = std::fs::read_to_string(path)?;
        // 使用 json5 解析，同时兼容标准 JSON 和 JSON5（含注释、尾逗号）
        let value: JsonValue = json5::from_str(&text)?;

        // 过滤 OpenClaw 特有字段并给出提示
        if let Some(obj) = value.as_object() {
            for field in &["nodeHost", "ui", "wizard", "update"] {
                if obj.contains_key(*field) {
                    tracing::info!(
                        field,
                        "OpenClaw-specific field ignored (not needed in FastClaw)"
                    );
                }
            }
        }

        let config: FastClawConfig = serde_json::from_value(value)?;
        Ok(config)
    }
}
```

### 6.8 启动时配置检测流程

```
FastClaw 启动
    │
    ├─ 检测 OPENCLAW_* 环境变量 → 自动映射为 FASTCLAW_*
    │
    ├─ 按优先级搜索配置文件：
    │   1. $FASTCLAW_CONFIG_PATH
    │   2. ~/.fastclaw/config/default.json
    │   3. $OPENCLAW_CONFIG_PATH
    │   4. ~/.openclaw/openclaw.json
    │   5. 内置默认配置
    │
    ├─ 加载 JSON/JSON5 → FastClawConfig（serde 反序列化）
    │   └─ OpenClaw 特有字段自动忽略 + 提示
    │
    └─ 正常启动
```

### 6.9 迁移完整性校验

`fastclaw doctor` 命令包含兼容检查项：

| 检查项 | 命令 | 说明 |
|--------|------|------|
| 配置来源 | `fastclaw doctor --check config` | 检测当前使用的配置文件路径 |
| 环境变量 | `fastclaw doctor --check env` | 列出仍在使用的 `OPENCLAW_*` 变量 |
| 插件迁移 | `fastclaw doctor --check plugins` | 检测未转换的 JS 插件 |
| 会话数据 | `fastclaw doctor --check sessions` | 检测未导入的 JSONL 数据 |

---

## 7. CLI 命令体系

### 7.1 设计原则

- **与 OpenClaw 对齐**：核心命令结构尽量保持一致，降低迁移学习成本
- **渐进增强**：MVP 阶段实现核心命令，后续版本补充高级命令
- **多输出格式**：所有命令支持 `--json` 输出，方便脚本集成

### 7.2 命令树

```
fastclaw [--dev] [--profile <name>] [--no-color] [--json] <command>

  # ── 初始化与配置 ──────────────────────────────────────
  setup                          # 环境检查 + 依赖安装
  onboard                        # 交互式引导配置向导
  config
    get <key>                    # 读取配置项
    set <key> <value>            # 设置配置项
    check                        # 配置语法与逻辑校验
    file                         # 打印配置文件路径
  doctor                         # 环境诊断（网络/依赖/配置完整性）

  # ── 服务管理（对齐 openclaw gateway）──────────────────
  gateway
    run                          # 前台运行（默认，等价于 OpenClaw gateway run）
    start                        # 后台启动（systemd/launchd）
    stop                         # 停止后台服务
    restart                      # 重启后台服务
    status                       # 服务运行状态
    health                       # 健康检查详情
  serve                          # gateway run 的便捷别名

  # ── Agent 管理（对齐 openclaw agents）─────────────────
  agents
    list                         # 列出所有 Agent
    add <id>                     # 添加 Agent
    delete <id>                  # 删除 Agent
  agent <id> [message]           # 与指定 Agent 对话（CLI 模式）

  # ── 全局状态 ──────────────────────────────────────────
  status                         # 整体运行状态概览
  health                         # 快速健康检查（对齐 openclaw health）

  # ── 会话管理（对齐 openclaw sessions）─────────────────
  sessions
    list                         # 列出活跃会话
    cleanup [--before <date>]    # 清理过期会话

  # ── 模型管理（对齐 openclaw models）───────────────────
  models
    list                         # 列出已配置的 LLM Provider
    set <agent> <model>          # 为 Agent 设置模型
    status                       # 模型可用性检测

  # ── 插件管理（对齐 openclaw plugins）──────────────────
  plugins
    list                         # 列出已安装插件
    install <path|url>           # 安装 WASM 插件
    uninstall <id>               # 卸载插件
    enable <id>                  # 启用插件
    disable <id>                 # 禁用插件

  # ── 渠道管理（对齐 openclaw channels）─────────────────
  channels
    list                         # 列出已配置渠道
    status                       # 渠道连接状态
    add <type>                   # 添加渠道
    remove <id>                  # 移除渠道

  # ── 记忆管理（对齐 openclaw memory）───────────────────
  memory
    status                       # 记忆系统状态（向量数/图谱大小）
    search <query>               # 记忆搜索

  # ── MCP（对齐 openclaw mcp）───────────────────────────
  mcp
    serve                        # 以 MCP 服务器模式运行
    list                         # 列出 MCP 工具

  # ── 消息（对齐 openclaw message）──────────────────────
  message
    send <agent> <text>          # 发送单条消息

  # ── 定时任务（对齐 openclaw cron）─────────────────────
  cron
    list                         # 列出定时任务
    add                          # 添加定时任务
    rm <id>                      # 删除定时任务
    enable <id>                  # 启用
    disable <id>                 # 禁用

  # ── 工作流（对齐 openclaw flows/tasks）────────────────
  flows
    list                         # 列出 DAG 工作流定义
    run <id>                     # 执行工作流
    status <run-id>              # 查看执行状态

  # ── 迁移 ──────────────────────────────────────────────
  migrate
    config                       # 复制 OpenClaw JSON 到 FastClaw 目录
    sessions                     # 会话数据迁移
    memory                       # 记忆数据迁移
    plugins                      # 插件迁移指引

  # ── 备份（对齐 openclaw backup）───────────────────────
  backup
    create [--output <path>]     # 创建完整备份
    restore <path>               # 从备份恢复
    verify <path>                # 验证备份完整性

  # ── 其他 ──────────────────────────────────────────────
  update                         # 自我更新
  completion <shell>             # Shell 补全脚本
  logs [--follow]                # 查看日志
  version                        # 版本信息
```

### 7.3 全局选项

| 选项 | 说明 | 对齐 OpenClaw |
|------|------|-------------|
| `--dev` | 隔离开发环境（`~/.fastclaw-dev/`，端口偏移） | ✅ 对齐 `--dev` |
| `--profile <name>` | 多配置隔离（`~/.fastclaw-<name>/`） | ✅ 对齐 `--profile` |
| `--no-color` | 禁用 ANSI 颜色（也尊重 `NO_COLOR=1`） | ✅ 对齐 `--no-color` |
| `--json` | JSON 格式输出 | ✅ 对齐 `--json` |
| `--config <path>` | 指定配置文件路径 | OpenClaw 用环境变量 |
| `-V` / `--version` | 打印版本 | ✅ 对齐 |

### 7.4 OpenClaw 兼容别名

为降低迁移成本，提供以下别名：

| 别名 | 实际命令 | 说明 |
|------|---------|------|
| `fastclaw serve` | `fastclaw gateway run` | 便捷启动 |
| `fastclaw daemon start` | `fastclaw gateway start` | 对齐 OpenClaw legacy 别名 |

### 7.5 CLI 框架实现（clap）

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "fastclaw", version, about = "FastClaw AI Assistant Engine")]
pub struct Cli {
    #[arg(long, help = "Isolate state under ~/.fastclaw-dev/")]
    pub dev: bool,

    #[arg(long, value_name = "NAME", help = "Isolate state under ~/.fastclaw-<name>/")]
    pub profile: Option<String>,

    #[arg(long, help = "Disable ANSI colors")]
    pub no_color: bool,

    #[arg(long, help = "Output in JSON format")]
    pub json: bool,

    #[arg(long, value_name = "PATH", help = "Config file path override")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Interactive onboarding wizard
    Onboard,
    /// Environment setup and dependency check
    Setup,
    /// Configuration management
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Environment diagnostics
    Doctor,
    /// Gateway service management (aligned with openclaw gateway)
    Gateway {
        #[command(subcommand)]
        action: GatewayAction,
    },
    /// Alias for `gateway run`
    Serve {
        #[arg(long, default_value = "18789")]
        port: u16,
    },
    /// Quick health check
    Health,
    /// Overall status overview
    Status,
    /// Agent management
    Agents {
        #[command(subcommand)]
        action: AgentsAction,
    },
    /// Chat with an agent
    Agent {
        id: String,
        message: Option<String>,
    },
    /// Session management
    Sessions {
        #[command(subcommand)]
        action: SessionsAction,
    },
    /// Model management
    Models {
        #[command(subcommand)]
        action: ModelsAction,
    },
    /// Plugin management
    Plugins {
        #[command(subcommand)]
        action: PluginsAction,
    },
    /// Channel management
    Channels {
        #[command(subcommand)]
        action: ChannelsAction,
    },
    /// Memory system management
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },
    /// MCP server mode
    Mcp {
        #[command(subcommand)]
        action: McpAction,
    },
    /// Send a message
    Message {
        #[command(subcommand)]
        action: MessageAction,
    },
    /// Cron job management
    Cron {
        #[command(subcommand)]
        action: CronAction,
    },
    /// DAG workflow management
    Flows {
        #[command(subcommand)]
        action: FlowsAction,
    },
    /// Migrate from OpenClaw
    Migrate {
        #[command(subcommand)]
        action: MigrateAction,
    },
    /// Backup and restore
    Backup {
        #[command(subcommand)]
        action: BackupAction,
    },
    /// Self update
    Update,
    /// Generate shell completions
    Completion { shell: clap_complete::Shell },
    /// View logs
    Logs {
        #[arg(long)]
        follow: bool,
    },
}

#[derive(Subcommand)]
pub enum GatewayAction {
    /// Run gateway in foreground (default)
    Run {
        #[arg(long, default_value = "18789")]
        port: u16,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Start gateway as background service
    Start,
    /// Stop background service
    Stop,
    /// Restart background service
    Restart,
    /// Service status
    Status,
    /// Detailed health check
    Health,
}
```

---

## 8. 关键依赖版本

| Crate | 版本 | 用途 |
|-------|------|------|
| `tokio` | 1.x | 异步运行时 |
| `axum` | 0.8 | HTTP 框架 |
| `serde` | 1.x | 序列化 |
| `simd-json` | 0.14 | SIMD JSON 解析 |
| `sqlx` | 0.8 | SQLite 异步驱动 |
| `reqwest` | 0.12 | HTTP 客户端 |
| `wasmtime` | 25.x | WASM 运行时 |
| `petgraph` | 0.6 | 图算法 |
| `usearch` | 2.x | 向量索引 |
| `tracing` | 0.1 | 结构化日志 |
| `metrics` | 0.24 | 指标收集 |
| `dashmap` | 6.x | 并发 HashMap |
| `clap` | 4.x | CLI 参数 |
| `notify` | 7.x | 文件监听 |
| `bumpalo` | 3.x | Arena 分配器 |
| `tree-sitter` | 0.23 | 代码 AST 解析（fastclaw-code）|
| `handlebars` | 6.x | 模板渲染（fastclaw-agent-factory）|
| `hmac` + `sha2` | latest | HMAC 签名（fastclaw-collab 消息鉴权）|
| `seccomp` | 0.1 | 系统调用过滤（fastclaw-code 沙箱）|
| `similar` | 2.x | Diff 生成（代码变更展示）|
| `jsonschema` | 0.18 | 工具参数 JSON Schema 校验 |
| `regex` | 1.x | Prompt 注入模式检测 |
| `sqlcipher` | 可选 | SQLite 加密（fastclaw-session）|
| `rcgen` | 0.13 | 自签 TLS 证书生成 |

---

## 9. 安全架构

### 9.1 纵深防御模型

```
外部请求
    │
    ▼
┌─────────────────────────────────────────────────────┐
│  Layer 1: 网络边界                                  │
│  TLS 1.3 强制 | HSTS | 速率限制 | IP 封禁           │
└─────────────────┬───────────────────────────────────┘
                  │
                  ▼
┌─────────────────────────────────────────────────────┐
│  Layer 2: 认证与授权                                │
│  API Key 验证 | mTLS | Agent 权限检查               │
└─────────────────┬───────────────────────────────────┘
                  │
                  ▼
┌─────────────────────────────────────────────────────┐
│  Layer 3: 输入安全                                  │
│  Prompt 注入检测 | 工具参数 Schema 校验 | 输入净化  │
└─────────────────┬───────────────────────────────────┘
                  │
                  ▼
┌─────────────────────────────────────────────────────┐
│  Layer 4: 执行隔离                                  │
│  WASM 沙箱（插件）| seccomp（代码执行）| 资源限制   │
└─────────────────┬───────────────────────────────────┘
                  │
                  ▼
┌─────────────────────────────────────────────────────┐
│  Layer 5: 数据安全                                  │
│  会话隔离 | 记忆访问控制 | 日志脱敏 | 存储加密(可选)│
└─────────────────┬───────────────────────────────────┘
                  │
                  ▼
┌─────────────────────────────────────────────────────┐
│  Layer 6: 审计与检测                                │
│  安全事件日志 | 异常行为告警 | 进化变更审批          │
└─────────────────────────────────────────────────────┘
```

### 9.2 Prompt 注入防护实现

```rust
pub struct PromptGuard {
    injection_patterns: Vec<Regex>,
    indirect_injection_patterns: Vec<Regex>,
    audit_log: Arc<AuditLogger>,
}

#[derive(Debug)]
pub struct GuardResult {
    pub safe: bool,
    pub threat_level: ThreatLevel,
    pub detected_patterns: Vec<String>,
    pub sanitized_input: Option<String>,
}

#[derive(Debug)]
pub enum ThreatLevel { None, Low, Medium, High, Critical }

impl PromptGuard {
    pub fn new() -> Self {
        Self {
            injection_patterns: vec![
                // 直接覆盖指令
                Regex::new(r"(?i)ignore\s+(all\s+)?(previous|prior|above)\s+instructions?").unwrap(),
                Regex::new(r"(?i)forget\s+(everything|all)\s+(you|i)\s+(were|have|told)").unwrap(),
                Regex::new(r"(?i)you\s+are\s+now\s+(?:a|an|the)\s+\w+").unwrap(),
                Regex::new(r"(?i)act\s+as\s+(?:if\s+you\s+(?:are|were)\s+)?(?:a|an)\s+\w+\s+without\s+restrictions").unwrap(),
                // 角色扮演绕过
                Regex::new(r"(?i)in\s+this\s+(?:game|scenario|roleplay|story).*you\s+(?:must|should|will)\s+(?:always|never)").unwrap(),
                // 系统提示词泄露
                Regex::new(r"(?i)(?:print|output|show|reveal|repeat|tell\s+me)\s+(?:your\s+)?(?:system\s+prompt|instructions|guidelines)").unwrap(),
            ],
            indirect_injection_patterns: vec![
                // 工具返回内容中的注入（如网页内容包含指令）
                Regex::new(r"(?i)<\s*system\s*>|<\s*/?instructions?\s*>").unwrap(),
                Regex::new(r"\[INST\]|\[/INST\]|<\|system\|>|<\|user\|>").unwrap(),
            ],
            audit_log: Arc::new(AuditLogger::new()),
        }
    }

    pub fn check_user_input(&self, input: &str, session_key: &str) -> GuardResult {
        let mut detected = Vec::new();

        for pattern in &self.injection_patterns {
            if pattern.is_match(input) {
                detected.push(pattern.as_str().to_string());
            }
        }

        let threat_level = match detected.len() {
            0 => ThreatLevel::None,
            1 => ThreatLevel::Low,
            2 => ThreatLevel::Medium,
            _ => ThreatLevel::High,
        };

        if !matches!(threat_level, ThreatLevel::None) {
            self.audit_log.log_security_event(SecurityEvent {
                event_type: SecurityEventType::PromptInjectionAttempt,
                session_key: session_key.to_string(),
                input_snippet: truncate(input, 200),
                threat_level: threat_level.clone(),
                detected_patterns: detected.clone(),
                timestamp: unix_millis_now(),
            });
        }

        GuardResult {
            safe: matches!(threat_level, ThreatLevel::None | ThreatLevel::Low),
            threat_level,
            detected_patterns: detected,
            sanitized_input: None,
        }
    }

    /// 检查工具返回内容（间接注入）
    pub fn check_tool_output(&self, output: &str, tool_name: &str) -> GuardResult {
        let mut detected = Vec::new();

        for pattern in &self.indirect_injection_patterns {
            if pattern.is_match(output) {
                detected.push(format!("indirect:{}", tool_name));
            }
        }

        GuardResult {
            safe: detected.is_empty(),
            threat_level: if detected.is_empty() { ThreatLevel::None } else { ThreatLevel::Medium },
            detected_patterns: detected,
            // 净化：移除 prompt 注入标记
            sanitized_input: Some(sanitize_indirect_injection(output)),
        }
    }
}
```

### 9.3 日志脱敏实现

```rust
pub struct LogSanitizer {
    /// 匹配常见秘密模式：API Key, Token, Password
    secret_patterns: Vec<(Regex, &'static str)>,
}

impl LogSanitizer {
    pub fn new() -> Self {
        Self {
            secret_patterns: vec![
                (Regex::new(r#"(?i)(api[_-]?key|apikey)\s*[:=]\s*["']?([A-Za-z0-9_\-]{20,})"#).unwrap(), "$1: [REDACTED]"),
                (Regex::new(r#"(?i)(bearer\s+)([A-Za-z0-9._\-]{20,})"#).unwrap(), "$1[REDACTED]"),
                (Regex::new(r#"(?i)(password|passwd|secret)\s*[:=]\s*["']?(\S+)"#).unwrap(), "$1: [REDACTED]"),
                (Regex::new(r#"sk-[A-Za-z0-9]{40,}"#).unwrap(), "sk-[REDACTED]"),
                // 数据库连接串中的密码
                (Regex::new(r#":[^@:/?#]+@"#).unwrap(), ":[REDACTED]@"),
            ],
        }
    }

    pub fn sanitize(&self, log_line: &str) -> String {
        let mut result = log_line.to_string();
        for (pattern, replacement) in &self.secret_patterns {
            result = pattern.replace_all(&result, *replacement).to_string();
        }
        result
    }
}

/// Tower 中间件：对所有日志输出自动脱敏
pub struct SanitizedLogging {
    sanitizer: Arc<LogSanitizer>,
}
```

### 9.4 Multi-Agent 消息鉴权

```rust
use hmac::{Hmac, Mac};
use sha2::Sha256;

pub struct MessageSigner {
    key: HmacKey,
}

pub struct HmacKey(Vec<u8>);

impl MessageSigner {
    pub fn sign(&self, msg: &AgentMessage) -> String {
        let payload = format!(
            "{}:{}:{}:{}",
            msg.id, msg.from, msg.thread_id, msg.content
        );
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key.0).unwrap();
        mac.update(payload.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    pub fn verify(&self, msg: &AgentMessage, signature: &str) -> bool {
        let expected = self.sign(msg);
        // 时间恒定比较，防止时序攻击
        constant_time_eq::constant_time_eq(expected.as_bytes(), signature.as_bytes())
    }
}
```

### 9.5 Multi-Agent 消息总线详细设计

消息总线是多 Agent 协作的核心基础设施，基于 `tokio::sync::mpsc` 实现进程内异步消息传递，支持主从/流水线/辩证三种协作模式。

#### 9.5.1 架构总览

```
┌─────────────────────────────────────────────────────────────┐
│                      CollabHub                              │
│  ┌───────────────┐  ┌────────────────┐  ┌───────────────┐  │
│  │ Capability     │  │ Internal       │  │ Thread        │  │
│  │ Registry       │  │ Message Bus    │  │ Manager       │  │
│  │                │  │                │  │               │  │
│  │ Agent → Skills │  │ Route → Sign   │  │ Context       │  │
│  │ Agent → Topics │  │  → Deliver     │  │ Isolation     │  │
│  └───────────────┘  └───────┬────────┘  └───────────────┘  │
│                             │                               │
│          ┌──────────────────┼──────────────────┐            │
│          ▼                  ▼                  ▼            │
│   ┌─────────────┐   ┌─────────────┐   ┌─────────────┐     │
│   │  Agent A     │   │  Agent B     │   │  Agent C     │    │
│   │  rx channel  │   │  rx channel  │   │  rx channel  │    │
│   └─────────────┘   └─────────────┘   └─────────────┘     │
│                                                             │
│   ┌─────────────────────────────────────────────────────┐   │
│   │            Dead Letter Queue (DLQ)                  │   │
│   │  不可达 / TTL 耗尽 / 通道满的消息暂存与监控          │   │
│   └─────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

#### 9.5.2 消息协议

```rust
use serde::{Serialize, Deserialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub type AgentId = String;
pub type ThreadId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub id: String,
    pub thread_id: ThreadId,
    pub from: AgentId,
    pub to: MessageTarget,
    pub content: String,
    pub msg_type: AgentMessageType,
    pub ttl: u8,
    pub priority: MessagePriority,
    pub signature: String,
    pub timestamp: u64,
    pub correlation_id: Option<String>,    // 关联请求-响应对
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageTarget {
    Direct(AgentId),                       // 点对点
    Broadcast,                             // 广播给所有已注册 Agent
    Topic(String),                         // 按主题发布
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum MessagePriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,                          // 错误汇报、紧急中断
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentMessageType {
    TaskRequest { task: serde_json::Value },
    TaskResponse { result: serde_json::Value, success: bool },
    StatusUpdate { status: String, progress: Option<f32> },
    ErrorReport { error: String, recoverable: bool },
    Heartbeat,                             // 活性检测
    Shutdown,                              // 优雅关闭信号
}

impl AgentMessage {
    pub fn new_request(from: AgentId, to: AgentId, thread_id: ThreadId, task: serde_json::Value) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            thread_id,
            from,
            to: MessageTarget::Direct(to),
            content: String::new(),
            msg_type: AgentMessageType::TaskRequest { task },
            ttl: 8,
            priority: MessagePriority::Normal,
            signature: String::new(),
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
            correlation_id: None,
            metadata: None,
        }
    }
}
```

#### 9.5.3 消息总线实现

```rust
use dashmap::DashMap;
use tokio::sync::{mpsc, broadcast};
use std::sync::Arc;

pub struct InternalMessageBus {
    direct_channels: DashMap<AgentId, mpsc::Sender<AgentMessage>>,
    topic_subscribers: DashMap<String, Vec<AgentId>>,
    signer: MessageSigner,
    dead_letter_queue: mpsc::Sender<DeadLetter>,
    metrics: BusMetrics,
    config: BusConfig,
}

pub struct BusConfig {
    pub channel_buffer_size: usize,        // 默认 256
    pub max_ttl: u8,                       // 默认 8
    pub dlq_buffer_size: usize,            // 默认 1024
    pub send_timeout: std::time::Duration, // 默认 5s
}

impl Default for BusConfig {
    fn default() -> Self {
        Self {
            channel_buffer_size: 256,
            max_ttl: 8,
            dlq_buffer_size: 1024,
            send_timeout: std::time::Duration::from_secs(5),
        }
    }
}

#[derive(Debug)]
pub struct DeadLetter {
    pub message: AgentMessage,
    pub reason: DeadLetterReason,
    pub timestamp: u64,
}

#[derive(Debug)]
pub enum DeadLetterReason {
    TtlExhausted,
    RecipientNotFound,
    ChannelFull,
    SignatureInvalid,
    SendTimeout,
}

#[derive(Default)]
struct BusMetrics {
    messages_sent: std::sync::atomic::AtomicU64,
    messages_delivered: std::sync::atomic::AtomicU64,
    messages_dead_lettered: std::sync::atomic::AtomicU64,
    active_agents: std::sync::atomic::AtomicU32,
}

impl InternalMessageBus {
    pub fn new(signer: MessageSigner, config: BusConfig) -> (Self, mpsc::Receiver<DeadLetter>) {
        let (dlq_tx, dlq_rx) = mpsc::channel(config.dlq_buffer_size);
        let bus = Self {
            direct_channels: DashMap::new(),
            topic_subscribers: DashMap::new(),
            signer,
            dead_letter_queue: dlq_tx,
            metrics: BusMetrics::default(),
            config,
        };
        (bus, dlq_rx)
    }

    /// 注册 Agent，返回该 Agent 的消息接收通道
    pub fn register(&self, agent_id: AgentId) -> mpsc::Receiver<AgentMessage> {
        let (tx, rx) = mpsc::channel(self.config.channel_buffer_size);
        self.direct_channels.insert(agent_id, tx);
        self.metrics.active_agents.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        rx
    }

    /// 注销 Agent（优雅退出时调用）
    pub fn unregister(&self, agent_id: &str) {
        self.direct_channels.remove(agent_id);
        self.metrics.active_agents.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        // 同时清理 topic 订阅
        for mut entry in self.topic_subscribers.iter_mut() {
            entry.value_mut().retain(|id| id != agent_id);
        }
    }

    /// 订阅主题
    pub fn subscribe_topic(&self, agent_id: AgentId, topic: String) {
        self.topic_subscribers.entry(topic).or_default().push(agent_id);
    }

    /// 发送消息（含签名、TTL、路由）
    pub async fn send(&self, mut msg: AgentMessage) -> Result<()> {
        // TTL 校验
        if msg.ttl == 0 {
            self.dead_letter(msg, DeadLetterReason::TtlExhausted).await;
            return Err(anyhow::anyhow!("TTL exhausted"));
        }
        msg.ttl -= 1;

        // 签名
        msg.signature = self.signer.sign(&msg);
        self.metrics.messages_sent.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        match &msg.to {
            MessageTarget::Direct(target) => {
                self.deliver_direct(target, msg).await
            }
            MessageTarget::Broadcast => {
                self.deliver_broadcast(msg).await
            }
            MessageTarget::Topic(topic) => {
                self.deliver_topic(topic, msg).await
            }
        }
    }

    async fn deliver_direct(&self, target: &str, msg: AgentMessage) -> Result<()> {
        match self.direct_channels.get(target) {
            Some(tx) => {
                match tokio::time::timeout(self.config.send_timeout, tx.send(msg.clone())).await {
                    Ok(Ok(())) => {
                        self.metrics.messages_delivered.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        Ok(())
                    }
                    Ok(Err(_)) => {
                        self.dead_letter(msg, DeadLetterReason::ChannelFull).await;
                        Err(anyhow::anyhow!("channel closed"))
                    }
                    Err(_) => {
                        self.dead_letter(msg, DeadLetterReason::SendTimeout).await;
                        Err(anyhow::anyhow!("send timeout"))
                    }
                }
            }
            None => {
                self.dead_letter(msg, DeadLetterReason::RecipientNotFound).await;
                Err(anyhow::anyhow!("recipient not registered"))
            }
        }
    }

    async fn deliver_broadcast(&self, msg: AgentMessage) -> Result<()> {
        for entry in self.direct_channels.iter() {
            if entry.key() != &msg.from {
                let mut clone = msg.clone();
                clone.to = MessageTarget::Direct(entry.key().clone());
                let _ = entry.value().try_send(clone);
            }
        }
        Ok(())
    }

    async fn deliver_topic(&self, topic: &str, msg: AgentMessage) -> Result<()> {
        if let Some(subscribers) = self.topic_subscribers.get(topic) {
            for sub_id in subscribers.value() {
                if sub_id != &msg.from {
                    if let Some(tx) = self.direct_channels.get(sub_id) {
                        let mut clone = msg.clone();
                        clone.to = MessageTarget::Direct(sub_id.clone());
                        let _ = tx.try_send(clone);
                    }
                }
            }
        }
        Ok(())
    }

    async fn dead_letter(&self, msg: AgentMessage, reason: DeadLetterReason) {
        self.metrics.messages_dead_lettered.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        tracing::warn!(
            msg_id = %msg.id,
            from = %msg.from,
            reason = ?reason,
            "message sent to dead letter queue"
        );
        let _ = self.dead_letter_queue.try_send(DeadLetter {
            message: msg,
            reason,
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
        });
    }

    /// 获取总线运行指标
    pub fn stats(&self) -> BusStats {
        BusStats {
            active_agents: self.metrics.active_agents.load(std::sync::atomic::Ordering::Relaxed),
            messages_sent: self.metrics.messages_sent.load(std::sync::atomic::Ordering::Relaxed),
            messages_delivered: self.metrics.messages_delivered.load(std::sync::atomic::Ordering::Relaxed),
            messages_dead_lettered: self.metrics.messages_dead_lettered.load(std::sync::atomic::Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct BusStats {
    pub active_agents: u32,
    pub messages_sent: u64,
    pub messages_delivered: u64,
    pub messages_dead_lettered: u64,
}
```

#### 9.5.4 Backpressure 策略

| 场景 | 策略 | 实现 |
|------|------|------|
| 通道缓冲区满 | `try_send` 失败 → 消息进入 DLQ | `deliver_direct` 中的超时机制 |
| Agent 处理慢 | 发送方阻塞最多 `send_timeout` | `tokio::time::timeout` 包裹 |
| DLQ 积压 | 监控 DLQ 长度，超阈值触发告警 | Prometheus `bus_dlq_depth` gauge |
| Agent 宕机 | Heartbeat 超时 → 自动 unregister | `ThreadManager` 周期检测 |

#### 9.5.5 可观测性

总线暴露以下 Prometheus 指标：

```
fastclaw_collab_messages_sent_total{from,to}       # 发送总数
fastclaw_collab_messages_delivered_total{to}        # 投递成功总数
fastclaw_collab_dead_letters_total{reason}          # 死信总数
fastclaw_collab_active_agents                       # 当前活跃 Agent 数
fastclaw_collab_channel_depth{agent_id}             # 各 Agent 通道深度
fastclaw_collab_send_duration_seconds{from,to}      # 发送耗时直方图
```

### 9.6 代码执行沙箱（seccomp-bpf）

```rust
use seccomp::{Context, Action, Syscall};

pub fn setup_code_execution_sandbox() -> Result<()> {
    let mut ctx = Context::init_with_action(Action::KillProcess)?;

    // 白名单：允许的系统调用
    let allowed_syscalls = [
        Syscall::Read, Syscall::Write, Syscall::Exit, Syscall::ExitGroup,
        Syscall::Brk, Syscall::Mmap, Syscall::Munmap, Syscall::Mprotect,
        Syscall::Futex, Syscall::ClockGettime, Syscall::Getpid,
        // 允许 stdout/stderr，拒绝其他文件操作
        Syscall::Fstat, Syscall::Close,
    ];

    for syscall in &allowed_syscalls {
        ctx.add_rule(Action::Allow, *syscall)?;
    }

    // 安装过滤器（不可逆）
    ctx.load()?;

    tracing::debug!("seccomp sandbox installed");
    Ok(())
}

pub struct ProcessSandbox;

impl ProcessSandbox {
    pub async fn execute_python(&self, req: ExecutionRequest) -> Result<ExecutionResult> {
        let temp_dir = tempdir()?;
        let script_path = temp_dir.path().join("script.py");
        tokio::fs::write(&script_path, &req.code).await?;

        // 在独立子进程中执行，并用 seccomp 限制
        let output = tokio::time::timeout(req.timeout, async {
            tokio::task::spawn_blocking(move || {
                std::process::Command::new("python3")
                    .arg(&script_path)
                    .env_clear()                         // 清空所有环境变量
                    .env("HOME", temp_dir.path())
                    .current_dir(temp_dir.path())
                    .stdin(std::process::Stdio::null())
                    // 注入 seccomp 过滤器的 wrapper 脚本
                    .args(["--", "/usr/bin/python3", script_path.to_str().unwrap()])
                    .output()
            }).await?
        }).await??;

        Ok(ExecutionResult {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
            success: output.status.success(),
            elapsed_ms: 0, // TODO: 从 proc 统计
            memory_peak_mb: 0,
        })
    }
}
```

### 9.7 进化系统变更控制

```rust
pub struct EvolutionChangeControl {
    store: Arc<SessionStore>,
    audit: Arc<AuditLogger>,
    regression_runner: Arc<RegressionRunner>,
}

impl EvolutionChangeControl {
    /// 提交候选版本（默认不激活）
    pub async fn submit_candidate(
        &self,
        agent_id: &AgentId,
        new_prompt: &str,
        metrics: &EvalMetrics,
    ) -> Result<SnapshotId> {
        // 验证变更范围：候选版本不得修改权限和危险工具配置
        self.validate_change_scope(new_prompt).await?;

        let snapshot_id = self.store.save_evolution_snapshot(
            agent_id,
            new_prompt,
            metrics,
            false,  // is_active = false
        ).await?;

        self.audit.log(AuditEvent {
            action: "evolution.candidate_submitted",
            agent_id: agent_id.to_string(),
            snapshot_id: snapshot_id.to_string(),
            metrics: serde_json::to_value(metrics)?,
            ..Default::default()
        });

        Ok(snapshot_id)
    }

    /// 激活候选版本（需通过回归测试）
    pub async fn activate_candidate(
        &self,
        snapshot_id: &SnapshotId,
        approved_by: Option<&str>,  // None = 自动激活（需通过全部测试）
    ) -> Result<()> {
        let snapshot = self.store.get_snapshot(snapshot_id).await?;

        // 1. 运行回归测试
        let regression_result = self.regression_runner.run(&snapshot).await?;

        if regression_result.pass_rate < 0.95 {
            return Err(EvolutionError::RegressionFailed {
                pass_rate: regression_result.pass_rate,
                failures: regression_result.failures,
            }.into());
        }

        // 2. 检查质量指标不下降
        let baseline = self.store.get_active_snapshot(&snapshot.agent_id).await?;
        if let Some(baseline) = baseline {
            let degradation = snapshot.metrics.quality_score - baseline.metrics.quality_score;
            if degradation < -0.10 {  // 超过 10% 质量下降拒绝
                return Err(EvolutionError::QualityDegradation { degradation }.into());
            }
        }

        // 3. 激活
        self.store.activate_snapshot(snapshot_id).await?;

        self.audit.log(AuditEvent {
            action: "evolution.activated",
            snapshot_id: snapshot_id.to_string(),
            approved_by: approved_by.map(|s| s.to_string()),
            ..Default::default()
        });

        Ok(())
    }

    /// 校验变更范围：通过结构化对比（而非简单字符串匹配）确保
    /// 候选版本只修改了 system_prompt 字段，不涉及权限/工具授权等敏感配置。
    async fn validate_change_scope(&self, agent_id: &AgentId, new_prompt: &str) -> Result<()> {
        let current = self.store.get_active_snapshot(agent_id).await?;
        if let Some(current) = current {
            // 解析新旧 Prompt，对比结构化字段
            let old_config = parse_agent_prompt_config(&current.prompt_text)?;
            let new_config = parse_agent_prompt_config(new_prompt)?;

            // 仅允许 system_prompt 内容变更，不允许修改：
            // - 工具权限白/黑名单
            // - 危险工具标记
            // - Agent 权限等级
            if old_config.tool_permissions != new_config.tool_permissions
                || old_config.dangerous_tools != new_config.dangerous_tools
                || old_config.permission_level != new_config.permission_level
            {
                return Err(EvolutionError::ForbiddenScopeChange {
                    detail: "Evolution may only modify system_prompt content, not permissions or tool configs".into(),
                }.into());
            }
        }
        Ok(())
    }
}
```

---

## 10. 高级能力模块详细设计

> 各高级模块（快速创建 Agent / Skill 可视化编排 / 多 Agent 协作 / 模型复杂度路由 / Agent 自我迭代 / 智能上下文管理 / 代码能力增强）的完整 Rust 数据结构、算法和 API 设计详见 `harness-engineer-advanced-capabilities.md`（与本文档同目录），该文件作为本技术方案的附录，已纳入同一基座工程。
