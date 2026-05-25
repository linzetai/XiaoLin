## Overview

`ClientOp` 是类型化的客户端操作枚举，替代当前 gateway 的字符串 method 分发。每个客户端可以发起的操作都是 `ClientOp` 的一个变体。

## Codex 参考

### Codex 的 Op 枚举（codex-rs/protocol/src/protocol.rs:404-777）

Codex 的 `Op` 是 actor mailbox 消息，包含 ~25 个变体：

```rust
pub enum Op {
    Interrupt,
    UserInput { items, environments, final_output_json_schema, ... },
    UserInputWithTurnContext { items, ..., cwd, approval_policy, model, ... },
    UserTurn { items, cwd, approval_policy, model, ... },
    ExecApproval { id, turn_id, decision: ReviewDecision },
    PatchApproval { id, decision },
    UserInputAnswer { id, response },
    RequestPermissionsResponse { id, response },
    DynamicToolResponse { id, response },
    Compact,
    ThreadRollback { num_turns },
    Shutdown,
    RunUserShellCommand { command },
    // ... realtime, MCP, inter-agent, config reload, review, guardian
}
```

**关键设计**：
- `#[non_exhaustive]` — `submission_loop` 的 match 有 `_ => false` 兜底
- 三种 user input 变体（`UserInput` / `UserInputWithTurnContext` / `UserTurn`）— 历史演进产物，我们可以统一为一个

### Codex 的 app-server ClientRequest（codex-rs/app-server-protocol/src/protocol/common.rs:435-1034）

外部 JSON-RPC 方法通过宏生成，每个方法有 params/response/serialization scope：

```rust
client_request_definitions! {
    ThreadStart { params: ThreadStartParams, response: ThreadStartResponse,
                  serialization: Global("threads") },
    TurnStart { params: TurnStartParams, response: TurnStartResponse,
                serialization: Thread { thread_id } },
    TurnInterrupt { params: TurnInterruptParams, response: (),
                    serialization: Thread { thread_id } },
    // ~80+ methods
}
```

**关键设计**：
- `ClientRequestSerializationScope` — per-scope 并发控制（Global、Thread、CommandExecProcess 等）
- `#[experimental("...")]` — 实验性 API 门控

### 我们的差异

1. 不需要 actor mailbox 语义 — gateway 直接路由到 AgentRuntime
2. 不需要 80+ 个 RPC 方法 — 我们的 agent 接口更紧凑
3. 需要 FastClaw 特色操作 — agent message bus、cron、channel

## Requirements

### CLOP-001: ClientOp 完整定义

```rust
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(tag = "op", rename_all = "snake_case")]
#[non_exhaustive]
#[ts(export)]
pub enum ClientOp {
    // === 对话（对标 Codex Op::UserInput / UserTurn） ===
    StartTurn {
        session_id: SessionId,
        agent_id: Option<AgentId>,
        messages: Vec<UserInput>,
        /// 可选的 per-turn 上下文覆盖
        #[serde(default)]
        turn_context: Option<TurnContextOverrides>,
    },
    /// 向运行中的 turn 注入输入（对标 Codex turn/steer）
    SteerTurn {
        session_id: SessionId,
        turn_id: TurnId,
        input: Vec<UserInput>,
    },
    /// 中断当前 turn（对标 Codex Op::Interrupt）
    InterruptTurn {
        session_id: SessionId,
        turn_id: TurnId,
    },

    // === 审批（对标 Codex Op::ExecApproval / PatchApproval） ===
    ResolveApproval {
        approval_id: SubmissionId,
        decision: ApprovalDecision,
    },
    /// 回答 ask_question 工具（对标 Codex Op::UserInputAnswer）
    AnswerQuestion {
        request_id: SubmissionId,
        session_id: SessionId,
        answer: String,
    },

    // === 会话管理（对标 Codex thread/* 系列） ===
    CreateSession {
        agent_id: AgentId,
        #[serde(default)]
        work_dir: Option<String>,
    },
    ResumeSession {
        session_id: SessionId,
    },
    ForkSession {
        session_id: SessionId,
        /// 从第 N 个 user message 处截断
        #[serde(default)]
        truncate_at: Option<usize>,
    },
    DeleteSession {
        session_id: SessionId,
    },
    ListSessions {
        #[serde(default)]
        agent_id: Option<AgentId>,
        #[serde(default)]
        limit: Option<usize>,
    },
    GetSession {
        session_id: SessionId,
    },

    // === 上下文管理（对标 Codex Op::Compact / ThreadRollback） ===
    CompactSession {
        session_id: SessionId,
    },
    RollbackTurns {
        session_id: SessionId,
        num_turns: u32,
    },

    // === 模式切换 ===
    SetExecutionMode {
        session_id: SessionId,
        mode: ExecutionMode,
    },

    // === 配置（对标 Codex config/* 系列） ===
    GetConfig,
    UpdateConfig {
        patch: serde_json::Value,
    },
    ReloadAgents,
    ReloadMcpServers,
    ReloadSkills,

    // === 工具 & MCP ===
    ListTools {
        #[serde(default)]
        agent_id: Option<AgentId>,
    },
    ListMcpServers,

    // === 技能 ===
    ListSkills {
        #[serde(default)]
        agent_id: Option<AgentId>,
    },

    // === 模型 ===
    ListModels,

    // === Agent ===
    ListAgents,
    GetAgent {
        agent_id: AgentId,
    },

    // === FastClaw 特色：多 agent 消息 ===
    SendAgentMessage {
        from: AgentId,
        to: MessageTarget,
        topic: String,
        payload: serde_json::Value,
    },

    // === FastClaw 特色：定时任务 ===
    ListCronJobs,
    CreateCronJob {
        schedule: String,
        agent_id: AgentId,
        prompt: String,
    },
    DeleteCronJob {
        job_id: String,
    },

    // === 系统 ===
    Ping,
    Subscribe {
        topics: Vec<String>,
    },
    Unsubscribe {
        topics: Vec<String>,
    },
}
```

### CLOP-002: TurnContextOverrides

对标 Codex 的 `UserInputWithTurnContext` 中的 per-turn 覆盖字段：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TurnContextOverrides {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub work_dir: Option<String>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// 工具允许列表覆盖
    #[serde(default)]
    pub tools: Option<Vec<ToolDefinition>>,
}
```

### CLOP-003: UserInput 类型

对标 Codex 的 `UserInput` 枚举（`codex-protocol/src/user_input.rs`）：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum UserInput {
    Text { text: String },
    Image { url: String },
    LocalImage { path: String },
}
```

### CLOP-004: Gateway 分发改造

当前 gateway 字符串分发（`fastclaw-gateway/src/ws/mod.rs:271-430`）：

```rust
// 当前：字符串匹配
match method.as_str() {
    "chat" => chat::spawn_chat(state, id, params, bg_tx).await,
    "chat.submit" => chat::handle_chat_submit(state, id, params, bg_tx).await,
    // ... 30+ 分支
}
```

改造为：

```rust
// 目标：类型化枚举匹配
match parse_client_op(raw_msg)? {
    ClientOp::StartTurn { session_id, agent_id, messages, turn_context } => {
        handle_start_turn(state, sub_id, session_id, agent_id, messages, turn_context, bg_tx).await
    }
    ClientOp::InterruptTurn { session_id, turn_id } => {
        handle_interrupt(state, sub_id, session_id, turn_id).await
    }
    // 编译器保证穷举
}
```

兼容层：

```rust
fn parse_client_op(raw: &[u8]) -> Result<(SubmissionId, ClientOp)> {
    // 尝试新格式
    if let Ok(envelope) = serde_json::from_slice::<Envelope<ClientOp>>(raw) {
        return Ok((envelope.id, envelope.payload));
    }
    // 回退旧格式
    let legacy: WsRequest = serde_json::from_slice(raw)?;
    let op = ClientOp::try_from_legacy(&legacy.method, &legacy.params)?;
    let id = legacy.id.unwrap_or_else(generate_submission_id);
    Ok((SubmissionId(id), op))
}
```

### CLOP-005: 旧方法到 ClientOp 的映射表

| 旧 method 字符串 | ClientOp 变体 | 参数映射 |
|-----------------|--------------|---------|
| `chat` | `StartTurn` | `params.messages` → `messages`; `params.agentId` → `agent_id`; `params.sessionId` → `session_id` |
| `chat.submit` | `StartTurn`（统一） | `params.message` → 单条 `messages`; 使用 `QueryEngine` session |
| `chat.cancel` | `InterruptTurn` | `params.sessionId` → `session_id` |
| `chat.answer` | `AnswerQuestion` | `params.requestId` → `request_id`; `params.answer` → `answer` |
| `chat.set_mode` | `SetExecutionMode` | `params.mode` → `mode` |
| `sessions.list` | `ListSessions` | 直接映射 |
| `sessions.get` | `GetSession` | `params.id` → `session_id` |
| `sessions.delete` | `DeleteSession` | `params.id` → `session_id` |
| `config.get` | `GetConfig` | 无参数 |
| `config.update` | `UpdateConfig` | `params` → `patch` |
| `agents.list` | `ListAgents` | 无参数 |
| `agents.get` | `GetAgent` | `params.id` → `agent_id` |
| `tools.list` | `ListTools` | `params.agentId` → `agent_id` |
| `models.list` | `ListModels` | 无参数 |
| `mcp.list` | `ListMcpServers` | 无参数 |
| `skills.list` | `ListSkills` | `params.agentId` → `agent_id` |
| `ping` | `Ping` | 无参数 |
| `subscribe` | `Subscribe` | `params.topics` → `topics` |
| `unsubscribe` | `Unsubscribe` | `params.topics` → `topics` |

## 门禁

| 检查项 | 验证方式 | 阻断条件 |
|--------|---------|---------|
| ClientOp 枚举穷举 | gateway 的 match 无 `_ =>` 兜底（除 `#[non_exhaustive]` 的 unreachable） | 编译警告 |
| 旧方法全覆盖 | 单元测试：每个旧 method 字符串都能 `TryFrom` 转换成功 | 任何转换失败 |
| 新格式往返 | 单元测试：每个 ClientOp 变体序列化→反序列化一致 | 任何不一致 |
| TypeScript 同步 | `ClientOp` 的 ts-rs 生成文件与前端 import 编译通过 | TS 编译失败 |
