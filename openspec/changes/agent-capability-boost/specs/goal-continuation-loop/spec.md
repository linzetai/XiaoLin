## MODIFIED Requirements

### Requirement: Stop hook API error protection
`evaluate_stop_hooks` SHALL 在以下条件下跳过 continuation hooks（goal / todo / truncation），防止死循环：

#### Scenario: Skip hooks after reactive compact failure
- **WHEN** `has_attempted_reactive_compact` 为 true 且 context 占用超过 85%
- **THEN** 所有 continuation hooks SHALL 返回 `should_continue: false`

#### Scenario: Skip truncation hook after max_output recovery exhausted
- **WHEN** `max_output_recovery_count >= 3` 且 `finish_reason == "length"`
- **THEN** Hook 2（output_truncated）SHALL 不触发 continuation

### Requirement: PTL connect-fail single compact guard
LLM 连接失败路径的 `prompt_too_long` 处理 SHALL 共用 `has_attempted_reactive_compact` guard，确保 reactive compact 最多执行一次。

#### Scenario: PTL on connection failure
- **WHEN** LLM 连接失败且错误为 prompt_too_long
- **AND** `has_attempted_reactive_compact` 为 false
- **THEN** 执行 reactive compact 并重试
- **AND** 设置 `has_attempted_reactive_compact = true`

#### Scenario: PTL on connection failure after compact
- **WHEN** LLM 连接失败且错误为 prompt_too_long
- **AND** `has_attempted_reactive_compact` 为 true
- **THEN** SHALL 返回 FatalError，不再重试
