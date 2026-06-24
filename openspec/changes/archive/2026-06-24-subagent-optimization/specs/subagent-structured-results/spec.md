## ADDED Requirements

### Requirement: Structured completion notification
reactive loop 在 worker 完成时回注给主 agent 的 completion notification SHALL 以结构化格式呈现，至少包含 `run_id`、`subagent_type`、`task`、`status`、`result`、`tool_calls_made`、`elapsed_ms` 字段，而非自由文本摘要。

#### Scenario: Worker completion includes structured fields
- **WHEN** 一个 worker sub-agent 成功完成
- **THEN** 主 agent 收到的 notification 包含明确标注的 run_id、status、result 等字段，可被可靠解析

#### Scenario: Worker failure includes error context
- **WHEN** 一个 worker sub-agent 失败
- **THEN** notification 的 status 标记为 failed，并包含错误原因，主 agent 可据此决策

### Requirement: Active sub-agent progress injection
主 agent 收到的 active_runs 状态 SHALL 包含进度信息（`tool_calls_made` 和当前/最新工具名），而非仅 task 与 elapsed time。

#### Scenario: Main agent perceives worker progress
- **WHEN** 主 agent 在 reactive loop 中检查活跃 worker 状态
- **THEN** 每个活跃 worker 的状态包含已执行工具数和最新工具名

### Requirement: Completion notification cache safety
结构化 completion notification SHALL 注入到 messages 末尾（user context 层），MUST NOT 修改或污染已缓存的 system prompt。

#### Scenario: Notification does not break system cache
- **WHEN** reactive loop 注入 completion notification 并 re-prompt
- **THEN** 主 agent 的 Tier-1/Tier-2 system message 保持 byte-stable
