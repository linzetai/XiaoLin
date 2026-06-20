## Milestone 规划

| Milestone | 包含 Groups | 任务数 | 预估 | 评分目标 |
|-----------|------------|--------|------|----------|
| **M1: 基础修复 + 即时提升** | 1, 2, 6, 8 | 19 | ~3d | 30→50 |
| **M2: 流式渲染 + 视觉完善** | 3, 4, 7 | 25 | ~5d | 50→70 |
| **M3: 审批增强 + 执行追踪** | 5, 10 | 24 | ~6d | 70→85 |
| **M4: 容错 + 发现 + 测试** | 11, 12, 9 | 29 | ~4d | 85→95+ |

**依赖关系**: M1 → M2 → M3 → M4 (整体顺序)
**最高风险**: 3.2-3.3 (JSON 状态机), 5.10 (clearContext session), 10.4 (路径匹配准确率)

---

## 1. Protocol and Bug Fixes

- [ ] 1.1 Add AgentEvent::PlanDelta variant in event.rs
- [ ] 1.2 Add AgentStep::PlanDelta in agent_step.rs
- [ ] 1.3 Add ClientOp::ExecutionRejectPlan in op.rs
- [ ] 1.4 Fix PlanFileStore::new(None) in llm_call.rs → use shared instance
- [ ] 1.5 Fix PlanFileStore::new(None) in end_turn.rs → use shared instance
- [ ] 1.6 Add optional `content` field to PlanFileUpdate event
- [ ] 1.7 Update frontend TypeScript types for PlanDelta + PlanFileUpdate content

## 2. Phase 1 Content Push

- [ ] 2.1 Backend: populate PlanFileUpdate.content after write_file completes
- [ ] 2.2 Frontend: PlanPanel uses content field from WS event without HTTP refetch
- [ ] 2.3 Frontend: PlanPanel fallback to getPlanFile() when content field absent

## 3. Phase 2 Streaming Interceptor

- [ ] 3.1 Create plan_arg_interceptor.rs module
- [ ] 3.2 Implement JSON state machine for key/value boundary tracking
- [ ] 3.3 Implement JSON string unescaping (\n, \t, \", \\, \uXXXX)
- [ ] 3.4 Implement path-first streaming (path known → emit content deltas)
- [ ] 3.5 Implement content-first buffer (200 char cap, flush on path match)
- [ ] 3.6 Unit tests: all escape sequences, cross-chunk boundaries, path order
- [ ] 3.7 Integrate PlanArgInterceptor into llm_call.rs tool call accumulation loop

## 4. Frontend Streaming

- [ ] 4.1 Plan delta state in stream-store.ts (buffer, stableContent, isStreaming)
- [ ] 4.2 plan_delta WS handler in useMessageStreamChat.ts
- [ ] 4.3 PlanPanel line-commit strategy (commit on \n, buffer otherwise)
- [ ] 4.4 PlanPanel streaming cursor (2px blink animation, 0.8s cycle)
- [ ] 4.5 PlanPanel new-line fadeSlideIn animation (0.15s ease-out)
- [ ] 4.6 PlanPanel auto-scroll with user-interrupt detection
- [ ] 4.7 PlanPanel stream-complete → static transition (remove cursor, use final content)

## 5. Approval Gate Enhancement

- [ ] 5.1 PlanApprovalCard: Plan 全文 Markdown 默认展开 (max-h 600px)
- [ ] 5.2 PlanApprovalCard: 「开始实施」按钮 + 发送引导消息
- [ ] 5.3 PlanApprovalCard: 「清除上下文并实施」按钮 + 上下文使用率 % 显示
- [ ] 5.4 PlanApprovalCard: 「继续规划」按钮
- [ ] 5.5 PlanApprovalCard: 「给反馈后继续」→ 展开多行输入框 (Enter/Shift+Enter/Esc)
- [ ] 5.6 PlanApprovalCard: 「在编辑器中打开」→ Tauri opener 集成
- [ ] 5.7 PlanApprovalCard: 「记住选择」复选框 + localStorage 持久化
- [ ] 5.8 PlanApprovalCard: 审批后卡片状态更新 (已审批 + 操作描述 + 按钮禁用)
- [ ] 5.9 Backend: approvePlan API 新增 feedback 参数 → 注入用户消息
- [ ] 5.10 Backend: approvePlan API 新增 clearContext 参数 → 新建 session + plan 注入
- [ ] 5.11 Backend: gateway reject_plan handler (保持 Plan + 发送反馈消息)
- [ ] 5.12 PlanApprovalCard: 使用 --plan-tint-* 替代 var(--tint)

## 6. Plan 色系统

- [ ] 6.1 index.css: 定义 --plan-tint-* CSS tokens (light theme)
- [ ] 6.2 index.css: 定义 --plan-tint-* CSS tokens (dark theme)
- [ ] 6.3 ModeSelector: Plan 选项色从 oklch(56% 0.18 310) 改为 var(--plan-tint)
- [ ] 6.4 Plan Banner: 样式从 var(--tint) 改为 var(--plan-tint-*)
- [ ] 6.5 PlanPanel: 头部样式从 var(--tint) 改为 var(--plan-tint-*)

## 7. Mode Entry and Visual

- [ ] 7.1 Synthetic user message on UI mode switch (ModeSelector → Plan)
- [ ] 7.2 Plan mode message left border (2px --plan-tint-border)
- [ ] 7.3 Plan mode message badge (🧭 Plan, 8px font, --plan-tint)
- [ ] 7.4 Composer border color: transition to --plan-tint-border in Plan mode (300ms)
- [ ] 7.5 Mode switch animation: Plan Banner slideDown/fadeOut (200ms/150ms)
- [ ] 7.6 PlanPanel auto-open on first plan_file_update (with slideFromRight)
- [ ] 7.7 PlanPanel no auto-open after user manual close
- [ ] 7.8 enter_plan_mode tool result: 简洁状态行 (● 已进入 Plan 模式)
- [ ] 7.9 exit_plan_mode tool result (no plan): 简洁状态行 (● 已退出 Plan 模式)
- [ ] 7.10 write_file plan: 工具结果简化为「方案已更新」hint + 查看链接
- [ ] 7.11 Plan mode Composer placeholder: "探索代码、讨论方案...（只读模式）"

## 8. Prompt Upgrade

- [ ] 8.1 mode_attachments.rs: plan_full_en rewrite (three-phase workflow)
- [ ] 8.2 mode_attachments.rs: plan_sparse_en rewrite (concise reminder)
- [ ] 8.3 Plan mode tool prompts update (enter/exit_plan_mode)
- [ ] 8.4 Demote todo_write in plan mode tool profile

## 9. Integration Tests

- [ ] 9.1 E2E: PlanDelta events flow (backend → WS → frontend)
- [ ] 9.2 E2E: Phase 1 content push (plan_file_update with content)
- [ ] 9.3 E2E: Approval gate 5 options (implement, clear+implement, continue, feedback, editor)
- [ ] 9.4 E2E: PlanFileStore path consistency across crate boundaries
- [ ] 9.5 E2E: todo_write unavailability in plan mode
- [ ] 9.6 E2E: Plan color system consistency (--plan-tint applied everywhere)

## 10. Plan Execution Tracking

- [ ] 10.1 Frontend: parsePlanSteps() — 从 `## Changes` 章节自动提取步骤到 checklist
- [ ] 10.2 Frontend: PlanPanel 追踪视图 — checklist + 进度条 + 折叠方案全文
- [ ] 10.3 Frontend: 文件路径模糊匹配逻辑（相对路径 vs 绝对路径尾部匹配）
- [ ] 10.4 Frontend: write_file/edit_file 工具结果事件 → 自动标记步骤完成
- [ ] 10.5 Frontend: 手动点击步骤状态图标切换（○ → ✅ → ○）
- [ ] 10.6 Frontend: 全部步骤完成 → 显示 "🎉 方案已全部实施" + 建议运行测试
- [ ] 10.7 Frontend: PlanPanel 模式切换时保持可见（Agent 模式 + plan 存在时不关闭）
- [ ] 10.8 Backend: compact_boundary_plan_reference — Compact 后重注入 plan 全文
- [ ] 10.9 Backend: sparse_implementation_reminder — 每 5 轮注入 plan 进度提醒
- [ ] 10.10 Backend: approvePlan 审批后注入引导消息（保持上下文 / 清除上下文两种路径）
- [ ] 10.11 Frontend: checklist 无 Changes 章节时降级为纯 Markdown 展示
- [ ] 10.12 E2E: 执行追踪全流程（审批 → checklist 显示 → write_file → 步骤标记 → 完成检测）

## 13. update_plan 结构化步骤工具（替代 Markdown 解析方案）

- [x] 13.1 Protocol: PlanStepStatus 枚举 + PlanStep 结构体 (xiaolin-protocol/src/event.rs)
- [x] 13.2 Protocol: AgentEvent::PlanUpdate 事件变体 + turn_id() 匹配
- [x] 13.3 Protocol: pub use PlanStep, PlanStepStatus (lib.rs)
- [x] 13.4 Backend: UpdatePlanTool 实现 (builtin_tools/update_plan.rs)
- [x] 13.5 Backend: PlanStepStore 内存存储 (update/snapshot)
- [x] 13.6 Backend: 工具注册到 ToolRegistry (mod.rs + builder.rs)
- [x] 13.7 Backend: plan_mode.rs prompt 更新 (update_plan 列入可用工具)
- [x] 13.8 Frontend: PlanStep/PlanStepStatus/PlanUpdateData 类型 (types.ts)
- [x] 13.9 Frontend: "plan_update" 事件注册 (transport.ts + useMessageStreamChat.ts)
- [x] 13.10 Frontend: PlanChecklist 组件 (进度条 + 步骤列表 + 状态图标)
- [x] 13.11 Frontend: StepIcon 组件 (Circle/CircleNotch/CheckCircle)
- [x] 13.12 E2E: dev 测试验证 update_plan → PlanPanel checklist 渲染

## 11. Plan Recovery

- [ ] 11.1 Backend: execution.get_plan_meta RPC handler (查 PlanFileStore + 检查文件 + 查 Registry)
- [ ] 11.2 Backend: infer_mode_from_last_message fallback (从最近消息推断模式)
- [ ] 11.3 Frontend: activateSession 中调用 get_plan_meta hydrate plan/mode 状态
- [ ] 11.4 Frontend: syncBackendData 后 hydrate 当前活跃 session
- [ ] 11.5 Frontend: WS "reconnected" 事件后 hydrate 当前活跃 session
- [ ] 11.6 Frontend: syncSessionsForAgent 不再硬编码 executionMode: "agent"（保留已知值）
- [ ] 11.7 Frontend: PlanPanel 自动恢复 (planFileExists + plan mode → 自动展开)
- [ ] 11.8 Frontend: "未完成方案" 提示卡片 (planFileExists + agent mode → 小提示)
- [ ] 11.9 Backend: plan_mode_reentry attachment 实现 (重入 Plan + 已有 plan → 注入)
- [ ] 11.10 Backend: handle_sessions_delete 中调用 plan 文件 + 索引 + registry 清理
- [ ] 11.11 Backend: PlanFileStore 新增 remove_slug() / has_slug() 方法
- [ ] 11.12 E2E: 页面刷新后 PlanPanel + mode 恢复验证
- [ ] 11.13 E2E: Session 删除后 plan 文件不存在验证

## 12. Plan Nudge / Discovery

- [ ] 12.1 Frontend: usePlanNudge hook (关键词检测 + 复杂度启发式 + 首次使用教育)
- [ ] 12.2 Frontend: containsPlanKeyword() — 中英文关键词匹配 (word boundary)
- [ ] 12.3 Frontend: detectComplexity() — 多维度启发式 (长度 + 列表 + @引用 + 动词)
- [ ] 12.4 Frontend: PlanNudge 内联提示条组件 (plan-tint 样式 + slideDown/fadeOut 动画)
- [ ] 12.5 Frontend: Nudge dismiss 逻辑 (Esc/✕/发送/清空/超时消退)
- [ ] 12.6 Frontend: Nudge localStorage 持久化 (dismissed rules, ever-used, last-shown, education-count)
- [ ] 12.7 Frontend: Nudge 频率控制 (per-session + 全局限制 + 5分钟间隔)
- [ ] 12.8 Frontend: Ctrl+Shift+P 快捷键注册 (MentionInput handleKeyDown)
- [ ] 12.9 Frontend: Plan 模式进入时标记 "xiaolin:plan-mode-ever-used"
- [ ] 12.10 E2E: 关键词输入 → nudge 显示 → 点击切换 → 进入 Plan 模式
