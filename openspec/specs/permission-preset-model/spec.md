## ADDED Requirements

### Requirement: Permission preset data model
系统 SHALL 定义 `PermissionPreset` 结构体，包含 `id`（kebab-case 唯一标识）、`name`（显示名称）、`description`（一句话描述）、`icon`（可选 emoji）、`behavior_override`（映射到 BehaviorConfig 的参数子集）。

#### Scenario: Preset structure
- **WHEN** 定义一个权限预设
- **THEN** 预设包含 `id`、`name`、`description`、`behavior_override`
- **AND** `behavior_override` 至少包含 `approval_strategy`、`file_access`、`tools_ask`、`tools_deny` 四个字段

### Requirement: Built-in presets
系统 SHALL 提供 4 个内置预设，不可删除但可被用户自定义覆盖。

#### Scenario: Built-in preset — suggest
- **WHEN** 查询预设 id = "suggest"
- **THEN** 返回预设：name = "Suggest edits"，approval_strategy = "interactive"，file_access = "workspace"，tools_ask = ["write_file", "edit_file", "shell_exec", "mcp_*"]

#### Scenario: Built-in preset — auto-edit
- **WHEN** 查询预设 id = "auto-edit"
- **THEN** 返回预设：name = "Auto edit"，approval_strategy = "interactive"，file_access = "workspace"，tools_ask = ["shell_exec", "mcp_*"]

#### Scenario: Built-in preset — full-auto
- **WHEN** 查询预设 id = "full-auto"
- **THEN** 返回预设：name = "Full auto"，approval_strategy = "auto_approve"，file_access = "full"，tools_ask = []，tools_deny = []

#### Scenario: Built-in preset — plan-only
- **WHEN** 查询预设 id = "plan-only"
- **THEN** 返回预设：name = "Plan only"，approval_strategy = "interactive"，file_access = "workspace"，tools_deny = ["write_file", "edit_file", "shell_exec"]

### Requirement: Preset to BehaviorConfig resolution
系统 SHALL 提供 `resolve_behavior(preset, global_config) → BehaviorConfig` 函数，将预设参数合并到全局 BehaviorConfig 上。

#### Scenario: Preset overrides global config
- **WHEN** 全局 BehaviorConfig 的 file_access = "workspace"
- **AND** 预设的 file_access = "full"
- **THEN** resolve_behavior 返回的 BehaviorConfig 中 file_access = "full"

#### Scenario: Non-overridden fields use global default
- **WHEN** 预设未设置 max_tool_calls_per_turn
- **AND** 全局 BehaviorConfig 的 max_tool_calls_per_turn = 25
- **THEN** resolve_behavior 返回的 BehaviorConfig 中 max_tool_calls_per_turn = 25
