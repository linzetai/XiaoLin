## ADDED Requirements

### Requirement: 拒绝并反馈操作
PlanApprovalCard SHALL 提供「给反馈后继续」选项，允许用户在多行文本输入框中输入修改意见，反馈作为用户消息发送到 agent，agent 继续在 Plan 模式下修改 plan。

#### Scenario: 用户拒绝 plan 并提供反馈
- **WHEN** 用户在 PlanApprovalCard 点击「给反馈后继续」
- **THEN** SHALL 展开多行文本输入框（至少 3 行高），用户提交后该文本作为用户消息发送，execution 模式保持 Plan
- **THEN** 输入框下方 SHALL 有「发送反馈」和「取消」两个按钮

#### Scenario: 用户拒绝但不提供反馈
- **WHEN** 用户点击「给反馈后继续」后提交空文本
- **THEN** SHALL 发送默认反馈消息 "请修改规划方案"，模式保持 Plan

#### Scenario: 反馈输入框的键盘快捷键
- **WHEN** 用户在反馈输入框中按 Enter
- **THEN** SHALL 提交反馈（等同点击「发送反馈」）
- **WHEN** 用户按 Shift+Enter
- **THEN** SHALL 插入换行（多行输入）
- **WHEN** 用户按 Escape
- **THEN** SHALL 取消反馈（收起输入框）

### Requirement: 清空上下文实现操作
PlanApprovalCard SHALL 提供「清除上下文并实施」选项，清空当前对话历史，以 plan 全文作为新会话的首条消息开始实现。此选项参考 Codex 的 "Yes, clear context and implement" 设计。

#### Scenario: 用户选择清空上下文实现
- **WHEN** 用户点击「清除上下文并实施」
- **THEN** 对话历史 SHALL 被清空，新的用户消息 "请按照以下计划实现:\n\n{plan_content}" 被发送，execution 模式切换到 Agent

#### Scenario: 显示上下文使用率提示
- **WHEN** PlanApprovalCard 显示「清除上下文并实施」选项
- **THEN** SHALL 在选项旁以 dimColor 显示当前上下文使用百分比（如 "已用 67%"），帮助用户判断是否需要清除

### Requirement: 在编辑器中打开 plan 文件
PlanApprovalCard SHALL 提供「在编辑器中打开」选项，使用 Tauri opener 插件在外部编辑器中打开 plan 文件。此选项参考 Claude Code 的 Ctrl+G 外部编辑功能。

#### Scenario: 用户选择在编辑器中打开
- **WHEN** 用户点击「在编辑器中打开」
- **THEN** SHALL 使用 Tauri shell opener 在系统默认编辑器中打开 plan 文件路径
- **THEN** 审批卡片 SHALL 保持显示，不因此操作消失

### Requirement: Plan 全文 Markdown 预览
PlanApprovalCard SHALL 内嵌 plan 全文的 Markdown 渲染预览，默认展开（不同于当前的折叠式）。

#### Scenario: 审批卡片自动加载 plan 内容
- **WHEN** PlanApprovalCard 渲染且 plan 文件存在
- **THEN** SHALL 自动加载并展示 plan 全文 Markdown 渲染
- **THEN** 预览区域 SHALL 有 max-height（600px）和 overflow-y-auto，长 plan 可滚动

#### Scenario: plan 内容来源
- **WHEN** exit_plan_mode 工具结果中含 plan 预览文本
- **THEN** SHALL 优先使用内联预览文本（避免额外 HTTP 请求）
- **WHEN** 内联预览不完整（被截断）
- **THEN** SHALL 通过 getPlanFile() 获取完整内容

### Requirement: 记住选择自动审批
PlanApprovalCard SHALL 提供「记住选择」复选框，启用后后续 plan 审批 SHALL 自动以上次选择的方式处理。

#### Scenario: 记住选择后自动审批
- **WHEN** 用户勾选「记住选择」并选择「开始实施」
- **THEN** 后续同一 session 中的 plan 审批 SHALL 自动以「开始实施」方式处理，不再显示审批卡片

#### Scenario: 重置记住选择
- **WHEN** 用户在设置中或通过 PlanPanel 中的开关取消自动审批
- **THEN** 后续 plan 审批 SHALL 恢复为手动审批

#### Scenario: 记住选择不适用于反馈操作
- **WHEN** 用户勾选「记住选择」
- **THEN**「给反馈后继续」选项 SHALL 不受记住选择影响（因为每次反馈内容不同）

### Requirement: 审批后卡片状态更新
PlanApprovalCard 在用户审批后 SHALL 更新为「已审批」状态，显示所选操作和时间戳。

#### Scenario: 审批后显示已完成状态
- **WHEN** 用户完成 plan 审批（任一选项）
- **THEN** PlanApprovalCard SHALL 更新标题为「已审批」，显示所选操作描述（如 "已开始实施" / "已给反馈" / "继续规划"），禁用所有按钮

#### Scenario: 已审批卡片保留 plan 预览
- **WHEN** 审批完成后 PlanApprovalCard 更新为已审批状态
- **THEN** plan 预览区域 SHALL 保留，可折叠/展开回顾

### Requirement: 审批卡片使用 plan 色系统
PlanApprovalCard SHALL 使用统一的 `--plan-tint` 色 token，与 PlanPanel、plan banner 保持视觉一致。

#### Scenario: 审批卡片色彩
- **THEN** 卡片左边框 SHALL 使用 `var(--plan-tint)` 而非 `var(--tint)`
- **THEN** 卡片背景 SHALL 使用 `var(--plan-tint-soft)` 而非 `color-mix(tint 4%)`
- **THEN** 图标和标题 SHALL 使用 `var(--plan-tint)` 而非 `var(--tint)`

## Implementation Reference

### 与竞品的审批流对比

| 维度 | XiaoLin (当前) | Codex | Claude Code | XiaoLin (目标) |
|------|------------|-------|-------------|----------------|
| 选项数 | 2 | 3 | 4+ | 5 |
| 开始实施 | ✓ | ✓ | ✓ | ✓ |
| 清除上下文 | ✗ | ✓ | ✓ | ✓ |
| 继续规划 | ✓ | ✓ | ✓ | ✓ |
| 反馈输入 | ✗ | ✗ | ✓（CLI 文本） | ✓（多行输入框） |
| 外部编辑 | ✗ | ✗ | ✓（Ctrl+G） | ✓（Tauri opener） |
| Plan 预览 | 折叠式 Markdown | 内联流式 | 全屏 Markdown | 默认展开 Markdown |
| 记住选择 | ✗ | ✗ | ✗ | ✓ |
| 上下文使用率 | ✗ | ✗ | ✗ | ✓ |
| 审批后状态 | 无变化 | 弹窗消失 | 工具结果摘要 | 「已审批」状态 |

### 各选项的技术实现

| 选项 | 前端 API 调用 | 后端处理 | 备注 |
|------|-------------|---------|------|
| 开始实施 | `approvePlan(sid, "agent")` | `session_modes.transition(sid, Agent)` → mode_change 事件 | 参考 Codex：发送 "请按照规划方案开始实现" 用户消息 |
| 清除上下文并实施 | `approvePlan(sid, "agent", {clearContext: true})` | 新建 session → 注入 plan 内容 → 切 Agent | 参考 Codex ClearUiAndSubmitUserMessage |
| 继续规划 | `approvePlan(sid, "plan")` | 保持 Plan 模式 | 无额外消息 |
| 给反馈后继续 | `approvePlan(sid, "plan", {feedback: "..."})` | 保持 Plan → 将 feedback 作为 user message 发送 | 参考 Claude Code "No, with feedback" |
| 在编辑器中打开 | `tauriOpener.open(planFilePath)` | 无 | 本地操作 |

### PlanApprovalCard 布局参考

```
┌──────────────────────────────────────────────────────────────┐
│  🧭 方案规划完成                             ~/plans/xxx.md │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│  ┌────── plan 预览（react-markdown，max-h 600px）──────────┐ │
│  │  ## Context                                             │ │
│  │  ...                                                    │ │
│  │  ## Approach                                            │ │
│  │  ...                                                    │ │
│  │  ## Changes                                             │ │
│  │  ...                                                    │ │
│  └─────────────────────────────────────────────────────────┘ │
│                                                              │
├──────────────────────────────────────────────────────────────┤
│  第一行按钮（主要操作）：                                     │
│  [✓ 开始实施]  [✓ 清除上下文并实施 (已用 67%)]               │
│                                                              │
│  第二行按钮（次要操作）：                                     │
│  [↻ 继续规划]  [✎ 给反馈后继续]  [↗ 在编辑器中打开]          │
│                                                              │
│  [ ] 记住选择                                                │
└──────────────────────────────────────────────────────────────┘
```

### 反馈输入框展开态

```
┌──────────────────────────────────────────────────────────────┐
│  ✎ 修改意见                                                 │
│  ┌────────────────────────────────────────────────────────┐  │
│  │  textarea（min 3 行，auto-grow）                       │  │
│  │  placeholder: "描述需要修改的内容..."                   │  │
│  └────────────────────────────────────────────────────────┘  │
│  [发送反馈 ↩]  [取消]                                        │
│  提示: Enter 发送，Shift+Enter 换行，Esc 取消                │
└──────────────────────────────────────────────────────────────┘
```
