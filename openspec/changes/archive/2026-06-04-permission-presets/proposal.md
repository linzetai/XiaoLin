## Why

当前权限控制能力丰富（`BehaviorConfig` 有 `approval_strategy`、`file_access`、`tools_ask/deny/allow` 等 12+ 个参数，`PermissionRuleEngine` 支持精细规则链），但只能通过配置文件修改，前端仅有 Settings 中的 `SecurityTab`（4 个预设执行模式）和逐条审批弹窗。原型图中 InputBar 有明确的 "Default permissions ▾" 选择器，用户需要在对话中便捷切换权限策略，而非每次都去 Settings 修改配置文件。

## What Changes

- **新增权限预设（Permission Preset）概念**：将 `BehaviorConfig` 中的多个参数打包为命名预设（如 "Suggest edits" / "Auto edit" / "Full auto"），用户通过单一选择器切换
- **InputBar 权限选择器**：在输入框工具栏增加 "🔒 权限 ▾" 下拉选择器，实时切换当前 session 的权限策略
- **Per-session 权限覆盖**：权限预设可在 session 粒度覆盖全局默认值，不影响其他 session
- **权限状态同步**：权限变更通过 WS API 实时同步到后端 `BehaviorConfig`
- **审批卡片增强**：将当前简单的 approve/deny 弹窗升级为更丰富的审批面板，显示操作详情、风险等级、影响范围

## Capabilities

### New Capabilities
- `permission-preset-model`: 权限预设数据模型，定义预设与 BehaviorConfig 参数的映射关系
- `permission-selector`: InputBar 中的权限选择器组件，支持预设切换和自定义配置入口
- `session-permission-override`: Per-session 权限覆盖机制，允许单个 session 使用不同于全局的权限策略
- `permission-websocket-api`: 权限相关 WS API（获取/设置 session 权限、获取预设列表）

### Modified Capabilities
- `chat-input-bar`: InputBar 工具栏增加权限选择器组件
- `approval-ux`: 审批卡片增强，显示操作详情和风险等级

## Impact

- **后端**：
  - `xiaolin-core/src/agent_config.rs`：新增 `PermissionPreset` 结构体和预设→BehaviorConfig 映射
  - `xiaolin-gateway/src/ws/`：新增 `permissions.rs` handler
  - `xiaolin-agent/src/runtime/`：支持运行时 BehaviorConfig 动态切换
- **前端**：
  - 新增 `PermissionSelector` 组件
  - `usePermissionStore` (Zustand) 管理 per-session 权限状态
  - 修改 InputBar 集成选择器
  - 增强审批卡片 UI
- **配置**：预设定义可来自内置默认 + 用户自定义（`.xiaolin/permissions.json`）
