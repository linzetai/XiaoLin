## ADDED Requirements

### Requirement: Per-session permission override
后端 SHALL 支持在 session 粒度覆盖全局权限预设，覆盖值存储在内存中。

#### Scenario: Set session override
- **WHEN** 收到 `permissions.set { session_id, preset_id: "full-auto" }`
- **THEN** 后端为该 session 设置权限覆盖
- **AND** 后续该 session 的工具审批使用 "full-auto" 预设对应的 BehaviorConfig

#### Scenario: No override uses global default
- **WHEN** session 没有权限覆盖
- **THEN** 工具审批使用全局 AgentConfig.behavior

#### Scenario: Override does not affect other sessions
- **WHEN** session A 设置覆盖为 "full-auto"
- **AND** session B 没有设置覆盖
- **THEN** session B 仍使用全局默认权限

### Requirement: Override takes effect on next turn
权限变更 SHALL 在下一个 turn 生效，当前正在执行的 turn 不受影响。

#### Scenario: Mid-turn permission change
- **WHEN** session 正在执行 turn（Agent 正在调用工具）
- **AND** 用户切换权限预设
- **THEN** 当前 turn 继续使用旧权限
- **AND** 下一个 turn 使用新权限
- **AND** 前端显示提示 "权限将在下一轮对话生效"

### Requirement: Override reset on session close
Session 的权限覆盖 SHALL 在 session 关闭或应用重启时重置为无。

#### Scenario: Session close resets override
- **WHEN** session 被关闭
- **THEN** 该 session 的权限覆盖被清除
- **WHEN** 用户重新打开同一 session
- **THEN** 使用全局默认权限

### Requirement: Permission resolver
后端 SHALL 提供 `PermissionResolver` 接口，工具审批路径通过它获取有效 BehaviorConfig。

#### Scenario: Resolver with override
- **WHEN** PermissionResolver.resolve(session_id) 被调用
- **AND** 该 session 有权限覆盖（preset_id = "auto-edit"）
- **THEN** 返回 "auto-edit" 预设对应的 BehaviorConfig（合并全局非覆盖字段）

#### Scenario: Resolver without override
- **WHEN** PermissionResolver.resolve(session_id) 被调用
- **AND** 该 session 无权限覆盖
- **THEN** 返回全局 AgentConfig.behavior
