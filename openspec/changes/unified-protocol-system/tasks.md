## Tasks

### Phase 1: Protocol Crate 基础（PROTO）

- [x] **T1.1** 创建 `fastclaw-protocol` crate，配置 Cargo.toml（仅 serde + ts-rs + thiserror + uuid + strum 依赖）
  - 门禁：`cargo tree -p fastclaw-protocol | grep -E "tokio|reqwest|sqlx|axum"` 返回空
  - 文件：`crates/fastclaw-protocol/Cargo.toml`, `crates/fastclaw-protocol/src/lib.rs`
  - 关联 spec：`protocol-crate/spec.md` PROTO-001, PROTO-002

- [x] **T1.2** 实现 ID 类型体系（`SessionId`, `TurnId`, `SubmissionId`, `AgentId`）
  - 从 `fastclaw-core/src/types.rs` 迁移 `AgentId`，新增 `TurnId`, `SubmissionId` 为 newtype
  - 门禁：编译通过 + serde 往返测试 + ts-rs 生成
  - 文件：`crates/fastclaw-protocol/src/id.rs`

- [x] **T1.3** 实现 `Envelope<T>` 传输信封
  - 文件：`crates/fastclaw-protocol/src/envelope.rs`

- [~] **T1.4** 迁移基础共享类型（`Role`, `ContentPart`, `TokenUsage`, `ExecutionMode`, `CompactTrigger` 等）
  - 从 `fastclaw-core/src/types.rs` 迁移
  - 门禁：`fastclaw-core` 通过 re-export 编译通过
  - 文件：`crates/fastclaw-protocol/src/message.rs`, `crates/fastclaw-protocol/src/usage.rs`
  - **状态**：类型已在 protocol 中定义，但 fastclaw-core 中仍有重复定义，未 re-export

- [~] **T1.5** 迁移工具规格类型（`ToolDefinition`, `ToolKind`, `ToolParameterSchema`）到 `tool_spec.rs`
  - 仅规格，不含 `Tool` trait 和 `ToolRegistry`（留在 `fastclaw-core`）
  - 文件：`crates/fastclaw-protocol/src/tool_spec.rs`
  - **状态**：类型已在 protocol 中定义，但 fastclaw-core 中仍有重复定义

- [~] **T1.6** `fastclaw-core` 添加 `fastclaw-protocol` 依赖并 re-export 所有迁移的类型
  - ~~`StreamEvent` 加 `#[deprecated]` 类型别名指向 `AgentEvent`~~（已直接删除 StreamEvent）
  - 门禁：`cargo check --workspace` 全编译通过
  - 文件：`crates/fastclaw-core/Cargo.toml`, `crates/fastclaw-core/src/types.rs`
  - **状态**：依赖已添加，re-export 为 `pub use fastclaw_protocol as protocol`。但 types.rs 中仍有重复类型定义未清理

**Phase 1 门禁**: `cargo clippy --workspace -- -D warnings` 零警告 ✅（改动 crate 通过）

---

### Phase 2: AgentEvent 定义（AGET）

- [~] **T2.1** 实现 `AgentEvent` 枚举（全部变体 + `#[non_exhaustive]` + serde tag）
  - 对照 `StreamEvent` 的每个变体逐一映射（见 AGET-004 映射表）
  - ~~新增 `TurnStarted`, `TurnAborted`, `ApprovalRequired`, `ApprovalResolved`, `MemoryRecalled`, `MemoryStored`, `MessageComplete`~~
  - 门禁：每个变体有 serde 往返测试
  - 文件：`crates/fastclaw-protocol/src/event.rs`
  - **状态**：核心变体已实现（~25个），缺少 `TurnAborted`, `ApprovalRequired`, `ApprovalResolved`, `MessageComplete`

- [~] **T2.2** 实现 `TurnSummary`, `ContextWarningLevel`, `PendingAction`, `ApprovalDecision` 等辅助类型
  - 文件：`crates/fastclaw-protocol/src/approval.rs`, `crates/fastclaw-protocol/src/event.rs`
  - **状态**：`TurnSummary`, `ContextWarningLevel` 已实现。`PendingAction`, `ApprovalDecision` 未实现，`approval.rs` 不存在

- [x] ~~**T2.3** 实现 `AgentEvent` ↔ `StreamEvent` 双向转换~~ **[跳过 — 已直接删除 StreamEvent，无需兼容]**

- [ ] **T2.4** 实现 `StreamDelta` 的 Serialize/Deserialize（当前已有，确认迁移）
  - 文件：`crates/fastclaw-protocol/src/message.rs`
  - **状态**：`StreamDelta` 仍在 `fastclaw-core/src/types.rs`，未迁移

**Phase 2 门禁**: `cargo test -p fastclaw-protocol` 所有测试通过 ✅

---

### Phase 3: ClientOp 定义（CLOP）

- [~] **T3.1** 实现 `ClientOp` 枚举（全部变体 + `#[non_exhaustive]` + serde tag）
  - 参照 CLOP-001 完整定义
  - 门禁：每个变体有 serde 往返测试
  - 文件：`crates/fastclaw-protocol/src/op.rs`
  - **状态**：已实现 30+ 变体覆盖当前 WS 方法。但 spec 中的高级变体未实现：`StartTurn`, `SteerTurn`, `InterruptTurn`, `ResolveApproval`, `ForkSession`。部分变体 params 仍为 `serde_json::Value`（未强类型化）

- [~] **T3.2** 实现 `UserInput`, `TurnContextOverrides`, `MessageTarget` 等辅助类型
  - 文件：`crates/fastclaw-protocol/src/op.rs`
  - **状态**：`MessageTarget` 在 `message.rs` 中已实现。`UserInput`, `TurnContextOverrides` 未定义

- [x] **T3.3** ~~实现 `ClientOp::try_from_legacy(method: &str, params: &Value)` 旧格式转换~~ → 重命名为 `ClientOp::parse_request`
  - 覆盖所有 WS method
  - 门禁：每个方法有单元测试
  - 文件：`crates/fastclaw-protocol/src/op.rs`

**Phase 3 门禁**: `cargo test -p fastclaw-protocol` ✅

---

### Phase 4: HistoryItem + 持久化（HIST）

- [x] **T4.1** 实现 `HistoryItem` 枚举
  - 参照 HIST-001 定义
  - 文件：`crates/fastclaw-protocol/src/history.rs`

- [ ] **T4.2** 实现 `ChatMessage` ↔ `HistoryItem` 互转
  - 参照 HIST-004
  - 门禁：往返转换无信息丢失
  - **状态**：未实现。无转换逻辑

- [x] **T4.3** Session 数据库迁移：新增 `reasoning_content`, `compact_metadata_json` 列
  - 参照 HIST-005
  - 门禁：migration 成功 + 现有数据不受影响
  - 文件：`crates/fastclaw-session/src/models.rs`, `crates/fastclaw-session/src/store.rs`

- [~] **T4.4** 实现 EventLog（append-only 事件日志）
  - 参照 HIST-006
  - 门禁：1000 事件 append < 100ms
  - 文件：`crates/fastclaw-session/src/event_log.rs`
  - **状态**：代码已实现并导出，单元测试通过。但未接入生产运行时，性能基准未验证

**Phase 4 门禁**: 持久化无损验证测试通过 ✅ | EventLog 性能基准 ❌

---

### Phase 5: Gateway 适配层改造（GWAD）

- [x] **T5.1** 实现 `parse_client_op` 函数
  - 参照 GWAD-001
  - 文件：`crates/fastclaw-gateway/src/ws/mod.rs`
  - **状态**：gateway dispatch 中调用 `ClientOp::parse_request()`

- [x] **T5.2** 改造 WS dispatch 为 `ClientOp` 枚举 match
  - 替换 30+ 个字符串 match 分支
  - 文件：`crates/fastclaw-gateway/src/ws/mod.rs`
  - **状态**：已改为 `match op { ClientOp::* => ... }`。有 `_ =>` 兜底（因为 `#[non_exhaustive]`）

- [ ] **T5.3** 实现 `forward_agent_event` 替代 `event_to_response`
  - 参照 GWAD-002
  - 门禁：`event_to_response` 函数已删除
  - 文件：`crates/fastclaw-gateway/src/ws/chat.rs`
  - **状态**：`event_to_response` 仍存在并被使用。`forward_agent_event` 未实现。这是一个关键差距——当前 gateway 将 `AgentEvent` 翻译为 `chat.delta`/`chat.tool_executing` 等旧格式字符串

- [x] **T5.4** 改造 `AgentRuntime.execute_stream` 接口
  - `mpsc::channel<StreamEvent>` → `mpsc::channel<AgentEvent>` ✅
  - 返回值 → `TurnSummary` ✅
  - `turn_id` 在 `execute_stream_inner` 内部生成（非参数传入）
  - 文件：`crates/fastclaw-agent/src/runtime/mod.rs`

- [x] **T5.5** 改造 agent 循环内所有 `send_stream_event` 调用
  - 将 `StreamEvent::*` 替换为 `AgentEvent::*`（加入 turn_id）
  - 门禁：无 StreamEvent 引用 ✅
  - 文件：`crates/fastclaw-agent/src/runtime/mod.rs`, `tool_executor.rs`, `stream_engine.rs`

- [x] **T5.6** 改造 HTTP SSE 路径
  - 参照 GWAD-003
  - 文件：`crates/fastclaw-gateway/src/routes/chat.rs`

- [~] **T5.7** 升级连接初始化协议
  - 参照 GWAD-004
  - 文件：`crates/fastclaw-gateway/src/ws/mod.rs`
  - **状态**：仍发送 `"protocol": "fastclaw-ws/1"` + 平面 `methods` 数组。未升级到 `fastclaw-ws/2` + `capabilities` 对象

**Phase 5 门禁**: `cargo clippy` 零警告 ✅ | Gateway 启动不 panic ✅ | WS/HTTP 集成测试通过 ✅

---

### Phase 6: TypeScript 类型生成（TSCG）

- [~] **T6.1** 配置 ts-rs 生成目标路径
  - 文件：`crates/fastclaw-protocol/Cargo.toml`
  - **状态**：`ts-rs` 作为 optional dep 已配置，但无 `[package.metadata.ts-rs]`，Rust 类型无 `#[derive(TS)]`

- [ ] **T6.2** 实现 `export_bindings` 测试用于 CI 生成
  - 文件：`crates/fastclaw-protocol/tests/export_bindings.rs`
  - **状态**：文件不存在

- [~] **T6.3** 生成所有 TypeScript 类型文件 + index.ts barrel export
  - 门禁：`npm run type-check` 编译通过
  - **状态**：`crates/fastclaw-protocol/generated/protocol.ts` 存在（手写/半生成）。但无自动化生成，变体名与 Rust 有漂移

- [~] **T6.4** 迁移前端 `transport.ts`：删除手写类型，import 生成类型
  - 参照 TSCG-004, TSCG-007
  - 文件：`crates/fastclaw-app/src/lib/transport.ts`
  - **状态**：已 import 生成类型，但仍有手写 `ChatStreamEvent` 接口和类型强转

- [ ] **T6.5** 迁移前端事件处理：使用 discriminated union switch-case
  - 修改 `useMessageStreamChat.ts` 等消费 event 的文件
  - 门禁：所有事件类型有类型推断
  - **状态**：switch 中使用的事件名过时（`chat.complete` vs 实际 `chat.done` 等）。未使用 `AgentEvent` discriminated union

- [ ] **T6.6** CI 同步检查脚本
  - 参照 TSCG-005
  - 文件：`.github/workflows/` 或 CI 配置
  - **状态**：CI 中无 TS 类型同步检查

**Phase 6 门禁**: ❌ 大部分未完成

---

### Phase 7: 回归验证 & Codex 对齐检查

- [~] **T7.1** 端到端回归测试
  - Tauri 桌面应用：WS 连接 → 发消息 → 工具调用 → 收到结果
  - CLI TUI：同上
  - HTTP API：SSE 流式响应
  - **状态**：Gateway E2E 测试 20/22 通过（2 个已有失败）。Tauri/CLI 路径未自动验证

- [x] ~~**T7.2** 旧格式兼容测试~~ **[跳过 — 已全部迁移新格式，不保留兼容]**

- [ ] **T7.3** Codex 对齐检查表
  - [ ] 类型化操作枚举：ClientOp vs Codex Op — **部分对齐**（缺 StartTurn/SteerTurn/ForkSession 等高级 Op）
  - [x] 事件枚举：AgentEvent vs Codex EventMsg — **核心事件已覆盖**，但 Codex 有 70+ 变体 vs FastClaw 25
  - [ ] 模型历史：HistoryItem vs Codex ResponseItem — **已定义但未使用**
  - [x] ID 关联：SubmissionId/TurnId vs Codex Submission.id — **类型已定义**
  - [x] 前向兼容：`#[non_exhaustive]` + `#[serde(other)]` — ✅
  - [ ] 权限模型：ApprovalDecision vs Codex ReviewDecision — **未实现**
  - [ ] 事件映射无损：~~删除 event_to_response 后 WS 功能完整~~ — **event_to_response 仍存在**

- [ ] **T7.4** 性能基准
  - AgentEvent 序列化延迟 < 1ms（p99）
  - EventLog append 吞吐 > 10000 events/s
  - WS 消息处理延迟不退化
  - **状态**：未实现

- [~] **T7.5** Dead code 清理
  - ~~删除旧 `StreamEvent`~~ ✅ 已删除
  - `cargo clippy` 零警告（改动 crate）✅
  - **状态**：`compat.rs` 已删除。core 中仍有重复类型定义（Role, ExecutionMode 等）

**Phase 7 门禁**: 部分通过

---

## Codex 架构对比 — 关键差距

### 已对齐 ✅
| Codex 概念 | FastClaw 等价物 |
|-----------|----------------|
| `Op` 操作枚举 | `ClientOp`（覆盖当前所有 WS 方法）|
| `EventMsg` 事件枚举 | `AgentEvent`（核心事件覆盖）|
| `Submission { id, op }` 信封 | `Envelope<ClientOp>`（已定义）|
| `Event { id, msg }` 信封 | `Envelope<AgentEvent>`（已定义）|
| `ResponseItem` 历史 | `HistoryItem`（已定义）|
| 强类型 ID | `SessionId`, `TurnId`, `SubmissionId`, `AgentId` |
| `#[non_exhaustive]` 前向兼容 | ✅ |

### 未对齐 ❌ — 架构层面
| Codex 能力 | FastClaw 状态 | 优先级 |
|-----------|-------------|-------|
| Submission Queue（有序异步 Op 处理）| 无 — 直接同步 dispatch | 中 |
| Session Actor 模型 | 无 — Gateway 直接处理 | 中 |
| ToolOrchestrator（审批→沙箱→执行→升级）| 仅有 confirm 工具 | **高** |
| Guardian 自动审查 | 无 | 高 |
| TurnContext 持久化（resume/fork）| 无 | 中 |
| TurnItem / StreamItem UI 投影层 | 无 | 中 |
| Envelope 端到端使用 | 已定义未接入 | 低 |
| RolloutItem JSONL 持久化 | EventLog 已定义未接入 | 低 |
| 前端直接消费 AgentEvent | 仍经 event_to_response 翻译 | **高** |
| TS 类型自动生成 CI 管线 | 手写 protocol.ts | 中 |
| ApprovalDecision / PendingAction | 未定义 | 高 |
| Mid-turn steer input | 无 | 低 |
| Thread rollback / fork | 无 | 低 |
| W3C trace propagation | 无 | 低 |

### FastClaw 独有优势 ✅
- 多通道 Gateway（WS + HTTP SSE + 飞书）
- `MemoryStored` / `MemoryRecalled` 事件
- `BriefMessage`, `Suggestions`, `PlanFileUpdate` 事件
- 简化的 `ExecutionMode`（agent/plan）

---

## 实施顺序与依赖关系

```
Phase 1 (Protocol Crate) ──► Phase 2 (AgentEvent) ──► Phase 5 (Gateway)
                          ├─► Phase 3 (ClientOp)   ──► Phase 5 (Gateway)
                          └─► Phase 4 (HistoryItem) ──► Phase 5 (Gateway)
                                                        ──► Phase 6 (TS Codegen)
                                                        ──► Phase 7 (Validation)
```

Phase 2/3/4 可并行开发（都只依赖 Phase 1），Phase 5 依赖 2/3/4 全部完成，Phase 6 依赖 5，Phase 7 依赖 6。

## 总体门禁清单

| 阶段 | 门禁 | 阻断级别 |
|------|------|---------|
| 每个 Phase | `cargo clippy --workspace -- -D warnings` | 硬阻断 |
| 每个 Phase | `cargo test --workspace` | 硬阻断 |
| Phase 5 后 | Gateway 启动不 panic | 硬阻断 |
| Phase 6 后 | `npm run type-check` | 硬阻断 |
| Phase 7 | 端到端回归 | 硬阻断 |
| Phase 7 | Codex 对齐检查表 100% | 软阻断（允许记录差异并说明理由） |
| Phase 7 | 性能不退化 | 硬阻断 |
