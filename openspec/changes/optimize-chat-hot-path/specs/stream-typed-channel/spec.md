## ADDED Requirements

### Requirement: UserTurn 携带类型化数据
`SessionOp::UserTurn` SHALL 直接携带 `Arc<ChatSetup>` 和 `Arc<AgentConfig>` 类型的字段，而非将它们序列化为 `serde_json::Value` 放入通用 `extra` map。

#### Scenario: stream path 提交 UserTurn
- **WHEN** gateway 的 handle_stream 构建 SessionOp::UserTurn
- **THEN** 直接将 ChatSetup 和 AgentConfig wrap 为 Arc 放入 op 的类型化字段，不调用 serde_json::to_value

#### Scenario: session_bridge 接收 UserTurn
- **WHEN** RuntimeTurnExecutor 处理 UserTurn op
- **THEN** 直接从 op 中取出 `Arc<ChatSetup>` 和 `Arc<AgentConfig>`，不调用 serde_json::from_value

### Requirement: 消除 enriched_request 的 JSON 中转
stream path 中 enriched_request 的传递 SHALL 不经过 JSON 序列化/反序列化。从 setup_chat 构建到 agent runtime 使用，消息数据 SHALL 以 Rust 类型直接传递。

#### Scenario: 200 条消息的 session
- **WHEN** 一个包含 200 条消息历史的 session 发送新请求（stream=true）
- **THEN** 消息历史从 setup_chat 到 agent runtime 的传递过程中不产生 JSON 序列化，不分配 serde_json::Value 树

#### Scenario: 与非 stream 路径的一致性
- **WHEN** stream=false 的请求走 handle_non_stream 路径
- **THEN** 该路径不受影响，继续直接调用 runtime.execute_with_subagent_prompt
