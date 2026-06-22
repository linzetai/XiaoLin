## MODIFIED Requirements

### Requirement: Plan 审批门控
ExitPlanModeTool 的执行结果 SHALL 触发前端审批门控。当 plan 文件存在时，工具结果通过 `metadata.approval_pending = true` 通知前端渲染 PlanApprovalCard。当 plan 文件不存在时，直接切换到 Agent 模式（无审批）。

#### Scenario: 有 plan 文件时触发审批门控
- **WHEN** exit_plan_mode 执行且 plan 文件存在
- **THEN** 工具结果 SHALL 包含 `metadata: { approval_pending: true, plan_path, plan_exists: true }`
- **THEN** 执行模式 SHALL 保持 Plan（不立即切换），等待用户审批

#### Scenario: 无 plan 文件时跳过审批
- **WHEN** exit_plan_mode 执行且 plan 文件不存在
- **THEN** SHALL 直接切换到 Agent 模式，不设 approval_pending metadata

### Requirement: Plan 审批选项
PlanApprovalCard SHALL 提供以下 5 个审批选项 + 1 个辅助操作，分两行布局：

**主要操作行（绿色/强调色按钮）：**
1. **开始实施** — 切换到 Agent 模式，发送 "请按照规划方案开始实现" 引导消息
2. **清除上下文并实施** — 清空对话历史，以 plan 全文作为首条消息开始新 session

**次要操作行（淡色/轮廓按钮）：**
3. **继续规划** — 保持 Plan 模式，不发送额外消息
4. **给反馈后继续** — 展开反馈输入框，提交后作为用户消息发送，保持 Plan 模式
5. **在编辑器中打开** — 使用 Tauri opener 在外部编辑器中打开 plan 文件

**底部：**
6. **记住选择** — 复选框，启用后后续审批自动以上次选择处理

#### Scenario: 用户选择开始实施
- **WHEN** 用户点击「开始实施」
- **THEN** execution 模式 SHALL 切换到 Agent
- **THEN** SHALL 自动发送用户消息 "请按照规划方案开始实现"，引导 agent 开始执行

#### Scenario: 用户选择清除上下文并实施
- **WHEN** 用户点击「清除上下文并实施」
- **THEN** 对话历史 SHALL 被清空
- **THEN** 以 "请按照以下计划实现:\n\n{plan_content}" 开始新轮次，execution 模式切换到 Agent

#### Scenario: 用户选择继续规划
- **WHEN** 用户点击「继续规划」
- **THEN** 模式保持 Plan，不发送任何消息，审批卡片更新为已审批状态

#### Scenario: 用户选择给反馈后继续
- **WHEN** 用户点击「给反馈后继续」
- **THEN** SHALL 展开多行文本输入框
- **WHEN** 用户提交反馈文本
- **THEN** 反馈文本 SHALL 作为用户消息发送，模式保持 Plan

#### Scenario: 用户选择在编辑器中打开
- **WHEN** 用户点击「在编辑器中打开」
- **THEN** SHALL 通过 Tauri opener 打开 plan 文件，审批卡片保持显示

#### Scenario: 记住选择后自动处理
- **WHEN** 用户勾选「记住选择」+ 点击「开始实施」
- **THEN** 后续该 session 中的 plan 审批 SHALL 自动以「开始实施」处理，不显示审批卡片

### Requirement: 后端 approvePlan API 扩展
`execution.approve_plan` WebSocket API SHALL 支持新增参数：

#### Scenario: 带反馈的审批
- **WHEN** 前端调用 `approvePlan(sid, "plan", {feedback: "..."})`
- **THEN** 后端 SHALL 保持 Plan 模式，并将 feedback 作为用户消息注入到对话中

#### Scenario: 清除上下文的审批
- **WHEN** 前端调用 `approvePlan(sid, "agent", {clearContext: true})`
- **THEN** 后端 SHALL 清空 session 对话历史，以 plan 全文 + 实施指令作为新 session 首条消息，切换到 Agent 模式

## Implementation Reference

### 竞品审批流架构对比

**Codex 审批流**：
- Turn 结束后自动弹窗（`maybe_prompt_plan_implementation`）
- 3 个选项：Yes / Yes+清上下文 / No
- "Yes" 发送 "Implement the plan." 用户消息，切换到 Default 模式
- "Yes, clear context" 新建 session，注入 plan markdown 作为首条消息

**Claude Code 审批流**：
- ExitPlanMode 走权限对话框通道（PermissionRequest）
- 4+ 选项：实施 / 清+实施 / 继续规划 / 反馈继续 / Ctrl+G 外部编辑
- 全屏 Fullscreen 模式支持（sticky footer），长 plan 可滚动阅读
- "No, with feedback" 允许文本输入拒绝原因

**XiaoLin 目标审批流**：
- 工具结果触发 PlanApprovalCard（保持当前模式，最自然）
- 5 个选项 + 辅助操作，超越 Codex 和 Claude Code
- GUI 优势：多行文本输入框 > CLI 文本输入
- GUI 优势：默认展开 Markdown 预览 > 折叠式
- 独有：上下文使用率提示、记住选择
