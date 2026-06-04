## ADDED Requirements

### Requirement: permissions.get WS API
后端 SHALL 提供 `permissions.get` WS 方法，返回指定 session 的有效权限信息。

#### Scenario: Get permissions for session
- **WHEN** 收到 `permissions.get { session_id }`
- **THEN** 返回 `{ active_preset_id, active_preset_name, is_override, presets: [...] }`
- **AND** `active_preset_id` 为当前生效的预设 ID
- **AND** `is_override` 标记是否为 session 级覆盖
- **AND** `presets` 包含所有可用预设的完整信息列表

#### Scenario: Get permissions without override
- **WHEN** session 没有权限覆盖
- **THEN** `active_preset_id` 为全局默认预设 ID
- **AND** `is_override` = false

### Requirement: permissions.set WS API
后端 SHALL 提供 `permissions.set` WS 方法，设置指定 session 的权限预设。

#### Scenario: Set valid preset
- **WHEN** 收到 `permissions.set { session_id, preset_id: "auto-edit" }`
- **AND** "auto-edit" 是有效的预设 ID
- **THEN** 设置 session 权限覆盖
- **AND** 返回 `{ success: true, active_preset_id: "auto-edit" }`
- **AND** 广播 `permissions.changed { session_id, preset_id }` WS 事件

#### Scenario: Set invalid preset
- **WHEN** 收到 `permissions.set { session_id, preset_id: "nonexistent" }`
- **THEN** 返回错误 `{ error: "Unknown preset: nonexistent" }`

#### Scenario: Reset to default
- **WHEN** 收到 `permissions.set { session_id, preset_id: null }`
- **THEN** 清除 session 权限覆盖，回退到全局默认
- **AND** 返回 `{ success: true, active_preset_id: "<global_default>" }`

### Requirement: permissions.changed WS event
后端 SHALL 在权限变更时广播 `permissions.changed` 事件。

#### Scenario: Broadcast on change
- **WHEN** session 权限预设被修改
- **THEN** 广播 `permissions.changed { session_id, preset_id, preset_name, is_override }`
- **AND** 所有连接的前端客户端收到事件并更新 UI
