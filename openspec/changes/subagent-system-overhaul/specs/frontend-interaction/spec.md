## ADDED Requirements

### Requirement: sub_agent_notification event handled in frontend

前端 SHALL 处理 `sub_agent_notification` WebSocket 事件。

#### Scenario: Notification received
- **WHEN** WebSocket 收到 `{ type: "sub_agent_notification", data: { run_id, message, timestamp } }`
- **THEN** stream-store 中对应 run 的 notifications 数组追加该条目
- **AND** SubAgentMonitor UI 显示该通知

#### Scenario: Notification for unknown run
- **WHEN** notification 的 run_id 不在当前已知的 runs 中
- **THEN** 忽略该通知（可能是过期的 run）

### Requirement: Steering input in SubAgentCard

SubAgentCard 展开态 SHALL 提供消息输入框，允许用户向 running agent 发送消息。

#### Scenario: User sends steering message
- **GIVEN** SubAgentCard 展开且 agent 状态为 running
- **WHEN** 用户在输入框输入文字并提交
- **THEN** 前端通过 WebSocket 发送 `{ type: "steering_message", data: { run_id, message } }`
- **AND** 后端将消息入队到目标 agent 的 MessageQueue

#### Scenario: Agent not running
- **GIVEN** SubAgentCard 对应的 agent 已完成
- **WHEN** 查看 card
- **THEN** 输入框不显示（或 disabled），无法发送消息

### Requirement: Approval bubble card in chat stream

权限 bubble 事件 SHALL 在聊天流中显示审批卡片。

#### Scenario: Approval card rendered
- **WHEN** 前端收到 `{ type: "approval_bubble", data: { run_id, tool_name, args_preview, request_id } }`
- **THEN** 在当前聊天流中渲染 `ApprovalBubbleCard` 组件
- **AND** 卡片显示: 子 agent 名称、工具名称、参数预览、Approve/Deny 按钮

#### Scenario: User approves
- **WHEN** 用户点击 Approve
- **THEN** 前端发送 `{ type: "approval_respond", data: { request_id, approved: true } }`
- **AND** 卡片状态更新为 "已批准 ✓"

#### Scenario: User denies
- **WHEN** 用户点击 Deny（可选填理由）
- **THEN** 前端发送 `{ type: "approval_respond", data: { request_id, approved: false, reason: "..." } }`
- **AND** 卡片状态更新为 "已拒绝 ✗"

#### Scenario: Timeout card state
- **WHEN** 审批请求超时（30s 后端自动 deny）
- **THEN** 前端收到 `{ type: "approval_resolved", data: { request_id, result: "timeout" } }`
- **AND** 卡片状态更新为 "已超时"

### Requirement: Coordinator monitoring panel

当有 Coordinator 模式 agent 运行时 SHALL 显示专用监控面板。

#### Scenario: Coordinator active
- **GIVEN** 当前 session 有一个 coordinator agent 在运行
- **WHEN** 前端检测到 coordinator 类型的 subagent_start
- **THEN** 在 WorkspacePanel 中显示 Coordinator 标签页

#### Scenario: Panel content
- **GIVEN** Coordinator panel 可见
- **THEN** 显示:
  - Coordinator 的任务描述
  - Worker 列表（name, task, status, elapsed time）
  - 每个 worker 的最新 activity（last tool call or content delta）
  - Worker 之间的依赖/通信可视化（SendMessage 箭头）

#### Scenario: Coordinator completes
- **WHEN** coordinator agent 完成
- **THEN** panel 转为 summary 视图（显示所有 worker 最终结果 + 总耗时）

### Requirement: Cancel button in SubAgentCard inline

SubAgentCard 在消息流中 SHALL 包含 cancel 按钮（非仅 Monitor）。

#### Scenario: Cancel from inline card
- **GIVEN** running subagent 的 SubAgentCard 在聊天流中展示
- **WHEN** 用户点击卡片上的 cancel 按钮
- **THEN** 调用 `cancelSubAgentRun(runId)` API
- **AND** 卡片状态更新为 "已取消"
