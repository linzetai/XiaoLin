## Overview

`HistoryItem` 是模型可见的对话历史类型，与 UI 事件（`AgentEvent`）完全分离。它是 LLM 上下文的序列化单元，也是 compaction、resume、fork 的操作对象。

## Codex 参考

### Codex 的 ResponseItem（codex-rs/protocol/src/models.rs:752-903）

```rust
pub enum ResponseItem {
    Message { id, role, content: Vec<ContentItem>, phase: Option<MessagePhase> },
    Reasoning { id, summary, content, encrypted_content },
    LocalShellCall { id, call_id, status, action: LocalShellAction },
    FunctionCall { id, name, namespace, arguments: String, call_id },
    FunctionCallOutput { call_id, output: FunctionCallOutputPayload },
    // ... ToolSearchCall, CustomToolCall, WebSearchCall, ImageGenerationCall
    Compaction { encrypted_content },
    CompactionTrigger,
    ContextCompaction { encrypted_content },
    #[serde(other)] Other,
}
```

**关键设计**：
- `FunctionCall.arguments` 保持为 `String`（JSON 字符串），不做 eager parse
- `MessagePhase`（Commentary / FinalAnswer）区分中间推理和最终输出
- `Compaction` 变体标记压缩后的内容替换点
- `#[serde(other)] Other` 前向兼容未知变体

### Codex 的 RolloutItem（codex-rs/protocol/src/protocol.rs:2785-2791）

```rust
pub enum RolloutItem {
    SessionMeta(SessionMetaLine),
    ResponseItem(ResponseItem),
    Compacted(CompactedItem),
    TurnContext(TurnContextItem),
    EventMsg(EventMsg),
}
```

Rollout 文件是 append-only JSONL，包含 model 历史 + UI 事件 + turn 上下文。

### 当前 FastClaw 的问题

`ChatMessage`（`fastclaw-core/src/types.rs:178-242`）同时承担模型历史和 UI 表示：

```rust
pub struct ChatMessage {
    pub role: Role,
    pub content: Option<serde_json::Value>,        // 模型用
    pub reasoning_content: Option<String>,          // 模型用，但 session 不持久化
    pub name: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tool_call_id: Option<ToolCallId>,
    pub compact_metadata: Option<CompactMetadata>,  // 模型用，但 session 不持久化
}
```

`SessionMessage`（`fastclaw-session/src/models.rs`）是 `ChatMessage` 的有损投影：

```rust
pub struct SessionMessage {
    pub role: String,           // 不是 Role 枚举
    pub content: Option<String>, // JSON 字符串
    pub tool_calls_json: Option<String>,
    // 缺少 reasoning_content、compact_metadata
}
```

## Requirements

### HIST-001: HistoryItem 定义

```rust
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
#[ts(export)]
pub enum HistoryItem {
    /// 用户/系统/助手消息
    Message {
        #[serde(default)]
        id: Option<String>,
        role: Role,
        content: Vec<ContentPart>,
        /// 推理内容（DeepSeek thinking mode 等）
        #[serde(default)]
        reasoning_content: Option<String>,
        /// 消息阶段（Commentary / FinalAnswer）
        #[serde(default)]
        phase: Option<MessagePhase>,
    },
    /// 工具调用请求
    ToolCall {
        #[serde(default)]
        id: Option<String>,
        call_id: String,
        name: String,
        /// JSON 字符串，保持原始格式不 eager parse
        arguments: String,
    },
    /// 工具调用输出
    ToolOutput {
        call_id: String,
        output: String,
        #[serde(default)]
        success: bool,
    },
    /// Compaction 摘要替换
    Compaction {
        summary: String,
        /// 原始 token 数
        #[serde(default)]
        original_tokens: Option<usize>,
        /// 压缩后 token 数
        #[serde(default)]
        compressed_tokens: Option<usize>,
    },
    /// 前向兼容
    #[serde(other)]
    Unknown,
}
```

### HIST-002: ContentPart

对标 Codex 的 `ContentItem`：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum ContentPart {
    Text { text: String },
    Image { url: String },
    #[serde(other)]
    Unknown,
}
```

### HIST-003: MessagePhase

对标 Codex 的 `MessagePhase`：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MessagePhase {
    Commentary,
    FinalAnswer,
}
```

### HIST-004: ChatMessage 与 HistoryItem 的桥接

保持 `ChatMessage` 向后兼容，但提供与 `HistoryItem` 的互转：

```rust
impl From<&ChatMessage> for Vec<HistoryItem> {
    fn from(msg: &ChatMessage) -> Vec<HistoryItem> {
        let mut items = Vec::new();
        
        // 消息本体
        items.push(HistoryItem::Message {
            id: None,
            role: msg.role.clone(),
            content: parse_content_parts(&msg.content),
            reasoning_content: msg.reasoning_content.clone(),
            phase: None,
        });
        
        // 工具调用
        if let Some(tool_calls) = &msg.tool_calls {
            for tc in tool_calls {
                items.push(HistoryItem::ToolCall {
                    id: None,
                    call_id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    arguments: tc.function.arguments.clone(),
                });
            }
        }
        
        items
    }
}
```

### HIST-005: Session 持久化无损化

改造 `fastclaw-session` 的持久化层：

1. `SessionMessage` 表新增列：
   - `reasoning_content TEXT` — 保存推理内容
   - `compact_metadata_json TEXT` — 保存压缩元数据

2. `append_message` 函数改造：

```rust
// 当前（有损）
sqlx::query("INSERT INTO messages (session_id, role, content, ...) VALUES (?, ?, ?, ...)")
    .bind(&session_id)
    .bind(role_str)
    .bind(content_json)
    // 缺少 reasoning_content、compact_metadata

// 改造后（无损）
sqlx::query("INSERT INTO messages (session_id, role, content, reasoning_content, compact_metadata_json, ...) VALUES (?, ?, ?, ?, ?, ...)")
    .bind(&session_id)
    .bind(role_str)
    .bind(content_json)
    .bind(&msg.reasoning_content)
    .bind(compact_metadata_json)
```

3. `parse_chat_messages_from_rows` 改造：

```rust
// 当前（有损）
ChatMessage {
    reasoning_content: None,        // 丢失
    compact_metadata: None,         // 丢失
    ..Default::default()
}

// 改造后（无损）
ChatMessage {
    reasoning_content: row.reasoning_content.clone(),
    compact_metadata: row.compact_metadata_json
        .as_ref()
        .and_then(|j| serde_json::from_str(j).ok()),
    ..
}
```

### HIST-006: 事件日志（Rollout）

新增 append-only 事件日志，对标 Codex 的 rollout JSONL：

```rust
// fastclaw-session 新增
pub struct EventLog {
    path: PathBuf,  // ~/.fastclaw/sessions/{session_id}/events.jsonl
}

impl EventLog {
    pub async fn append(&self, event: &AgentEvent) -> Result<()>;
    pub async fn replay(&self) -> Result<Vec<AgentEvent>>;
    pub async fn replay_from(&self, turn_id: &TurnId) -> Result<Vec<AgentEvent>>;
}
```

每行一个 JSON 对象：

```jsonl
{"ts":1716400000,"type":"turn_started","turn_id":"t-1","session_id":"s-1","agent_id":"main","model":"gpt-4o"}
{"ts":1716400001,"type":"delta","turn_id":"t-1","delta":{"content":"Hello"}}
{"ts":1716400002,"type":"tool_executing","turn_id":"t-1","tool_name":"shell","call_id":"tc-1","args":"ls"}
{"ts":1716400003,"type":"tool_result","turn_id":"t-1","tool_name":"shell","call_id":"tc-1","output":"...","success":true}
{"ts":1716400004,"type":"turn_complete","turn_id":"t-1","session_id":"s-1","summary":{...}}
```

## 门禁

| 检查项 | 验证方式 | 阻断条件 |
|--------|---------|---------|
| HistoryItem 可序列化往返 | 单元测试：每个变体 serialize→deserialize 一致 | 任何不一致 |
| ChatMessage→HistoryItem 无损 | 单元测试：ChatMessage 往返转换信息不丢失 | 信息丢失 |
| Session 持久化无损 | 集成测试：save→load ChatMessage 后 reasoning_content 和 compact_metadata 保留 | 字段为 None |
| EventLog append 性能 | 基准测试：1000 事件 append < 100ms | 超时 |
| 数据库迁移 | `sqlx migrate` 成功且不破坏现有数据 | 迁移失败 |
