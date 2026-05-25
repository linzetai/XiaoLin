## Why

FastClaw 当前的协议层存在六个核心架构缺陷，严重制约了系统的可扩展性和可维护性：

1. **StreamEvent 不可序列化** — `StreamEvent` 仅有 `#[derive(Debug, Clone)]`，无 `Serialize/Deserialize`。每个传输层（WS `event_to_response`、HTTP SSE `handle_stream`）手动重新映射，同一语义产生不同字符串名（`chat.tool.start` vs `tool_executing`），没有编译时类型检查。
2. **无统一操作枚举** — 客户端请求通过 `method: String` + `params: serde_json::Value` 传递，gateway 用 30+ 个字符串 match 分发。Codex 用类型化的 `Op` 枚举实现编译器穷举检查，FastClaw 的字符串分发缺少这一保障。
3. **双重聊天路径** — `chat`（无状态，发完整消息列表）与 `chat.submit`（有状态，`QueryEngine` 维护历史）并存，语义重叠，前端需要理解两套协议。
4. **Session 持久化有损** — `SessionMessage` 是 `ChatMessage` 的有损投影，`reasoning_content`、`compact_metadata` 在持久化时丢失，导致 resume 后上下文信息不完整。
5. **前后端类型手动同步** — TypeScript 前端在 `transport.ts` 中手写了平行的接口定义（`SessionSummary`、`ChatStreamEvent` 等），与 Rust 类型容易不一致。
6. **三套 Session ID 概念** — `SessionId` 类型别名、`build_session_key()` 路由键、WS `owned_sessions` claim set 三个平行概念缺乏统一。

参考 OpenAI Codex 的三层协议架构（`codex-protocol` 内部协议 → `codex-core` 实现 → `codex-app-server-protocol` 外部 API），我们需要建立 FastClaw 自己的类型安全协议层，同时融入 Codex 没有的记忆/进化/多 agent/通道等 FastClaw 特色能力。

## What Changes

- **新增 `fastclaw-protocol` crate** — 零运行时依赖的纯协议类型定义 crate，包含 `ClientOp`（客户端操作枚举）、`AgentEvent`（运行时事件枚举）、`HistoryItem`（模型可见对话历史）、`StreamItem`（UI 消费的结构化项）、审批/沙箱/权限类型、三级 ID 体系（`SessionId`/`TurnId`/`SubmissionId`）。
- **改造 `fastclaw-core`** — 将协议相关类型（当前 `types.rs` 中的 `StreamEvent`、`ChatMessage`、`ChatRequest`、ID 类型等）迁移到 `fastclaw-protocol`，`fastclaw-core` 通过 re-export 保持向后兼容。
- **改造 `fastclaw-gateway` WS 层** — 将字符串 method 分发改为 `ClientOp` 枚举匹配；将 `event_to_response` 手动映射改为 `AgentEvent` 的 serde 序列化；统一 WS/HTTP 两个传输层的事件格式。
- **改造 `fastclaw-agent` 运行时** — `execute_stream` 改为发射 `AgentEvent`（替代当前 `StreamEvent`）；引入 `TurnId` 用于事件关联。
- **改造 `fastclaw-session`** — 持久化层支持 `HistoryItem`（无损保存 `reasoning_content`、`compact_metadata`）；新增 append-only 事件日志（rollout）用于 debug/replay。
- **新增 TypeScript 类型生成** — 通过 `ts-rs` 从 Rust 协议类型自动生成 `.d.ts` 文件，替代前端手写的平行类型。

## Capabilities

### New Capabilities

- `protocol-crate`: 独立协议 crate，零运行时依赖，承载所有跨层共享类型
- `client-op`: 类型化客户端操作枚举，替代字符串分发
- `agent-event`: 可序列化的运行时事件枚举，统一 WS/HTTP/SSE 事件格式
- `history-item`: 模型可见对话历史类型，与 UI 事件分离，支持无损持久化和 resume/fork
- `gateway-adapter`: Gateway 适配层改造，基于协议类型的类型安全分发
- `ts-codegen`: 编译时 TypeScript 类型生成，替代手写平行类型

### Modified Capabilities

- `fastclaw-core` 的类型导出路径变化（通过 re-export 保持兼容）
- `fastclaw-agent` 的事件发射接口变化（`StreamEvent` → `AgentEvent`）
- `fastclaw-session` 的持久化模型增强（无损保存）
- `fastclaw-gateway` 的 WS/HTTP 分发逻辑重构

## Impact

- **新增 crate**: `fastclaw-protocol` 加入 workspace
- **修改 crate**: `fastclaw-core`（类型迁移 + re-export）、`fastclaw-agent`（事件发射）、`fastclaw-gateway`（分发逻辑）、`fastclaw-session`（持久化增强）、`fastclaw-app`（前端类型生成）
- **新增依赖**: `ts-rs`（TypeScript 生成）加入 `fastclaw-protocol`
- **配置文件**: 无变化
- **API 变化**: WS 协议新增 `ClientOp` 格式（旧格式通过适配层兼容）；`AgentEvent` 的 serde 格式成为标准（替代手动映射的字符串类型名）
- **性能影响**: 消除手动映射开销；serde 序列化略有开销但可忽略
- **向后兼容**: Gateway 同时接受新旧 WS 格式（适配期 2 个版本）；旧 `StreamEvent` 通过 `From<AgentEvent>` 桥接
