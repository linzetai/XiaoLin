## ADDED Requirements

### Requirement: 审批后引导消息注入
当用户在 PlanApprovalCard 选择「开始实施」时，SHALL 自动向对话历史注入一条用户消息，引导 agent 开始按 plan 执行。

#### Scenario: 保持上下文实施
- **WHEN** 用户选择「开始实施」（保持上下文）
- **THEN** SHALL 注入用户消息 "请按照规划方案开始实现。方案文件: {plan_path}"
- **THEN** agent 可从对话历史中的 exit_plan_mode 工具结果读取 plan 内容

#### Scenario: 清除上下文实施
- **WHEN** 用户选择「清除上下文并实施」
- **THEN** SHALL 以 "请按照以下计划实现:\n\n{plan_content}" 作为新 session 首条用户消息
- **THEN** plan 全文嵌入消息中，模型无需额外读文件

### Requirement: Compact 后 Plan File Reference 注入
当对话历史被 compact（上下文压缩）且 plan 文件存在时，SHALL 在 compact 边界注入 `plan_file_reference` attachment，确保模型在长对话中不丢失 plan。

#### Scenario: Compact 后 plan 重注入
- **WHEN** 对话历史被 compact 且 plan 文件存在
- **THEN** SHALL 在 compact 后的首条 mode_attachment 中注入 plan file reference
- **THEN** 注入内容 SHALL 包含 plan 文件路径和完整内容
- **THEN** 注入文本末尾 SHALL 包含 "如果此方案与当前工作相关且尚未完成，请继续按方案执行。"

#### Scenario: Plan 文件不存在时不注入
- **WHEN** 对话历史被 compact 但 plan 文件不存在或为空
- **THEN** SHALL 不注入 plan_file_reference

### Requirement: PlanPanel Agent 模式下实施追踪视图
当处于 Agent 模式且 plan 文件存在时，PlanPanel SHALL 显示实施追踪视图（checklist + 进度条），替代纯 Markdown 全文展示。

#### Scenario: PlanPanel 自动切换到追踪视图
- **WHEN** 从 Plan 模式切换到 Agent 模式且 plan 文件存在
- **THEN** PlanPanel SHALL 切换到实施追踪视图
- **THEN** 视图 SHALL 包含：进度指示（N/M 步骤完成）、checklist、可折叠的完整方案

#### Scenario: PlanPanel 在 Agent 模式下保持可见
- **WHEN** 审批通过切换到 Agent 模式
- **THEN** PlanPanel SHALL 继续显示（如果审批前已打开），不因模式切换关闭

### Requirement: Plan → Checklist 自动解析
PlanPanel SHALL 从 plan 文件的 `## Changes` 章节自动提取步骤，生成 checklist 项目列表。

#### Scenario: 从 Changes 章节提取步骤
- **WHEN** plan 文件包含 `## Changes` 章节
- **THEN** SHALL 解析该章节中的每个顶级列表项（`-` 或 `*` 开头）作为一个步骤
- **THEN** 每个步骤 SHALL 识别文件路径（如果有）和简短描述

#### Scenario: 无 Changes 章节时降级
- **WHEN** plan 文件不包含 `## Changes` 章节
- **THEN** SHALL 降级为完整 Markdown 展示，不显示 checklist

#### Scenario: Checklist 项目格式
- **THEN** 每个 checklist 项目 SHALL 显示：完成状态图标（○/⏳/✅）、文件路径（代码字体）、描述文字

### Requirement: 文件修改自动标记步骤完成
当 agent 在实施阶段通过 write_file/edit_file 修改了 plan Changes 中列出的文件时，PlanPanel SHALL 自动将对应步骤标记为完成。

#### Scenario: write_file 触发步骤完成
- **WHEN** agent 执行 write_file 工具且目标文件路径匹配 plan Changes 中的某个步骤
- **THEN** PlanPanel SHALL 将该步骤标记为 ✅ 完成
- **THEN** 进度指示 SHALL 更新（如 "3/5 步骤完成"）

#### Scenario: 模糊匹配文件路径
- **WHEN** plan 中写的是相对路径（如 `src/theme.css`）而 write_file 用的是绝对路径
- **THEN** SHALL 对路径尾部进行模糊匹配

#### Scenario: 手动标记
- **WHEN** 用户点击 checklist 项目的状态图标
- **THEN** SHALL 在 ○ → ✅ → ○ 之间切换（手动覆盖自动检测）

### Requirement: 进度百分比和进度条
PlanPanel 追踪视图 SHALL 在顶部显示进度百分比和进度条。

#### Scenario: 进度条渲染
- **WHEN** PlanPanel 处于追踪视图
- **THEN** 顶部 SHALL 显示：`📋 实施进度 N/M` + 进度条（宽度 = completed/total 百分比）
- **THEN** 进度条颜色 SHALL 使用 `var(--plan-tint)`

#### Scenario: 全部完成
- **WHEN** checklist 所有步骤标记为完成
- **THEN** SHALL 显示 "🎉 方案已全部实施" 替代进度信息
- **THEN** 可选：建议运行 plan Verification 章节中的测试命令

### Requirement: 实施期 Sparse Reminder Attachment
Agent 模式下如果 plan 文件存在且未全部完成，SHALL 每 N 轮（默认 5）注入一次轻量 plan reminder。

#### Scenario: 实施期 reminder 注入
- **WHEN** Agent 模式下 plan 文件存在且 checklist 未全部完成
- **AND** 距上次 reminder 已过 5 轮
- **THEN** mode_attachments SHALL 注入 sparse implementation reminder
- **THEN** reminder 内容 SHALL 包含：plan 文件路径、当前进度（N/M）、下一步描述

#### Scenario: Plan 全部完成后停止 reminder
- **WHEN** checklist 所有步骤标记为完成
- **THEN** SHALL 停止注入 implementation reminder

#### Scenario: 无 plan 文件时不注入
- **WHEN** Agent 模式下 plan 文件不存在
- **THEN** SHALL 不注入任何 plan-related reminder

## Implementation Reference

### 竞品执行追踪对标

| 能力 | Codex | Claude Code | XiaoLin (目标) |
|------|-------|-------------|----------------|
| 审批后引导消息 | "Implement the plan." | "Implement the following plan:" | ✅ 两种路径都有 |
| Plan 在实施期可见 | 对话 history（滚过即不可见） | `/plan` 命令 + 磁盘文件 | ✅ PlanPanel 始终可见 |
| 步骤 checklist | `update_plan` 工具（模型手动） | TodoWrite（与 plan 无关联） | ✅ 自动从 plan 生成 |
| 自动进度标记 | ❌ | ❌ | ✅ 文件修改自动标记 |
| 进度可视化 | Terminal title "Tasks N/M" | ❌ | ✅ PlanPanel 进度条 |
| Compact 后保持 | ❌ | ✅ plan_file_reference | ✅ plan_file_reference |
| 实施期 reminder | ❌ | ❌ (仅 todo_reminder) | ✅ sparse plan reminder |
| 自动完成检测 | ❌ | ❌ (VerifyPlan dead code) | ✅ checklist 全完成检测 |

### 这是 XiaoLin 可以超越竞品的核心差异化能力

Codex 和 Claude Code 在执行追踪上的核心问题：
1. **Plan 和 Progress 脱节** — Codex 的 `update_plan` 和 Claude Code 的 `TodoWrite` 都需要模型手动维护，与 plan 文件无结构化关联
2. **Plan 完成后无反馈** — 两者都没有自动检测 plan 步骤全部完成的机制
3. **长对话中 Plan 丢失** — Codex 完全依赖 history，Claude Code 仅在 compact 边界注入

XiaoLin 的 GUI + PlanPanel 侧边栏天然解决了「plan 可见性」问题，自动文件匹配进一步解决了「进度感知」问题。

### PlanPanel 追踪视图布局参考

```
┌── PlanPanel ──────────────────────────────────────────┐
│  📋 实施进度 3/5                         [▾ 折叠方案] │
│  ▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░ 60%                          │
│  ──────────────────────────────────────────────────── │
│  ✅ src/styles/theme.css                              │
│     添加 dark mode CSS custom properties              │
│  ✅ src/components/Login.tsx                           │
│     应用 theme class 和 dark mode toggle              │
│  ✅ src/lib/theme-provider.ts                          │
│     创建 ThemeProvider context                        │
│  ⏳ src/hooks/useTheme.ts                             │
│     创建 useTheme hook                                │
│  ○  tests/theme.test.ts                               │
│     添加暗色模式切换的单元测试                         │
│  ──────────────────────────────────────────────────── │
│  [展开完整方案 ▾]                                     │
│                                                       │
│  ┌─ 方案全文（折叠态）─────────────────────────────┐  │
│  │ ## Context                                      │  │
│  │ Login page needs dark mode support...           │  │
│  │ ...                                             │  │
│  └─────────────────────────────────────────────────┘  │
└───────────────────────────────────────────────────────┘
```

### 技术实现要点

**1. Plan Checklist 解析器（前端）**

```typescript
interface PlanStep {
  filePath?: string;
  description: string;
  status: 'pending' | 'in_progress' | 'completed';
  autoCompleted?: boolean; // 是否由文件修改自动标记
}

function parsePlanSteps(planContent: string): PlanStep[] {
  const changesSection = extractSection(planContent, "Changes");
  if (!changesSection) return [];
  
  return changesSection
    .split('\n')
    .filter(line => /^\s*[-*]/.test(line))
    .map(line => {
      const pathMatch = line.match(/`([^`]+\.\w+)`|(\S+\.\w+)/);
      return {
        filePath: pathMatch?.[1] ?? pathMatch?.[2],
        description: line.replace(/^\s*[-*]\s*/, '').trim(),
        status: 'pending',
      };
    });
}
```

**2. 文件修改监听（hook 到工具执行事件）**

```typescript
// useMessageStreamChat.ts 中监听工具执行结果
case "tool_result": {
  const toolName = event.data?.toolName;
  const filePath = event.data?.arguments?.file_path;
  if ((toolName === "write_file" || toolName === "edit_file") && filePath) {
    markPlanStepCompleted(filePath);
  }
}
```

**3. Compact 后 plan_file_reference（后端）**

在 `mode_attachments.rs` 中的 `compact_boundary_attachment()` 添加：

```rust
fn compact_boundary_plan_reference(plan_store: &PlanFileStore, session_id: &str) -> Option<String> {
    if !plan_store.plan_exists(session_id) { return None; }
    let content = plan_store.read_plan(session_id).ok()?;
    if content.is_empty() { return None; }
    let path = plan_store.plan_path(session_id);
    Some(format!(
        "实施方案文件位于: {}\n\n方案内容:\n\n{}\n\n\
         如果此方案与当前工作相关且尚未完成，请继续按方案执行。",
        path.display(), content
    ))
}
```
