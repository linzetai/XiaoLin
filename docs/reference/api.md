---
title: REST API 参考
summary: FastClaw 网关 HTTP API 路径、方法与常见请求体说明。
---

# REST API 参考

默认基址：`http://127.0.0.1:18789`（端口以 `gateway.port` 为准）。若启用 **API Key**，请在网关实现所要求的头或参数中携带。

## 健康与指标

| 方法 | 路径 | 认证 | 说明 |
|------|------|------|------|
| GET | `/health` | 无需 | 存活探测 |
| GET | `/ready` | 无需 | 就绪（依赖项可用） |
| GET | `/metrics` | **需要** | Prometheus 文本指标 |
| GET | `/api/v1/metrics` | **需要** | 结构化指标（JSON） |

> **注意**：当配置了 `security.apiKeys` 时，`/metrics` 和 `/api/v1/metrics` 需携带有效 API Key。仅 `/health` 和 `/ready` 免认证。

## Chat API（SSE 流式）

| 方法 | 路径 | 说明 |
|------|------|------|
| POST | `/api/v1/chat` | OpenAI 风格 Chat Completions |
| POST | `/api/v1/chat/completions` | 同上别名 |

请求体为 `ChatRequest` JSON，核心字段：

- `model`：可为逻辑模型名或路由键，常与 `agent_id` 解析联动。
- `messages`：OpenAI 风格消息数组。
- `stream`：`true` 时响应 **`Content-Type: text/event-stream`**（SSE）；`false` 时返回完整 JSON。

## Agent 与工具

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/v1/agents` | 列出可用 Agent |
| GET | `/api/v1/tools` | 列出工具 schema |

## 会话 API

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/v1/sessions` | 列出会话 |
| GET | `/api/v1/sessions/:session_id` | 会话详情 |
| DELETE | `/api/v1/sessions/:session_id` | 删除会话 |
| GET | `/api/v1/sessions/:session_id/messages` | 分页消息 |

## Memory API

所有 Memory 端点均需认证。写操作（POST/DELETE）会生成审计日志。

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/v1/memory/episodes` | 情景列表 |
| GET | `/api/v1/memory/episodes/search` | 情景检索（关键词中 LIKE 通配符自动转义） |
| GET | `/api/v1/memory/facts` | 语义事实列表 |
| POST | `/api/v1/memory/facts` | 创建/更新事实 |
| GET | `/api/v1/memory/facts/search` | 语义搜索 |
| DELETE | `/api/v1/memory/facts/:fact_id` | 删除事实（生成审计日志） |

## Evolution API

| 方法 | 路径 | 说明 |
|------|------|------|
| POST | `/api/v1/evolution/feedback` | 提交反馈 |
| GET | `/api/v1/evolution/feedback/:agent_id` | 读取反馈聚合 |
| GET | `/api/v1/evolution/evaluate/:agent_id` | 评估 |
| POST | `/api/v1/evolution/distill/:agent_id` | 提示蒸馏 |
| GET | `/api/v1/evolution/candidates/:agent_id` | 列出候选 |
| POST | `/api/v1/evolution/candidates/:candidate_id/accept` | 接受候选 |
| POST | `/api/v1/evolution/candidates/:candidate_id/reject` | 拒绝候选 |

## DAG API

| 方法 | 路径 | 说明 |
|------|------|------|
| POST | `/api/v1/dag/validate` | 校验 DAG JSON |
| POST | `/api/v1/dag/execute` | 执行 DAG（code 节点仅允许 python/javascript/rust，代码上限 100KB） |

## 动态路由 API

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/v1/routes` | 列出动态路由 |
| POST | `/api/v1/routes` | 新增路由 |
| PUT | `/api/v1/routes/:id` | 更新路由 |
| DELETE | `/api/v1/routes/:id` | 删除路由 |

## 多 Agent 总线（Bus）

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/v1/bus/agents` | 可参与总线的 Agent |
| POST | `/api/v1/bus/send` | 发送单向消息 |
| POST | `/api/v1/bus/request` | 请求-应答 |

## 插件与其他

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/v1/plugins` | 列出 WASM 插件 |
| POST | `/api/v1/plugins/:plugin_id/invoke/:capability` | 调用插件能力 |
| GET | `/api/v1/channels` | 已注册渠道 |
| POST | `/webhook/:channel_id` | 渠道回调入口 |

## Cron

| 方法 | 路径 | 说明 |
|------|------|------|
| GET/POST | `/api/v1/cron/jobs` | 列出/创建或更新任务 |
| GET/DELETE | `/api/v1/cron/jobs/:job_id` | 获取/删除任务 |

## WebSocket

- `GET /ws`：实时事件与 TUI 协议（JSON-RPC 风格 `chat` 方法等）。

## 相关文档

- [CLI 参考](../cli/index.md)
- [DAG 工作流](../dag/index.md)
- [安全](../security/index.md)
