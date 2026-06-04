## Context

XiaoLin 已具备完整的 cron 自动化后端：

| 组件 | 职责 |
|------|------|
| `xiaolin-cron::CronJobStore` | SQLite CRUD、`CronJob` / `CronJobRun` |
| `xiaolin-cron::CronScheduler` | 按 cron 表达式调度执行 |
| `xiaolin-gateway::cron_tool` | Agent 通过 tool 创建/管理 job |
| `xiaolin-gateway::ws/cron.rs` | WS：`cron.list_jobs`、`cron.upsert_job`、`cron.delete_job`、`cron.list_runs` 等 |
| `xiaolin-app::TasksPage` | 旧版全页任务 UI（NavRail「任务」入口），未接入 Codex 侧栏 |

Codex 原型（`docs/prototype-codex-layout.html`）侧栏顶部有 **Automations** 按钮。`layout-overhaul` 的 `app-sidebar` spec 已定义该按钮，当前行为为 ComingSoon。本 change 将其落地为 overlay 管理面板，并补齐用户向 API 与实时同步。

## Goals / Non-Goals

**Goals:**

- 用户可从侧栏 **Automations** 一键打开管理面板，查看所有 cron job（含 Agent 创建的）
- 完整的 CRUD + 启用/禁用 + 执行历史查看
- `automations.*` WS API 与 `automations.changed` 事件，前端 store 自动同步
- Cron 表达式辅助：预设（每小时/每天/每周）+ 自定义输入 + 人类可读描述

**Non-Goals:**

- 重新实现调度器或存储层——直接复用 `CronJobStore` / `CronScheduler`
- 新增动作类型（仅 `AgentChat` / `Webhook`，与现有 `JobAction` 一致）
- 复杂的可视化流程编辑器（if-this-then-that 链式自动化）
- 多用户/多租户权限隔离（单用户桌面应用）
- 立即删除旧 `cron.*` WS 方法——可保留兼容，由实现阶段决定是否 deprecate

## Decisions

### D1: 自动化面板为侧栏触发的 modal/overlay

**选择**：点击侧栏 **Automations** 打开全屏或居中大尺寸 overlay（非独立路由页、不替换主 Chat 区域）。

**替代方案**：
- 全页替换（类似现有 `TasksPage`）→ 破坏 Codex「主区域始终是 Chat」的布局范式
- 右侧 slide-over panel → 与 WorkspacePanel 竞争空间；overlay 更贴近原型「弹出管理」心智

**理由**：与 Plugins、Settings 等「从侧栏唤起浮层」模式一致；用户可在管理自动化后一键关闭回到对话。

### D2: WS API 封装 CronJobStore，命名空间 `automations.*`

**选择**：新增用户向方法名，内部委托 `CronJobStore`：

| 方法 | 映射 |
|------|------|
| `automations.list` | `cron_store.list()` |
| `automations.create` | 新建 `CronJob` + `upsert` |
| `automations.update` | `get` + merge fields + `upsert` |
| `automations.delete` | `cron_store.delete(id)` |
| `automations.runs` | `cron_store.list_runs(job_id, limit)` |

**替代方案**：
- 前端继续用 `cron.*` → 命名偏 Agent/内部，与 UI「Automations」不一致
- REST 端点 → 前端已统一 WS，增加双通道维护成本

**理由**：语义清晰；`create`/`update` 拆分比 `upsert` 更利于表单校验；现有 `cron.*` handler 可复用实现。

### D3: 创建/编辑表单字段

**选择**：表单包含：

- **name**（必填，显示名）
- **schedule**（cron 五段表达式，必填；配合 D5 辅助输入）
- **action**（`AgentChat`: agent_id + message + 可选 session_id；`Webhook`: url + method + body）
- **enabled**（布尔，默认 true）
- **notify_channels**（`NotifyChannel[]`：channel_id + target_id + target_type）

编辑时预填现有 job；`update` 仅提交变更字段或完整 job 对象（实现时二选一，推荐完整对象简化合并逻辑）。

### D4: `automations.changed` WS 事件

**选择**：在以下时机向所有已连接客户端广播 `{ type: "automations.changed", data: { jobId?: string, action: "created"|"updated"|"deleted"|"run_completed" } }`：

- `automations.create` / `update` / `delete` 成功
- Agent `cron_tool` 写入 store 后
- Job 执行完成（`CronJobRun` 写入后，可选 `run_completed`）

前端 `useAutomationStore` 收到后：对 `created|updated|deleted` 重新 `list` 或增量 patch；对 `run_completed` 若当前选中该 job 则刷新 runs。

**替代方案**：仅依赖前端轮询 → 无法反映 Agent 侧创建的任务

### D5: Cron 表达式简易构建器

**选择**：UI 提供预设下拉 + 自定义输入：

| 预设 | 表达式示例 | 描述 |
|------|-----------|------|
| Every hour | `0 * * * *` | 每小时整点 |
| Daily (9:00) | `0 9 * * *` | 每天 09:00 |
| Weekly (Mon 9:00) | `0 9 * * 1` | 每周一 09:00 |
| Custom | 用户输入 | 显示下一行人类可读摘要（如 "At 09:00 every Monday"） |

人类可读摘要可用轻量解析（常见模式）或显示原始表达式；完整 cron 校验调用后端/库（如 `cron` crate 解析失败则表单报错）。

### D6: 与现有 TasksPage 的关系

**选择**：本 change 新建 `AutomationPanel` 组件；`TasksPage` 标记为 legacy，实现完成后从 NavRail 移除或重定向到同一 store（非本 change 必须项，在 tasks.md 验证项中确认不重复维护两套 UI）。

## Architecture Sketch

```
┌─────────────┐     automations.*      ┌──────────────────┐
│ AppSidebar  │ ──────────────────────▶│ ws/automations   │
│ [Automations]│                        │   handlers       │
└──────┬──────┘                        └────────┬─────────┘
       │ open overlay                           │
       ▼                                        ▼
┌─────────────┐     subscribe          ┌──────────────────┐
│ Automation  │◀── automations.changed│ CronJobStore     │
│ Panel       │                        │ (SQLite)         │
└──────┬──────┘                        └────────┬─────────┘
       │ useAutomationStore                     │
       ▼                                        ▼
┌─────────────┐                        ┌──────────────────┐
│ Zustand     │                        │ CronScheduler    │
│ store       │                        │ + cron_tool      │
└─────────────┘                        └──────────────────┘
```

## Risks / Open Questions

- **表达式校验**：五段 cron 与六段（秒）是否统一？需与 `CronScheduler` 解析器对齐
- **AgentChat 表单的 agent 选择**：是否列出当前已配置 agent？空 agent_id 时的错误提示
- **Webhook 测试**：是否在表单提供「发送测试请求」——建议 Non-Goal，后续迭代
- **并发编辑**：多窗口同时编辑同一 job 的最后写入胜出；可接受（桌面单用户）
