## Why

Plan Mode 目前存在严重的实现矛盾和功能缺失。核心问题是 prompt 告诉 Agent "只有计划文件可以编辑"，但 dispatcher 在 Plan 模式下阻塞**所有** `ToolKind::Edit` 工具（无路径白名单），导致 Agent 无法写入计划文件。此外，审批闸门 `PlanApprovalCard` 的 `onImplement` 回调从未接线，是死代码。Claude Code 已证明 Plan Mode 对复杂多文件任务的价值，我们需要修复已有基础设施并补齐缺失环节。

## What Changes

- **修复 Dispatcher 计划文件写入白名单**：在 `is_blocked_for_tool` 和 `pre_execution_checks` 中为计划文件路径添加例外，允许 Plan 模式下写入/编辑计划文件
- **修复并行执行漏洞**：`execute_unguarded_standalone` 缺少 `ToolKind::Edit` / `Execute` 的 Plan mode 检查
- **接线审批闸门**：`exit_plan_mode` 不再直接切换模式，而是返回待审批状态；前端 `PlanApprovalCard` 的 `onImplement` 回调正确接线，支持用户选择执行模式
- **新增计划面板**：在 chat 侧栏或分屏中展示计划文件内容，支持实时预览和基础编辑
- **统一模式切换广播**：`execution.set_mode`（UI toggle）也通过 stream 广播 `mode_change` 事件，确保多窗口同步
- **执行模式选择 UI**：审批时提供 "自动执行" / "逐步确认" / "继续规划" 选项

## Capabilities

### New Capabilities
- `plan-file-allowlist`: Dispatcher 计划文件路径白名单机制，Plan 模式下仅允许编辑计划文件
- `plan-approval-gate`: 审批闸门 UI，exit_plan_mode 后的执行模式选择交互
- `plan-panel`: 计划文件侧栏/分屏面板，支持 Markdown 预览和基础编辑

### Modified Capabilities
- `connection-detail-view`: 无变更

## Impact

- **后端**：`crates/xiaolin-agent/src/runtime/dispatcher.rs`（白名单逻辑）、`crates/xiaolin-agent/src/builtin_tools/plan_mode.rs`（审批流程重构）、`crates/xiaolin-gateway/src/ws/execution.rs`（事件广播）、`crates/xiaolin-gateway/src/ws/chat.rs`
- **前端**：`PlanApprovalCard.tsx`（接线 + 执行模式选择 UI）、`StepIndicator.tsx`（传递 onImplement）、`StreamFooter.tsx`（计划面板入口）、新增 `PlanPanel.tsx`
- **协议**：`crates/xiaolin-protocol/src/event.rs`（可能需要新的审批事件类型）
- **Prompt**：`crates/xiaolin-agent/src/runtime/prompt_sections/dynamic.rs`（更新计划文件写入指导）
