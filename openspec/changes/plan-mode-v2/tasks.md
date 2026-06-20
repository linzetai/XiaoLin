## Milestone 规划 (Revised — post update_plan)

| Milestone | 包含内容 | 任务数 | 预估 | 评分目标 |
|-----------|---------|--------|------|----------|
| **M1: 审批增强** | Approval Gate v2 + reject_plan | 10 | ~3d | 50→70 |
| **M2: 视觉完善** | Plan 色彩系统 + Mode 切换动画 | 16 | ~3d | 70→80 |
| **M3: 状态恢复** | Plan Recovery + 持久化 | 13 | ~3d | 80→88 |
| **M4: 发现 + Prompt + 测试** | Nudge + Prompt 升级 + E2E | 14 | ~3d | 88→95+ |
| **执行追踪增强** | 可与 M1-M2 交叉进行 | 8 | ~2d | — |

**依赖关系**: M1 → M2 (色彩被审批卡片使用) → M3 → M4
**已完成基础**: update_plan 结构化步骤, PlanArgInterceptor 流式推送, PlanPanel Markdown+Checklist, 基础审批 (2 按钮)

---

## DONE: 已完成的基础设施 (Groups 1-4, 13)

以下任务已在 update_plan 实现和之前的 Milestone 1 中完成：

- [x] ~~1.1~~ AgentEvent::PlanDelta — 已存在于 event.rs
- [x] ~~1.2~~ AgentStep::PlanDelta — 已存在
- [x] ~~1.4~~ PlanFileStore shared instance — 已通过 PlanContext 解决
- [x] ~~1.5~~ PlanFileStore shared instance — 同上
- [x] ~~1.6~~ PlanFileUpdate.content 字段 — 已存在
- [x] ~~1.7~~ Frontend types — 已在 types.ts 中定义
- [x] ~~2.1~~ Backend content push — agent runtime 已发送 content
- [x] ~~2.2~~ Frontend PlanPanel 使用 content — 通过 onWsEvent 直接消费
- [x] ~~2.3~~ Fallback to getPlanFile() — 已实现
- [x] ~~3.1~~ plan_arg_interceptor.rs — 已创建并集成
- [x] ~~3.2~~ JSON state machine — 已实现
- [x] ~~3.3~~ JSON string unescaping — 已实现
- [x] ~~3.4~~ Path-first streaming — 已实现
- [x] ~~3.5~~ Content-first buffer — 已实现
- [x] ~~3.6~~ Unit tests — 已有
- [x] ~~3.7~~ Integrate into llm_call.rs — 已集成
- [x] ~~4.1~~ Plan delta state — 已在 PlanPanel 中
- [x] ~~4.2~~ plan_delta WS handler — 已注册
- [x] ~~4.3~~ Line-commit strategy — 已实现
- [x] ~~4.4~~ Streaming cursor — 已实现
- [x] ~~4.5~~ Line fadeSlideIn — 已实现
- [x] ~~4.6~~ Auto-scroll — 已实现
- [x] ~~4.7~~ Stream-complete transition — 已实现
- [x] 13.1–13.12 update_plan 全部完成

## CANCELLED: 被 update_plan 替代

- [~] ~~10.1~~ parsePlanSteps from Markdown — 被 update_plan 结构化步骤替代
- [~] ~~10.2~~ PlanPanel 追踪视图 (markdown checklist) — 被 PlanChecklist 组件替代
- [~] ~~10.3~~ 文件路径模糊匹配 — 被 update_plan step status 替代
- [~] ~~10.4~~ write_file 事件自动标记 — 被 LLM 主动调 update_plan 替代
- [~] ~~10.11~~ 无 Changes 章节降级 — 不再需要 markdown 解析

---

## M1: 审批增强 (Approval Gate v2)

核心目标：将 2 按钮审批升级为完整 5 选项交互

- [ ] 1.3 Backend: ClientOp::ExecutionRejectPlan in op.rs
- [ ] 5.1 PlanApprovalCard: Plan 全文 Markdown 默认展开 (max-h 600px)
- [ ] 5.3 PlanApprovalCard:「清除上下文并实施」按钮 + 上下文使用率 % 显示
- [ ] 5.5 PlanApprovalCard:「给反馈后继续」→ 展开多行输入框 (Enter/Shift+Enter/Esc)
- [ ] 5.6 PlanApprovalCard:「在编辑器中打开」→ Tauri opener 集成
- [ ] 5.7 PlanApprovalCard:「记住选择」复选框 + localStorage 持久化
- [ ] 5.8 PlanApprovalCard: 审批后卡片状态更新 (已审批 + 操作描述 + 按钮禁用)
- [ ] 5.9 Backend: approvePlan API 新增 feedback 参数 → 注入用户消息
- [ ] 5.10 Backend: approvePlan API 新增 clearContext 参数 → 新建 session + plan 注入
- [ ] 5.11 Backend: gateway reject_plan handler (保持 Plan + 发送反馈消息)

## M2: 视觉完善 (Color System + Mode Entry)

核心目标：Plan 模式有独立视觉身份，切换体验流畅

### 色彩系统

- [ ] 6.1 index.css: 定义 --plan-tint-* CSS tokens (light theme)
- [ ] 6.2 index.css: 定义 --plan-tint-* CSS tokens (dark theme)
- [ ] 6.3 ModeSelector: Plan 选项色从 oklch(56% 0.18 310) 改为 var(--plan-tint)
- [ ] 6.4 Plan Banner: 样式从 var(--tint) 改为 var(--plan-tint-*)
- [ ] 6.5 PlanPanel: 头部样式从 var(--tint) 改为 var(--plan-tint-*)
- [ ] 5.12 PlanApprovalCard: 使用 --plan-tint-* 替代 var(--tint)

### Mode Entry & Visual

- [ ] 7.1 Synthetic user message on UI mode switch (ModeSelector → Plan)
- [ ] 7.2 Plan mode message left border (2px --plan-tint-border)
- [ ] 7.3 Plan mode message badge (Plan, 8px font, --plan-tint)
- [ ] 7.4 Composer border color: transition to --plan-tint-border in Plan mode (300ms)
- [ ] 7.5 Mode switch animation: Plan Banner slideDown/fadeOut (200ms/150ms)
- [ ] 7.6 PlanPanel auto-open on first plan_file_update (with slideFromRight)
- [ ] 7.7 PlanPanel no auto-open after user manual close
- [ ] 7.8 enter_plan_mode tool result: 简洁状态行 (已进入 Plan 模式)
- [ ] 7.9 exit_plan_mode tool result (no plan): 简洁状态行 (已退出 Plan 模式)
- [ ] 7.11 Plan mode Composer placeholder: "探索代码、讨论方案...（只读模式）"

## M3: 状态恢复 (Plan Recovery)

核心目标：页面刷新、断线重连后 plan 状态完整恢复

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

## M4: 发现 + Prompt + 测试

### Prompt 升级

- [ ] 8.1 mode_attachments.rs: plan_full_en rewrite (three-phase workflow, reference update_plan)
- [ ] 8.2 mode_attachments.rs: plan_sparse_en rewrite (concise reminder)
- [ ] 8.3 Plan mode tool prompts update (enter/exit_plan_mode descriptions)
- [ ] 8.4 Demote todo_write in plan mode tool profile

### Plan Discovery (Nudge)

- [ ] 12.1 Frontend: usePlanNudge hook (关键词检测 + 复杂度启发式 + 首次使用教育)
- [ ] 12.2 Frontend: containsPlanKeyword() — 中英文关键词匹配 (word boundary)
- [ ] 12.3 Frontend: detectComplexity() — 多维度启发式 (长度 + 列表 + @引用 + 动词)
- [ ] 12.4 Frontend: PlanNudge 内联提示条组件 (plan-tint 样式 + slideDown/fadeOut 动画)
- [ ] 12.5 Frontend: Nudge dismiss + 频率控制
- [ ] 12.8 Frontend: Ctrl+Shift+P 快捷键注册

### E2E 集成测试

- [ ] 9.1 E2E: update_plan → plan_update event → PlanChecklist 渲染
- [ ] 9.2 E2E: PlanDelta streaming (write_file → plan_delta → PlanPanel 实时更新)
- [ ] 9.3 E2E: Approval gate 5 options 完整交互
- [ ] 9.4 E2E: Plan mode 恢复 (刷新 + 重连)

## 执行追踪增强 (可与 M1-M2 交叉)

这些任务增强 update_plan 的执行追踪体验：

- [ ] 10.5 Frontend: 手动点击步骤状态图标切换（○ → ✅ → ○）
- [ ] 10.6 Frontend: 全部步骤完成 → 显示 "方案已全部实施" + 建议运行测试
- [ ] 10.7 Frontend: PlanPanel 模式切换时保持可见（Agent 模式 + plan 存在时不关闭）
- [ ] 10.8 Backend: compact_boundary_plan_reference — Compact 后重注入 plan 全文
- [ ] 10.9 Backend: sparse_implementation_reminder — 每 5 轮注入 plan 进度提醒
- [ ] 10.10 Backend: approvePlan 审批后注入引导消息（保持上下文 / 清除上下文两种路径）
- [ ] 10.12 E2E: 执行追踪全流程（update_plan → checklist 显示 → 步骤标记 → 完成检测）
- [ ] 7.10 write_file plan: 工具结果简化为「方案已更新」hint + 查看链接
