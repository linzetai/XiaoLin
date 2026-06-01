## ADDED Requirements

### Requirement: WebSocket UserTurn 使用 TypedTurnData
WebSocket 聊天路径提交 `SessionOp::UserTurn` 时 SHALL 使用 `TypedTurnData::wrap` 设置 `typed_data` 字段，与 HTTP 路径行为一致，避免 `serde_json::to_value(enriched_request)` 和 `serde_json::to_value(agent_config)` 的全量 JSON 序列化。

#### Scenario: WS 提交携带 typed_data
- **WHEN** WebSocket 客户端发送聊天请求，gateway 构建 `SessionOp::UserTurn`
- **THEN** `typed_data` MUST 为 `Some(TypedTurnData::wrap(enriched_request, config))`

#### Scenario: WS extra 不再携带序列化副本
- **WHEN** `typed_data` 已设置
- **THEN** `extra` 中 SHALL NOT 包含 `_enriched_request` 和 `_agent_config` 的 JSON Value

#### Scenario: Session bridge 正确提取 WS typed_data
- **WHEN** session bridge 收到来自 WS 路径的 `UserTurn`
- **THEN** bridge SHALL 从 `typed_data` 提取 `ChatRequest` 和 `AgentConfig`，行为与 HTTP 路径一致
