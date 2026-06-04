## Why

用户无法通过 UI 查看、管理或监控自动化任务。Agent 通过 `cron_tool` 创建的定时任务对用户不可见；即便存在 `TasksPage` 组件，也未接入 Codex 布局侧栏的 **Automations** 入口（当前为 ComingSoon 占位）。原型图 `docs/prototype-codex-layout.html` 在侧栏顶部操作区提供了 **Automations** 按钮，用户期望在此统一管理定时自动化（cron jobs）。

底层能力已具备：`xiaolin-cron` 提供 `CronJobStore`（SQLite CRUD + 执行历史）、`CronScheduler` 调度执行、`cron_tool.rs` 供 Agent 创建任务；Gateway 已有 `cron.list_jobs` / `cron.upsert_job` 等 WS 方法，但缺少面向用户的 `automations.*` 命名空间、实时变更事件，以及符合新布局的自动化管理面板。

## What Changes

- **侧栏 Automations 入口**：`AppSidebar` 顶部操作区增加 **Automations** 按钮（位于 Plugins 与 Pinned 分组之间），点击打开自动化管理面板 overlay
- **自动化管理面板**：列表展示名称、调度表达式、状态、上次运行；支持启用/禁用、创建、编辑、删除；每项可查看执行历史
- **创建/编辑表单**：名称、cron 调度（含可读性辅助与预设）、动作类型（AgentChat / Webhook）、通知渠道
- **执行历史查看器**：按 job 展示 `CronJobRun` 记录（时间、状态、输出/错误）
- **WS API**：新增 `automations.list` / `create` / `update` / `delete` / `runs` 及 `automations.changed` 广播事件（封装现有 `CronJobStore`）
- **前端 Store**：`useAutomationStore`（Zustand）管理列表、加载态、选中 job、CRUD 与 WS 同步

## Capabilities

### New Capabilities

- `automation-panel`: 自动化管理面板组件（列表、表单、历史、空状态）
- `automation-websocket-api`: 前端可调用的 `automations.*` WS 方法与 `automations.changed` 事件
- `automation-store`: 前端 `useAutomationStore` Zustand store，对接 WS API 与实时更新

### Modified Capabilities

- `app-sidebar`: 侧栏增加 Automations 按钮，触发自动化面板 overlay（替换 ComingSoon 占位行为）

## Impact

- **后端**：
  - `xiaolin-gateway/src/ws/`：新增或扩展 `automations` handler（可复用 `ws/cron.rs` 逻辑，委托 `CronJobStore`）
  - `xiaolin-protocol/src/op.rs`：注册 `automations.*` 客户端操作
  - `CronScheduler` / `cron_tool.rs`：在 job 变更时广播 `automations.changed`
- **前端**：
  - 新增 `AutomationPanel` 及相关子组件（列表、表单、历史）
  - 新增 `useAutomationStore`、`CronScheduleHelper`（预设 + 自定义表达式）
  - 修改 `AppSidebar`：Automations 按钮 → 打开 overlay
  - 现有 `TasksPage` / `transport.ts` 中 `cron.*` 调用可逐步迁移至 `automations.*`（实现阶段决策）
- **数据**：无 schema 变更；复用 `CronJob` / `CronJobRun` 表结构
- **依赖**：`layout-overhaul` 的 `app-sidebar` spec（侧栏结构）；与 `plugin-ui` 无直接冲突
