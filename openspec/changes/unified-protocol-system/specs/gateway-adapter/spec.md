## Overview

Gateway 适配层改造，将 WS/HTTP 的字符串分发替换为基于 `ClientOp`/`AgentEvent` 的类型安全分发。

## Codex 参考

### Codex app-server 的桥接模式（codex-rs/app-server/src/request_processors/turn_processor.rs）

```rust
// Codex: JSON-RPC method → Op → submit to core
async fn submit_core_op(&self, request_id, thread, op: Op) -> CodexResult<String> {
    thread.submit_with_trace(op, self.request_trace_context(request_id).await).await
}
```

Codex 的 app-server 是 JSON-RPC → Op 的适配器：
1. 接收 `ClientRequest`（JSON-RPC with camelCase）
2. 转换为内部 `Op`
3. 提交到 `CodexThread`
4. 监听 `Event` 流
5. 映射为 `ServerNotification`（camelCase）推送给客户端

### Codex 事件映射（codex-rs/app-server-protocol/src/protocol/event_mapping.rs）

```rust
pub fn item_event_to_server_notification(msg: EventMsg, thread_id, turn_id) -> ServerNotification {
    match msg {
        EventMsg::ItemStarted(e) => ServerNotification::ItemStarted { ... },
        EventMsg::ExecCommandOutputDelta(e) => ServerNotification::CommandExecutionOutputDelta { ... },
        // ... ~20 个映射
    }
}
```

### 我们的优势

因为 `AgentEvent` 已经是可序列化的（Codex 的 `EventMsg` 需要通过 `item_event_to_server_notification` 映射到不同格式的 `ServerNotification`），我们的 gateway **不需要映射层**，直接序列化转发。

## Requirements

### GWAD-001: WS 处理器改造

当前架构（`fastclaw-gateway/src/ws/mod.rs`）：

```rust
// 当前：字符串匹配 + untyped params
async fn dispatch(state, method: &str, id: Option<String>, params: Value, bg_tx) {
    match method {
        "chat" => chat::spawn_chat(state, id, params, bg_tx).await,
        "sessions.list" => session::handle_sessions_list(state, id, params, bg_tx).await,
        // ... 30+ 分支
        _ => send_error(bg_tx, id, "unknown method"),
    }
}
```

改造为：

```rust
// 目标：类型化分发
async fn dispatch(state: &AppState, raw: &[u8], bg_tx: &WsSender) {
    let (sub_id, op) = parse_client_op(raw)?;
    
    match op {
        ClientOp::StartTurn { session_id, agent_id, messages, turn_context } => {
            let turn_id = TurnId::new();
            // 发送 TurnStarted
            send_event(&bg_tx, &sub_id, AgentEvent::TurnStarted {
                turn_id: turn_id.clone(),
                session_id: session_id.clone(),
                agent_id: agent_id.clone().unwrap_or_default(),
                model: "".to_string(), // runtime 填充
            }).await;
            // 启动 agent 循环
            spawn_agent_turn(state, sub_id, turn_id, session_id, agent_id, messages, turn_context, bg_tx.clone()).await;
        }
        ClientOp::InterruptTurn { session_id, turn_id } => {
            handle_interrupt(state, &sub_id, &session_id, &turn_id).await;
        }
        ClientOp::ResolveApproval { approval_id, decision } => {
            handle_approval(state, &sub_id, &approval_id, decision).await;
        }
        ClientOp::ListSessions { agent_id, limit } => {
            let sessions = state.store.session.list_sessions(agent_id.as_ref(), limit).await?;
            send_result(&bg_tx, &sub_id, serde_json::to_value(&sessions)?).await;
        }
        // ... 编译器保证穷举
    }
}
```

### GWAD-002: 事件转发简化

当前 `event_to_response`（`fastclaw-gateway/src/ws/chat.rs`，~100 行手动映射）：

```rust
// 当前：手动映射
fn event_to_response(event: &StreamEvent, id: &Option<String>) -> WsResponse {
    match event {
        StreamEvent::Delta(d) => WsResponse {
            msg_type: "chat.delta".to_string(),
            data: Some(serde_json::to_value(d).unwrap()),
            ..
        },
        StreamEvent::ToolExecuting { tool_name, call_id, args } => WsResponse {
            msg_type: "chat.tool.start".to_string(),
            data: Some(json!({ "toolName": tool_name, "callId": call_id, "args": args })),
            ..
        },
        // ... 每个变体都手动映射
    }
}
```

改造后：

```rust
// 目标：直接序列化
fn forward_agent_event(event: &AgentEvent, sub_id: &SubmissionId) -> WsResponse {
    WsResponse {
        id: Some(sub_id.as_str().to_string()),
        msg_type: "agent_event".to_string(),
        data: Some(serde_json::to_value(event).expect("AgentEvent is always serializable")),
        error: None,
    }
}
```

### GWAD-003: HTTP SSE 统一

当前 HTTP SSE（`fastclaw-gateway/src/routes/chat.rs`）：

```rust
// 当前：又一套手动映射
StreamEvent::ToolExecuting { tool_name, call_id, args } => {
    format!("event: tool\ndata: {}\n\n", json!({
        "type": "tool_executing",
        "toolName": tool_name,
        // ...
    }))
}
```

改造后：

```rust
// 目标：统一 event 格式
AgentEvent => {
    let json = serde_json::to_string(&event)?;
    format!("event: agent_event\ndata: {json}\n\n")
}
```

保留 OpenAI 兼容模式（通过 `format=openai` query param）用于第三方集成。

### GWAD-004: 连接初始化协议升级

当前连接初始化（`fastclaw-gateway/src/ws/mod.rs:96-105`）：

```rust
// 当前：发送方法列表
{
    "type": "connected",
    "data": { "protocol": "fastclaw-ws/1", "methods": [...] }
}
```

升级为：

```rust
// 目标：发送协议能力
{
    "type": "connected",
    "data": {
        "protocol": "fastclaw-ws/2",
        "capabilities": {
            "client_op": true,           // 支持 ClientOp 格式
            "legacy_methods": true,      // 仍支持旧格式（兼容期）
            "agent_event": true,         // 事件格式为 AgentEvent
            "ts_types_version": "0.1.0"  // 对应的 TS 类型版本
        }
    }
}
```

### GWAD-005: AgentRuntime 接口适配

当前 `AgentRuntime.execute_stream` 通过 `mpsc::channel<StreamEvent>` 发射事件。改造为 `mpsc::channel<AgentEvent>`：

```rust
// 当前
pub async fn execute_stream(
    &self,
    // ...
    stream_tx: tokio::sync::mpsc::Sender<StreamEvent>,
) -> anyhow::Result<(u32, u32)>;

// 改造后
pub async fn execute_stream(
    &self,
    // ...
    stream_tx: tokio::sync::mpsc::Sender<AgentEvent>,
    turn_id: TurnId,  // 新增：由 gateway 生成并传入
) -> anyhow::Result<TurnSummary>;
```

Gateway 生成 `TurnId`，传入 runtime，runtime 在每个 `AgentEvent` 中填充 `turn_id`。返回值从 `(u32, u32)` 改为 `TurnSummary`（结构化）。

## 门禁

| 检查项 | 验证方式 | 阻断条件 |
|--------|---------|---------|
| `event_to_response` 已删除 | `rg "event_to_response" crates/fastclaw-gateway/` 返回空 | 函数仍存在 |
| 旧格式兼容 | 集成测试：旧格式 WS 消息仍能正确处理 | 旧格式失败 |
| 新格式可用 | 集成测试：ClientOp 格式 WS 消息能正确处理 | 新格式失败 |
| WS/HTTP 事件一致 | 对比测试：同一操作 WS 和 HTTP 返回的 AgentEvent JSON 相同 | 不一致 |
| 所有旧方法有映射 | 旧方法覆盖率测试 | 有旧方法无法映射 |
| AgentRuntime 接口改造 | `cargo check -p fastclaw-agent` 编译通过 | 编译失败 |
| 无运行时 panic | gateway 启动 + 基本操作不 panic | panic |
