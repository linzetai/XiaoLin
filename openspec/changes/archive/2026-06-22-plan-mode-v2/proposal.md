## Why

XiaoLin 的 Plan 模式体验（自评 30/100）远落后于 Codex（78/100）和 Claude Code（80/100）。核心问题：Plan 产物不能流式呈现（用户等待数秒后才看到完整内容）、审批门控只有「实现/继续」两个选项（缺少拒绝/反馈/清空上下文）、模式入口分裂（UI toggle 和 agent tool 注入不同上下文）、消息流中无视觉区分（plan 模式下的对话看起来和普通模式一模一样）。通过对标 Codex 的流式 Plan 解析器和 Claude Code 的审批门控，可以将体验提升到 80+ 分。

## What Changes

- **流式 Plan 渲染**：新增 `PlanArgInterceptor` 从 LLM 工具参数流中实时提取 plan 文件内容，通过 `AgentEvent::PlanDelta` 事件推送到前端，PlanPanel 实时流式渲染 markdown
- **审批门控增强**：PlanApprovalCard 新增 5 选项（开始实施 / 清除上下文并实施 / 继续规划 / 给反馈后继续 / 在编辑器中打开）+ 记住选择复选框 + Plan 全文默认展开预览 + 反馈文本多行输入框
- **统一模式入口**：UI 模式切换时注入合成用户消息，确保对话上下文与执行模式状态同步
- **Plan 色系统**：新增 `--plan-tint-*` CSS token（teal 色系），与 Agent 蓝色和 Goal 橙色形成三色区分，贯穿所有 Plan UI 元素（banner、面板、审批卡、消息标识、ModeSelector、Composer 边框）
- **模式视觉反馈**：消息流中 Plan 模式的 assistant 消息带左边框和 Plan 徽章、Composer 边框变色、模式切换过渡动画、工具 Badge 增强（enter/exit_plan_mode 专用渲染）
- **提示词升级**：Plan 模式 mode_attachment 升级为三阶段工作流（探索→意图→实现规划），增加探索优先、决策完整性、模式锁定、结束行为约束、两类未知区分、结构化提问等高级指令
- **PlanFileStore 修复**：`llm_call.rs` 和 `end_turn.rs` 中 `PlanFileStore::new(None)` 替换为会话共享实例，消除路径不一致 bug
- **todo_write 与 Plan 统一**：Plan 模式下抑制 `todo_write` 工具（避免两个规划渠道冲突），Plan 步骤即为 todo 项
- **Plan 执行追踪**：审批后 PlanPanel 自动切换到追踪视图（checklist + 进度条），从 `## Changes` 章节自动解析步骤，agent write_file/edit_file 自动标记步骤完成，Compact 后自动重注入 plan_file_reference，实施期每 N 轮注入 sparse plan reminder
- **Plan 恢复与持久化**：Session 激活时主动 hydrate plan 元数据（新增 `execution.get_plan_meta` RPC），刷新/重连后自动恢复 PlanPanel 和模式状态，重入 Plan 模式时注入 reentry attachment 引导修改而非从头规划，Session 删除时清理 plan 文件
- **Plan 自动发现**：Composer 输入含 plan 关键词或具有复杂任务特征时显示内联 Plan Nudge 提示条，新增 Ctrl+Shift+P 快捷键一键切换 Plan 模式，首次使用教育提示

## Capabilities

### New Capabilities
- `plan-streaming`: 从 LLM 工具参数流中实时提取 plan 内容并推送到前端进行流式渲染
- `plan-approval-v2`: 增强的 Plan 审批门控，支持 5 选项（实施 / 清上下文 / 继续 / 反馈 / 编辑器打开）、记住选择、全文预览、多行反馈输入
- `plan-mode-visual`: Plan 色系统（`--plan-tint-*` CSS token），消息标识（左边框+徽章），Composer 边框变色，模式切换动画，工具 Badge 增强
- `plan-prompt-v2`: 升级的 Plan 模式提示词，三阶段工作流和结构化输出
- `plan-execution-tracking`: 审批后 PlanPanel 追踪视图（自动 checklist + 进度条 + 文件修改自动标记 + Compact 重注入 + 实施期 sparse reminder）
- `plan-recovery`: Session 激活时 plan 元数据 hydrate（get_plan_meta RPC）、PlanPanel 自动恢复、未完成方案提示、reentry attachment、session 删除清理
- `plan-nudge`: Plan 模式自动发现（关键词检测 + 复杂度启发式 + 首次使用教育）+ Ctrl+Shift+P 快捷键切换

### Modified Capabilities
- `plan-approval-gate`: 审批选项从 2 个扩展到 5+1 个，新增拒绝反馈/清空上下文/在编辑器中打开/记住选择，后端 API 新增 feedback 和 clearContext 参数
- `plan-panel`: PlanPanel 从静态渲染改为流式 markdown 渲染
- `plan-file-allowlist`: 修复 PlanFileStore 路径不一致 bug
- `mode-attachments`: Plan 模式 attachment 内容升级为三阶段工作流

## Impact

- **协议层** (`xiaolin-protocol`): 新增 `AgentEvent::PlanDelta`、`AgentStep::PlanDelta`、`ClientOp::ExecutionRejectPlan`
- **Agent runtime** (`xiaolin-agent`): `llm_call.rs` 新增 PlanArgInterceptor 集成、`accumulator.rs` 可能新增拦截器接口、`plan_mode.rs` 修改工具 profile、`mode_attachments.rs` 提示词重写 + reentry/compact_boundary/sparse_reminder 新增
- **Gateway** (`xiaolin-gateway`): `execution.rs` 新增 `execution.reject_plan` 和 `execution.get_plan_meta` RPC 处理、`session.rs` handle_sessions_delete 新增 plan 清理
- **前端** (`xiaolin-app`): `PlanPanel.tsx` 重写为流式渲染 + 追踪视图、`PlanApprovalCard.tsx` 新增选项 UI、`MessageStream.tsx` 新增 Plan 内联块、`stream-store.ts` 新增 plan delta 状态、`useMessageStreamChat.ts` 新增 plan_delta handler、`ComposerCore.tsx` 新增 PlanNudge 组件 + Ctrl+Shift+P 快捷键、`store.ts` 新增 hydratePlanMeta 逻辑
- **Plan 执行追踪**: `PlanPanel.tsx` 新增追踪视图（checklist + 进度条）、`mode_attachments.rs` 新增 compact_boundary_plan_reference 和 sparse plan reminder、`stream-store.ts` 新增 plan step 完成状态追踪
- **现有 spec**: plan-approval-gate、plan-panel、plan-file-allowlist、mode-attachments 需要 delta spec
