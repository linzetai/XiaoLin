## Context

Plan Mode 的基础设施已经存在：`ExecutionModeState` per-session 模式管理、`PlanFileStore` 计划文件持久化、`enter_plan_mode`/`exit_plan_mode` 工具、`PlanApprovalCard` 前端组件、`mode_change`/`plan_file_update` 事件协议。

但存在三类问题：
1. **Bug**：Dispatcher 在 Plan 模式下阻塞所有 `ToolKind::Edit` 工具，无计划文件路径白名单；`execute_unguarded_standalone` 缺少 Plan mode 检查
2. **Dead code**：`PlanApprovalCard.onImplement` 从未传入；`PlanFileStore::write_plan` 零调用方
3. **功能缺失**：无审批闸门交互、无计划面板、UI toggle 不广播 stream 事件

## Goals / Non-Goals

**Goals:**
- 修复 Plan 模式下计划文件写入被阻塞的 bug
- 修复并行工具执行绕过 Plan mode 检查的漏洞
- 接线审批闸门：`exit_plan_mode` 后用户可选择执行模式
- 新增计划面板：侧栏展示计划文件内容，支持预览
- 统一模式切换广播，确保多窗口同步

**Non-Goals:**
- 不实现 Ultraplan（云端审阅），这是 Claude Code 特有的云基础设施功能
- 不实现模型路由（opusplan 模式），当前单模型架构暂不支持
- 不实现 VS Code 级别的 inline comments，我们是 Tauri 桌面应用
- 不改变 `PlanFileStore` 的持久化策略（仍然 `~/.xiaolin/plans/`）

## Decisions

### D1: 计划文件白名单 — 在 Dispatcher 层添加路径检查

**方案选择：** 在 `is_blocked_for_tool` 中增加路径参数检查，当目标路径是 `PlanFileStore::plan_path()` 时放行 `Edit` 类工具。

**替代方案 A**：注册一个专用 `write_plan_file` 工具（`ToolKind::Think`）绕过 Edit 检查。
- 优点：隔离性好
- 缺点：Agent 已有 `write_file`/`edit_file`，新增工具增加 prompt 复杂度

**替代方案 B**：在 `pre_execution_checks` 中为特定路径前缀开放。
- 优点：灵活
- 缺点：安全风险高，可能意外放行非计划文件

**选择 D1 的理由：** 最小改动，精确匹配计划文件路径，不增加新工具。需要把当前 session 的 plan path 注入到 `DispatchContext` 中。

**实现要点：**
- `DispatchContext` 新增 `plan_file_path: Option<PathBuf>` 字段
- `is_blocked_for_tool` 增加签名参数或查询 `DispatchContext`
- `pre_execution_checks` 在 `ToolKind::Edit` 拦截处，检查 tool arguments 中的 `path` 是否匹配 `plan_file_path`
- 仅匹配 `write_file` 和 `edit_file` 两个工具的 `path` 参数

### D2: 并行执行修复 — 在 `execute_unguarded_standalone` 添加 Edit/Execute 检查

当前 `execute_unguarded_standalone` 只对 `shell_exec` 做 readonly 检查。需要加入与 `pre_execution_checks` 一致的 `ToolKind::Edit`/`Execute` 阻塞逻辑。

**实现要点：**
- 在 `execute_unguarded_standalone` 中加入 `tool_kind` 查询
- 在 Plan 模式下阻塞 `ToolKind::Edit`（除计划文件白名单）和 `ToolKind::Execute`（除 `shell_exec`）

### D3: 审批闸门 — 改为 "pending approval" 中间状态

**当前流程：** `exit_plan_mode` → 直接 `transition(Agent)` → 返回结果

**新流程：** `exit_plan_mode` → 不切换模式 → 返回 `PlanApprovalPending` 状态 → 前端显示审批 UI → 用户选择 → 前端调用 `execution.approve_plan` → 后端切换模式

**实现要点：**
- `exit_plan_mode` 工具不再调用 `transition(Agent)`
- 新增 `ToolResult.metadata` 中携带 `{ "approval_pending": true, "plan_path": "..." }`
- 前端 `PlanApprovalCard` 检测 `metadata.approval_pending`，渲染执行模式选择 UI
- 新增 WS RPC `execution.approve_plan`，参数 `{ sessionId, mode: "agent" | "auto" }`
- 后端 `handle_approve_plan` 执行 `transition(Agent)`，广播 `mode_change`
- 如果用户选"继续规划"，不调用 approve，保持 Plan 模式

**替代方案：** 前端拦截 mode_change 事件并弹出审批 UI（不修改后端 exit_plan_mode）。
- 缺点：tool result 已经包含"All tools now available"文案，暗示切换已完成

### D4: 计划面板 — StreamFooter banner 可展开为侧栏面板

**方案选择：** 点击 StreamFooter 的 plan banner 可展开一个侧栏面板（`PlanPanel`），显示计划文件 Markdown 预览。

**UI 设计：**
- 默认：banner 显示路径 + 未创建/已创建状态
- 点击 banner → 右侧展开 `PlanPanel`（宽度 ~360px）
- 面板内容：Markdown 渲染 + 只读预览
- 面板底部：编辑按钮（跳转到 Agent 模式 write_file）/ 关闭按钮
- 面板在 Agent 模式下也可查看（如果有计划文件）

**数据源：** `transport.getPlanFile(sessionId)` → WS `execution.get_plan`

**替代方案 A：** 分屏编辑器（左 chat + 右 plan file）— 过于复杂，需要集成代码编辑器组件。
**替代方案 B：** 弹窗展示 — `PlanApprovalCard` 已经做了，但只在 `exit_plan_mode` 后才出现。

### D5: 模式切换事件统一 — `execution.set_mode` 广播到 stream

当前 `handle_chat_set_mode`（agent tool 路径）广播 `mode_change`，但 `handle_execution_set_mode`（UI toggle 路径）不广播。

**修复：** 在 `handle_execution_set_mode` 成功后，通过 `bg_tx` 广播 `mode_change` 事件（与 `handle_chat_set_mode` 一致）。

## Risks / Trade-offs

- **[Risk] 计划文件白名单路径匹配可能被绕过** → Mitigation: 使用 `canonicalize` 后精确匹配 `PlanFileStore::plan_path()` 返回的路径，不允许前缀匹配
- **[Risk] 审批闸门改为中间状态后，Agent 可能困惑于"exit 了但模式没变"** → Mitigation: `exit_plan_mode` 的返回文案明确说"等待用户审批"，prompt guidance 也更新
- **[Risk] PlanPanel 增加了前端复杂度** → Mitigation: 最小实现，只做 Markdown 预览 + getPlanFile，不做编辑器
- **[Trade-off] 审批闸门需要新增 WS RPC** → 收益大于成本，这是 Plan Mode 的核心交互
