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
  <a href="docs/MANUAL.md">使用手册</a> ·
  <a href="#许可证">许可证</a>
</p>

---

## 什么是 FastClaw

FastClaw 是一个用 **Rust** 构建的 AI Agent 编排引擎，专为构建、运行和管理多 Agent 系统而设计。它提供统一的 HTTP/WebSocket 网关、内置 30+ 工具、多模型路由、会话持久化、语义记忆和 WASM 插件系统，让你可以快速搭建从命令行到飞书机器人的完整 AI 应用。

### 核心亮点

- **极致性能** — 纯 Rust 实现，异步 I/O，单二进制文件部署，内存占用 < 50MB
- **多 Agent 编排** — Agent 间消息总线、DAG 工作流、Sub-agent 任务分发
- **丰富的工具生态** — 文件系统、Shell、代码智能（LSP）、Web 搜索、记忆检索等 30+ 内置工具
- **多模型路由** — 支持 OpenAI / Anthropic / DashScope / DeepSeek / Ollama 等，按复杂度自动路由
- **语义记忆** — 向量检索 + 知识图谱双引擎，支持本地嵌入（无需外部 API）
- **MCP 协议** — 同时支持 MCP Server 和 Client，与外部 Agent 生态互通
- **WASM 插件** — 通过 Wasmtime 运行沙箱化插件，热加载无需重启
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
| **Tool System** | 30+ 内置工具，OpenAI 兼容 tool calling 协议，并行执行调度 |
| **MCP** | stdio 传输的 MCP Server + 连接外部 MCP Server 的 Client |
| **Plugins (WASM)** | Wasmtime Component Model，沙箱化执行，热加载 |
| **Channels** | 飞书机器人扩展，支持 WebSocket 长连接和 Webhook 模式 |
| **Evolution** | Agent prompt 自动优化：收集反馈 → 评估 → 蒸馏 → 候选 → 激活 |
| **Cron** | 定时任务调度，支持 Agent 聊天触发和 Webhook 触发 |
| **DAG Workflow** | 有向无环图工作流，支持并行节点、检查点恢复 |
| **Observability** | Prometheus 指标导出、结构化日志（JSON/pretty） |
| **Security** | API Key 认证、速率限制、Prompt 注入检测、Shell 命令沙箱 |
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
│   ├── fastclaw-context/      # 上下文引擎
│   ├── fastclaw-cron/         # 定时任务调度器
│   ├── fastclaw-treesitter/   # Shell AST 分析
│   ├── fastclaw-self-iter/    # 自迭代优化
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
<summary>展开查看 30+ 内置工具</summary>

**文件系统**
- `read_file` — 读取文件内容
- `write_file` — 写入文件
- `edit_file` — 字符串替换编辑
- `multi_edit` — 批量多文件编辑
- `apply_patch` — 应用 unified diff 补丁
- `glob` — 文件名模式搜索
- `grep` — 正则表达式搜索
- `list_directory` — 列出目录内容

**Shell 执行**
- `shell_exec` — 沙箱化 Shell 命令执行（可配置安全策略）

**代码智能**
- `lsp` — 统一 LSP 工具（Go to Definition / Find References / Workspace Symbols）
- `file_outline` — 文件结构大纲
- `code_chunk` — 代码块提取

**网络**
- `web_search` — 网页搜索（支持 Tavily / SearXNG / Google / Baidu / Bing 等）
- `web_fetch` — 获取网页内容
- `http_fetch` — HTTP 请求

**记忆**
- `memory_search` — 搜索语义记忆
- `memory_store` — 存储记忆条目

**实用工具**
- `calculator` — 数学计算
- `current_time` — 获取当前时间
- `sleep` — 等待指定时间

**任务管理**
- `todo_write` — 待办事项管理
- `task_create` / `task_list` / `task_get` / `task_stop` — 子任务管理

**交互**
- `ask_question` — 向用户提问
- `confirm` — 确认操作
- `send_user_message` — 发送中间消息

**其他**
- `notebook_edit` — Jupyter Notebook 编辑
- `terminal_capture` — 终端输出捕获
- `tool_search` — 搜索可用工具
- `skill` — 技能管理（读取/写入）
- `identity` — 身份配置管理

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
