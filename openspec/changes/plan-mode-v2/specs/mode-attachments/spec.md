## MODIFIED Requirements

### Requirement: Plan 模式 attachment 工作流引导
Plan 模式的 full attachment SHALL 包含三阶段工作流引导：
1. **探索阶段** — 指示模型读取相关代码文件、搜索符号、理解架构
2. **意图阶段** — 指示模型与用户对话确认需求、优先级、约束
3. **规划阶段** — 指示模型输出决策完整的 plan 到 plan 文件

#### Scenario: full attachment 包含三阶段指导
- **WHEN** turn_count 为 0（首次 Plan 模式回复）
- **THEN** attachment SHALL 包含三阶段工作流的完整描述和每阶段的具体指令

#### Scenario: sparse attachment 提醒三阶段
- **WHEN** turn_count 为 5 的倍数（sparse 触发）
- **THEN** attachment SHALL 简要提醒当前所处的阶段和关键约束

### Requirement: 结构化 plan 输出格式指导
Plan 模式 attachment SHALL 指定 plan 文件的推荐格式：Context、Approach、Changes、Verification、Assumptions，总长度建议 60 行以内。

#### Scenario: attachment 包含格式模板
- **WHEN** full attachment 被注入
- **THEN** SHALL 包含 plan 文件的 markdown 格式模板，包含五个章节

### Requirement: 模式锁定声明
Full attachment SHALL 包含显式模式锁定声明：Plan 模式不因用户的命令式语言而改变（"帮我实现 X" → "规划 X 的实现"）。

#### Scenario: attachment 包含锁定声明
- **WHEN** full attachment 被注入
- **THEN** SHALL 包含 "Plan mode is NOT changed by user intent, tone, or imperative language" 等效声明

### Requirement: 结束行为约束
Attachment SHALL 明确规定每轮只能以两种方式结束：`ask_question`（澄清需求）或 `exit_plan_mode`（提交审批）。SHALL 禁止在文本中询问审批。

#### Scenario: attachment 包含结束约束
- **WHEN** full 或 sparse attachment 被注入
- **THEN** SHALL 包含 "End turn with ask_question or exit_plan_mode" 等效指令，以及 "Do NOT ask about approval in text" 等效禁令

### Requirement: 两类未知区分
Attachment SHALL 指导模型区分两类未知：可发现事实（通过代码探索解决，不应问用户）和偏好/权衡（只能由用户决定，应尽早提问并提供推荐）。

#### Scenario: attachment 包含两类未知指导
- **WHEN** full attachment 被注入
- **THEN** SHALL 包含 "discoverable facts" vs "preferences/tradeoffs" 的区分描述和处理方式

### Requirement: ask_question 优先于自由文本提问
Attachment SHALL 指导模型优先使用 `ask_question` 工具提出结构化问题（带选项及推荐），而非在 assistant text 中自由提问。

#### Scenario: attachment 推荐结构化提问
- **WHEN** full attachment 被注入
- **THEN** SHALL 包含 "Strongly prefer ask_question over free-text questions" 等效指令

## Implementation Reference

升级后的 full/sparse attachment 完整文本见 `plan-prompt-v2/spec.md` 的 Reference 章节。实施时将其翻译到 `mode_attachments.rs` 的 `plan_full_en` 和 `plan_sparse_en` 函数中。
