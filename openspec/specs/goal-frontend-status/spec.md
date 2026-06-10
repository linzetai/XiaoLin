## ADDED Requirements

### Requirement: Goal status card in chat view
当存在 goal 时，前端 SHALL 在聊天区域展示 goal 状态卡片。

#### Scenario: Active goal display
- **WHEN** 当前 session 有 active goal
- **THEN** 聊天区域顶部或消息区显示状态卡片，包含 goal 描述（截断至合理长度）、"Active" 状态标签、token 进度条（如有预算）

#### Scenario: No goal
- **WHEN** 当前 session 无 goal
- **THEN** 不显示 goal 状态卡片

### Requirement: Real-time status update via WebSocket
Goal 状态变化 SHALL 通过 WebSocket 事件实时推送到前端。

#### Scenario: Goal status change
- **WHEN** goal 状态从 active 变为 paused / completed / budget_limited
- **THEN** 前端收到 GoalUpdated 事件，状态卡片实时更新

### Requirement: User goal control actions
前端 SHALL 提供 goal 操控按钮。

#### Scenario: Pause active goal
- **WHEN** 用户点击 Pause 按钮
- **THEN** 发送 pause 请求到后端，goal 状态变为 paused，自动续轮停止

#### Scenario: Resume paused goal
- **WHEN** 用户点击 Resume 按钮
- **THEN** 发送 resume 请求到后端，goal 状态变为 active，自动续轮恢复

#### Scenario: Clear goal
- **WHEN** 用户点击 Clear 按钮
- **THEN** goal 被删除，状态卡片消失
