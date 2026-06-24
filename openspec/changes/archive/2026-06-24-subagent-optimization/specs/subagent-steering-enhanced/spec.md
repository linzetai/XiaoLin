## ADDED Requirements

### Requirement: Quick steering actions
Steering 输入区域 SHALL 提供快捷操作按钮（如聚焦文件、加速完成、跳过当前、停下解释），点击后 SHALL 填入输入框供用户修改，而非立即发送。

#### Scenario: Quick action fills input
- **WHEN** 用户点击"聚焦文件"快捷按钮
- **THEN** 输入框被填入对应的预设 steering 文本，用户可编辑后再发送

#### Scenario: Quick action with context substitution
- **WHEN** 用户点击"聚焦文件"且当前有运行中的工具操作某文件
- **THEN** 预设文本中的文件占位符被替换为该文件名

### Requirement: Steering send feedback and history
Steering 发送 SHALL 提供状态反馈（发送中、已发送）；前端 SHALL 维护本 session 内的 steering 发送历史。

#### Scenario: Send shows progress and confirmation
- **WHEN** 用户发送 steering 消息
- **THEN** 发送按钮显示发送中状态，成功后短暂显示确认标记

#### Scenario: History lists previous steers
- **WHEN** 用户在本 session 已发送过 steering 消息
- **THEN** steering 区域显示历史记录（时间 + 消息内容 + 状态）

### Requirement: Steering priority selection
Steering 输入 SHALL 允许用户切换消息优先级（普通/紧急），对应后端 MessageQueue 的 normal/high priority。

#### Scenario: High priority steering
- **WHEN** 用户切换到紧急模式并发送 steering
- **THEN** 消息以 high priority 推入目标 sub-agent 的 MessageQueue

### Requirement: Steering target selection
在 coordinator 场景下，Steering SHALL 允许用户选择发送目标（coordinator 或某个活跃 worker）。

#### Scenario: Steer specific worker
- **WHEN** coordinator 有多个活跃 worker，用户从下拉选择某个 worker 并发送
- **THEN** steering 消息被路由到该 worker 的 run_id，而非 coordinator
