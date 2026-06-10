## ADDED Requirements

### Requirement: Auto-continue on active goal
当 agent 完成一个 turn（LLM 返回无 tool_calls）且存在 active goal 时，系统 SHALL 自动注入 continuation prompt 并开启下一轮 LLM 调用。

#### Scenario: Turn ends with active goal
- **WHEN** LLM response 无 tool_calls 且 GoalStore 中存在 status=active 的 goal
- **THEN** stop hook 返回 `should_continue=true`，注入 continuation prompt 作为下一轮的 user message

#### Scenario: Turn ends without active goal
- **WHEN** LLM response 无 tool_calls 且 GoalStore 中无 active goal
- **THEN** goal stop hook 不触发，由其他 hook（如 todo hook）或默认行为决定是否停止

### Requirement: Agent marks goal complete via update_goal
当 agent 调用 `update_goal(status=completed)` 时，系统 SHALL 停止自动续轮。

#### Scenario: Goal marked complete
- **WHEN** agent 调用 `update_goal` 将 goal 设为 completed
- **THEN** GoalStore 中该 goal 状态变为 completed，后续 stop hook 不再触发 continuation

### Requirement: Max continuation rounds safety limit
系统 SHALL 限制单个 goal 的最大自动续轮次数，防止无限循环。

#### Scenario: Max rounds reached
- **WHEN** goal 已自动续轮达到最大次数（默认 50 轮）
- **THEN** 系统自动将 goal 状态设为 paused，注入提示告知用户已暂停，停止续轮

### Requirement: Goal continuation skipped in Plan mode
当 ExecutionMode 为 Plan 时，goal 自动续轮 SHALL 被跳过。Goal 仍然保持 active 状态但不驱动续轮。

#### Scenario: Plan mode with active goal
- **WHEN** turn 结束时存在 active goal，但当前 ExecutionMode 为 Plan
- **THEN** stop hook 不触发 goal continuation，goal 保持 active 不变

#### Scenario: Switch from Plan to Agent with active goal
- **WHEN** 用户从 Plan 切换到 Agent mode，且存在 active goal
- **THEN** goal continuation 恢复，agent 在下一个 idle 时刻自动开始推进 goal

### Requirement: Goal is an overlay, not a mode
Goal 是一个持久目标层（overlay），叠加在 ExecutionMode 之上，不是与 Agent/Plan 并列的模式。

#### Scenario: Mode switch preserves goal
- **WHEN** 用户在 goal active 时切换 ExecutionMode（如 Agent → Plan → Agent）
- **THEN** goal 状态不受 mode 切换影响，保持原状态

### Requirement: User input takes priority over goal continuation
当 goal active 且 turn 结束时，如果存在待处理的用户输入（消息或 slash command），系统 SHALL 优先处理用户输入，不触发 goal continuation。

#### Scenario: User message queued during goal turn
- **WHEN** goal 正在自动续轮，用户提交了新消息
- **THEN** 当前 turn 完成后，先处理用户消息，不注入 continuation prompt

#### Scenario: No pending input with active goal
- **WHEN** turn 结束，无待处理用户输入，goal 仍然 active
- **THEN** 正常触发 goal continuation

### Requirement: User interrupt pauses active goal
当用户在 goal 执行过程中发送中断信号时，系统 SHALL 暂停 active goal。

#### Scenario: User sends message during goal execution
- **WHEN** goal 正在自动续轮过程中，用户发送新消息
- **THEN** 当前 turn 正常完成后，goal 状态变为 paused，用户消息被处理

#### Scenario: User clicks stop button
- **WHEN** 用户在前端点击停止按钮
- **THEN** 当前 turn 被中断，goal 状态变为 paused
