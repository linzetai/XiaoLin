## Overview

`fastclaw-protocol` 是零运行时依赖的纯协议类型定义 crate，承载所有跨层共享类型。它是整个 FastClaw 系统的类型契约层。

## Codex 参考

Codex 的 `codex-protocol` crate（`codex-rs/protocol/Cargo.toml`）包含 `serde`、`serde_json`、`ts-rs`、`schemars` 等序列化依赖，但也引入了 `tokio`、`reqwest`、`globset`、`landlock` 等运行时依赖。**我们做得更严格**：`fastclaw-protocol` 只允许序列化和代码生成依赖，绝不引入 IO/网络/文件系统库。

## Requirements

### PROTO-001: Crate 依赖约束

`fastclaw-protocol` 的 `Cargo.toml` **仅允许**以下依赖类别：

- 序列化：`serde`, `serde_json`, `serde_with`
- TypeScript 生成：`ts-rs`
- 错误处理：`thiserror`
- 工具：`uuid`（ID 生成）, `strum`/`strum_macros`（枚举工具）
- Schema：`schemars`（可选，用于 JSON Schema 导出）

**禁止**：`tokio`, `reqwest`, `sqlx`, `axum`, `hyper`, `globset`, `landlock`, `seccompiler` 等任何运行时/IO/平台相关依赖。

验证方式：CI 中运行 `cargo tree -p fastclaw-protocol` 检查无禁止依赖。

### PROTO-002: 模块结构

```
fastclaw-protocol/src/
├── lib.rs          # re-export 所有公开类型
├── id.rs           # SessionId, TurnId, SubmissionId, AgentId
├── envelope.rs     # Envelope<T> 传输信封
├── op.rs           # ClientOp 枚举
├── event.rs        # AgentEvent 枚举
├── history.rs      # HistoryItem 枚举（模型可见历史）
├── items.rs        # StreamItem 枚举（UI 投影）
├── message.rs      # ChatMessage, ContentPart, Role
├── tool_spec.rs    # ToolDefinition, ToolKind, ToolParameterSchema（仅规格，不含执行逻辑）
├── approval.rs     # ApprovalDecision, PendingAction, ApprovalPolicy
├── sandbox.rs      # PermissionProfile, FileSystemPolicy, NetworkPolicy
├── error.rs        # ProtocolError
├── usage.rs        # TokenUsage, TurnSummary
├── memory.rs       # MemoryFragment（FastClaw 特色）
├── subagent.rs     # SubAgentType, SubAgentStatus（FastClaw 特色）
├── channel.rs      # ChannelMessage（FastClaw 特色）
└── compat.rs       # 旧类型的 From/TryFrom 桥接
```

### PROTO-003: 所有公开类型必须 derive

所有 pub 类型必须 derive：
- `Debug`, `Clone`
- `Serialize`, `Deserialize`（serde）
- `TS`（ts-rs，带 `#[ts(export)]`）

枚举类型额外要求：
- `#[serde(tag = "type", rename_all = "snake_case")]`（tagged union）
- `#[non_exhaustive]`（前向兼容）

### PROTO-004: Re-export 兼容

`fastclaw-core` 通过以下方式保持兼容：

```rust
// fastclaw-core/src/types.rs
pub use fastclaw_protocol::{
    SessionId, TurnId, SubmissionId, AgentId,
    AgentEvent, ClientOp, HistoryItem, StreamItem,
    ChatMessage, Role, ContentPart,
    // ...
};

// 旧名称的类型别名（deprecated）
#[deprecated(note = "Use AgentEvent instead")]
pub type StreamEvent = fastclaw_protocol::AgentEvent;
```

## Codex 代码参考

### Codex 的 Submission/Event 信封模式

```rust
// codex-rs/protocol/src/protocol.rs:126-134
pub struct Submission {
    pub id: String,
    pub op: Op,
    pub trace: Option<W3cTraceContext>,
}

// codex-rs/protocol/src/protocol.rs:1258-1265
pub struct Event {
    pub id: String,
    pub msg: EventMsg,
}
```

### 我们的 Envelope 设计

```rust
// fastclaw-protocol/src/envelope.rs
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Envelope<T> {
    pub id: SubmissionId,
    #[serde(flatten)]
    pub payload: T,
}

pub type ClientRequest = Envelope<ClientOp>;
pub type AgentResponse = Envelope<AgentEvent>;
```

## 门禁

| 检查项 | 验证方式 | 阻断条件 |
|--------|---------|---------|
| 零运行时依赖 | `cargo tree -p fastclaw-protocol \| grep -E "tokio\|reqwest\|sqlx\|axum"` 返回空 | 任何匹配即阻断 |
| 所有类型可序列化 | `cargo test -p fastclaw-protocol` 含 serde 往返测试 | 任何序列化失败即阻断 |
| TypeScript 类型同步 | CI 运行 `cargo test -p fastclaw-protocol` 自动生成 `.ts` 并 diff | `.ts` 文件与 Rust 不一致即阻断 |
| 无 dead code | `cargo clippy -p fastclaw-protocol -- -D warnings` | 任何 warning 即阻断 |
| re-export 兼容 | `cargo check -p fastclaw-core` 无编译错误 | 编译失败即阻断 |
