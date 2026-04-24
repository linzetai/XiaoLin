---
title: 系统架构
summary: FastClaw 组件划分、crate 职责、端到端请求流与设计原则。
---

# 系统架构概览

## 总体视图

FastClaw 将 **渠道接入、会话与路由、Agent 运行时、工具执行、记忆与工作流** 解耦在多个 crate 中，由 **Gateway** 暴露 HTTP/WebSocket 与 Webhook，由 **CLI** 提供运维与本地 TUI，由 **桌面应用**（Tauri 2）将网关内嵌于进程提供零配置 GUI。核心配置使用 JSON5，加载路径兼容 OpenClaw 旧路径，便于迁移。桌面应用与 CLI 共享同一配置与数据目录。

## Crate 结构（摘要）

| Crate | 职责 |
|--------|------|
| `fastclaw-app` | **Tauri 2 桌面应用**：React 19 前端 + 内嵌网关，IPC 命令、系统托盘、LSP 资源 |
| `fastclaw-core` | 配置类型（`FastClawConfig`）、消息路由、工具注册、消息总线与类型、**配置 ACL**（读写白名单+脱敏） |
| `fastclaw-gateway` | Axum 路由、聊天 API、会话、记忆与 DAG 等 HTTP 接口、WebSocket、渠道 Webhook |
| `fastclaw-agent` | Agent 运行时、LLM Provider、内置工具装配、**LSP 会话管理**（`LspSessionManager`） |
| `fastclaw-session` | 会话持久化（SQLite WAL 等），支持 **`work_dir`** |
| `fastclaw-memory` | 工作 / 情景 / 语义记忆与嵌入检索 |
| `fastclaw-dag` | DAG 定义、校验、执行与检查点 |
| `fastclaw-plugin` | WASM 插件宿主与热更新监听 |
| `fastclaw-evolution` | 反馈、评估、提示蒸馏与技能候选 |
| `fastclaw-eval` | 评估框架 |
| `fastclaw-collab` | 多 Agent 委托、流水线、辩证、委员会与 CollabHub |
| `fastclaw-security` | API Key、速率限制等横切安全 |
| `fastclaw-observe` | 指标与结构化日志 |
| `fastclaw-model-router` | 模型路由、成本追踪、**`max_context_for_model`** 上下文窗口查询 |
| `fastclaw-context` | 上下文压缩、用户画像、**上下文窗口裁剪**（`fit_to_context_window`） |
| `fastclaw-self-iter` | 执行诊断与自动恢复指导（已集成至 gateway） |
| `fastclaw-cron` | 定时任务与 DAG 触发 |
| `fastclaw-cli` | 二进制入口、TUI、网关守护进程 |
| `extensions/*` | 各渠道原生扩展（飞书、Slack、Telegram 等） |

## 请求流：消息 → 渠道 → 路由 → Agent → LLM → 响应

1. **入站**：用户从 Feishu / Slack 等渠道发送消息，或由 TUI / HTTP 发起 `chat`。
2. **渠道层**：扩展将厂商协议转为内部统一消息；Webhook 经 `POST /webhook/:channel_id` 进入网关。
3. **路由**：根据 `bindings` 与动态路由规则选择目标 `agent_id`；群聊可结合 `@` 与 `mention_patterns`。
4. **会话**：解析 `dmScope`、身份链接与会话 ID，读写会话历史。
5. **Agent 运行时**：组装系统提示、工具列表、记忆注入；按需走模型路由。
6. **上下文窗口裁剪**：解析模型的有效 `contextWindow`（优先级：模型配置 > Agent 配置 > `ModelRouter::max_context_for_model` > 安全默认值），调用 `ContextEngine::fit_to_context_window` 确保总 token 不超限。裁剪策略：重要性压缩 → 滑动窗口兜底，永不丢弃系统消息与当前用户轮。
7. **LLM 与工具循环**：调用模型；若返回 tool_calls，则执行内置 / WASM / MCP 工具并将结果回灌。
8. **出站**：将最终文本（及卡片等）通过对应渠道 SDK 或 WebSocket 事件推回客户端。流式完成事件包含 `contextTokens` 与 `contextWindow` 供客户端展示上下文用量。

异步任务（DAG 执行、Cron、后台 evolution 扫描）与上述主路径共享 `AppState` 中的存储与总线能力。

## 关键设计原则

- **配置驱动**：Agent 列表、绑定、渠道、模型与记忆尽量落在 JSON5，便于 Git 管理与审计。
- **热更新与安全失败**：Agent 目录文件变更或 `SIGHUP` 触发重载；校验失败时保持上一版有效路由（原子回滚语义）。
- **可观测**：Prometheus 指标与健康检查端点便于挂载到现有运维栈。
- **扩展优先**：渠道以扩展 crate 交付；工具以 WASM 与 MCP 外接，减少改内核频率。
- **与 OpenClaw 对齐**：配置键名、会话语义与部分运维习惯保持兼容，降低迁移成本。

更多细节见 [技术设计](../design/technical-design.md) 与 [核心能力](../design/core-capabilities.md)。
