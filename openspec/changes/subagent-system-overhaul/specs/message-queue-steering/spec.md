## ADDED Requirements

### Requirement: MessageQueue with priority levels

系统 SHALL 提供 per-agent 的 MessageQueue，支持优先级消息排队。

#### Scenario: Message enqueued at Next priority
- **WHEN** 通过 `send_message` 工具向目标 agent 发送消息
- **THEN** 消息入队为 `Priority::Next`，在目标 agent 的下一个 tool-round boundary 被注入

#### Scenario: Priority ordering
- **GIVEN** queue 中有 `Now`、`Next`、`Later` 三种优先级消息
- **WHEN** 到达 tool-round boundary 时 drain
- **THEN** `Now` 优先级消息先被注入，然后 `Next`，`Later` 暂不 drain（等待显式触发如 Sleep）

#### Scenario: Worker notification uses Next priority
- **GIVEN** Coordinator 有 running workers
- **WHEN** worker 完成，CompletionSummary 推入 coordinator 的 MessageQueue
- **THEN** notification 使用 `Priority::Next`（每个 tool-round boundary 都 drain）
- **NOTE** 这与 claude-code 的 `later` 优先级不同——coordinator 需要及时获知 worker 状态

### Requirement: Messages injected at tool-round boundary

MessageQueue 中的 pending 消息 SHALL 在 tool-round boundary 被注入到对话中。

#### Scenario: Injection timing
- **GIVEN** agent 完成一轮 tool 执行（所有 tool_results 收集完毕）
- **WHEN** 下一次 LLM 调用之前
- **THEN** drain MessageQueue 中 priority <= Next 的消息，作为 user messages 追加到对话历史

#### Scenario: No pending messages
- **GIVEN** MessageQueue 为空
- **WHEN** tool-round boundary 到达
- **THEN** 正常继续下一轮 LLM 调用，无额外消息注入

#### Scenario: Message format
- **WHEN** 消息从 queue 注入对话
- **THEN** 格式为 `ChatMessage { role: User, content: "[Steering from {source}]: {message}" }`

### Requirement: SendMessage tool for inter-agent communication

系统 SHALL 提供 `send_message` 工具，允许 agent 向其他 agent 发送消息。

#### Scenario: Coordinator sends to worker
- **GIVEN** Coordinator agent 有一个 running worker（run_id = "abc"）
- **WHEN** Coordinator 调用 `send_message({ to: "abc", message: "also check error handling" })`
- **THEN** 消息入队到 worker "abc" 的 MessageQueue (Priority::Next)
- **AND** worker 在下一个 tool-round boundary 收到该消息

#### Scenario: Send to non-existent agent
- **WHEN** `send_message({ to: "invalid_id", message: "..." })`
- **THEN** 返回 ToolResult::err("agent run_id not found: invalid_id")

#### Scenario: Send to completed agent
- **WHEN** `send_message` 目标 agent 已完成
- **THEN** 返回 ToolResult::err("agent has already completed, cannot send message")

### Requirement: SendMessage tool schema

`send_message` 工具的参数 SHALL 包含 `to` 和 `message`。

#### Scenario: Tool definition
- **GIVEN** SendMessage 工具注册在 tool registry
- **THEN** schema 为: `{ "to": { "type": "string", "description": "Target agent run_id" }, "message": { "type": "string", "description": "Message content to send" } }`

### Requirement: Queue observable via events

消息注入时 SHALL 产出对应的 AgentStep/AgentEvent。

#### Scenario: Steering message event
- **WHEN** 消息从 queue 注入对话
- **THEN** stream yield `AgentStep::SteeringInjected { source, message }`
- **AND** 前端收到 `AgentEvent::SteeringMessage` 事件
