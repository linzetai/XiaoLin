## Overview

通过 `ts-rs` 从 Rust 协议类型自动生成 TypeScript 类型定义文件，替代前端手写的平行类型，消除前后端类型不一致风险。

## Codex 参考

Codex 在 `codex-protocol` 和 `codex-app-server-protocol` 中使用 `ts-rs` 生成 TypeScript 类型：

```rust
// codex-rs/protocol/Cargo.toml
[dependencies]
ts-rs = { version = "10", features = ["serde-compat", "serde-json-impl"] }
```

Codex 的 app-server-protocol 还通过宏自动导出 JSON Schema 用于客户端校验。

### 当前 FastClaw 的问题

前端 `transport.ts`（680 行）手写了平行类型，容易与 Rust 不一致：

```typescript
// crates/fastclaw-app/src/lib/transport.ts:204-211
export interface SessionMessage {
  id: number;          // Rust 是 i64
  role: string;        // Rust 是 Role enum
  content: unknown;    // Rust 是 Option<serde_json::Value>
  name: string | null;
  toolCallId: string | null;
  createdAt: string;   // Rust 没有这个字段名（是 created_at）
}
```

这些差异在运行时才会暴露，增加调试成本。

## Requirements

### TSCG-001: ts-rs 配置

`fastclaw-protocol/Cargo.toml` 中配置 ts-rs：

```toml
[dependencies]
ts-rs = { version = "10", features = ["serde-compat", "serde-json-impl"] }

[package.metadata.ts-rs]
# 生成到前端 src 目录
export_to = "../../crates/fastclaw-app/src/lib/generated/"
```

### TSCG-002: 所有协议类型标注 `#[ts(export)]`

每个 pub 类型都标注：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TurnSummary { ... }

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum AgentEvent { ... }
```

### TSCG-003: 生成的文件结构

```
crates/fastclaw-app/src/lib/generated/
├── AgentEvent.ts
├── ClientOp.ts
├── HistoryItem.ts
├── StreamItem.ts
├── SessionId.ts
├── TurnId.ts
├── SubmissionId.ts
├── AgentId.ts
├── TurnSummary.ts
├── TokenUsage.ts
├── StreamDelta.ts
├── ApprovalDecision.ts
├── PendingAction.ts
├── ExecutionMode.ts
├── AskQuestionOption.ts
├── ToolDefinition.ts
├── ContentPart.ts
├── Role.ts
├── UserInput.ts
├── TurnContextOverrides.ts
├── MessageTarget.ts
├── MemoryFragment.ts
├── CompactTrigger.ts
├── ContextWarningLevel.ts
└── index.ts          # barrel export
```

### TSCG-004: 前端迁移

当前前端手写类型（`transport.ts`）逐步替换为 import 生成类型：

```typescript
// 当前
export interface ChatStreamEvent {
  type: string;
  data?: Record<string, unknown>;
}

// 改造后
import { AgentEvent } from './generated/AgentEvent';

// 直接使用生成的类型，有完整的 discriminated union 支持
function handleEvent(event: AgentEvent) {
    switch (event.type) {
        case 'delta':
            // TypeScript 自动推断 event.turn_id, event.delta
            break;
        case 'tool_executing':
            // TypeScript 自动推断 event.tool_name, event.call_id
            break;
        // 编译器保证穷举
    }
}
```

### TSCG-005: CI 同步检查

```bash
# CI 脚本
#!/bin/bash
set -e

# 1. 生成最新 TypeScript 类型
cargo test -p fastclaw-protocol export_bindings

# 2. 检查是否有未提交的变更
cd crates/fastclaw-app/src/lib/generated/
if ! git diff --quiet .; then
    echo "ERROR: Generated TypeScript types are out of sync!"
    echo "Run 'cargo test -p fastclaw-protocol export_bindings' and commit the changes."
    git diff .
    exit 1
fi

# 3. TypeScript 编译检查
cd ../../../
npm run type-check
```

### TSCG-006: 生成的类型质量要求

- 所有 Rust `Option<T>` 映射为 TypeScript `T | null`（不是 `T | undefined`）
- 所有枚举使用 discriminated union（通过 `#[serde(tag = "type")]`）
- 所有 `serde_json::Value` 映射为 `unknown`（不是 `any`）
- 所有日期/时间字段为 `string`（ISO 8601）
- 所有 newtype wrapper（`SessionId`、`TurnId`）映射为 branded type 或 plain `string`

### TSCG-007: transport.ts 瘦身

迁移完成后，`transport.ts` 应只保留：

1. WebSocket 连接管理逻辑
2. 重连/心跳逻辑
3. `send()` / `subscribe()` 方法
4. 错误处理

所有类型定义移除，改为 import 自 `generated/`。目标：从 680 行减少到 ~200 行。

## 门禁

| 检查项 | 验证方式 | 阻断条件 |
|--------|---------|---------|
| 生成文件与 Rust 同步 | CI：`cargo test export_bindings` + `git diff --quiet` | 有未提交的变更 |
| TypeScript 编译 | CI：`npm run type-check` | 编译失败 |
| 无手写平行类型 | `rg "export interface.*Session\|Chat\|Stream" transport.ts` 只返回连接相关类型 | 仍有业务类型定义 |
| 生成的枚举有 discriminated union | TypeScript 测试：switch-case 有类型推断 | 无法推断 |
| index.ts barrel export | `import { AgentEvent } from './generated'` 可用 | import 失败 |
