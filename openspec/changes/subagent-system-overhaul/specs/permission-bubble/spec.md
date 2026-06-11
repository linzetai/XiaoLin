## ADDED Requirements

### Requirement: SubAgentDef supports permission_mode field

`SubAgentDef` SHALL 支持 `permission_mode` 字段控制子 agent 的权限行为。

#### Scenario: AutoApprove mode (default)
- **GIVEN** SubAgentDef 未指定 permission_mode 或 permission_mode = "auto_approve"
- **WHEN** 子 agent 执行工具
- **THEN** 所有工具自动批准，无需确认（现有行为不变）

#### Scenario: Bubble mode
- **GIVEN** SubAgentDef 指定 `permission_mode: "bubble"`
- **WHEN** 子 agent 尝试执行需确认的工具
- **THEN** 审批请求 bubble 到父级（通过 event channel）
- **NOTE** 与 claude-code 不同：claude-code 不允许 markdown agents 声明 bubble（仅编程式设置）。我们选择允许用户在 frontmatter 中显式启用，赋予更多控制权。

#### Scenario: Deny mode
- **GIVEN** SubAgentDef 指定 `permission_mode: "deny"`
- **WHEN** 子 agent 尝试执行需确认的工具
- **THEN** 直接拒绝，返回 ToolResult::err("permission denied by policy")

### Requirement: Approval bubble via event channel

Permission bubble 模式下，审批请求 SHALL 通过 AgentEvent 传递到父级。

#### Scenario: Bubble event emitted
- **GIVEN** 子 agent 在 bubble 模式下调用需确认的工具
- **WHEN** ApprovalStrategy 判定需要确认
- **THEN** 向 parent_tx 发送 `AgentEvent::ApprovalBubble { run_id, tool_name, tool_args_preview, respond_tx }`
- **AND** 子 agent 阻塞等待 respond_tx 的 oneshot 回复

#### Scenario: Parent approves
- **GIVEN** 父级 UI 收到 approval bubble 事件
- **WHEN** 用户点击 Approve
- **THEN** 通过 oneshot channel 发送 `ApprovalResult::Approved`
- **AND** 子 agent 继续执行该工具

#### Scenario: Parent denies
- **GIVEN** 父级 UI 收到 approval bubble 事件
- **WHEN** 用户点击 Deny
- **THEN** 通过 oneshot channel 发送 `ApprovalResult::Denied { reason }`
- **AND** 子 agent 收到 ToolResult::err("permission denied by user: {reason}")

### Requirement: Approval timeout defaults to deny

如果父级未在超时时间内回复 SHALL 默认拒绝。

#### Scenario: Timeout
- **GIVEN** 审批请求已发送
- **WHEN** 30 秒内 oneshot channel 未收到回复（如 UI 断连）
- **THEN** 子 agent 收到 `ApprovalResult::Denied { reason: "approval timeout" }`
- **AND** 记录 warning 日志

### Requirement: Approval bubble forwarded to WebSocket

父级收到的 ApprovalBubble event SHALL 转发到前端 WebSocket。

#### Scenario: Frontend receives bubble
- **GIVEN** gateway session 收到 `AgentEvent::ApprovalBubble`
- **WHEN** 转发到前端
- **THEN** WebSocket 发送 `{ type: "approval_bubble", data: { run_id, tool_name, args_preview, request_id } }`

#### Scenario: Frontend responds
- **GIVEN** 前端用户做出审批决定
- **WHEN** 前端发送 `{ type: "approval_respond", data: { request_id, approved: bool, reason? } }`
- **THEN** gateway 通过 respond_tx 将结果传回子 agent
