<p align="center">
  <h1 align="center">FastClaw</h1>
  <p align="center">
    高性能 AI Agent 编排引擎<br/>
    <em>Multi-agent orchestration · Tool calling · Memory · WebSocket streaming · MCP · WASM plugins</em>
  </p>
</p>

<p align="center">
  <a href="#快速开始">快速开始</a> ·
  <a href="#功能特性">功能特性</a> ·
  <a href="#架构概览">架构概览</a> ·
  <a href="#模块技术详解">模块详解</a> ·
  <a href="docs/MANUAL.md">使用手册</a> ·
  <a href="#许可证">许可证</a>
</p>

---

## 什么是 FastClaw

FastClaw 是一个用 **Rust** 构建的 AI Agent 编排引擎，专为构建、运行和管理多 Agent 系统而设计。它提供统一的 HTTP/WebSocket 网关、内置 30+ 工具、多模型路由、会话持久化、语义记忆和 WASM 插件系统，让你可以快速搭建从命令行到飞书机器人的完整 AI 应用。

### 核心亮点

- **极致性能** — 纯 Rust 实现，异步 I/O，单二进制文件部署，内存占用 < 50MB
- **多 Agent 编排** — Agent 间消息总线、Sub-agent 任务分发
- **丰富的工具生态** — 文件系统、Shell、PTY 终端、代码智能（LSP）、Web 搜索、记忆检索等 35+ 内置工具
- **多模型路由** — 支持 OpenAI / Anthropic / DashScope / DeepSeek / Ollama 等，按复杂度自动路由
- **语义记忆** — 向量检索 + 知识图谱双引擎，支持本地嵌入（无需外部 API）
- **MCP 协议** — 同时支持 MCP Server 和 Client，与外部 Agent 生态互通
- **WASM 插件** — 通过 Wasmtime 运行沙箱化插件，热加载无需重启
- **自进化** — 自动从对话轨迹中提取可复用技能，持续优化 Agent 能力
- **多渠道接入** — 飞书机器人（WebSocket/Webhook）、HTTP API、WebSocket、TUI 终端

---

## 功能特性

| 模块 | 说明 |
|------|------|
| **Gateway** | Axum 构建的 HTTP/WebSocket 网关，支持 CORS、速率限制、API Key 认证、gzip 压缩 |
| **Agent Runtime** | 多轮对话执行引擎，流式输出、工具调用、自动重试、成本追踪 |
| **Model Router** | 按任务复杂度（tiny → frontier）自动选择最优模型，支持 fallback 链 |
| **Session** | SQLite 持久化会话，自动压缩、TTL 过期清理 |
| **Memory** | Episodic（情景记忆）+ Semantic（语义事实），向量检索（usearch）+ 知识图谱 |
| **Tool System** | 35+ 内置工具，OpenAI 兼容 tool calling 协议，per-tool 并行调度（RwLock gate），Pre/Post Hook 管线，ToolContributor 扩展插件 |
| **MCP** | stdio 传输的 MCP Server + 连接外部 MCP Server 的 Client |
| **Plugins (WASM)** | Wasmtime Component Model，沙箱化执行，热加载 |
| **Channels** | 飞书机器人扩展，支持 WebSocket 长连接和 Webhook 模式 |
| **Evolution** | Agent 自进化：轨迹记录 → 技能提取 → 自动激活，默认启用 |
| **Context Engine** | 智能上下文管理：分层压缩、Token 预算追踪、自动摘要 |
| **Self-Iter** | 自迭代优化：错误诊断 → 沙箱验证 → 自动修复 |
| **Cron** | 定时任务调度，支持 Agent 聊天触发和 Webhook 触发 |
| **Observability** | Prometheus 指标导出、结构化日志（JSON/pretty） |
| **Security** | API Key 认证、速率限制、Prompt 注入检测、Shell 命令沙箱、SSRF 防护 |
| **TUI** | 基于 Ratatui 的终端交互界面，连接 Gateway WebSocket |
| **Desktop App** | Tauri v2 桌面应用（可选） |

---

## 快速开始

### 系统要求

- Rust 1.82+（MSRV）
- SQLite 3
- （可选）GCC 12+ / C++17 编译器（usearch 向量检索后端）

### 从源码安装

```bash
git clone https://github.com/example/fastclaw.git
cd fastclaw
cargo build --release
```

编译产物：`target/release/fastclaw`

### 首次配置

```bash
# 交互式引导
fastclaw setup

# 或手动配置
mkdir -p ~/.fastclaw/config
cp config/default.json ~/.fastclaw/config/default.json
# 编辑 ~/.fastclaw/config/default.json 填写 LLM API Key
```

### 启动网关

```bash
# 前台运行
fastclaw serve

# 后台守护进程
fastclaw gateway start

# 检查状态
fastclaw health
fastclaw doctor
```

### 开始对话

```bash
# 终端 TUI
fastclaw tui

# HTTP API
curl -X POST http://localhost:18789/api/v1/chat \
  -H "Content-Type: application/json" \
  -d '{"messages": [{"role": "user", "content": "你好"}]}'

# WebSocket
wscat -c ws://localhost:18789/ws
```

### Docker 部署

```bash
# 构建镜像
docker build -t fastclaw .

# Docker Compose
docker compose up -d
```

---

## 架构概览

```
┌─────────────────────────────────────────────────────────────────┐
│                         FastClaw Gateway                        │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌───────────────┐  │
│  │ HTTP API │  │WebSocket │  │ Webhook  │  │  TUI Client   │  │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘  └───────┬───────┘  │
│       └──────────────┼────────────┼─────────────────┘          │
│                      ▼                                          │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │                   Chat Pipeline                          │  │
│  │  Session → Agent Router → Model Router → LLM Provider    │  │
│  │     ↕           ↕             ↕                          │  │
│  │  Memory    Tool Executor   Cost Tracker                  │  │
│  └──────────────────────────────────────────────────────────┘  │
│                                                                 │
│  ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐ ┌────────────┐  │
│  │Session │ │Memory  │ │  MCP   │ │ WASM   │ │ Evolution  │  │
│  │ Store  │ │ Engine │ │Server/ │ │Plugins │ │  Pipeline  │  │
│  │(SQLite)│ │(Vector │ │Client  │ │        │ │            │  │
│  │        │ │+Graph) │ │        │ │        │ │            │  │
│  └────────┘ └────────┘ └────────┘ └────────┘ └────────────┘  │
│                                                                 │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │ Extensions: Feishu Bot · Cron Scheduler · Agent Bus      │  │
│  └──────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

---

## 模块技术详解

### fastclaw-core

核心基础库，定义所有 crate 共享的类型和 trait。

| 子模块 | 职责 |
|--------|------|
| `config` | 配置数据模型（`FastClawConfig`），支持 JSON5、`$include` 引用、多 profile |
| `agent_config` | Agent 定义（模型、System Prompt、工具权限、MCP 绑定） |
| `tool` | 工具 trait（`Tool`）、注册表（`ToolRegistry`）、ToolContributor 扩展插件、DynamicTool 动态工具 |
| `skill` | 技能 trait 和注册表，支持 SKILL.md 文件加载 |
| `bus` | Agent 消息总线：跨 Agent 异步消息传递和请求-响应模式 |
| `channel` | 渠道抽象（`ChannelPlugin` trait），统一飞书/Webhook 等入站消息 |
| `routing` | Agent 路由器：按渠道、关键词、默认规则将消息分发到正确的 Agent |
| `complexity` | 任务复杂度评估（`ComplexityTier`: tiny/small/medium/large/frontier） |
| `workspace` | Agent 工作目录管理，沙箱路径隔离 |

### fastclaw-agent

Agent 运行时引擎，负责完整的对话循环。

| 子模块 | 职责 |
|--------|------|
| `runtime` | 核心执行循环：消息构建 → LLM 调用 → 流式输出 → 工具调用 → 递归 |
| `llm` | LLM Provider 抽象层：支持 OpenAI / Anthropic / DashScope 等多种协议 |
| `builtin_tools` | 35+ 内置工具实现（文件系统、Shell、PTY、LSP、搜索、目标管理等） |
| `prompt_engine` | Prompt 组装引擎：静态段 + 动态段，按 token 预算裁剪 |
| `stream_engine` | SSE 流式输出引擎：增量 token、工具调用、状态事件 |
| `tool_executor` | 工具并行调度器：RwLock gate（per-tool 并行声明）、Pre/Post Hook、批量执行、结果截断 |
| `trajectory` | 轨迹记录：记录每轮 LLM 调用和工具执行的步骤 |
| `file_state_cache` | 文件状态缓存：跟踪 Agent 读写过的文件，优化重复访问 |

**关键能力**:
- 流式输出：通过 `mpsc` channel 逐 token 推送到 Gateway
- 工具调用：per-tool 并行声明 + RwLock gate 并发控制，Pre/Post Hook 管线
- 流式工具执行：LLM 输出期间即刻启动 unguarded 工具（StreamingToolExecutor）
- 子任务分发：`task_create` 工具创建独立子 Agent 执行复杂子任务
- Token 预算管理：自动计算并遵守模型 context window 限制
- 目标管理：`get_goal` / `create_goal` / `update_goal` 追踪 Agent 目标与 token 预算
- 自迭代：检测工具执行失败后自动诊断并重试（需 `self-iter` feature）

### fastclaw-gateway

HTTP/WebSocket 网关层，是所有外部请求的入口。

| 子模块 | 职责 |
|--------|------|
| `routes` | Axum 路由定义：REST API + WebSocket + Webhook |
| `state` | 应用状态管理：五阶段初始化，热重载 Agent 配置 |
| `ws` | WebSocket 实时聊天：双向流式通信，多会话复用 |
| `chat_pipeline` | 完整聊天流水线：认证 → 路由 → 模型选择 → Agent 执行 → 流式响应 |

**技术特性**:
- 五阶段启动（config → session → tools → channels → cron），确保依赖顺序
- 热重载：通过 `notify` 文件监听自动重新加载 Agent 配置
- CORS + gzip + 速率限制（tower-http 中间件栈）
- SSE 和 WebSocket 双模式流式输出

### fastclaw-session

SQLite 持久化会话管理。

| 能力 | 说明 |
|------|------|
| 会话 CRUD | 创建/查询/删除会话及消息 |
| 自动压缩 | 消息数超过阈值时自动触发上下文压缩 |
| TTL 清理 | 定时清理过期会话（默认 7 天） |
| DM 作用域 | 支持 per-channel-peer / per-user / global 三种会话隔离模式 |
| 对话 Trace | 记录每轮对话的完整执行轨迹（工具调用、token 用量、耗时） |

### fastclaw-memory

双引擎记忆系统：向量检索 + 知识图谱。

| 子模块 | 职责 |
|--------|------|
| `episodic` | 情景记忆：存储对话中的关键事件，支持时间衰减和遗忘策略 |
| `semantic` | 语义记忆：事实（Fact）+ 关系（Relationship）图谱，支持实体链接 |
| `embedding` | 嵌入层：本地（hypembed 纯 Rust）或远程（OpenAI API）向量化 |
| `importance` | 重要性评分：基于 LLM 或规则的记忆重要性自动打分 |
| `dreaming` | 梦境周期：后台定期回顾记忆，提取事实、补充嵌入、遗忘低价值条目 |
| `working` | 工作记忆：当前会话的短期记忆缓冲 |
| `manager` | 统一检索入口：融合 episodic + semantic + working 的召回结果 |

**技术特性**:
- 向量索引使用 usearch（C++ 后端，高性能 HNSW）
- 支持 384 维（MiniLM）/ 1536 维（OpenAI）嵌入
- 遗忘策略：半衰期指数衰减 + 最大条目数限制
- 梦境周期：后台异步运行，从高重要性 episode 中自动提取技能候选

### fastclaw-model-router

多模型智能路由与预算追踪。

| 子模块 | 职责 |
|--------|------|
| `router` | 路由决策：根据策略（Fixed / Fallback / CostOptimized）选择模型 |
| `budget` | 预算追踪：按日/周/月统计 token 用量和成本，超限自动降级 |
| `estimator` | 成本估算：根据模型定价预估请求费用 |
| `tier` | 复杂度分级：分析任务特征（tool 数量、历史长度）推断最适合的模型层级 |

**路由策略**:
- `Fixed`: 固定使用配置的默认模型
- `Fallback`: 按 fallback 链依次尝试，主模型超时/限流时降级
- `CostOptimized`: 在满足质量约束的前提下选择最低成本模型

### fastclaw-context

智能上下文管理引擎，确保 LLM 输入始终在 token 预算内。

| 子模块 | 职责 |
|--------|------|
| `engine` | 上下文装配引擎：分层组装（system + memory + history + tools） |
| `compressor` | 压缩策略：基于重要性评分的消息裁剪 |
| `collapse` | 折叠引擎：将多轮对话压缩为摘要，保留关键信息 |
| `budget` | Token 预算追踪：实时监控并决定何时触发压缩 |
| `pipeline` | 自动压缩管线：熔断器 + 渐进式压缩 + 元数据记录 |
| `reactive` | 响应式压缩：当 LLM 返回 context_length_exceeded 时自动裁剪重试 |
| `snip` | Snip 压缩器：按 API 轮次分组，优先保留最近和最重要的轮次 |

### fastclaw-evolution

Agent 自进化系统，从对话轨迹中自动学习和优化（默认启用）。

| 子模块 | 职责 |
|--------|------|
| `feedback` | 用户反馈存储：thumbs up/down、评分、纠正文本 |
| `trajectory` | 轨迹记录：完整的 Agent 执行路径（工具调用序列、成功/失败） |
| `skill_extractor` | 技能提取：通过 LLM 分析高价值轨迹，抽象为可复用模式 |
| `skill_store` | 技能生命周期：Candidate → Active → Retired，基于使用统计自动晋升/退役 |
| `distiller` | Prompt 蒸馏：从高评分对话中优化 System Prompt |

**工作流程**:
1. Agent 每轮对话自动记录轨迹步骤
2. 后台定时（默认 600s）扫描高价值轨迹
3. LLM 提取抽象模式，生成候选技能（Candidate）
4. 技能按使用频率和成功率自动晋升为 Active
5. 下次相似任务时，Active 技能自动注入到 System Prompt

### fastclaw-self-iter

自迭代优化引擎：当工具执行失败时自动诊断和修复。

| 子模块 | 职责 |
|--------|------|
| `engine` | 迭代控制器：最多 N 次重试，每次基于错误分析生成修复策略 |
| `diagnosis` | 错误诊断：分析错误消息，推断根因（语法错误、路径不存在、权限等） |
| `sandbox_runner` | 沙箱执行器：在隔离环境中验证修复是否有效 |

### fastclaw-mcp

Model Context Protocol 双向实现。

| 能力 | 说明 |
|------|------|
| MCP Server | 通过 stdio 暴露 FastClaw 工具给外部 Agent（Claude Desktop、Cursor 等） |
| MCP Client | 连接外部 MCP Server，将其工具纳入 Agent 可用工具集 |
| 协议支持 | JSON-RPC 2.0 over stdio，支持 tools/list、tools/call、resources 等方法 |
| 动态发现 | 启动时自动发现并注册外部 MCP Server 的工具 |

### fastclaw-security

多层安全防护。

| 子模块 | 职责 |
|--------|------|
| `auth` | API Key 认证：请求头 / Bearer token 验证 |
| `rate_limit` | 速率限制：令牌桶算法，支持 per-IP 和全局限流 |
| `prompt_guard` | Prompt 注入检测：多层规则 + 启发式分析，标注风险等级 |
| `dangerous_ops` | 危险操作管控：Shell 命令白/黑名单，敏感路径防护 |
| `ssrf` | SSRF 防护：阻止对私有 IP/内网地址的请求，支持例外白名单 |

### fastclaw-cron

定时任务调度器。

| 能力 | 说明 |
|------|------|
| Cron 表达式 | 标准 cron 语法（秒级精度） |
| 动作类型 | `agent_chat`（触发 Agent 对话）、`webhook`（HTTP 回调） |
| 通知 | 任务完成/失败时通过飞书等渠道推送通知 |
| 持久化 | SQLite 存储任务定义和执行记录 |

### fastclaw-observe

可观测性基础设施。

| 能力 | 说明 |
|------|------|
| Prometheus | 自动注册核心指标：请求延迟、token 用量、工具调用计数 |
| 结构化日志 | 基于 tracing：支持 JSON / pretty 格式，可配级别过滤 |
| Metrics 端点 | `/metrics` 暴露 Prometheus 格式指标，可接入 Grafana |

### fastclaw-treesitter

基于 Tree-sitter 的代码分析。

| 能力 | 说明 |
|------|------|
| Shell AST | 分析 Shell 命令结构，用于安全策略中的命令白名单匹配 |
| 代码大纲 | 提取文件函数/类/结构体大纲，供 `file_outline` 工具使用 |

---

## 项目结构

```
fastclaw/
├── crates/
│   ├── fastclaw-cli/          # CLI 入口（fastclaw 二进制）
│   ├── fastclaw-core/         # 核心类型、配置、路由、工具 trait
│   ├── fastclaw-gateway/      # HTTP/WebSocket 网关
│   ├── fastclaw-agent/        # Agent 运行时、30+ 内置工具
│   ├── fastclaw-session/      # 会话持久化（SQLite）
│   ├── fastclaw-memory/       # 向量记忆 + 知识图谱
│   ├── fastclaw-model-router/ # 多模型路由与预算追踪
│   ├── fastclaw-mcp/          # MCP Server/Client
│   ├── fastclaw-security/     # 认证、速率限制、Prompt 注入检测
│   ├── fastclaw-observe/      # 可观测性（Prometheus 指标、日志）
│   ├── fastclaw-evolution/    # Agent 自进化（反馈→蒸馏→激活）
│   ├── fastclaw-context/      # 上下文引擎（压缩、预算、折叠）
│   ├── fastclaw-self-iter/    # 自迭代优化（诊断→沙箱→修复）
│   ├── fastclaw-cron/         # 定时任务调度器
│   ├── fastclaw-treesitter/   # Shell AST 分析
│   └── fastclaw-app/          # Tauri v2 桌面应用
├── extensions/
│   └── feishu/                # 飞书机器人扩展
├── config/                    # 默认配置和 Agent 定义
│   ├── default.json
│   └── agents/main.json
├── prompts/                   # System prompt 模板库
├── deploy/kubernetes/         # Kubernetes 部署清单
├── scripts/                   # 构建和发布脚本
├── Dockerfile                 # 多阶段 Docker 构建
├── docker-compose.yml
└── Cargo.toml                 # Workspace 根 manifest
```

---

## CLI 命令参考

```
fastclaw <COMMAND>

Commands:
  setup        交互式首次配置
  onboard      新手引导
  serve        启动网关（前台）
  health       健康检查
  doctor       环境诊断
  tui          终端交互界面
  config       配置管理（get/set/check/file/path/fix）
  gateway      网关管理（run/start/stop/restart/status/health）
  sessions     会话管理（list/get/delete/cleanup）
  agents       Agent 管理（list/get）
  tools        工具管理（list）
  trace        对话 Trace 管理（list/show/export）
  backup       备份与恢复（create/restore）
  mcp-server   启动 MCP Server（stdio）
  completions  生成 Shell 补全脚本

Global Flags:
  --dev        使用开发环境目录 (~/.fastclaw-dev/)
  --profile    使用命名配置文件 (~/.fastclaw-<name>/)
  --no-color   禁用彩色输出
  --json       JSON 格式输出
```

---

## 支持的 LLM 提供商

| 提供商 | 协议 | 默认模型 | 备注 |
|--------|------|----------|------|
| OpenAI | OpenAI API | gpt-4o | 支持 Vision / Tool Calling |
| Anthropic | Anthropic API | claude-sonnet-4-20250514 | 支持 Reasoning / Vision |
| DashScope (Qwen) | OpenAI 兼容 | qwen3.5-plus | 阿里云通义千问 |
| DeepSeek | OpenAI 兼容 | deepseek-chat | |
| Google Gemini | OpenAI 兼容 | gemini-2.5-flash | |
| Ollama | OpenAI 兼容 | llama3.1:8b | 本地推理，无需 API Key |
| Custom | OpenAI 兼容 | 自定义 | 任意 OpenAI 兼容端点 |

---

## 内置工具一览

<details>
<summary>展开查看 35+ 内置工具</summary>

**文件系统**
- `read_file` — 读取文件内容（支持并行）
- `write_file` — 写入文件
- `edit_file` — 字符串替换编辑
- `multi_edit` — 批量多文件编辑（deferred）
- `apply_patch` — 应用 unified diff 补丁（deferred）
- `glob` — 文件名模式搜索（支持并行）
- `search_in_files` — 正则表达式搜索（支持并行）
- `list_directory` — 列出目录内容（支持并行）

**Shell 与终端**
- `shell_exec` — 沙箱化 Shell 命令执行（通过 Orchestrator 审批）
- `exec_command` — PTY 交互式终端会话（deferred）
- `write_stdin` — 向 PTY 会话发送输入（deferred）
- `terminal_capture` — 终端输出捕获（支持并行）

**代码智能**
- `lsp` — 统一 LSP 工具：Go to Definition / Find References / Workspace Symbols（支持并行）
- `file_outline` — 文件结构大纲（deferred）
- `code_sections` — 语义代码分段（deferred）

**网络**
- `web_search` — 网页搜索：Tavily / SearXNG / Google / Baidu / Bing 等（支持并行）
- `web_fetch` — 获取网页内容（支持并行）
- `http_fetch` — HTTP 请求（支持并行）

**记忆**
- `memory` — 统一记忆工具（搜索 + 存储语义记忆）

**目标管理**
- `get_goal` — 获取当前目标与 token 预算（deferred）
- `create_goal` — 创建目标与预算约束（deferred）
- `update_goal` — 标记目标完成/失败（deferred）

**任务管理**
- `todo_write` / `todo_read` — 待办事项管理
- `task_create` / `task_list` / `task_get` / `task_update` / `task_stop` — 子任务（Sub-agent）管理

**交互与权限**
- `ask_question` — 向用户提问
- `confirm` — 确认操作
- `send_user_message` — 发送中间消息
- `request_permissions` — 请求额外文件/网络权限（deferred）

**计划模式**
- `enter_plan_mode` / `exit_plan_mode` — Agent/Plan 执行模式切换（deferred）

**实用工具**
- `current_time` — 获取当前时间（deferred）
- `sleep` — 等待指定时间（deferred）
- `tool_search` — BM25 模糊搜索可用工具（支持并行）
- `screenshot` — 屏幕截图（支持并行）
- `git` — Git 版本控制操作（支持并行）
- `notebook_edit` — Jupyter Notebook 编辑（deferred）
- `skill` — 技能管理：列出/读取/写入
- `identity` — Agent 身份配置管理（SOUL.md / USER.md / AGENTS.md）

**媒体生成**
- `image_generate` — 图像生成（deferred，需 API 配置）
- `tts` — 文本转语音（deferred，需 API 配置）

</details>

---

## API 端点

完整 OpenAPI 规范可通过 `GET /api/v1/openapi.json` 获取。主要端点：

| 方法 | 路径 | 说明 |
|------|------|------|
| `POST` | `/api/v1/chat` | 聊天补全（支持流式） |
| `GET` | `/ws` | WebSocket 实时聊天 |
| `GET` | `/api/v1/agents` | 列出所有 Agent |
| `GET/POST` | `/api/v1/agents/:id` | Agent CRUD |
| `GET` | `/api/v1/sessions` | 列出会话 |
| `GET` | `/api/v1/sessions/:id/messages` | 获取会话消息 |
| `GET/POST` | `/api/v1/memory/facts` | 语义事实管理 |
| `GET` | `/api/v1/memory/episodes/search` | 情景记忆搜索 |
| `POST` | `/api/v1/evolution/feedback` | 提交 Agent 反馈 |
| `GET` | `/api/v1/evolution/candidates/:agent_id` | 列出候选技能 |
| `GET/POST` | `/api/v1/cron/jobs` | 定时任务管理 |
| `GET` | `/api/v1/traces` | 对话 Trace |
| `GET/POST` | `/api/v1/bus/send` | Agent 消息总线 |
| `POST` | `/webhook/:channel_id` | 渠道 Webhook |
| `GET` | `/health` | 健康检查 |
| `GET` | `/ready` | 就绪探针 |
| `GET` | `/metrics` | Prometheus 指标 |

---

## 配置说明

配置文件路径：`~/.fastclaw/config/default.json`（支持 JSON5 注释）

```bash
# 查看完整配置
fastclaw config file

# 读取单个值
fastclaw config get gateway.port

# 修改配置
fastclaw config set gateway.port 8080

# 验证配置
fastclaw config check
```

主要配置段落参见 [docs/MANUAL.md](docs/MANUAL.md)。

---

## 部署方式

### 二进制直接运行

```bash
fastclaw serve
# 或后台守护
fastclaw gateway start
```

### Docker

```bash
docker compose up -d
```

### Kubernetes

```bash
kubectl apply -f deploy/kubernetes/deployment.yaml
```

---

## 桌面应用（FastClaw Desktop）

FastClaw 提供基于 **Tauri v2 + React 19** 的原生桌面应用，内嵌 Gateway 引擎，开箱即用。

### 特性

- **内嵌 Gateway** — 启动即运行，无需单独启动服务端，零配置开始对话
- **原生 IM 界面** — React 19 + TailwindCSS 4 构建的现代聊天界面，支持 Markdown 渲染和代码高亮
- **流式对话** — 通过 Tauri IPC Channel 直接与内嵌 Gateway 通信，无 WebSocket 开销
- **系统托盘** — 最小化到托盘，左键点击显示，关闭窗口仅隐藏
- **全局快捷键** — `Ctrl+Shift+Space` 随时呼出/隐藏窗口
- **系统通知** — 定时任务完成/失败时弹出原生通知，托盘显示未读数
- **多 Agent 管理** — 创建、编辑、删除 Agent，配置模型和工具
- **会话管理** — 多会话切换、历史消息、标题自动生成
- **MCP 集成** — 通过 tauri-plugin-mcp-bridge 连接外部 MCP Server
- **定时任务** — 在桌面端创建和管理 Cron 定时任务
- **自动更新** — 内置 OTA 更新检查（tauri-plugin-updater）
- **开机自启** — 支持 macOS / Windows / Linux 开机自启动
- **数据导入/导出** — 支持会话数据的 ZIP 打包导入导出
- **跨平台** — 支持 Linux（deb / AppImage）、Windows（NSIS）、macOS

### 构建

```bash
cd crates/fastclaw-app

# 安装前端依赖
pnpm install

# 开发模式（前端 HMR + Rust 热编译）
pnpm tauri dev

# 生产构建
pnpm tauri build
```

构建产物：
- Linux: `target/release/bundle/deb/*.deb` / `target/release/bundle/appimage/*.AppImage`
- Windows: `target/release/bundle/nsis/*.exe`
- macOS: `target/release/bundle/macos/*.app`

### 架构

```
┌────────────────────────────────────────────┐
│             Tauri v2 Desktop App           │
│                                            │
│  ┌──────────────────────────────────────┐  │
│  │   React 19 + TailwindCSS Frontend   │  │
│  │  ┌──────────┐ ┌────────┐ ┌───────┐  │  │
│  │  │ Chat UI  │ │Agents  │ │Config │  │  │
│  │  │(Stream)  │ │Manager │ │Panel  │  │  │
│  │  └────┬─────┘ └────┬───┘ └───┬───┘  │  │
│  │       └─────────────┼────────┘       │  │
│  │             Tauri IPC                │  │
│  └──────────────┬───────────────────────┘  │
│                 ▼                           │
│  ┌──────────────────────────────────────┐  │
│  │      Embedded FastClaw Gateway       │  │
│  │  (In-process, no external server)    │  │
│  └──────────────────────────────────────┘  │
│                                            │
│  Plugins: Tray · GlobalShortcut ·          │
│    Notification · Autostart · Updater ·    │
│    MCP Bridge · Dialog · FS · Shell        │
└────────────────────────────────────────────┘
```

详细桌面端使用说明参见 [docs/MANUAL.md](docs/MANUAL.md#桌面应用fastclaw-desktop)。

---

## 开发

```bash
# 开发模式（使用 ~/.fastclaw-dev/ 隔离数据）
fastclaw --dev serve

# 运行测试
cargo test --workspace

# 运行 Clippy
cargo clippy --workspace --all-targets

# 生成 Shell 补全
fastclaw completions bash > ~/.bash_completion.d/fastclaw
fastclaw completions zsh > ~/.zfunc/_fastclaw
```

---

## 许可证

[MIT License](LICENSE) &copy; 2026 linzetai
