## 1. WS API（Gateway + Protocol）

- [x] 1.1 在 `xiaolin-protocol/src/op.rs` 注册 `AutomationsList`、`AutomationsCreate`、`AutomationsUpdate`、`AutomationsDelete`、`AutomationsRuns` 及 `from_typed` 解析
- [x] 1.2 创建 `xiaolin-gateway/src/ws/automations.rs`，实现 `handle_automations_list`（委托 `CronJobStore::list`）
- [x] 1.3 实现 `handle_automations_create`（校验 schedule/action，生成 id，`upsert`，广播 `automations.changed`）
- [x] 1.4 实现 `handle_automations_update`（get + merge + upsert，广播事件）
- [x] 1.5 实现 `handle_automations_delete`（delete + 广播事件）
- [x] 1.6 实现 `handle_automations_runs`（`list_runs(job_id, limit)`）
- [x] 1.7 在 `ws/mod.rs` 添加 `automations.*` 路由；在 `cron_tool.rs` 与 scheduler 完成写入后调用广播 helper
- [x] 1.8 在 `transport.ts` / `api.ts` 添加 `automationsList`、`automationsCreate`、`automationsUpdate`、`automationsDelete`、`automationsRuns` 及 `automations.changed` 监听

## 2. Frontend store

- [x] 2.1 创建 `xiaolin-app/src/lib/stores/automation-store.ts`，定义 `useAutomationStore`（jobs, loading, error, selectedJobId, runs, panelOpen）
- [x] 2.2 实现 `loadJobs`、`createJob`、`updateJob`、`deleteJob`、`fetchRuns` actions
- [x] 2.3 在 WS 连接时订阅 `automations.changed`，实现 created/updated/deleted/run_completed 处理
- [x] 2.4 在 `stores/index.ts` 导出 `useAutomationStore`

## 3. Automation panel UI

- [x] 3.1 创建 `AutomationPanel.tsx` overlay 容器（打开/关闭、Escape、遮罩）
- [x] 3.2 实现 `AutomationList` 表格：name, schedule, status, last run, 行操作按钮
- [x] 3.3 实现空状态与「Create automation」入口
- [x] 3.4 实现 `AutomationForm`（create/edit 共用）：name, schedule, action type, AgentChat/Webhook 字段, enabled, notify_channels
- [x] 3.5 实现删除确认对话框
- [x] 3.6 实现 `AutomationHistory` 执行历史列表（runs 展示）
- [x] 3.7 接入 `useAutomationStore`：面板打开时 loadJobs，操作后刷新
- [x] 3.8 样式对齐 Codex 原型（`--bg-card`、`--card-r`、表格 hover/active）

## 4. Cron expression helper

- [x] 4.1 创建 `CronScheduleHelper` 组件：预设下拉（hourly / daily 9:00 / weekly Mon 9:00 / custom）
- [x] 4.2 预设选择时写入对应五段 cron 表达式到表单
- [x] 4.3 显示人类可读摘要；自定义模式下前端或提交前校验表达式（与 scheduler 一致）

## 5. Sidebar integration

- [x] 5.1 修改 `AppSidebar`：Automations 按钮 onClick 调用 `useAutomationStore.openPanel()`，移除 ComingSoon
- [x] 5.2 在 `AppShell` / layout 根节点挂载 `AutomationPanel`，由 `panelOpen` 控制可见性

## 6. Validation

- [x] 6.1 通过 Agent `cron_tool` 创建 job 后，打开 Automations 面板可见该 job
- [x] 6.2 E2E（Tauri MCP）：侧栏点击 Automations → 列表显示 → 创建/编辑/删除/查看历史 → 关闭 overlay
- [x] 6.3 `cargo clippy -- -D warnings` 与前端 typecheck 通过；确认无 dead_code
