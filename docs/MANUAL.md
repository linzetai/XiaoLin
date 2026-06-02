# XiaoLin 使用手册

> **版本**: 0.0.5 &nbsp;|&nbsp; **最后更新**: 2026-04-30

---

## 目录

- [1. 安装与环境准备](#1-安装与环境准备)
- [2. 首次配置](#2-首次配置)
- [3. 配置文件详解](#3-配置文件详解)
- [4. 启动与运行](#4-启动与运行)
- [5. CLI 命令手册](#5-cli-命令手册)
- [6. HTTP API 接口](#6-http-api-接口)
- [7. WebSocket 实时聊天](#7-websocket-实时聊天)
- [8. Agent 配置](#8-agent-配置)
- [9. 内置工具系统](#9-内置工具系统)
- [10. 模型路由](#10-模型路由)
- [11. 会话管理](#11-会话管理)
- [12. 记忆系统](#12-记忆系统)
- [13. MCP 协议](#13-mcp-协议)
- [14. WASM 插件](#14-wasm-插件)
- [15. 定时任务（Cron）](#15-定时任务cron)
- [16. 渠道接入（飞书）](#16-渠道接入飞书)
- [17. Agent 自进化（Evolution）](#17-agent-自进化evolution)
- [18. 安全配置](#18-安全配置)
- [19. 可观测性](#19-可观测性)
- [20. 备份与恢复](#20-备份与恢复)
- [21. 部署指南](#21-部署指南)
- [22. 桌面应用（XiaoLin Desktop）](#22-桌面应用xiaolin-desktop)
- [23. 故障排查](#23-故障排查)

---

## 1. 安装与环境准备

### 1.1 系统要求

| 依赖 | 版本 | 必需 | 说明 |
|------|------|------|------|
| Rust | 1.82+ | 是 | 编译工具链 |
| SQLite 3 | 3.x | 是 | 会话和记忆存储 |
| GCC / C++17 | 12+ | 可选 | usearch 向量检索后端 |
| Docker | 20+ | 可选 | 容器化部署 |
| Node.js | 18+ | 可选 | Tauri 桌面应用前端 |

### 1.2 从源码编译

```bash
git clone https://github.com/example/xiaolin.git
cd xiaolin

# 开发构建（快速编译，依赖 opt-level=1）
cargo build

# 发布构建（thin-LTO，strip debuginfo）
cargo build --release
```

编译产物位于 `target/release/xiaolin`。将其复制到 `$PATH` 目录即可：

```bash
sudo cp target/release/xiaolin /usr/local/bin/
```

### 1.3 交叉编译（aarch64）

项目提供 `Cross.toml` 配置，使用 [cross](https://github.com/cross-rs/cross) 进行交叉编译：

```bash
cargo install cross
cross build --release --target aarch64-unknown-linux-gnu
```

### 1.4 验证安装

```bash
xiaolin --version
# XiaoLin 0.0.5

xiaolin doctor
```

---

## 2. 首次配置

### 2.1 交互式设置（推荐）

```bash
xiaolin setup
```

引导流程：
1. 选择 LLM 提供商（OpenAI / Anthropic / DashScope / DeepSeek / Gemini / Ollama / 自定义）
2. 输入 API Key
3. 设置 Gateway 端口（默认 18789）
4. 设置 Gateway 认证密钥（可选）

完成后会生成：
- `~/.xiaolin/config/default.json` — 主配置
- `config/agents/main.json` — 默认 Agent

### 2.2 新手引导

```bash
xiaolin onboard
```

在 `setup` 基础上额外展示功能概览和 Quick Start 指南。

### 2.3 手动配置

将项目 `config/default.json` 拷贝到状态目录并编辑：

```bash
mkdir -p ~/.xiaolin/config
cp config/default.json ~/.xiaolin/config/default.json
```

至少需要配置 `credentials` 段落的 LLM API Key。

---

## 3. 配置文件详解

配置文件支持 **JSON5** 格式（允许注释和尾逗号）。路径：`~/.xiaolin/config/default.json`

### 3.1 Gateway 配置

```json5
{
  "gateway": {
    "port": 18789,              // 监听端口
    "tls": {
      "cert": "",               // TLS 证书路径（留空禁用 TLS）
      "key": ""                 // TLS 私钥路径
    },
    "maxConnections": 1024,     // 最大并发连接数
    "corsOrigins": ["*"],       // CORS 允许源（"*" 为全部）
    "rateLimit": {
      "requestsPerSecond": 100, // 每秒请求限制
      "burst": 200              // 突发允许量
    }
  }
}
```

> **安全提示**：当 Gateway 绑定到非 loopback 地址（如 `0.0.0.0`）时，必须配置 `security.apiKeys`，否则启动会被拒绝。

### 3.2 日志配置

```json5
{
  "logging": {
    "level": "info",     // trace / debug / info / warn / error
    "format": "pretty"   // pretty（彩色人类可读）/ json（结构化）
  }
}
```

可通过环境变量 `RUST_LOG` 覆盖，例如 `RUST_LOG=xiaolin_gateway=debug,xiaolin_agent=trace`。

### 3.3 LLM 凭证

```json5
{
  "credentials": {
    "openai":    { "apiKey": "sk-...", "baseUrl": "https://api.openai.com/v1" },
    "anthropic": { "apiKey": "sk-...", "baseUrl": "https://api.anthropic.com" },
    "dashscope": { "apiKey": "sk-...", "baseUrl": "https://coding.dashscope.aliyuncs.com/v1" },
    "deepseek":  { "apiKey": "sk-...", "baseUrl": "https://api.deepseek.com/v1" }
  }
}
```

API Key 也可通过环境变量设置：`OPENAI_API_KEY`、`ANTHROPIC_API_KEY`、`DASHSCOPE_API_KEY`、`DEEPSEEK_API_KEY`。

### 3.4 模型定义

```json5
{
  "models": {
    "openai": {
      "providerType": "openai_compatible",  // openai_compatible / anthropic
      "baseUrl": "https://api.openai.com/v1",
      "defaultModel": "gpt-4o",
      "contextWindow": 128000,
      "supportsVision": true,
      "supportsToolCalling": true,
      "supportsReasoning": false,
      "costPer1kInput": 0.0025,
      "costPer1kOutput": 0.01,
      "maxConcurrent": 10,
      "timeoutSecs": 120
    }
    // ... 其他模型
  }
}
```

### 3.5 会话配置

```json5
{
  "session": {
    "dbPath": "data/sessions.db",           // SQLite 数据库路径
    "ttlHours": 168,                        // 会话过期时间（默认 7 天）
    "dmScope": "per-channel-peer",          // 会话作用域
    "maxMessagesPerSession": 10000,         // 每会话最大消息数
    "compressionThresholdTokens": 4000      // 自动压缩阈值（token 数）
  }
}
```

**dmScope 选项：**
- `per-channel-peer` — 每个渠道+用户独立会话
- `per-channel` — 同一渠道共享会话
- `global` — 全局共享

### 3.6 记忆配置

```json5
{
  "memory": {
    "enabled": true,
    "vectorIndexPath": "data/memory/vectors.usearch",
    "vectorDimensions": 384,
    "knowledgeGraphPath": "data/memory/knowledge.db",
    "forgetting": {
      "enabled": true,
      "maxEntries": 100000,      // 最大记忆条目
      "decayHalfLifeDays": 30    // 衰减半衰期
    },
    "embedding": {
      "provider": "local",       // local / remote / none
      "model": "sentence-transformers/all-MiniLM-L6-v2"  // 本地嵌入模型
    }
  }
}
```

**嵌入提供商：**
- `local` — 纯 Rust 本地推理（hypembed），无需外部 API，首次使用自动下载模型到 `~/.xiaolin/models/`
- `remote` — 调用 OpenAI 兼容 embedding API（需设置 `baseUrl` 和 `apiKey`）
- `none` — 禁用向量检索，仅使用关键词搜索

**推荐本地模型：**
| 模型 | 维度 | 大小 | 特点 |
|------|------|------|------|
| `sentence-transformers/all-MiniLM-L6-v2` | 384 | 22MB | 英文通用（默认） |
| `sentence-transformers/all-MiniLM-L12-v2` | 384 | 33MB | 更高精度 |
| `sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2` | 384 | 45MB | 多语言 |

### 3.7 模型路由配置

```json5
{
  "modelRouter": {
    "enabled": false,
    "defaultTier": "medium",
    "tierMapping": {
      "tiny":     "models.local",
      "small":    "models.dashscope",
      "medium":   "models.dashscope",
      "large":    "models.anthropic",
      "frontier": "models.anthropic"
    }
  }
}
```

### 3.8 插件配置

```json5
{
  "plugins": {
    "directory": "plugins",
    "hotReload": true,
    "defaults": {
      "maxMemoryMb": 64,
      "maxExecutionTimeSecs": 30,
      "maxFuel": 1000000000
    }
  }
}
```

### 3.9 安全配置

```json5
{
  "security": {
    "promptInjectionDetection": true,  // Prompt 注入检测
    "apiKeys": ["your-api-key"]        // Gateway 认证密钥（空数组=禁用认证）
  }
}
```

### 3.10 自定义路径

```json5
{
  "paths": {
    "stateDir": "~/.xiaolin",
    "configDir": "~/.xiaolin/config",
    "pluginsDir": "plugins",
    "extensionsDir": "extensions",
    "skillsDir": "skills",
    "agentsDir": "config/agents"
  }
}
```

---

## 4. 启动与运行

### 4.1 前台运行

```bash
xiaolin serve
```

输出：

```
  ⚡ XiaoLin v5
  ➜  Local:   http://localhost:18789/
  ➜  Network: http://0.0.0.0:18789/

  ✓  Gateway ready on http://127.0.0.1:18789/
```

### 4.2 守护进程模式（仅 Unix）

```bash
# 启动后台守护
xiaolin gateway start
# Started XiaoLin gateway daemon (pid 12345). PID file: ~/.xiaolin/daemon.pid

# 查看状态
xiaolin gateway status

# 重启
xiaolin gateway restart

# 停止
xiaolin gateway stop
```

日志写入 `~/.xiaolin/logs/gateway-daemon.log`。

### 4.3 多环境配置

```bash
# 开发环境（数据隔离在 ~/.xiaolin-dev/）
xiaolin --dev serve

# 命名环境
xiaolin --profile staging serve
# 数据在 ~/.xiaolin-staging/
```

### 4.4 环境变量

| 环境变量 | 说明 |
|----------|------|
| `RUST_LOG` | 日志级别过滤（覆盖配置） |
| `XIAOLIN_STATE_DIR` | 状态目录路径 |
| `OPENAI_API_KEY` | OpenAI API Key |
| `ANTHROPIC_API_KEY` | Anthropic API Key |
| `DASHSCOPE_API_KEY` | DashScope API Key |
| `DEEPSEEK_API_KEY` | DeepSeek API Key |
| `GOOGLE_API_KEY` | Google API Key |
| `XIAOLIN_API_KEYS` | Gateway 认证密钥（逗号分隔） |

---

## 5. CLI 命令手册

### 5.1 `xiaolin setup`

交互式首次配置。选择 LLM 提供商、输入 API Key、设置端口和认证。

### 5.2 `xiaolin onboard`

包含 setup 的新手引导，额外展示功能概览和 Quick Start。

### 5.3 `xiaolin serve`

前台启动 Gateway。等价于 `xiaolin gateway run`。

### 5.4 `xiaolin health`

探测正在运行的 Gateway 的健康状态。读取配置获取端口，请求 `/health`。

### 5.5 `xiaolin doctor`

全面环境诊断，检查项包括：

| 检查项 | 说明 |
|--------|------|
| version | 当前版本 |
| state_dir | 状态目录路径 |
| data_dir | 数据目录是否存在 |
| config_file | 配置文件是否存在 |
| agents | Agent 配置数量 |
| tools | 内置工具数量 |
| session_db | 会话数据库是否存在 |
| llm_api_key | LLM 凭证是否配置 |
| api_auth | Gateway 认证是否启用 |
| gateway | Gateway 是否运行 |
| agent:* | 各 Agent 的模型凭证检查 |
| docker | Docker 是否可用 |

支持 `--json` 输出 JSON 格式结果，方便自动化检查。

### 5.6 `xiaolin config`

```bash
# 查看配置文件路径
xiaolin config path

# 查看完整配置（JSON）
xiaolin config file

# 读取指定键
xiaolin config get gateway.port
xiaolin config get credentials.openai.apiKey

# 设置指定键（自动创建中间对象）
xiaolin config set gateway.port 8080
xiaolin config set logging.level debug

# 验证配置
xiaolin config check

# 修复损坏的配置
xiaolin config fix
```

### 5.7 `xiaolin gateway`

```bash
xiaolin gateway run       # 前台运行（同 serve）
xiaolin gateway start     # 后台启动（Unix only）
xiaolin gateway stop      # 停止后台进程
xiaolin gateway restart   # 重启后台进程
xiaolin gateway status    # 查看后台进程状态
xiaolin gateway health    # 健康检查
```

### 5.8 `xiaolin sessions`

```bash
# 列出最近会话
xiaolin sessions list
xiaolin sessions list --limit 50 --offset 0

# 查看会话详情
xiaolin sessions get <session_id>

# 删除会话
xiaolin sessions delete <session_id>

# 清理过期会话
xiaolin sessions cleanup
xiaolin sessions cleanup --ttl-hours 72
```

### 5.9 `xiaolin agents`

```bash
# 列出所有 Agent
xiaolin agents list

# 查看 Agent 详情
xiaolin agents get main
```

### 5.10 `xiaolin tools`

```bash
# 列出所有内置工具
xiaolin tools list
```

### 5.11 `xiaolin tui`

终端交互界面，通过 WebSocket 连接到运行中的 Gateway：

```bash
# 使用默认连接
xiaolin tui

# 指定 Gateway URL
xiaolin tui --url ws://192.168.1.100:18789/ws

# 带认证
xiaolin tui --token your-api-key

# 恢复会话
xiaolin tui --session <session_id>
```

### 5.12 `xiaolin trace`

```bash
# 列出对话 Trace
xiaolin trace list
xiaolin trace list --limit 100

# 查看 Trace 详情
xiaolin trace show <trace_id>

# 导出为 JSON
xiaolin trace export <trace_id>
```

### 5.13 `xiaolin backup`

```bash
# 创建备份（使用 VACUUM INTO 保证一致性）
xiaolin backup create
xiaolin backup create --output /path/to/backup/

# 恢复备份
xiaolin backup restore /path/to/backup/
```

备份包含：`sessions.db`、`memory.db`、`evolution.db`、`config.json`。

### 5.14 `xiaolin mcp-server`

启动 MCP Server（stdio 传输），暴露 XiaoLin 的内置工具供外部 Agent 调用：

```bash
xiaolin mcp-server
```

可在外部 Agent（如 Claude Desktop、Cursor）中将 XiaoLin 配置为 MCP Server。

### 5.15 `xiaolin completions`

生成 Shell 补全脚本：

```bash
# Bash
xiaolin completions bash > ~/.bash_completion.d/xiaolin

# Zsh
xiaolin completions zsh > ~/.zfunc/_xiaolin

# Fish
xiaolin completions fish > ~/.config/fish/completions/xiaolin.fish

# PowerShell
xiaolin completions powershell > _xiaolin.ps1
```

---

## 6. HTTP API 接口

### 6.1 认证

当 `security.apiKeys` 非空时，所有 API 请求需要携带认证头：

```
Authorization: Bearer <your-api-key>
```

或：

```
X-API-Key: <your-api-key>
```

查询认证状态：

```bash
curl http://localhost:18789/api/v1/auth/status
# {"authRequired": true}
```

### 6.2 聊天补全

**POST** `/api/v1/chat` 或 `/api/v1/chat/completions`

请求体：

```json
{
  "messages": [
    {"role": "user", "content": "帮我写一个 Python 快速排序"}
  ],
  "agent_id": "main",
  "session_id": "optional-session-id",
  "stream": true,
  "model": null,
  "max_tokens": 4096,
  "temperature": 0.7
}
```

**非流式响应**（`stream: false`）：

```json
{
  "id": "chatcmpl-...",
  "choices": [
    {
      "index": 0,
      "message": {
        "role": "assistant",
        "content": "..."
      },
      "finish_reason": "stop"
    }
  ],
  "usage": {
    "prompt_tokens": 100,
    "completion_tokens": 200,
    "total_tokens": 300
  },
  "model": "qwen3.5-plus"
}
```

**流式响应**（`stream: true`）：

返回 Server-Sent Events (SSE)，每行格式为 `data: <JSON>\n\n`，最后以 `data: [DONE]` 结尾。

### 6.3 Agent 管理

```bash
# 列出 Agent
curl http://localhost:18789/api/v1/agents

# 获取 Agent
curl http://localhost:18789/api/v1/agents/main

# 创建 Agent
curl -X POST http://localhost:18789/api/v1/agents \
  -H "Content-Type: application/json" \
  -d '{"agentId":"helper","name":"Helper","model":{"provider":"openai","model":"gpt-4o"}}'

# 更新 Agent
curl -X PUT http://localhost:18789/api/v1/agents/helper \
  -H "Content-Type: application/json" \
  -d '{"name":"Updated Helper"}'

# 删除 Agent
curl -X DELETE http://localhost:18789/api/v1/agents/helper

# 列出 Agent 工具
curl http://localhost:18789/api/v1/agents/main/tools
```

### 6.4 会话管理

```bash
# 列出会话
curl "http://localhost:18789/api/v1/sessions?limit=20&offset=0"

# 获取会话
curl http://localhost:18789/api/v1/sessions/<session_id>

# 获取会话消息
curl http://localhost:18789/api/v1/sessions/<session_id>/messages

# 删除会话
curl -X DELETE http://localhost:18789/api/v1/sessions/<session_id>
```

### 6.5 记忆 API

```bash
# 列出情景记忆
curl http://localhost:18789/api/v1/memory/episodes

# 搜索情景记忆
curl "http://localhost:18789/api/v1/memory/episodes/search?q=关键词"

# 列出语义事实
curl http://localhost:18789/api/v1/memory/facts

# 搜索语义事实
curl "http://localhost:18789/api/v1/memory/facts/search?q=关键词"

# 创建/更新事实
curl -X POST http://localhost:18789/api/v1/memory/facts \
  -H "Content-Type: application/json" \
  -d '{"subject":"用户","predicate":"偏好","object":"Python编程"}'

# 删除事实
curl -X DELETE http://localhost:18789/api/v1/memory/facts/<fact_id>
```

### 6.6 定时任务 API

```bash
# 列出定时任务
curl http://localhost:18789/api/v1/cron/jobs

# 创建/更新定时任务
curl -X POST http://localhost:18789/api/v1/cron/jobs \
  -H "Content-Type: application/json" \
  -d '{
    "name": "每日汇总",
    "schedule": "0 9 * * *",
    "action": {
      "type": "agent_chat",
      "agentId": "main",
      "message": "请总结今天的待办事项"
    }
  }'

# 获取定时任务详情
curl http://localhost:18789/api/v1/cron/jobs/<job_id>

# 删除定时任务
curl -X DELETE http://localhost:18789/api/v1/cron/jobs/<job_id>
```

### 6.7 Agent 消息总线

```bash
# 列出总线上的 Agent
curl http://localhost:18789/api/v1/bus/agents

# 发送消息（fire-and-forget）
curl -X POST http://localhost:18789/api/v1/bus/send \
  -H "Content-Type: application/json" \
  -d '{"targetAgentId":"main","message":"处理这个任务"}'

# 请求-回复
curl -X POST http://localhost:18789/api/v1/bus/request \
  -H "Content-Type: application/json" \
  -d '{"targetAgentId":"main","message":"你好"}'
```

### 6.8 健康与运维端点

```bash
# 健康检查
curl http://localhost:18789/health
# {"status":"ok"}

# 就绪探针
curl http://localhost:18789/ready
# {"status":"ready","agents":1,"checks":{"database":true,"agents_configured":true}}

# Prometheus 指标
curl http://localhost:18789/metrics

# 结构化指标
curl http://localhost:18789/api/v1/metrics

# OpenAPI 规范
curl http://localhost:18789/api/v1/openapi.json
```

---

## 7. WebSocket 实时聊天

连接 `ws://localhost:18789/ws`，协议为 JSON 文本帧。

### 7.1 发送消息

```json
{
  "type": "chat",
  "agentId": "main",
  "sessionId": "optional",
  "message": "你好",
  "stream": true
}
```

### 7.2 接收事件

**流式文本块：**
```json
{
  "type": "stream",
  "delta": "你好",
  "sessionId": "..."
}
```

**工具调用：**
```json
{
  "type": "tool_call",
  "name": "read_file",
  "arguments": {"path": "/tmp/test.txt"},
  "toolCallId": "call_xxx"
}
```

**工具结果：**
```json
{
  "type": "tool_result",
  "toolCallId": "call_xxx",
  "content": "文件内容..."
}
```

**完成：**
```json
{
  "type": "done",
  "sessionId": "...",
  "usage": {"prompt_tokens": 100, "completion_tokens": 200}
}
```

**通知事件：**
```json
{
  "type": "event",
  "event": "notification.new",
  "data": {"id": "...", "category": "cron", "title": "..."}
}
```

---

## 8. Agent 配置

Agent 配置文件位于 `config/agents/` 目录，每个 Agent 一个 JSON 文件。

### 8.1 配置结构

```json
{
  "agentId": "main",
  "name": "小林助手",
  "description": "通用型 AI 助手",
  "model": {
    "provider": "dashscope",
    "model": "qwen3.5-plus",
    "temperature": 0.7,
    "maxTokens": 4096,
    "contextWindow": null,
    "fallbacks": [],
    "maxConcurrentRequests": 10
  },
  "systemPrompt": null,
  "tools": [
    {
      "id": "feishu_send_message",
      "enabled": true,
      "config": null
    }
  ],
  "behavior": {
    "maxToolCallsPerTurn": 50,
    "maxConsecutiveErrors": 3,
    "requireConfirmationFor": [],
    "toolsAllow": [],
    "toolsDeny": [],
    "fileAccess": "workspace"
  },
  "mcpServers": [],
  "minTier": null,
  "maxTier": null
}
```

### 8.2 模型 Fallback

```json
{
  "model": {
    "provider": "anthropic",
    "model": "claude-sonnet-4-20250514",
    "fallbacks": [
      {"provider": "openai", "model": "gpt-4o"},
      {"provider": "dashscope", "model": "qwen3.5-plus"}
    ]
  }
}
```

当主模型不可用时，自动切换到 fallback 列表中的下一个。

### 8.3 工具控制

- `toolsAllow: []` — 空数组表示允许所有工具
- `toolsDeny: ["shell_exec"]` — 禁止特定工具
- `fileAccess: "workspace"` — 限制文件访问范围

### 8.4 热重载

修改 `config/agents/` 下的文件后，Gateway 会自动检测并热重载（无需重启）。也可以发送 `SIGHUP` 信号触发：

```bash
kill -HUP $(cat ~/.xiaolin/daemon.pid)
```

### 8.5 System Prompt 模板

项目在 `prompts/` 目录提供了预制 System Prompt 模板：

| 文件 | 用途 |
|------|------|
| `system-base.md` | 基础 System Prompt |
| `tool-usage-guide.md` | 工具使用指导 |
| `agents/main.md` | 通用助手 |
| `agents/code-assistant.md` | 编程助手 |
| `agents/code-reviewer.md` | 代码审查 |
| `agents/research.md` | 研究助手 |
| `agents/writing.md` | 写作助手 |
| `agents/data-analyst.md` | 数据分析 |
| `agents/devops.md` | DevOps 助手 |
| `agents/security-auditor.md` | 安全审计 |
| `agents/customer-support.md` | 客服 |
| `agents/product-manager.md` | 产品经理 |
| `agents/qa-tester.md` | QA 测试 |
| `agents/tutor.md` | 教学辅导 |
| `agents/api-builder.md` | API 构建 |

在 Agent 配置中引用：

```json
{
  "systemPrompt": "file://prompts/agents/code-assistant.md"
}
```

---

## 9. 内置工具系统

### 9.1 工具分类

XiaoLin 将工具按操作性质分为以下类别，用于并发调度：

| 类别 | 并发安全 | 说明 |
|------|---------|------|
| **Read** | 是 | 文件读取（read_file, list_dir 等） |
| **Search** | 是 | 搜索（grep, glob, workspace_symbols） |
| **Fetch** | 是 | 网络获取（web_fetch, web_search） |
| **Think** | 是 | 计算（calculator, current_time） |
| **Edit** | 否 | 文件写入（write_file, edit_file） |
| **Execute** | 否 | Shell 执行（shell_exec） |
| **Other** | 否 | 其他 |

Read / Search / Fetch / Think 类工具可以并行执行，Edit / Execute 类工具必须串行。

### 9.2 工具错误类型

工具返回结构化错误类型，帮助 Agent 理解失败原因并选择恢复策略：

- `file_not_found` — 文件不存在
- `permission_denied` — 权限不足
- `edit_no_occurrence_found` — 编辑目标未找到
- `edit_multiple_occurrences` — 编辑目标有多个匹配
- `shell_execute_error` — Shell 命令执行失败
- `path_not_in_workspace` — 路径越界

### 9.3 Deferred Tools

部分低频工具注册为 "deferred"（延迟加载），不会出现在默认工具列表中，但可通过 `tool_search` 发现并调用：

- `notebook_edit` — Jupyter Notebook 编辑
- `terminal_capture` — 终端输出捕获
- `task_list` / `task_get` / `task_stop` — 任务管理
- `enter_plan_mode` / `exit_plan_mode` — 计划模式

---

## 10. 模型路由

### 10.1 概念

模型路由根据任务复杂度自动选择最合适（性价比最优）的模型：

| 层级 | 典型场景 | 示例模型 |
|------|---------|---------|
| tiny | 简单格式化、分类 | 本地小模型 |
| small | 简单问答 | qwen-turbo |
| medium | 常规对话 | qwen3.5-plus |
| large | 复杂推理 | claude-sonnet |
| frontier | 最高质量 | claude-opus |

### 10.2 启用

在配置中设置：

```json5
{
  "modelRouter": {
    "enabled": true,
    "defaultTier": "medium",
    "tierMapping": {
      "tiny":     "models.local",
      "small":    "models.dashscope",
      "medium":   "models.dashscope",
      "large":    "models.anthropic",
      "frontier": "models.anthropic"
    }
  }
}
```

### 10.3 预算追踪

模型路由内置成本追踪，根据 `costPer1kInput` / `costPer1kOutput` 计算实际开销。

---

## 11. 会话管理

### 11.1 自动会话

API 请求如果不指定 `session_id`，Gateway 会自动创建新会话。指定 `session_id` 则恢复已有会话的上下文。

### 11.2 上下文压缩

当会话消息超过 `compressionThresholdTokens` 时，自动触发上下文压缩：将历史消息摘要化，保留最近的完整消息。

### 11.3 会话清理

```bash
# 清理超过 72 小时的会话
xiaolin sessions cleanup --ttl-hours 72
```

### 11.4 会话 DM Scope

`dmScope` 控制渠道消息如何映射到会话：

- `per-channel-peer` — 每个用户在每个渠道有独立会话
- `per-channel` — 同一渠道的所有用户共享会话
- `global` — 全局共享一个会话

---

## 12. 记忆系统

### 12.1 双引擎架构

XiaoLin 的记忆系统由两个引擎组成：

**Episodic Memory（情景记忆）**
- 记录对话片段（episodes）
- 向量化存储，支持语义相似度搜索
- 使用 usearch 进行高性能向量检索

**Semantic Memory（语义事实记忆）**
- 以三元组形式（Subject, Predicate, Object）存储结构化知识
- 支持知识图谱查询
- 支持人工管理（CRUD）

### 12.2 记忆衰减

启用 `forgetting` 后，长期未访问的记忆条目会逐渐衰减：

- `maxEntries` — 超过上限时淘汰最旧条目
- `decayHalfLifeDays` — 衰减半衰期（30天 = 30天后权重降至50%）

### 12.3 嵌入模型

本地嵌入使用 hypembed（纯 Rust 实现），首次使用自动下载模型到 `~/.xiaolin/models/`。支持 HuggingFace 上的 sentence-transformers 系列模型。

---

## 13. MCP 协议

### 13.1 作为 MCP Server

XiaoLin 可以作为 MCP Server 运行，向外部 Agent 暴露其内置工具：

```bash
xiaolin mcp-server
```

**在 Claude Desktop 中配置：**

```json
{
  "mcpServers": {
    "xiaolin": {
      "command": "/usr/local/bin/xiaolin",
      "args": ["mcp-server"]
    }
  }
}
```

### 13.2 连接外部 MCP Server

在 Agent 配置中指定要连接的外部 MCP Server：

```json
{
  "mcpServers": [
    {
      "id": "filesystem",
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
    }
  ]
}
```

---

## 14. WASM 插件

### 14.1 插件目录

插件放在 `plugins/` 目录下，每个 `.wasm` 文件是一个插件。

### 14.2 沙箱限制

每个插件在独立的 Wasmtime 沙箱中运行：

- `maxMemoryMb: 64` — 最大内存
- `maxExecutionTimeSecs: 30` — 最大执行时间
- `maxFuel: 1_000_000_000` — 最大指令数

### 14.3 热加载

设置 `plugins.hotReload: true` 后，修改 `plugins/` 目录中的文件会自动重新加载，无需重启 Gateway。

---

## 15. 定时任务（Cron）

### 15.1 任务类型

定时任务支持两种触发方式：

**Agent Chat** — 定时向 Agent 发送消息：
```json
{
  "action": {
    "type": "agent_chat",
    "agentId": "main",
    "message": "请总结今天的工作"
  }
}
```

**Webhook** — 定时调用 HTTP 端点：
```json
{
  "action": {
    "type": "webhook",
    "url": "https://example.com/api/trigger",
    "method": "POST",
    "body": {"key": "value"}
  }
}
```

### 15.2 Cron 表达式

使用标准 cron 表达式（5 字段）：

```
┌───────────── 分钟 (0-59)
│ ┌───────────── 小时 (0-23)
│ │ ┌───────────── 日 (1-31)
│ │ │ ┌───────────── 月 (1-12)
│ │ │ │ ┌───────────── 星期 (0-6, 0=周日)
│ │ │ │ │
* * * * *
```

示例：
- `0 9 * * *` — 每天早上 9 点
- `*/5 * * * *` — 每 5 分钟
- `0 9 * * 1-5` — 工作日早上 9 点

### 15.3 渠道通知

定时任务完成/失败时可通过渠道推送通知：

```json
{
  "notifyChannels": [
    {
      "channelId": "feishu_main",
      "targetId": "oc_xxx",
      "targetType": "group"
    }
  ]
}
```

---

## 16. 渠道接入（飞书）

### 16.1 配置飞书机器人

在 `config/default.json` 中的 `agents.list` 为 Agent 配置飞书渠道：

```json
{
  "agents": {
    "list": [
      {
        "id": "main",
        "channels": {
          "feishu_main": {
            "enabled": true,
            "app_id": "cli_xxxxx",
            "app_secret": "xxxxx",
            "verification_token": "xxxxx",
            "encrypt_key": "xxxxx",
            "connectionMode": "websocket",
            "replyMode": "mention_only",
            "domain": "https://open.feishu.cn"
          }
        }
      }
    ]
  }
}
```

### 16.2 连接模式

- `websocket` — 长连接模式（推荐，无需公网 IP）
- `webhook` — HTTP 回调模式（需配置公网可达的 Webhook URL）

### 16.3 回复模式

- `mention_only` — 仅在被 @ 时回复
- `all` — 回复所有消息

### 16.4 路由绑定

```json
{
  "bindings": [
    {
      "agentId": "main",
      "match": { "channel": "feishu_main" }
    }
  ]
}
```

---

## 17. Agent 自进化（Evolution）

Evolution 模块默认启用，无需额外配置。它负责从对话轨迹中自动提取、评估和管理可复用的技能（Skills）。

### 17.1 流程

1. **收集反馈** — 用户对 Agent 回复评分
2. **评估** — 分析反馈数据，计算质量分
3. **蒸馏** — 从高评分对话中提取优化后的 System Prompt
4. **候选** — 生成候选 Prompt
5. **激活** — 人工审核后激活新 Prompt

### 17.2 API

```bash
# 提交反馈
curl -X POST http://localhost:18789/api/v1/evolution/feedback \
  -d '{"agentId":"main","traceId":"...","rating":5,"comment":"回答很好"}'

# 查看反馈
curl http://localhost:18789/api/v1/evolution/feedback/main

# 评估 Agent
curl http://localhost:18789/api/v1/evolution/evaluate/main

# 蒸馏优化 Prompt
curl -X POST http://localhost:18789/api/v1/evolution/distill/main

# 列出候选
curl http://localhost:18789/api/v1/evolution/candidates/main

# 接受/拒绝候选
curl -X POST http://localhost:18789/api/v1/evolution/candidates/<id>/accept
curl -X POST http://localhost:18789/api/v1/evolution/candidates/<id>/reject
```

---

## 18. 安全配置

### 18.1 API Key 认证

```json
{
  "security": {
    "apiKeys": ["key1", "key2"]
  }
}
```

请求时携带 `Authorization: Bearer key1` 或 `X-API-Key: key1`。

### 18.2 速率限制

```json
{
  "gateway": {
    "rateLimit": {
      "requestsPerSecond": 100,
      "burst": 200
    }
  }
}
```

### 18.3 Prompt 注入检测

启用后会检测用户输入中的 Prompt 注入攻击：

```json
{
  "security": {
    "promptInjectionDetection": true
  }
}
```

### 18.4 Shell 沙箱

Shell 工具默认运行在沙箱模式下，限制危险命令的执行。通过 Agent 配置中的 `behavior.requireConfirmationFor` 要求用户确认高危操作。

### 18.5 非 Loopback 绑定保护

当 Gateway 绑定到非 loopback 地址（如 `0.0.0.0`）时，如果未配置 API Key，启动会被拒绝，防止意外暴露未认证的服务。

---

## 19. 可观测性

### 19.1 日志

```bash
# 彩色输出（开发）
xiaolin serve

# JSON 结构化输出（生产）
xiaolin --json serve

# 环境变量控制级别
RUST_LOG=debug xiaolin serve
RUST_LOG=xiaolin_gateway=trace,xiaolin_agent=debug xiaolin serve
```

### 19.2 Prometheus 指标

```bash
# 原始 Prometheus 格式
curl http://localhost:18789/metrics

# 结构化指标
curl http://localhost:18789/api/v1/metrics
```

Agent 重载事件也会记录指标。

### 19.3 对话 Trace

Trace 记录每次对话的完整执行过程，包括每轮的工具调用、延迟、模型选择等：

```bash
xiaolin trace list
xiaolin trace show <trace_id>
```

---

## 20. 备份与恢复

### 20.1 创建备份

```bash
# 自动备份到 ~/.xiaolin/backups/<timestamp>/
xiaolin backup create

# 指定输出目录
xiaolin backup create --output /mnt/backup/xiaolin/
```

使用 SQLite `VACUUM INTO` 命令创建一致性快照，即使在 Gateway 运行中也能安全备份。

### 20.2 恢复

```bash
# 停止 Gateway
xiaolin gateway stop

# 恢复
xiaolin backup restore /path/to/backup/

# 重启
xiaolin gateway start
```

### 20.3 备份内容

| 文件 | 说明 |
|------|------|
| `sessions.db` | 会话和消息 |
| `memory.db` | 记忆数据 |
| `evolution.db` | Agent 进化数据 |
| `config.json` | 配置快照 |

---

## 21. 部署指南

### 21.1 二进制部署

```bash
# 编译
cargo build --release

# 部署到服务器
scp target/release/xiaolin user@server:/usr/local/bin/

# 在服务器上配置
ssh user@server
xiaolin setup
xiaolin gateway start
```

### 21.2 Docker 部署

```bash
# 构建镜像
docker build -t xiaolin .

# 运行
docker run -d \
  --name xiaolin \
  -p 18789:18789 \
  -v xiaolin-data:/app/data \
  -v ./config:/app/config:ro \
  -e DASHSCOPE_API_KEY=sk-xxx \
  xiaolin
```

**Docker Compose：**

```bash
# 设置环境变量
export DASHSCOPE_API_KEY=sk-xxx

# 启动
docker compose up -d

# 查看日志
docker compose logs -f

# 停止
docker compose down
```

### 21.3 Kubernetes 部署

```bash
# 创建 ConfigMap
kubectl create configmap xiaolin-config --from-file=config/default.json

# 创建 Secret
kubectl create secret generic xiaolin-secrets \
  --from-literal=DASHSCOPE_API_KEY=sk-xxx

# 部署
kubectl apply -f deploy/kubernetes/deployment.yaml
```

提供的 `deployment.yaml` 包含：
- Deployment（含 liveness 和 readiness 探针）
- Service（ClusterIP）
- PersistentVolumeClaim（5Gi）

资源建议：
- 最小配置：256Mi 内存，250m CPU
- 推荐配置：1Gi 内存，1000m CPU

---

## 22. 桌面应用（XiaoLin Desktop）

XiaoLin 提供基于 Tauri v2 + React 19 的原生桌面应用。桌面端内嵌完整的 Gateway 引擎，启动即可使用，无需单独部署服务端。

### 22.1 技术栈

| 层 | 技术 |
|----|------|
| 原生外壳 | Tauri v2（Rust） |
| 前端框架 | React 19 + TypeScript 6 |
| 样式 | TailwindCSS 4 |
| 构建工具 | Vite 8 |
| 状态管理 | Zustand 5 |
| Markdown | react-markdown + rehype-highlight + remark-gfm |
| 虚拟列表 | react-virtuoso |
| 图标 | lucide-react |
| 测试 | Vitest + Testing Library |

### 22.2 功能概览

**核心对话**
- 多 Agent 流式聊天，Tauri IPC Channel 直传（非 WebSocket），延迟更低
- Markdown 渲染、代码高亮、GFM 表格
- 工具调用实时展示（tool_call → tool_result → 流式输出）
- 支持取消正在进行的聊天流
- 对话中可提交用户确认/回答（`submit_tool_answer`）

**会话管理**
- 多会话列表，创建 / 切换 / 删除
- 标题自动生成（smart title）
- 为每个会话设置独立的工作目录（`set_session_work_dir`）
- 历史消息浏览

**Agent 管理**
- 列出所有 Agent 及其工具
- 创建 / 编辑 / 删除 Agent
- 上传 Agent 头像
- 编辑 Agent 工具列表
- 读取身份文件（SOUL.md / USER.md / AGENTS.md）

**配置与模型**
- 在界面中查看和修改配置（credentials、logging、memory 等）
- 配置项安全读取：API Key 自动脱敏显示（`sk-12...cdef`）
- 写入保护：`gateway` 段为只读，不可通过 UI 修改
- 测试模型连接（`test_model_connection`）
- 列出所有可用模型

**渠道管理**
- 列出渠道状态
- 绑定 / 解绑 Agent 与渠道
- 重载渠道连接

**MCP 管理**
- 查看 MCP Server 连接状态
- 添加 / 删除 MCP Server
- 重载 MCP Server 列表
- 通过 `tauri-plugin-mcp-bridge` 桥接外部 MCP

**定时任务**
- 创建 / 编辑 / 删除定时任务
- 查看任务执行历史

**通知中心**
- 通知列表、详情
- 标记已读 / 全部已读
- 删除通知 / 清除已读
- 未读计数（托盘 tooltip 显示）
- 系统原生通知弹窗

**数据管理**
- 导入数据（ZIP 格式）
- 导出数据（ZIP 格式，包含会话、配置等）

**Skill 管理**
- 列出可用 Skills
- 刷新 Skill 列表
- 上传自定义 Skill

### 22.3 系统集成

**系统托盘**

桌面端始终保留系统托盘图标：
- 左键单击 → 显示/聚焦主窗口
- 右键菜单 → 「显示窗口」/「退出」
- 未读通知时 tooltip 显示未读计数，如 `XiaoLin (3 条未读)`

**全局快捷键**

- `Ctrl+Shift+Space` — 切换窗口显示/隐藏（全局可用，即使应用不在前台）

**窗口行为**

- 关闭按钮不退出应用，仅隐藏到托盘
- 自定义标题栏（`decorations: false`），提供原生拖拽区域
- 最小窗口尺寸 640×480

**开机自启**

使用 `tauri-plugin-autostart`，支持：
- macOS: LaunchAgent
- Windows / Linux: 对应平台的自启动机制

**自动更新**

使用 `tauri-plugin-updater`：
- 启动时自动检查 GitHub Releases 的 `latest.json`
- Windows 使用 passive 安装模式（静默后台更新）
- 更新签名验证（Ed25519 公钥）

### 22.4 内嵌 Gateway

桌面端启动时自动在进程内启动一个完整的 XiaoLin Gateway：

1. 加载配置（debug 模式使用 `~/.xiaolin-dev/`，release 模式使用 `~/.xiaolin/`）
2. 绑定配置的端口；如果端口已被占用，自动回退到随机端口
3. 等待 Gateway 健康检查通过（最多 10 秒）
4. 通知前端 Gateway 就绪（`gateway://started` 事件）
5. 前端通过 Tauri IPC 直接调用 Rust Commands，无需经过 HTTP

如果 Gateway 启动失败，弹出系统通知告知用户。

### 22.5 Tauri IPC Commands

桌面端通过 Tauri IPC 暴露以下 Rust Commands 给前端调用：

| 模块 | 命令 | 说明 |
|------|------|------|
| **Chat** | `chat_stream` | 流式聊天（通过 Tauri Channel 传输 delta） |
| | `cancel_chat_stream` | 取消正在进行的聊天 |
| | `submit_tool_answer` | 提交用户回答（confirm / ask_question） |
| **Config** | `get_config` | 获取配置（脱敏后） |
| | `set_config` | 修改配置 |
| | `list_models` | 列出模型 |
| | `test_model_connection` | 测试模型连接 |
| | `get_gateway_info` | 获取内嵌 Gateway 信息 |
| | `health_check` | 健康检查 |
| **Agent** | `list_agents` | 列出 Agent |
| | `get_agent` | 获取 Agent 详情 |
| | `create_agent` | 创建 Agent |
| | `update_agent` | 更新 Agent |
| | `delete_agent` | 删除 Agent |
| | `list_tools` / `list_agent_tools` | 列出工具 |
| | `update_agent_tools` | 更新 Agent 工具列表 |
| | `upload_agent_avatar` | 上传头像 |
| | `read_identity_files` | 读取身份文件 |
| **Session** | `list_sessions` | 列出会话 |
| | `create_session` | 创建会话 |
| | `get_session` / `get_session_messages` | 获取会话和消息 |
| | `update_session_title` | 更新标题 |
| | `delete_session` | 删除会话 |
| | `set_session_work_dir` | 设置工作目录 |
| **Channel** | `list_channels` | 列出渠道 |
| | `bind_agent_channel` / `unbind_agent_channel` | 绑定/解绑 |
| | `reload_channel` | 重载渠道 |
| **MCP** | `get_mcp_status` | MCP 状态 |
| | `add_mcp_server` / `remove_mcp_server` | 添加/删除 |
| | `reload_mcp_servers` | 重载 |
| **Cron** | `cron_list_jobs` / `cron_get_job` | 列出/获取任务 |
| | `cron_upsert_job` / `cron_delete_job` | 创建/删除任务 |
| | `cron_list_runs` | 执行历史 |
| **Notification** | `notification_list` / `notification_get` | 列出/获取通知 |
| | `notification_mark_read` / `notification_mark_all_read` | 标记已读 |
| | `notification_unread_count` | 未读计数 |
| | `notification_delete` / `notification_clear_read` | 删除/清除 |
| **Skill** | `list_skills` / `refresh_skills` / `upload_skill` | 技能管理 |
| **Migration** | `import_data` / `export_data` | 数据导入导出 |

### 22.6 前端项目结构

```
crates/xiaolin-app/
├── src/                        # React 前端
│   ├── App.tsx                 # 根组件
│   ├── main.tsx                # 入口
│   ├── index.css               # TailwindCSS 样式
│   ├── components/
│   │   ├── layout/             # 应用布局（侧边栏 + 主区域）
│   │   ├── message-stream/     # 聊天消息流（流式渲染）
│   │   ├── agent-list/         # Agent 列表
│   │   ├── agent-detail/       # Agent 详情/编辑
│   │   ├── settings/           # 设置面板
│   │   ├── notification/       # 通知中心
│   │   └── onboarding/         # 首次使用引导
│   ├── lib/                    # 状态管理（Zustand stores）
│   └── hooks/                  # React hooks
├── src-tauri/                  # Tauri Rust 后端
│   ├── src/
│   │   ├── lib.rs              # 应用入口、托盘、快捷键
│   │   ├── embedded.rs         # 内嵌 Gateway 启动/管理
│   │   └── commands/           # 12 个 IPC Command 模块
│   ├── tauri.conf.json         # Tauri 配置
│   └── Cargo.toml
├── package.json
├── vite.config.ts
└── tsconfig.json
```

### 22.7 安全配置（CSP）

桌面端配置了严格的 Content Security Policy：

```
default-src 'self' data: blob:;
connect-src 'self' http://127.0.0.1:* ws://127.0.0.1:* http://localhost:* ws://localhost:* https:;
img-src 'self' data: blob: https: http://asset.localhost asset:;
style-src 'self' 'unsafe-inline';
font-src 'self' data:;
media-src 'self' data: blob:;
object-src 'none';
frame-ancestors 'none';
base-uri 'self'
```

仅允许连接本地 Gateway（`127.0.0.1` / `localhost`）和 HTTPS 外部资源。

### 22.8 开发与构建

**环境准备**

```bash
# 安装 Tauri CLI
pnpm add -D @tauri-apps/cli

# 安装前端依赖
cd crates/xiaolin-app
pnpm install
```

**开发模式**

```bash
pnpm tauri dev
```

前端使用 Vite HMR（`localhost:1420`），Rust 后端自动热编译。Vite 开发服务器自动代理 `/api` 请求到 Gateway 端口。

**生产构建**

```bash
pnpm tauri build
```

构建产物包含：
- **Linux**: `.deb`（依赖 libwebkit2gtk-4.1-0, libgtk-3-0, libayatana-appindicator3-1）和 `.AppImage`
- **Windows**: NSIS 安装程序（支持简体中文和英文语言选择）
- **macOS**: `.app` 应用包

更新器会同时生成 `latest.json` 和签名文件，用于 OTA 更新。

---

## 23. 故障排查

### 23.1 环境诊断

```bash
xiaolin doctor
```

逐项检查版本、配置、凭证、Gateway 状态等。支持 `--json` 输出方便自动化。

### 23.2 常见问题

**Gateway 启动失败**

```
refusing to start gateway on non-loopback address without security.api_keys configured
```

原因：绑定到非 loopback 地址但未配置认证。解决：在 `security.apiKeys` 中添加 API Key。

**LLM 调用失败**

```
missing credentials.xxx.apiKey in config
```

原因：未配置 LLM API Key。解决：通过 `xiaolin config set credentials.xxx.apiKey sk-xxx` 设置，或设置对应环境变量。

**会话数据库未找到**

```
No session database found
```

原因：Gateway 未启动或未运行过。解决：先启动 `xiaolin serve`，数据库会自动创建。

**守护进程已存在**

```
gateway daemon already running (pid xxx)
```

原因：已有后台进程在运行。解决：先 `xiaolin gateway stop`，再重新启动。

**配置文件损坏**

```bash
# 尝试自动修复
xiaolin config fix

# 或重新生成
xiaolin setup
```

### 23.3 日志调试

```bash
# 开启详细日志
RUST_LOG=debug xiaolin serve

# 特定模块的 trace 日志
RUST_LOG=xiaolin_agent=trace xiaolin serve

# 查看守护进程日志
tail -f ~/.xiaolin/logs/gateway-daemon.log
```

### 23.4 健康检查

```bash
# CLI 健康检查
xiaolin health

# HTTP 健康检查
curl http://localhost:18789/health

# 就绪检查（含数据库和 Agent 配置验证）
curl http://localhost:18789/ready
```
