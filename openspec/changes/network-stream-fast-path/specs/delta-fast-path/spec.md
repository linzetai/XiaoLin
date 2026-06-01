## ADDED Requirements

### Requirement: ContentDelta 携带预序列化字节
`AgentEvent::ContentDelta` SHALL 支持携带可选的预序列化 JSON 字节（`raw_bytes: Option<bytes::Bytes>`），供 SSE 格式化时直接使用，避免重复序列化。

#### Scenario: Runtime 填充 raw_bytes
- **WHEN** runtime 从 LLM 流式响应解析出 `StreamDelta` 并构建 `ContentDelta` 事件
- **THEN** 事件 MUST 同时包含结构化 `delta: Value` 和原始 SSE data 行的 `raw_bytes`

#### Scenario: SSE 格式化优先使用 raw_bytes
- **WHEN** gateway 将 `ContentDelta` 格式化为 SSE 输出
- **AND** `raw_bytes` 为 `Some`
- **THEN** gateway SHALL 直接使用 `raw_bytes` 构建 `data: ...\n\n`，不调用 `serde_json::to_string`

#### Scenario: raw_bytes 为 None 时降级
- **WHEN** `ContentDelta` 的 `raw_bytes` 为 `None`（例如内部构造的事件）
- **THEN** gateway SHALL 回退到 `serde_json::to_string(&delta)` 格式化

### Requirement: HTTP UserTurn 跳过冗余 messages 序列化
当 `typed_data` 已设置时，`SessionOp::UserTurn` 的 `messages` 字段 SHALL 使用空占位值，避免对完整消息列表的 `to_value` 序列化。

#### Scenario: typed_data 存在时 messages 为空
- **WHEN** HTTP `handle_stream` 构建 `SessionOp::UserTurn` 且 `typed_data` 为 `Some`
- **THEN** `messages` 字段 MUST 为空 `Value::Array(vec![])`

#### Scenario: session actor 从 typed_data 获取消息
- **WHEN** session actor 收到 `UserTurn` 且 `typed_data` 为 `Some`
- **THEN** actor SHALL 从 `typed_data` 提取消息，忽略 `messages` 字段
