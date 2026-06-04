## ADDED Requirements

### Requirement: compute_level.get WS API
后端 SHALL 提供 `compute_level.get` WS 方法，返回指定 session 的有效计算等级及可用档位列表。

#### Scenario: Get compute level for session with override
- **WHEN** 收到 `compute_level.get { session_id }`
- **AND** 该 session 有计算等级覆盖（如 `extra_high`）
- **THEN** 返回 `{ level: "extra_high", level_label: "Extra High", is_override: true, levels: [...] }`
- **AND** `levels` 包含全部 5 档：`{ id, label, description, tier }`（tier 为 snake_case `ComplexityTier`）

#### Scenario: Get compute level without override
- **WHEN** 收到 `compute_level.get { session_id }`
- **AND** 该 session 无计算等级覆盖
- **THEN** 返回全局默认等级 `high`（`ComplexityTier::Medium`）
- **AND** `is_override` = false

### Requirement: compute_level.set WS API
后端 SHALL 提供 `compute_level.set` WS 方法，设置或清除指定 session 的计算等级覆盖。

#### Scenario: Set valid compute level
- **WHEN** 收到 `compute_level.set { session_id, level: "extra_high" }`
- **AND** `extra_high` 是有效的 `ComputeLevel` ID
- **THEN** 为该 session 设置 `compute_level_override`
- **AND** 返回 `{ success: true, level: "extra_high", level_label: "Extra High", is_override: true }`
- **AND** 广播 `compute_level.changed { session_id, level, level_label, is_override }` WS 事件

#### Scenario: Set invalid compute level
- **WHEN** 收到 `compute_level.set { session_id, level: "ultra" }`
- **THEN** 返回错误 `{ error: "Unknown compute level: ultra" }`
- **AND** 不修改 session 覆盖状态

#### Scenario: Reset to global default
- **WHEN** 收到 `compute_level.set { session_id, level: null }`
- **THEN** 清除该 session 的 `compute_level_override`
- **AND** 返回 `{ success: true, level: "high", level_label: "High", is_override: false }`
- **AND** 广播 `compute_level.changed` 事件

### Requirement: compute_level.changed WS event
后端 SHALL 在计算等级变更时广播 `compute_level.changed` 事件。

#### Scenario: Broadcast on change
- **WHEN** session 的计算等级被设置或清除
- **THEN** 广播 `compute_level.changed { session_id, level, level_label, is_override }`
- **AND** 所有已连接的前端客户端更新对应 session 的 store 与 InputBar UI

### Requirement: Per-session override resolution
后端 SHALL 按 `session_override.unwrap_or(global_default)` 解析有效计算等级，并映射为 `ComplexityTier` 供模型路由使用。

#### Scenario: Resolver with session override
- **WHEN** `ComputeLevelResolver.resolve(session_id)` 被调用
- **AND** 该 session 的 `compute_level_override` 为 `Some(ComputeLevel::Max)`
- **THEN** 有效 `ComplexityTier` 为 `Frontier`
- **AND** 传给 `ModelRouter` 的 `agent_min_tier` 为 `Some(Frontier)`

#### Scenario: Resolver without override uses global default
- **WHEN** `ComputeLevelResolver.resolve(session_id)` 被调用
- **AND** 该 session 无 `compute_level_override`
- **THEN** 有效等级为全局默认 `ComputeLevel::High` → `ComplexityTier::Medium`

#### Scenario: Override does not affect other sessions
- **WHEN** session A 设置覆盖为 `max`
- **AND** session B 无覆盖
- **THEN** session A 路由使用 `Frontier` 作为 min_tier 下限
- **AND** session B 仍使用默认 `Medium` 下限

### Requirement: Override takes effect on next turn
计算等级变更 SHALL 在下一个 turn 的模型路由中生效；当前正在执行的 turn 使用变更前的 min_tier snapshot。

#### Scenario: Mid-turn compute level change
- **WHEN** session 正在执行 turn
- **AND** 用户通过 `compute_level.set` 切换等级
- **THEN** 当前 turn 继续使用旧的 `agent_min_tier`
- **AND** 下一个 turn 使用新解析的 min_tier

### Requirement: Override reset on session close
Session 的计算等级覆盖 SHALL 在 session 关闭或应用重启时清除。

#### Scenario: Session close resets override
- **WHEN** session 被关闭
- **THEN** 该 session 的 `compute_level_override` 从内存中移除
- **WHEN** 用户重新打开同一 session（若支持恢复）
- **THEN** 使用全局默认计算等级 High
