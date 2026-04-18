---
title: 配置字段参考
summary: FastClawConfig 及主要子结构的字段说明与注意事项。
---

# 配置字段参考

下列说明以 Rust 类型 `FastClawConfig`（`crates/fastclaw-core/src/config.rs`）为主，并与仓库示例 `config/default.json` 对齐。Serde 默认 **忽略** 未知顶层键；若需缺失 include 即失败，设置 `strictIncludes: true`。

## FastClawConfig（顶层）

| 字段 | 类型 | 说明 |
|------|------|------|
| `gateway` | `GatewayConfig` | 监听端口、绑定模式、连接数、速率限制、CORS |
| `logging` | `LoggingConfig` | `level`、`format`（如 `json`） |
| `session` | `SessionConfig` | TTL、`dmScope`、重置策略、`identityLinks` 等 |
| `memory` | `MemoryConfig` | 总开关、嵌入、`dreamingIntervalSecs` |
| `models` | `Map<String, ModelProviderConfig>` | 各 provider 别名下的模型与端点 |
| `security` | `SecurityConfig` | `apiKeys`、`promptInjectionDetection` |
| `channels` | `Map<String, ChannelConfig>` | 按渠道名（如 `feishu`）覆盖凭证与模式 |
| `agents` | `AgentsConfig` | `defaults` + `list[]` |
| `bindings` | `BindingConfig[]` | 入站到 `agentId` 的静态路由 |
| `workspace` | string? | 默认工作区路径 |
| `skills` | `SkillsConfig` | `promptMode`、`allow`、`deny` |
| `paths` | `PathsConfig` | `stateDir`、`db_path`、`plugins_dir`、`extensions_dir`、`skills_dir`、`agents_dir` 等路径覆盖 |
| `strictIncludes` | bool | 为 true 时缺失的 `$include` 文件报错 |
| `credentials` | `CredentialsConfig` | 嵌套 provider → `apiKey` / `baseUrl` |
| `webSearch` | `WebSearchConfig` | 搜索后端：`duckduckgo` / `tavily` / `searxng` 等 |
| `modelRouter` | `ModelRouterConfig` | `enabled`、`strategy`、`dailyBudget`（原子预算追踪，超支严格拒绝）、`fallbackChain` |
| `evolution` | `EvolutionRuntimeConfig` | 网关中技能抽取与维护任务周期（秒，`0` 关闭） |

> 示例 JSON 中可能出现的 `plugins`、`dag`、`metrics`、`meta` 等键，用于插件沙箱默认值、DAG 检查点与可观测性；若Serde 结构体未收录，可能以 **忽略或部分映射** 方式存在，升级内核前请对照当前版本源码与示例。

### `$include` 合并

顶层可设 `"$include": "fragments/prod.json"` 或数组形式，按路径相对主文件目录合并对象（深度 merge）。支持递归 include（被引入的文件中也可包含 `$include`），最大递归深度为 **8 层**，超出时报错防止循环引用。

---

## GatewayConfig

| 字段 | 说明 |
|------|------|
| `port` | HTTP 端口，默认 `18789` |
| `bind` | `loopback` / `lan` / `custom` |
| `customBindHost` | `bind=custom` 时的地址文本，支持 IPv4 和 IPv6（如 `::` 或 `0.0.0.0`）。无效地址会回退到 loopback 并输出警告 |
| `maxConnections` | 最大并发连接 |
| `rateLimit` | 见下表 |
| `corsOrigins` | 允许的 CORS 来源；`["*"]` 为开发宽松模式（生产环境应指定具体域名） |

### RateLimitCfg（`gateway.rateLimit`）

| 字段 | 说明 |
|------|------|
| `enabled` | 是否启用 |
| `maxRequests` | 窗口内最大请求数 |
| `windowSecs` | 窗口长度（秒） |
| `trustedProxies` | 可信代理 IP，用于解析真实客户端 |

---

## MemoryConfig

| 字段 | 说明 |
|------|------|
| `enabled` | 是否启用记忆子系统 |
| `embedding` | 见下文 `EmbeddingConfig` |
| `dreamingIntervalSecs` | 自动 dream 周期；`0` 关闭 |

### EmbeddingConfig（`memory.embedding`）

| 字段 | 说明 |
|------|------|
| `provider` | `local` / `remote` |
| `model` | 本地 HF ID 或远端模型名 |
| `baseUrl` / `apiKey` | 远端嵌入 API；密钥可来自 `credentials` |
| `dimensions` | 可选，覆盖自动维度 |

---

## SecurityConfig

| 字段 | 说明 |
|------|------|
| `apiKeys` | 合法 API Key 列表（HTTP 认证，启用后 `/metrics` 等端点同样受保护） |
| `promptInjectionDetection` | 是否启用提示注入检测 |

> **注意**：配置了 `apiKeys` 后，除 `/health` 和 `/ready` 外的所有端点（包括 `/metrics`）均需认证。

---

## ChannelConfig（`channels.<name>`）

| 字段 | 说明 |
|------|------|
| `enabled` | 是否启用该渠道 |
| `appId` / `appSecret` | 厂商应用凭证 |
| `verificationToken` / `encryptKey` | Webhook 校验与加密 |
| `agentId` | 默认落点 Agent（若渠道实现支持） |
| `connectionMode` | 如 `websocket` / 轮询等 |
| `domain` | API 域名，如飞书 `https://open.feishu.cn` |
| `replyMode` | 如 `mention_only` |
| `userAccessToken` | 用户 OAuth token，用于任务/文档等高权限 API |

---

## ModelProviderConfig（`models.<key>`）

| 字段 | 说明 |
|------|------|
| `provider` / `providerType`（别名） | 提供方类型字符串 |
| `model` / `defaultModel`（别名） | 默认模型名 |
| `baseUrl` | OpenAI 兼容端点 |
| `apiKey` | 可内联；推荐放 `credentials` |
| `temperature` / `maxTokens` | 采样与长度 |
| 其他键 | 保留在 `extra`（如 `maxConcurrent`、`timeoutSecs`）供运行时读取 |

---

## AgentsConfig / AgentEntry

见 [Agent 概念](../concepts/agents.md)。核心字段：`agents.list[].id`、`default`、`model`、`tools`、`skills`、`workspace`、`groupChat`、`identity`。

---

## BindingConfig

| 字段 | 说明 |
|------|------|
| `agentId` | 目标 Agent |
| `match` | `channel`、`accountId`、`peer`（`kind` + `id`） |

---

## SessionConfig（摘要）

包含 `ttlHours`、`dmScope`（`main` / `per-peer` / `per-channel-peer` / `per-account-channel-peer`）、`reset`（按小时或空闲清理）、`identityLinks`（跨渠道身份合并）。

> **会话隔离**：`dmScope` 决定 session key 的生成策略。群聊场景下始终包含群组标识（`peer_id`），`per-account-channel-peer` 模式额外包含 `account_id` 实现多租户隔离。SQLite 连接池默认启用 `PRAGMA foreign_keys = ON`。

---

## PathsConfig

覆盖状态目录、数据库、插件、扩展、技能与 Agent 目录等，便于多实例与容器挂载。

---

## EvolutionRuntimeConfig

| 字段 | 说明 |
|------|------|
| `skillExtractionIntervalSecs` | 轨迹扫描与技能候选抽取间隔 |
| `skillMaintenanceIntervalSecs` | 技能库晋升/退役维护间隔 |

---

## 相关文档

- [网关配置说明](./configuration.md)
- [REST API](../reference/api.md)
