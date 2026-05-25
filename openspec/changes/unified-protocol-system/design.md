## Architecture

### 三层协议分离

FastClaw 统一协议采用三层分离架构，每层有明确的职责边界：

```
Layer 3: StreamItem (UI 投影)
  ├── 面向前端渲染的结构化卡片
  ├── 从 AgentEvent 投影，可丢弃不影响 agent 逻辑
  └── 通过 ts-rs 自动生成 TypeScript 类型

Layer 2: AgentEvent (运行时事件) + ClientOp (客户端操作)
  ├── agent 循环发射的所有事件，可序列化
  ├── 客户端所有操作的类型化枚举
  ├── gateway 做 1:1 转发，不重新映射
  └── 可持久化为 rollout 日志

Layer 1: HistoryItem (模型历史)
  ├── 发给 LLM 的对话历史条目
  ├── compaction 的操作对象
  ├── resume/fork 的序列化单元
  └── 与 UI 事件完全解耦
```

**Codex 参考**: Codex 的 `codex-protocol` 包含 `Op`（提交队列）+ `EventMsg`（事件队列）+ `ResponseItem`（模型历史）+ `TurnItem`（UI 项）四层。我们的设计将 `Op`/`EventMsg` 合并为 `ClientOp`/`AgentEvent`（因为 FastClaw 的 gateway 模式不需要 Codex 的 actor 内部提交队列语义），`ResponseItem` 对应 `HistoryItem`，`TurnItem` 对应 `StreamItem`。

### Codex 架构对照

| Codex 概念 | Codex 位置 | FastClaw 对应 | 差异说明 |
|-----------|-----------|-------------|---------|
| `Op` 枚举 | `codex-protocol/src/protocol.rs:404-777` | `ClientOp` | Codex 的 Op 是 actor mailbox 消息；我们的 ClientOp 是 WS/HTTP 请求 |
| `Submission { id, op }` | `codex-protocol/src/protocol.rs:126-134` | `Envelope<ClientOp>` | 相同的 id 关联模式 |
| `EventMsg` 枚举 | `codex-protocol/src/protocol.rs:1258-1460` | `AgentEvent` | Codex 有 70+ 变体含大量 legacy；我们从干净状态开始 |
| `Event { id, msg }` | `codex-protocol/src/protocol.rs:1258-1265` | `Envelope<AgentEvent>` | 相同的 id 关联模式 |
| `ResponseItem` | `codex-protocol/src/models.rs:752-903` | `HistoryItem` | 模型可见历史，语义一致 |
| `TurnItem` | `codex-protocol/src/items.rs:42-54` | `StreamItem` | UI 消费的结构化项 |
| `ThreadItem` | `app-server-protocol` | 不需要 | Codex 的 ThreadItem 是 app-server 对 TurnItem 的再投影，我们的 gateway 直接转发 AgentEvent |
| `submission_loop` | `codex-core/src/session/handlers.rs:733-923` | Gateway WS handler + ClientOp dispatch | Codex 用 actor 模式；我们用 gateway 路由 |
| `ToolOrchestrator` | `codex-core/src/tools/orchestrator.rs` | 后续单独 change | 审批→沙箱→执行管线，本 change 不涉及 |
| `item_event_to_server_notification` | `app-server-protocol/event_mapping.rs` | 不需要 | AgentEvent 直接序列化，无需手动映射 |

### 数据流

```
Client (Tauri/CLI/HTTP/飞书/SDK)
    │
    │ Envelope<ClientOp>
    ▼
Gateway (WS handler)
    │ match op { ClientOp::StartTurn{..} => ... }
    │
    ├─► AgentRuntime.execute_stream(...)
    │     │
    │     │ mpsc::channel<AgentEvent>
    │     ▼
    │   Query Loop
    │     ├─ LLM stream → AgentEvent::Delta
    │     ├─ Tool exec  → AgentEvent::ToolExecuting / ToolResult
    │     ├─ Approval   → AgentEvent::ApprovalRequired
    │     ├─ Compact    → AgentEvent::CompactBoundary
    │     └─ Done       → AgentEvent::TurnComplete
    │
    │ AgentEvent (serde_json::to_value)
    ▼
WsResponse { id, type: "agent_event", data: AgentEvent }
    │
    │ (自动生成的 TypeScript 类型)
    ▼
Frontend (React)
    │ StreamItem 投影
    ▼
MessageStream / ToolCallCard / SubAgentCard
```

### 持久化双轨

| 轨道 | 内容 | 用途 | Codex 参考 |
|------|------|------|-----------|
| **History Store** | `HistoryItem` 序列 | LLM 上下文重建、resume/fork | `RolloutItem::ResponseItem` in `codex-protocol/src/protocol.rs:2785-2791` |
| **Event Log** | `AgentEvent` 序列（append-only） | Debug、replay、审计 | `RolloutItem::EventMsg` + rollout JSONL files |

Session 表保持现有结构但增加 `reasoning_content`、`compact_metadata` 列，消除持久化有损问题。

### ID 体系

| ID | 类型 | 生命周期 | 用途 |
|----|------|---------|------|
| `SessionId` | newtype `String` | 跨 turn 持续 | 会话标识、持久化键 |
| `TurnId` | newtype `String` | 单 turn | agent 循环标识、事件关联 |
| `SubmissionId` | newtype `String` | 单操作 | 请求-响应关联 |

关系：一个 `SessionId` 包含多个 `TurnId`；每个 `ClientOp` 提交产生一个 `SubmissionId`；如果该 Op 触发 agent 循环则关联一个 `TurnId`。所有 `AgentEvent` 携带 `turn_id` 和 `submission_id`。

**Codex 参考**: Codex 在 `Submission.id` 中使用提交 ID，`TurnStartedEvent` 中使用 turn ID，`SessionConfiguredEvent` 中使用 thread ID。我们的三级 ID 与此对齐。

### 向后兼容策略

Gateway 同时支持新旧格式：

```rust
// 新格式：类型化 ClientOp
{ "id": "sub-1", "op": "start_turn", "session_id": "...", "messages": [...] }

// 旧格式：字符串 method + params（兼容期）
{ "id": "1", "method": "chat", "params": { "messages": [...] } }
```

Gateway 先尝试解析为 `Envelope<ClientOp>`，失败则回退到旧格式并通过 `TryFrom<LegacyWsRequest>` 转换。兼容期为 2 个版本，之后移除旧格式支持。

AgentEvent 的兼容更简单：新格式直接序列化；旧 `StreamEvent` 通过 `From<AgentEvent>` 桥接（用于尚未迁移的内部消费者）。

## Key Decisions

| 决策 | 选择 | 理由 |
|------|------|------|
| 协议是否独立 crate | 是 | 协议是系统脊梁，不应依附于任何实现 crate |
| 是否分离模型历史与 UI 事件 | 是 | compaction 语义不同、安全边界不同、演化速度不同 |
| TypeScript 生成工具 | ts-rs | 轻量、编译时生成、无运行时开销 |
| 向后兼容策略 | 适配层 + 2 版本过渡期 | 不破坏现有前端和集成 |
| ID 类型 | newtype wrapper（非 type alias） | 编译时防止 ID 混用 |
| AgentEvent 是否需要 non_exhaustive | 是 | 前向兼容，新变体不 break 旧客户端 |

## Alternatives Considered

1. **在 fastclaw-core 中直接改造（方案 B）** — StreamEvent 加 Serialize + ClientOp 枚举，不新建 crate。优点是改动量小，缺点是 fastclaw-core 已经承载了 Tool trait、config、routing 等重逻辑，协议类型与实现逻辑耦合，长期维护成本高。
2. **完全对齐 Codex 三层（方案 A）** — 包含 actor mailbox 模式的 Submission Queue。优点是与 Codex 1:1 对齐，缺点是 FastClaw 的 gateway 模式不需要 actor 语义，过度工程化。
3. **只做 StreamEvent 序列化** — 最小改动。优点是 1 天可完成，缺点是不解决 ClientOp 类型安全和 ID 统一问题。
