## Overview

`AgentEvent` 是可序列化的运行时事件枚举，替代当前的 `StreamEvent`。agent 循环发射 `AgentEvent`，gateway 直接序列化转发，无需手动映射。

## Codex 参考

### Codex 的 EventMsg（codex-rs/protocol/src/protocol.rs:1258-1460）

Codex 的 `EventMsg` 有 70+ 个变体，包含大量 legacy（v1 和 v2 共存）：

```rust
pub enum EventMsg {
    // 生命周期
    TurnStarted(TurnStartedEvent),      // serde: "task_started"
    TurnComplete(TurnCompleteEvent),     // serde: "task_complete"
    TurnAborted(TurnAbortedEvent),
    ShutdownComplete,
    
    // 内容流（legacy flat events）
    AgentMessage(AgentMessageEvent),
    AgentReasoning(AgentReasoningEvent),
    AgentMessageContentDelta(AgentMessageContentDeltaEvent),
    
    // 内容流（v2 item model）
    ItemStarted(ItemStartedEvent),
    ItemCompleted(ItemCompletedEvent),
    
    // 工具
    ExecCommandBegin(ExecCommandBeginEvent),
    ExecCommandOutputDelta(ExecCommandOutputDeltaEvent),
    ExecCommandEnd(ExecCommandEndEvent),
    PatchApplyBegin/Updated/End(...),
    McpToolCallBegin/End(...),
    
    // 审批
    ExecApprovalRequest(ExecApprovalRequestEvent),
    ApplyPatchApprovalRequest(...),
    GuardianAssessment(GuardianAssessmentEvent),
    
    // ... 还有 realtime, collab, hook, review, MCP startup, etc.
}
```

**问题**：legacy 累积导致一个枚举承载了太多职责。Codex 正在用 `TurnItem` + `ItemStarted/ItemCompleted` 逐步替代 flat events，但两套并存增加了复杂性。

### Codex 的 TurnItem（codex-rs/protocol/src/items.rs:42-54）

```rust
pub enum TurnItem {
    UserMessage(UserMessageItem),
    HookPrompt(HookPromptItem),
    AgentMessage(AgentMessageItem),
    Plan(PlanItem),
    Reasoning(ReasoningItem),
    WebSearch(WebSearchItem),
    ImageView(ImageViewItem),
    ImageGeneration(ImageGenerationItem),
    FileChange(FileChangeItem),
    McpToolCall(McpToolCallItem),
    ContextCompaction(ContextCompactionItem),
}
```

每个 item 有 `as_legacy_events()` 方法投影到 flat `EventMsg`。

### 我们的优势

从零设计，不需要 legacy 桥接。直接采用 item-centric 模型，flat events 作为 item 的 delta 子集。

## Requirements

### AGET-001: AgentEvent 完整定义

```rust
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
#[ts(export)]
pub enum AgentEvent {
    // === Turn 生命周期（对标 Codex TurnStarted/TurnComplete/TurnAborted） ===
    TurnStarted {
        turn_id: TurnId,
        session_id: SessionId,
        agent_id: AgentId,
        model: String,
    },
    TurnComplete {
        turn_id: TurnId,
        session_id: SessionId,
        summary: TurnSummary,
    },
    TurnAborted {
        turn_id: TurnId,
        session_id: SessionId,
        reason: String,
    },

    // === 内容流（对标 Codex AgentMessageContentDelta + ReasoningContentDelta） ===
    Delta {
        turn_id: TurnId,
        delta: StreamDelta,
    },
    MessageComplete {
        turn_id: TurnId,
        content: String,
        #[serde(default)]
        reasoning_content: Option<String>,
    },

    // === 工具生命周期（对标 Codex ExecCommandBegin/OutputDelta/End 等） ===
    ToolExecuting {
        turn_id: TurnId,
        tool_name: String,
        call_id: String,
        #[serde(default)]
        args: Option<String>,
    },
    ToolProgress {
        turn_id: TurnId,
        tool_name: String,
        call_id: String,
        message: String,
        #[serde(default)]
        progress: Option<f64>,
        #[serde(default)]
        partial_output: Option<String>,
    },
    ToolResult {
        turn_id: TurnId,
        tool_name: String,
        call_id: String,
        output: String,
        #[serde(default)]
        display_output: Option<String>,
        success: bool,
        #[serde(default)]
        metadata: Option<serde_json::Value>,
    },

    // === 审批（对标 Codex ExecApprovalRequest + GuardianAssessment） ===
    ApprovalRequired {
        turn_id: TurnId,
        approval_id: SubmissionId,
        action: PendingAction,
    },
    ApprovalResolved {
        turn_id: TurnId,
        approval_id: SubmissionId,
        decision: ApprovalDecision,
    },
    /// ask_question 工具（对标 Codex RequestUserInput）
    AskQuestion {
        turn_id: TurnId,
        request_id: SubmissionId,
        question: String,
        options: Vec<AskQuestionOption>,
        timeout_secs: u32,
        #[serde(default)]
        allow_multiple: bool,
    },

    // === 上下文管理（对标 Codex ContextCompacted + TokenCount） ===
    ContextUsage {
        turn_id: TurnId,
        used_tokens: u32,
        limit_tokens: u32,
        #[serde(default)]
        compressed: bool,
        #[serde(default)]
        tokens_saved: u32,
    },
    ContextWarning {
        turn_id: TurnId,
        used_tokens: u32,
        limit_tokens: u32,
        message: String,
        level: ContextWarningLevel,
    },
    CompactBoundary {
        turn_id: TurnId,
        trigger: CompactTrigger,
        pre_tokens: usize,
        post_tokens: usize,
        messages_removed: usize,
    },

    // === 模式（FastClaw 特色） ===
    ModeChange {
        turn_id: TurnId,
        from: ExecutionMode,
        to: ExecutionMode,
    },
    PlanUpdate {
        turn_id: TurnId,
        session_id: SessionId,
        path: String,
        exists: bool,
    },

    // === 子 agent（FastClaw 特色，对标 Codex Collab*Begin/End） ===
    SubAgentSpawned {
        turn_id: TurnId,
        run_id: String,
        agent_id: AgentId,
        subagent_type: String,
        task: String,
        depth: u32,
    },
    SubAgentDelta {
        turn_id: TurnId,
        run_id: String,
        content: String,
    },
    SubAgentToolExecuting {
        turn_id: TurnId,
        run_id: String,
        tool_name: String,
        call_id: String,
        #[serde(default)]
        args: Option<String>,
    },
    SubAgentToolResult {
        turn_id: TurnId,
        run_id: String,
        tool_name: String,
        call_id: String,
        output: String,
        success: bool,
    },
    SubAgentComplete {
        turn_id: TurnId,
        run_id: String,
        status: String,
        #[serde(default)]
        result: Option<String>,
        tool_calls_made: u32,
        iterations: u32,
        #[serde(default)]
        usage: Option<TokenUsage>,
        elapsed_ms: u64,
    },

    // === 记忆（FastClaw 特色，Codex 无） ===
    MemoryRecalled {
        turn_id: TurnId,
        memories: Vec<MemoryFragment>,
    },
    MemoryStored {
        turn_id: TurnId,
        memory_id: String,
        summary: String,
    },

    // === 简报（FastClaw 特色） ===
    BriefMessage {
        turn_id: TurnId,
        content: String,
        #[serde(default)]
        attachments: Vec<String>,
        mode: String,
    },

    // === 建议（FastClaw 特色） ===
    Suggestions {
        turn_id: TurnId,
        items: Vec<String>,
    },

    // === 错误 ===
    Error {
        #[serde(default)]
        turn_id: Option<TurnId>,
        message: String,
        #[serde(default)]
        error_type: Option<String>,
    },
}
```

### AGET-002: ContextWarningLevel

```rust
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ContextWarningLevel {
    Info,       // 85%
    Warning,    // 90%
    Critical,   // 95%
}
```

### AGET-003: TurnSummary

```rust
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TurnSummary {
    pub tool_calls_made: u32,
    pub iterations: u32,
    #[serde(default)]
    pub usage: Option<TokenUsage>,
    pub elapsed_ms: u64,
    #[serde(default)]
    pub context_tokens: Option<u32>,
    #[serde(default)]
    pub context_window: Option<u32>,
    #[serde(default)]
    pub terminal_reason: Option<String>,
}
```

### AGET-004: 从 StreamEvent 到 AgentEvent 的迁移映射

| 当前 StreamEvent | AgentEvent 变体 | 变化说明 |
|-----------------|----------------|---------|
| `Delta(StreamDelta)` | `Delta { turn_id, delta }` | 新增 turn_id |
| `ToolExecuting { tool_name, call_id, args }` | `ToolExecuting { turn_id, tool_name, call_id, args }` | 新增 turn_id |
| `ToolResult { ... }` | `ToolResult { turn_id, ... }` | 新增 turn_id |
| `ToolProgress { ... }` | `ToolProgress { turn_id, ... }` | 新增 turn_id |
| `AskQuestion { ... }` | `AskQuestion { turn_id, request_id: SubmissionId, ... }` | request_id 改为 SubmissionId |
| `Done { ... }` | `TurnComplete { turn_id, session_id, summary }` | 拆分为独立的 TurnSummary |
| `Error(String)` | `Error { turn_id, message, error_type }` | 结构化 |
| `ModeChange { from, to }` | `ModeChange { turn_id, from, to }` | 新增 turn_id |
| `PlanFileUpdate { ... }` | `PlanUpdate { turn_id, ... }` | 新增 turn_id |
| `ContextLimitWarning { ... }` | `ContextWarning { turn_id, ..., level: Critical }` | 合并为单一变体+level |
| `CompactWarning { ... }` | `ContextWarning { turn_id, ..., level: Warning }` | 合并 |
| `ContextUsageUpdate { ... }` | `ContextUsage { turn_id, ... }` | 新增 turn_id |
| `CompactBoundary { ... }` | `CompactBoundary { turn_id, ... }` | 新增 turn_id |
| `SubAgentStart { ... }` | `SubAgentSpawned { turn_id, ... }` | 新增 turn_id |
| `SubAgentDelta { ... }` | `SubAgentDelta { turn_id, ... }` | 新增 turn_id |
| `SubAgentToolExecuting { ... }` | `SubAgentToolExecuting { turn_id, ... }` | 新增 turn_id |
| `SubAgentToolResult { ... }` | `SubAgentToolResult { turn_id, ... }` | 新增 turn_id |
| `SubAgentComplete { ... }` | `SubAgentComplete { turn_id, ... }` | 新增 turn_id |
| `BriefMessage { ... }` | `BriefMessage { turn_id, ... }` | 新增 turn_id |
| `Suggestions { items }` | `Suggestions { turn_id, items }` | 新增 turn_id |
| 无 | `TurnStarted { ... }` | 新增 |
| 无 | `TurnAborted { ... }` | 新增 |
| 无 | `ApprovalRequired { ... }` | 新增 |
| 无 | `ApprovalResolved { ... }` | 新增 |
| 无 | `MemoryRecalled { ... }` | 新增（FastClaw 特色） |
| 无 | `MemoryStored { ... }` | 新增（FastClaw 特色） |
| 无 | `MessageComplete { ... }` | 新增（标记消息结束） |

### AGET-005: Gateway 事件转发

当前 gateway 的 `event_to_response`（`fastclaw-gateway/src/ws/chat.rs`）手动映射每个 StreamEvent 变体到 `WsResponse.msg_type` 字符串。

改造后：

```rust
// 不再需要 event_to_response 函数
// AgentEvent 直接序列化
fn forward_event(event: &AgentEvent, sub_id: &SubmissionId) -> WsResponse {
    WsResponse {
        id: Some(sub_id.0.clone()),
        msg_type: "agent_event".to_string(),
        data: Some(serde_json::to_value(event).unwrap()),
        error: None,
    }
}
```

前端通过 `data.type` 字段（serde tag）区分事件类型，不再依赖 `WsResponse.msg_type`。

### AGET-006: HTTP SSE 统一

当前 HTTP SSE（`fastclaw-gateway/src/routes/chat.rs`）使用与 WS 不同的事件名。改造后统一：

```
event: agent_event
data: {"type":"delta","turn_id":"t-1","delta":{...}}

event: agent_event
data: {"type":"tool_executing","turn_id":"t-1","tool_name":"shell",...}

event: agent_event
data: {"type":"turn_complete","turn_id":"t-1","summary":{...}}
```

保留 OpenAI 兼容模式作为可选项（通过 `Accept` header 或 query param 切换）。

## 门禁

| 检查项 | 验证方式 | 阻断条件 |
|--------|---------|---------|
| 所有 StreamEvent 变体已映射 | 编译时：移除旧 StreamEvent 后无编译错误 | 编译失败 |
| AgentEvent 可序列化往返 | 单元测试：每个变体 serialize→deserialize 一致 | 任何不一致 |
| gateway 无手动映射 | grep `event_to_response` 函数已删除 | 函数仍存在 |
| WS 和 SSE 事件格式一致 | 集成测试：同一操作通过 WS 和 HTTP 返回的事件 JSON 结构一致 | 格式不一致 |
| turn_id 全覆盖 | AgentEvent 的所有变体（除 Error 外）都有 turn_id | 缺少 turn_id |
| 前端可消费 | Tauri app 前端编译通过且能正确渲染所有事件类型 | 编译/运行失败 |
