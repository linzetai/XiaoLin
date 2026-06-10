## ADDED Requirements

### Requirement: Track token usage per turn
系统 SHALL 在每个 LLM turn 完成后，将该 turn 消耗的 token 数量累计到 active goal 的 `tokens_used` 字段。

#### Scenario: Normal turn with active goal
- **WHEN** LLM response 返回 token_usage 且存在 active goal
- **THEN** goal 的 tokens_used 增加 delta（= non_cached_input_tokens + output_tokens）

#### Scenario: Turn without active goal
- **WHEN** LLM response 返回 token_usage 但不存在 active goal
- **THEN** 不进行 token accounting

### Requirement: Track wall-clock time
系统 SHALL 跟踪 active goal 的累计执行时间（wall-clock seconds）。

#### Scenario: Time accumulation during turns
- **WHEN** 一个 turn 开始到结束经过 T 秒
- **THEN** goal 的 time_used_seconds 增加 T

#### Scenario: Time not counted during pause
- **WHEN** goal 处于 paused 状态
- **THEN** 时间不再累计

### Requirement: Token delta excludes cached input
Token accounting SHALL 只计算 non-cached input tokens + output tokens，排除 cached input。

#### Scenario: Request with cache hit
- **WHEN** LLM response 的 token_usage 包含 input_tokens=1000, cached_input_tokens=800, output_tokens=200
- **THEN** token delta = (1000 - 800) + 200 = 400
