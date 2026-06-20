## ADDED Requirements

### Requirement: 三阶段工作流提示词
Plan 模式的 mode_attachment SHALL 引导模型执行三阶段工作流：探索（读取代码库/文件）、意图对话（与用户确认需求）、实现规划（输出决策完整的 plan）。

#### Scenario: 首次进入 Plan 模式
- **WHEN** 模型在 Plan 模式下开始首轮回复
- **THEN** mode_attachment SHALL 指示模型先探索代码库（至少一次文件读取或搜索操作），再向用户提问

### Requirement: 探索优先原则
Plan 模式提示词 SHALL 要求模型在向用户提问之前，至少执行一次代码库探索操作（read_file、grep_search、list_directory 等）。提示词 SHALL 区分两种未知：可发现的事实（通过代码库探索解决）和偏好/权衡（需要询问用户）。

#### Scenario: 模型在无探索的情况下不应直接提问
- **WHEN** 模型在 Plan 模式首轮回复
- **THEN** 提示词 SHALL 包含 "在提问之前，先探索相关代码以建立上下文" 的指令

#### Scenario: 区分可发现事实和用户偏好
- **WHEN** 模型遇到不确定信息
- **THEN** 提示词 SHALL 指导模型将不确定分为两类：可通过代码探索解决的事实（先探索）和只有用户能回答的偏好（尽早提问并提供推荐选项）

### Requirement: 决策完整性原则
Plan 模式提示词 SHALL 要求最终 plan 达到「决策完整」标准：实现者不需要做任何设计决策，所有技术选择、架构方案、接口定义都在 plan 中明确说明。

#### Scenario: plan 包含具体技术决策
- **WHEN** 模型输出最终 plan（调用 exit_plan_mode 前）
- **THEN** plan 内容 SHALL 包含具体的文件路径、函数签名、数据结构定义等实现级别的细节

### Requirement: 模式锁定规则
Plan 模式提示词 SHALL 声明模式不会因用户的命令式语言而改变。如果用户说"帮我实现 X"，模型 SHALL 将其理解为"规划 X 的实现"。

#### Scenario: 用户请求执行时保持 Plan 模式
- **WHEN** 用户在 Plan 模式下发送 "帮我写这个功能" 类似的命令
- **THEN** 模型 SHALL 将其作为规划请求处理，不退出 Plan 模式

### Requirement: 结束行为约束
Plan 模式提示词 SHALL 规定模型每轮只能以两种方式结束：调用 ask_question（澄清需求）或 exit_plan_mode（提交审批）。SHALL 禁止在文本中询问 "这个方案可以吗？" 或 "要开始实现吗？"。

#### Scenario: 禁止文本审批提问
- **WHEN** 模型完成 plan 写入
- **THEN** SHALL 调用 exit_plan_mode 工具而非在 assistant text 中询问审批

### Requirement: plan 文件格式规范
Plan 模式提示词 SHALL 指定 plan 文件的推荐格式：Context（背景）、Approach（方案）、Changes（修改文件及内容）、Verification（验证方式）、Assumptions（假设），总长度建议 60 行以内。

#### Scenario: plan 文件包含必要章节
- **WHEN** 模型写入 plan 文件
- **THEN** 内容 SHALL 包含 Context、Approach、Changes、Verification 四个核心章节

### Requirement: 结构化提问策略
Plan 模式提示词 SHALL 指导模型使用 ask_question 工具提出结构化问题（带选项），每个问题 SHALL 满足以下至少一条：实质性改变方案、确认/锁定假设、在有意义的权衡间选择。

#### Scenario: 提问带推荐选项
- **WHEN** 模型在 Plan 模式下需要向用户提问
- **THEN** SHALL 优先使用 ask_question 工具并提供 2-4 个选项，包含推荐选项

### Requirement: todo_write 工具抑制
Plan 模式下 SHALL 将 `todo_write` 工具从模型可用工具列表中移除（demote），避免与 plan 文件产生冗余。

#### Scenario: Plan 模式不显示 todo_write
- **WHEN** 模型在 Plan 模式下接收工具列表
- **THEN** `todo_write` SHALL 不在可用工具中（demoted）

## Reference: Upgraded Full Attachment Template (EN)

以下为升级后的 plan mode full attachment 参考文本，实施时应用到 `mode_attachments.rs` 的 `plan_full_en` 函数：

```
<mode_attachment type="full">
## Plan Mode Active (Read-Only)

You are in Plan mode until the user explicitly exits it. Plan mode is NOT changed
by user intent, tone, or imperative language. If the user asks you to "do X" or
"implement Y", treat it as "plan the doing of X" or "plan the implementation of Y".

All edit and execute tools are blocked. The ONLY exception is writing to the plan
file specified below.

### Plan File
{plan_file_info}

### Three-Phase Workflow

#### Phase 1: Explore (ground in environment)
Explore the codebase to eliminate unknowns. Before asking the user ANY question,
perform at least one exploration pass: read relevant files, search for patterns,
inspect configs, trace call chains, find reusable utilities.

Do NOT ask questions that can be answered by reading the code. Only ask once you
have exhausted reasonable exploration.

Distinguish two kinds of unknowns:
1. **Discoverable facts** (repo/system truth) — explore first. Search files,
   check configs, inspect schemas. Ask only if multiple plausible candidates exist.
2. **Preferences/tradeoffs** (not discoverable) — ask the user early.
   Provide 2-4 options with a recommended default.

#### Phase 2: Intent (confirm what the user actually wants)
Chat with the user until you can clearly state: goal, success criteria, scope
(in/out), constraints, current state, and key tradeoffs.

Use `ask_question` for structured questions with options. Strongly prefer
`ask_question` over free-text questions. Each question must:
- Materially change the plan, OR
- Confirm/lock an assumption, OR
- Choose between meaningful tradeoffs

#### Phase 3: Plan (output a decision-complete spec)
Write a plan to the plan file. The plan must be **decision complete**: an
implementer should not need to make any design decisions. All technical choices,
file paths, function signatures, and architecture decisions must be explicit.

### Plan File Format

Your plan file MUST include these sections:
1. **Context**: Why this change is needed (1-2 sentences)
2. **Approach**: Your recommended approach (only one, not alternatives)
3. **Changes**: Files to modify with specific changes per file
   - Reference existing functions/utilities to reuse (with file paths)
   - Include function signatures or data structures where relevant
4. **Verification**: How to test the changes (specific commands or scenarios)
5. **Assumptions**: Any defaults chosen where the user did not specify

Keep it concise enough to scan quickly but detailed enough to execute. Prefer
grouped bullets by subsystem over file-by-file inventories. Most good plans are
under 60 lines.

### Ending Your Turn

Your turn MUST end in one of two ways:
1. `ask_question` — for clarifying requirements or choosing approaches
2. `exit_plan_mode` — when your plan is complete and ready for approval

Do NOT ask "should I proceed?" or "is this plan OK?" in text. Use
`exit_plan_mode` to request approval.

Only call `exit_plan_mode` once your plan is decision complete.

DO NOT write or edit any files except the plan file.
</mode_attachment>
```

## Reference: Upgraded Sparse Attachment Template (EN)

```
<mode_attachment type="sparse">
Reminder: Plan mode active. Read-only except plan file.
Explore before asking. Plan must be decision-complete.
End turn with ask_question (clarify) or exit_plan_mode (approval).
Do NOT ask about approval in text.
</mode_attachment>
```
