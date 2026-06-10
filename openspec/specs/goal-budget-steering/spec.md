## ADDED Requirements

### Requirement: Budget limit detection
当 active goal 的 tokens_used 达到或超过 token_budget 时，系统 SHALL 将 goal 状态设为 budget_limited。

#### Scenario: Tokens exceed budget
- **WHEN** turn 完成后 accounting 发现 tokens_used >= token_budget
- **THEN** goal 状态变为 budget_limited

#### Scenario: No budget set
- **WHEN** goal 没有设置 token_budget（为 None）
- **THEN** 永不触发 budget_limited，仅受 max continuation rounds 限制

### Requirement: Budget limit prompt injection
当 goal 变为 budget_limited 时，系统 SHALL 注入 budget_limit prompt 引导 model 收尾。

#### Scenario: Budget reached during turn
- **WHEN** token accounting 发现预算已达，且当前 turn 仍在进行
- **THEN** 注入 budget_limit prompt 到当前 conversation，引导 model 总结进度和剩余工作

### Requirement: Budget limited stops continuation
budget_limited 状态的 goal SHALL 不触发自动续轮。

#### Scenario: Stop hook with budget_limited goal
- **WHEN** turn 结束，goal 状态为 budget_limited
- **THEN** stop hook 返回 should_continue=false

### Requirement: Budget validation on creation
创建 goal 时如果指定了 token_budget，系统 SHALL 验证其为正整数。

#### Scenario: Invalid budget
- **WHEN** agent 调用 create_goal 且 token_budget <= 0
- **THEN** 返回错误提示 "goal budgets must be positive when provided"
