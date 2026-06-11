## ADDED Requirements

### Requirement: Coordinator is a SubAgentDef with restricted tools

Coordinator 模式 SHALL 通过 SubAgentDef 定义实现，不是全局 ExecutionMode。

#### Scenario: Coordinator agent spawned
- **GIVEN** SubAgentDef with `mode: "coordinator"`
- **WHEN** 父级调用 spawn_subagent 生成 coordinator
- **THEN** coordinator 的 tool registry 仅包含: `spawn_subagent`, `send_message`, `task_stop`, `subagent_list`, `subagent_get`

#### Scenario: Coordinator cannot directly edit files
- **GIVEN** coordinator agent 在运行
- **WHEN** LLM 尝试调用 `write_file` 或 `shell_exec`
- **THEN** 工具不可用（不在 registry 中），LLM 被告知只能通过 worker 执行

### Requirement: Coordinator spawns workers as async agents

Coordinator 通过 spawn_subagent 产出的子 agent SHALL 始终为 async (background)。

#### Scenario: Worker forced async
- **GIVEN** coordinator 调用 `spawn_subagent({ task: "...", background: false })`
- **WHEN** coordinator 模式下
- **THEN** 忽略 `background: false`，worker 始终 async 运行
- **AND** spawn_subagent 立即返回 run_id

### Requirement: Worker completion notifies coordinator via MessageQueue

Worker 完成后 SHALL 通过 MessageQueue 将通知注入 coordinator 的对话。

#### Scenario: Worker completes successfully
- **GIVEN** coordinator 有 running worker "w1"
- **WHEN** worker "w1" 完成
- **THEN** CompletionSummary 格式化为通知消息，push 到 coordinator 的 MessageQueue (Priority::Next)
- **AND** coordinator 在下一个 tool-round boundary 收到: `[Worker Completed: w1] Status: success | Result: ...`

#### Scenario: Worker fails
- **GIVEN** worker 执行出错
- **WHEN** worker run 结束（status = failed）
- **THEN** 通知包含错误信息: `[Worker Failed: w1] Error: ...`

### Requirement: Coordinator has specialized system prompt

Coordinator agent SHALL 使用编排专用的 system prompt。

#### Scenario: Coordinator prompt content
- **GIVEN** coordinator SubAgentDef 未指定自定义 system_prompt
- **WHEN** 使用默认 coordinator prompt
- **THEN** prompt 指导 coordinator:
  - 将用户任务分解为独立子任务
  - 使用 `spawn_subagent` 创建 worker
  - 使用 `send_message` 续联 worker
  - 不要用一个 worker 检查另一个的结果
  - Workers 完成后会自动通知
  - 综合所有 worker 结果产出最终回答

### Requirement: task_stop tool ends coordinator

`task_stop` 工具 SHALL 让 coordinator 主动结束编排。

#### Scenario: All workers done, coordinator wraps up
- **GIVEN** coordinator 收到所有 worker 完成通知
- **WHEN** coordinator 调用 `task_stop({ summary: "all tasks completed" })`
- **THEN** coordinator stream 正常结束，summary 作为 coordinator 的最终结果返回给父级

### Requirement: Coordinator limits concurrent workers

Coordinator 生成的 worker 数量 SHALL 受 SpawnController 限制。

#### Scenario: Max workers reached
- **GIVEN** SpawnConfig.max_per_session = 5，coordinator 已有 5 个 running workers
- **WHEN** coordinator 尝试 spawn 第 6 个 worker
- **THEN** spawn 排队等待 slot，直到某个 worker 完成或超时
