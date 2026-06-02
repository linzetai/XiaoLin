# LLM 提供商插件接入手册

> **版本**: 0.0.5 &nbsp;|&nbsp; **最后更新**: 2026-05-09

---

## 目录

- [概述](#概述)
- [快速开始](#快速开始)
- [插件目录与文件格式](#插件目录与文件格式)
- [中间件模式 (Middleware)](#中间件模式-middleware)
  - [基础配置](#基础配置)
  - [认证方式](#认证方式)
  - [模型映射](#模型映射)
- [外部进程模式 (Process)](#外部进程模式-process)
  - [stdio 协议规范](#stdio-协议规范)
  - [示例实现](#示例实现)
- [前端管理](#前端管理)
- [Agent 配置中使用插件](#agent-配置中使用插件)
- [REST API 参考](#rest-api-参考)
- [故障排除](#故障排除)

---

## 概述

LLM 提供商插件系统允许你在不修改 XiaoLin 源码的前提下接入任意 LLM 服务。适用场景包括：

- 企业内网网关（需要自定义请求头、OAuth2 鉴权）
- 私有部署的模型服务（自定义 base URL）
- 非标准协议的 LLM 服务（通过外部进程桥接）
- 需要在请求前动态获取 token 的认证体系

系统支持两种插件模式：

| 模式 | 适用场景 | 实现方式 |
|------|---------|---------|
| **中间件 (Middleware)** | 上游兼容 OpenAI/Anthropic 协议，仅需自定义 URL、请求头、鉴权 | 纯配置，无需编写代码 |
| **外部进程 (Process)** | 上游使用非标准协议，或需要复杂的请求变换 | 编写任意语言的可执行程序，通过 JSON-over-stdio 通信 |

插件提供商在系统中是一等公民——支持 fallback 链、模型路由、前端模型选择器等所有现有能力。

---

## 快速开始

### 1. 创建插件配置文件

在插件目录（默认 `~/.xiaolin/plugins/llm/`）下创建一个 JSON 文件：

```json
{
  "id": "my-gateway",
  "name": "My LLM Gateway",
  "version": "1.0.0",
  "type": "middleware",
  "middleware": {
    "baseUrl": "https://llm-gateway.mycompany.com/v1",
    "protocol": "openai",
    "headers": {
      "x-gateway-app": "xiaolin"
    },
    "auth": {
      "type": "bearer_token",
      "token": "your-api-key-here"
    }
  },
  "models": [
    { "id": "gpt-4o", "name": "GPT-4o (via Gateway)", "contextWindow": 128000 }
  ]
}
```

### 2. 重启 XiaoLin 或通过 API 创建

```bash
# 方式一：直接放置文件后重启
xiaolin serve

# 方式二：通过 REST API 动态创建（无需重启）
curl -X POST http://localhost:3000/api/v1/llm-plugins \
  -H "Content-Type: application/json" \
  -d @my-gateway.json
```

### 3. 在 Agent 配置中使用

```yaml
agents:
  - agentId: my-agent
    model:
      provider: "plugin:my-gateway"
      model: "gpt-4o"
```

### 4. 测试连接

```bash
curl -X POST http://localhost:3000/api/v1/llm-plugins/my-gateway/test
```

---

## 插件目录与文件格式

### 目录位置

默认：`~/.xiaolin/plugins/llm/`

可通过配置文件覆盖：

```yaml
llmPlugins:
  enabled: true
  pluginsDir: "/path/to/custom/plugins/llm"
```

### 全局开关

```yaml
llmPlugins:
  enabled: false   # 禁用所有 LLM 插件
```

### 通用字段

每个插件是一个独立的 JSON 文件（文件名任意，以 `.json` 结尾）：

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `id` | string | ✅ | 插件唯一标识，用于 `plugin:<id>` 引用 |
| `name` | string | ✅ | 显示名称 |
| `version` | string | | 版本号 |
| `description` | string | | 描述 |
| `type` | `"middleware"` \| `"process"` | ✅ | 插件模式 |
| `enabled` | boolean | | 默认 `true` |
| `middleware` | object | type=middleware 时 | 中间件配置 |
| `process` | object | type=process 时 | 进程配置 |
| `models` | array | | 此插件暴露的模型列表 |

### `models` 数组

```json
{
  "models": [
    {
      "id": "corp-gpt4",
      "name": "Corp GPT-4",
      "description": "企业版 GPT-4",
      "contextWindow": 128000
    }
  ]
}
```

模型列表会出现在前端模型选择器中，供 Agent 配置时选用。

---

## 中间件模式 (Middleware)

### 基础配置

```json
{
  "id": "example-mw",
  "name": "Example Middleware",
  "type": "middleware",
  "middleware": {
    "baseUrl": "https://api.example.com/v1",
    "protocol": "openai",
    "headers": {
      "x-custom-header": "value"
    },
    "auth": { "type": "none" },
    "modelMapping": {},
    "maxRetries": 3,
    "timeoutSecs": 300
  }
}
```

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `baseUrl` | string | ✅ 必填 | 上游 API 地址 |
| `protocol` | `"openai"` \| `"anthropic"` | `"openai"` | 上游使用的协议 |
| `headers` | object | `{}` | 附加到每个请求的静态 HTTP 头 |
| `auth` | object | `{"type":"none"}` | 认证配置 |
| `modelMapping` | object | `{}` | 模型名映射（见下文） |
| `maxRetries` | number | `3` | 最大重试次数 |
| `timeoutSecs` | number | `300` | 请求超时（秒） |

### 认证方式

#### 1. 无认证

```json
{ "type": "none" }
```

#### 2. Bearer Token

```json
{
  "type": "bearer_token",
  "token": "sk-your-api-key"
}
```

等效于 `Authorization: Bearer sk-your-api-key`。

#### 3. 自定义请求头

```json
{
  "type": "custom_header",
  "header": "x-api-key",
  "value": "your-secret-key"
}
```

适用于需要特定请求头名的鉴权方式。

#### 4. OAuth2 Client Credentials

```json
{
  "type": "oauth2_client_credentials",
  "tokenEndpoint": "https://auth.corp.example.com/oauth/token",
  "clientId": "your-client-id",
  "clientSecret": "your-client-secret",
  "scope": "llm:invoke",
  "tokenHeader": "Authorization",
  "tokenPrefix": "Bearer"
}
```

系统自动管理 token 生命周期：
- 首次请求时获取 access_token
- 根据 `expires_in` 缓存 token（提前 10% 刷新）
- token 过期后自动重新获取

| 字段 | 必填 | 默认值 | 说明 |
|------|------|--------|------|
| `tokenEndpoint` | ✅ | | OAuth2 token 端点 |
| `clientId` | ✅ | | Client ID |
| `clientSecret` | ✅ | | Client Secret |
| `scope` | | | 可选 scope |
| `tokenHeader` | | `"Authorization"` | 注入 token 的 HTTP 头名 |
| `tokenPrefix` | | `"Bearer"` | token 值前缀 |

#### 5. Pre-Request Hook

在每次 LLM 请求前，调用一个 HTTP 端点获取 token：

```json
{
  "type": "pre_request_hook",
  "url": "https://auth.internal/api/v1/token",
  "method": "POST",
  "body": { "grant_type": "client_credentials", "app": "xiaolin" },
  "headers": { "x-internal-key": "secret" },
  "extractPath": "data.accessToken",
  "tokenHeader": "Authorization",
  "tokenPrefix": "Bearer",
  "cacheTtlSecs": 300
}
```

| 字段 | 必填 | 默认值 | 说明 |
|------|------|--------|------|
| `url` | ✅ | | 认证端点 URL |
| `method` | | `"POST"` | HTTP 方法 |
| `body` | | | 请求体 JSON |
| `headers` | | `{}` | 认证请求的额外头 |
| `extractPath` | | `"access_token"` | 从响应 JSON 中提取 token 的点分路径 |
| `tokenHeader` | | `"Authorization"` | 注入 token 的头名 |
| `tokenPrefix` | | `"Bearer"` | token 前缀 |
| `cacheTtlSecs` | | `0` | token 缓存时间（秒），0 = 不缓存 |

`extractPath` 示例：若认证端点返回 `{"data": {"accessToken": "abc123"}}`，则设置 `extractPath` 为 `"data.accessToken"` 可提取到 `abc123`。

### 模型映射

当本地使用的模型名与上游不一致时，可配置映射：

```json
{
  "modelMapping": {
    "gpt-4o": "corp-gpt4-latest",
    "gpt-4o-mini": "corp-gpt4-mini-v2"
  }
}
```

Agent 配置 `model: "gpt-4o"` 时，实际发送给上游的 model 参数为 `corp-gpt4-latest`。未在映射表中的模型名原样传递。

---

## 外部进程模式 (Process)

### 配置

```json
{
  "id": "custom-llm",
  "name": "Custom LLM Provider",
  "type": "process",
  "process": {
    "command": "python3",
    "args": ["/path/to/provider.py"],
    "env": {
      "API_KEY": "secret",
      "API_URL": "https://custom-llm.example.com"
    },
    "transport": "stdio"
  },
  "models": [
    { "id": "custom-model-v1", "name": "Custom Model v1", "contextWindow": 32000 }
  ]
}
```

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `command` | string | ✅ 必填 | 可执行文件路径 |
| `args` | array | `[]` | 命令行参数 |
| `env` | object | `{}` | 环境变量 |
| `transport` | `"stdio"` | `"stdio"` | 通信方式 |

### stdio 协议规范

XiaoLin 通过 stdin 发送 JSON 请求（每行一个），进程通过 stdout 返回 JSON 响应（每行一个）。

#### 非流式请求

**XiaoLin → 进程 (stdin)**:

```json
{"method":"chat_completion","params":{"model":"custom-model-v1","messages":[{"role":"user","content":"Hello"}],"temperature":0.7,"max_tokens":1024}}
```

**进程 → XiaoLin (stdout)**:

```json
{"result":{"id":"resp-001","object":"chat.completion","created":1715234567,"model":"custom-model-v1","choices":[{"index":0,"message":{"role":"assistant","content":"Hi there!"},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":3,"total_tokens":8}}}
```

#### 错误响应

```json
{"error":{"message":"API rate limited","code":"rate_limit"}}
```

#### 响应格式

响应的 `result` 字段遵循 OpenAI Chat Completion 格式，包含：
- `id`: 响应 ID
- `object`: `"chat.completion"`
- `created`: Unix 时间戳
- `model`: 模型名
- `choices`: 包含 `index`, `message`, `finish_reason`
- `usage`: 包含 `prompt_tokens`, `completion_tokens`, `total_tokens`

### 示例实现

#### Python

```python
#!/usr/bin/env python3
"""Minimal LLM plugin process for XiaoLin."""

import json
import sys
import requests

API_URL = "https://custom-llm.example.com/v1/chat"
API_KEY = "your-key"

def handle_request(req):
    params = req["params"]
    resp = requests.post(API_URL, json={
        "model": params["model"],
        "messages": params["messages"],
        "temperature": params.get("temperature", 0.7),
        "max_tokens": params.get("max_tokens"),
    }, headers={"Authorization": f"Bearer {API_KEY}"})

    if resp.status_code != 200:
        return {"error": {"message": f"API error: {resp.status_code}", "code": "api_error"}}

    return {"result": resp.json()}

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        req = json.loads(line)
        result = handle_request(req)
        print(json.dumps(result), flush=True)
    except Exception as e:
        print(json.dumps({"error": {"message": str(e)}}), flush=True)
```

#### Node.js

```javascript
#!/usr/bin/env node
const readline = require('readline');

const rl = readline.createInterface({ input: process.stdin });

rl.on('line', async (line) => {
  try {
    const req = JSON.parse(line);
    const { params } = req;

    const resp = await fetch('https://custom-llm.example.com/v1/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'Authorization': 'Bearer your-key' },
      body: JSON.stringify({
        model: params.model,
        messages: params.messages,
        temperature: params.temperature,
        max_tokens: params.max_tokens,
      }),
    });

    const data = await resp.json();
    console.log(JSON.stringify({ result: data }));
  } catch (e) {
    console.log(JSON.stringify({ error: { message: e.message } }));
  }
});
```

---

## 前端管理

在 XiaoLin 设置面板中，「LLM 插件」标签页提供完整的可视化管理：

1. **查看已安装插件** — 显示名称、类型、状态、模型列表
2. **添加新插件** — 引导式表单，支持选择认证方式、配置模型
3. **编辑插件** — 修改已有插件配置
4. **测试连接** — 发送轻量级补全请求验证连通性
5. **删除插件** — 移除插件（同时删除配置文件）

---

## Agent 配置中使用插件

### 作为主 Provider

```yaml
agents:
  - agentId: corp-assistant
    model:
      provider: "plugin:corp-gateway"
      model: "corp-gpt4"
      temperature: 0.7
```

`provider` 字段使用 `plugin:` 前缀加上插件 ID。

### 作为 Fallback

```yaml
agents:
  - agentId: smart-agent
    model:
      provider: "openai"
      model: "gpt-4o"
      fallbacks:
        - provider: "plugin:corp-gateway"
          model: "corp-gpt4"
        - provider: "anthropic"
          model: "claude-sonnet-4-20250514"
```

插件 Provider 在 fallback 链中的行为与内置 Provider 完全一致。

---

## REST API 参考

### 列出所有插件

```
GET /api/v1/llm-plugins
```

响应：

```json
{
  "plugins": [
    {
      "id": "corp-gateway",
      "name": "Corporate Gateway",
      "version": "1.0.0",
      "type": "middleware",
      "enabled": true,
      "models": [...]
    }
  ],
  "count": 1
}
```

### 获取单个插件

```
GET /api/v1/llm-plugins/:id
```

### 创建插件

```
POST /api/v1/llm-plugins
Content-Type: application/json

{ ... 完整的插件配置 JSON ... }
```

### 更新插件

```
PUT /api/v1/llm-plugins/:id
Content-Type: application/json

{ ... 更新后的配置 ... }
```

### 删除插件

```
DELETE /api/v1/llm-plugins/:id
```

### 测试连接

```
POST /api/v1/llm-plugins/:id/test
```

响应：

```json
{
  "ok": true,
  "model": "corp-gpt4",
  "reply": "Hello"
}
```

或：

```json
{
  "ok": false,
  "error": "connection refused"
}
```

---

## 故障排除

### 插件未加载

- 确认文件位于正确目录（默认 `~/.xiaolin/plugins/llm/`）
- 确认文件扩展名为 `.json`
- 确认 JSON 格式正确（可使用 `jq . your-plugin.json` 验证）
- 检查 `id` 字段不为空
- 检查日志输出（`RUST_LOG=info`）中的 `loaded LLM provider plugin` 消息

### OAuth2 token 获取失败

- 确认 `tokenEndpoint` 可访问
- 确认 `clientId` 和 `clientSecret` 正确
- 查看日志中的 `OAuth2 token endpoint returned` 错误信息

### 进程插件启动失败

- 确认 `command` 路径正确且可执行
- 确认所需运行时已安装（如 Python、Node.js）
- 手动运行命令验证：`echo '{"method":"chat_completion","params":{"model":"test","messages":[{"role":"user","content":"hi"}],"temperature":0.7}}' | python3 provider.py`

### 模型在前端不显示

- 确认插件配置中的 `models` 数组非空
- 确认插件 `enabled: true`
- 刷新前端或重新打开设置面板

### 请求超时

- 调整 `timeoutSecs`（默认 300 秒）
- 检查上游 API 响应时间
- 确认网络连通性
