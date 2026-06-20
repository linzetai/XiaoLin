//! Mode-specific per-turn attachment injection.
//!
//! Instead of embedding mode instructions in the system prompt (which costs
//! ~800 tokens every turn), this module injects them as user-role messages
//! with a throttling schedule: full on the first turn, nothing for a few
//! turns, then a sparse reminder, cycling back to full periodically.
//!
//! Inspired by Claude Code's per-turn attachment pattern and Codex CLI's
//! contextual fragments.

use xiaolin_core::types::ExecutionMode;

/// Configuration + templates for a mode-specific attachment.
#[derive(Debug, Clone)]
pub struct ModeAttachment {
    pub mode: ExecutionMode,
    /// Full instruction template (~800 tokens for Plan mode).
    pub full_template: String,
    /// Abbreviated reminder template (~100 tokens).
    pub sparse_template: String,
    /// Number of turns between any attachments (default 5).
    pub turns_between: u32,
    /// Every N-th attachment is a full one (default 5).
    pub full_every_n: u32,
}

/// Decides what to inject on a given turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentDecision {
    /// Inject the full template.
    Full,
    /// Inject the sparse reminder.
    Sparse,
    /// Inject nothing this turn.
    Skip,
}

impl ModeAttachment {
    /// Determine what to inject given the number of turns since entering the mode.
    ///
    /// Schedule (with defaults turns_between=5, full_every_n=5):
    ///   turn 0  → Full   (first turn)
    ///   turn 1-4 → Skip
    ///   turn 5  → Sparse (attachment #1)
    ///   turn 6-9 → Skip
    ///   turn 10 → Full   (attachment #2, every 5th = full)
    ///   turn 11-14 → Skip
    ///   turn 15 → Sparse
    ///   ...
    pub fn decide(&self, turns_since_entry: u32) -> AttachmentDecision {
        if turns_since_entry == 0 {
            return AttachmentDecision::Full;
        }

        if self.turns_between == 0 {
            return AttachmentDecision::Skip;
        }

        if !turns_since_entry.is_multiple_of(self.turns_between) {
            return AttachmentDecision::Skip;
        }

        let attachment_index = turns_since_entry / self.turns_between;
        if self.full_every_n > 0 && attachment_index.is_multiple_of(self.full_every_n) {
            AttachmentDecision::Full
        } else {
            AttachmentDecision::Sparse
        }
    }

    /// Return the text to inject (if any) for the given turn.
    pub fn text_for_turn(&self, turns_since_entry: u32) -> Option<&str> {
        match self.decide(turns_since_entry) {
            AttachmentDecision::Full => Some(&self.full_template),
            AttachmentDecision::Sparse => Some(&self.sparse_template),
            AttachmentDecision::Skip => None,
        }
    }
}

/// Build the Plan mode attachment with default throttling parameters.
pub fn plan_mode_attachment(
    plan_file_path: Option<&str>,
    plan_file_exists: bool,
    lang: Option<&str>,
) -> ModeAttachment {
    let is_zh = matches!(lang, Some("zh" | "zh-CN" | "zh-TW"));
    let full = if is_zh {
        plan_full_zh(plan_file_path, plan_file_exists)
    } else {
        plan_full_en(plan_file_path, plan_file_exists)
    };
    let sparse = if is_zh {
        plan_sparse_zh()
    } else {
        plan_sparse_en()
    };

    ModeAttachment {
        mode: ExecutionMode::Plan,
        full_template: full,
        sparse_template: sparse,
        turns_between: 5,
        full_every_n: 5,
    }
}

/// One-time reentry notice when agent re-enters Plan mode after having exited.
pub fn plan_reentry_notice(lang: Option<&str>) -> String {
    let is_zh = matches!(lang, Some("zh" | "zh-CN" | "zh-TW"));
    if is_zh {
        "<mode_attachment type=\"reentry\">\n\
         你已重新进入计划模式。之前已退出过计划模式，现在回到只读探索阶段。\n\
         请先阅读之前的计划文件（如果存在），了解已完成和待办的内容。\n\
         </mode_attachment>"
            .to_string()
    } else {
        "<mode_attachment type=\"reentry\">\n\
         You have re-entered Plan mode. You previously exited Plan mode and are now \
         back in the read-only exploration phase. Read any existing plan file first \
         to understand what was done and what remains.\n\
         </mode_attachment>"
            .to_string()
    }
}

fn plan_full_en(plan_file_path: Option<&str>, plan_file_exists: bool) -> String {
    let plan_file_info = match plan_file_path {
        Some(path) if plan_file_exists => format!(
            "A plan file already exists at `{path}`. Read it first, then decide \
             whether to update or replace it."
        ),
        Some(path) => format!("No plan file exists yet. Write your plan to `{path}`."),
        None => "No plan file path configured.".to_string(),
    };

    format!(
        "<mode_attachment type=\"full\">\n\
         ## Plan Mode Active (Read-Only)\n\n\
         You are in Plan mode until the user explicitly exits it. Plan mode is NOT changed \
         by user intent, tone, or imperative language. If the user asks you to \"do X\" or \
         \"implement Y\", treat it as \"plan the doing of X\" or \"plan the implementation of Y\".\n\n\
         All edit and execute tools are blocked. The ONLY exception is writing to the plan \
         file specified below.\n\n\
         ### Plan File\n\
         {plan_file_info}\n\n\
         ### Three-Phase Workflow\n\n\
         #### Phase 1: Explore (ground in environment)\n\
         Explore the codebase to eliminate unknowns. Before asking the user ANY question, \
         perform at least one exploration pass: read relevant files, search for patterns, \
         inspect configs, trace call chains, find reusable utilities.\n\n\
         Do NOT ask questions that can be answered by reading the code. Only ask once you \
         have exhausted reasonable exploration.\n\n\
         Distinguish two kinds of unknowns:\n\
         1. **Discoverable facts** (repo/system truth) — explore first. Search files, \
         check configs, inspect schemas. Ask only if multiple plausible candidates exist.\n\
         2. **Preferences/tradeoffs** (not discoverable) — ask the user early. \
         Provide 2-4 options with a recommended default.\n\n\
         #### Phase 2: Intent (confirm what the user actually wants)\n\
         Chat with the user until you can clearly state: goal, success criteria, scope \
         (in/out), constraints, current state, and key tradeoffs.\n\n\
         Use `ask_question` for structured questions with options. Strongly prefer \
         `ask_question` over free-text questions. Each question must:\n\
         - Materially change the plan, OR\n\
         - Confirm/lock an assumption, OR\n\
         - Choose between meaningful tradeoffs\n\n\
         #### Phase 3: Plan (output a decision-complete spec)\n\
         Write a plan to the plan file. The plan must be **decision complete**: an \
         implementer should not need to make any design decisions. All technical choices, \
         file paths, function signatures, and architecture decisions must be explicit.\n\n\
         ### Plan File Format\n\n\
         Your plan file MUST include these sections:\n\
         1. **Context**: Why this change is needed (1-2 sentences)\n\
         2. **Approach**: Your recommended approach (only one, not alternatives)\n\
         3. **Changes**: Files to modify with specific changes per file\n\
            - Reference existing functions/utilities to reuse (with file paths)\n\
            - Include function signatures or data structures where relevant\n\
         4. **Verification**: How to test the changes (specific commands or scenarios)\n\
         5. **Assumptions**: Any defaults chosen where the user did not specify\n\n\
         Keep it concise enough to scan quickly but detailed enough to execute. Most good \
         plans are under 60 lines.\n\n\
         ### Ending Your Turn\n\n\
         Your turn MUST end in one of two ways:\n\
         1. `ask_question` — for clarifying requirements or choosing approaches\n\
         2. `exit_plan_mode` — when your plan is complete and ready for approval\n\n\
         Do NOT ask \"should I proceed?\" or \"is this plan OK?\" in text. Use \
         `exit_plan_mode` to request approval.\n\n\
         Only call `exit_plan_mode` once your plan is decision complete.\n\n\
         DO NOT write or edit any files except the plan file.\n\
         </mode_attachment>"
    )
}

fn plan_full_zh(plan_file_path: Option<&str>, plan_file_exists: bool) -> String {
    let plan_file_info = match plan_file_path {
        Some(path) if plan_file_exists => {
            format!("计划文件已存在于 `{path}`。请先阅读，再决定更新或替换。")
        }
        Some(path) => format!("尚无计划文件。请将计划写入 `{path}`。"),
        None => "未配置计划文件路径。".to_string(),
    };

    format!(
        "<mode_attachment type=\"full\">\n\
         ## 计划模式已激活（只读）\n\n\
         你当前处于计划模式，直到用户明确退出。计划模式不会因用户的意图、语气或命令式语言而改变。\
         如果用户说「帮我实现 X」或「做 Y」，将其理解为「规划 X 的实现」或「规划 Y 的执行」。\n\n\
         所有编辑和执行工具均被阻塞。唯一例外是写入下方指定的计划文件。\n\n\
         ### 计划文件\n\
         {plan_file_info}\n\n\
         ### 三阶段工作流\n\n\
         #### 阶段一：探索（基于环境建立上下文）\n\
         探索代码库以消除未知。在向用户提问之前，至少执行一次探索：读取相关文件、搜索模式、\
         检查配置、追踪调用链、发现可复用的工具函数。\n\n\
         不要提问那些可以通过阅读代码回答的问题。只有在合理探索后仍无法确定时才提问。\n\n\
         区分两类未知：\n\
         1. **可发现的事实**（仓库/系统中的真相）— 先探索。搜索文件、检查配置、查看 schema。\
         只在存在多个合理候选时才提问。\n\
         2. **偏好/权衡**（无法通过探索发现）— 尽早向用户提问。提供 2-4 个选项并给出推荐。\n\n\
         #### 阶段二：意图（确认用户真正想要什么）\n\
         与用户对话，直到你能清晰陈述：目标、成功标准、范围（包含/排除）、约束、现状和关键权衡。\n\n\
         使用 `ask_question` 提出带选项的结构化问题。强烈优先使用 `ask_question` 而非纯文本提问。\
         每个问题必须满足以下至少一条：\n\
         - 实质性改变方案\n\
         - 确认/锁定假设\n\
         - 在有意义的权衡间选择\n\n\
         #### 阶段三：规划（输出决策完整的规格）\n\
         将计划写入计划文件。计划必须**决策完整**：实现者不需要做任何设计决策。\
         所有技术选择、文件路径、函数签名和架构决策都必须明确。\n\n\
         ### 计划文件格式\n\n\
         计划文件必须包含以下章节：\n\
         1. **Context（背景）**：为什么需要这个变更（1-2 句）\n\
         2. **Approach（方案）**：推荐方案（只写一个，不列备选）\n\
         3. **Changes（修改）**：要修改的文件及每个文件的具体变更\n\
            - 引用可复用的已有函数/工具（附文件路径）\n\
            - 包含函数签名或数据结构定义\n\
         4. **Verification（验证）**：如何测试变更（具体命令或场景）\n\
         5. **Assumptions（假设）**：用户未指定时你选择的默认值\n\n\
         保持简洁易扫读但足够详细可执行。好的计划通常不超过 60 行。\n\n\
         ### 结束你的回合\n\n\
         你的回合必须以以下两种方式之一结束：\n\
         1. `ask_question` — 澄清需求或选择方案\n\
         2. `exit_plan_mode` — 计划完成时提交审批\n\n\
         不要在文本中询问「这个方案可以吗？」或「要开始实现吗？」。使用 `exit_plan_mode` 请求审批。\n\n\
         只有当计划决策完整后才调用 `exit_plan_mode`。\n\n\
         除计划文件外，不要写入或编辑任何文件。\n\
         </mode_attachment>"
    )
}

fn plan_sparse_en() -> String {
    "<mode_attachment type=\"sparse\">\n\
     Reminder: Plan mode active. Read-only except plan file.\n\
     Explore before asking. Plan must be decision-complete.\n\
     End turn with ask_question (clarify) or exit_plan_mode (approval).\n\
     Do NOT ask about approval in text.\n\
     </mode_attachment>"
        .to_string()
}

fn plan_sparse_zh() -> String {
    "<mode_attachment type=\"sparse\">\n\
     提醒：计划模式激活中。只读 — 除计划文件外不可编辑。\n\
     提问前先探索。计划必须决策完整。\n\
     以 ask_question（澄清）或 exit_plan_mode（审批）结束回合。\n\
     不要在文本中询问审批。\n\
     </mode_attachment>"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_attachment() -> ModeAttachment {
        ModeAttachment {
            mode: ExecutionMode::Plan,
            full_template: "FULL".into(),
            sparse_template: "SPARSE".into(),
            turns_between: 5,
            full_every_n: 5,
        }
    }

    #[test]
    fn first_turn_is_always_full() {
        let a = sample_attachment();
        assert_eq!(a.decide(0), AttachmentDecision::Full);
    }

    #[test]
    fn turns_1_through_4_are_skipped() {
        let a = sample_attachment();
        for t in 1..5 {
            assert_eq!(a.decide(t), AttachmentDecision::Skip, "turn {t}");
        }
    }

    #[test]
    fn turn_5_is_sparse() {
        let a = sample_attachment();
        assert_eq!(a.decide(5), AttachmentDecision::Sparse);
    }

    #[test]
    fn turn_25_is_full_cycle() {
        let a = sample_attachment();
        assert_eq!(a.decide(25), AttachmentDecision::Full);
    }

    #[test]
    fn turn_10_is_sparse_not_full() {
        let a = sample_attachment();
        // attachment_index = 10/5 = 2, 2 % 5 != 0 → Sparse
        assert_eq!(a.decide(10), AttachmentDecision::Sparse);
    }

    #[test]
    fn text_for_turn_returns_correct_content() {
        let a = sample_attachment();
        assert_eq!(a.text_for_turn(0), Some("FULL"));
        assert_eq!(a.text_for_turn(1), None);
        assert_eq!(a.text_for_turn(5), Some("SPARSE"));
    }

    #[test]
    fn plan_mode_attachment_en_full() {
        let a = plan_mode_attachment(Some("/tmp/plan.md"), false, None);
        assert!(a.full_template.contains("Plan Mode Active"));
        assert!(a.full_template.contains("/tmp/plan.md"));
        assert!(a.sparse_template.contains("Reminder"));
    }

    #[test]
    fn plan_mode_attachment_zh_full() {
        let a = plan_mode_attachment(Some("/tmp/plan.md"), true, Some("zh"));
        assert!(a.full_template.contains("计划模式已激活"));
        assert!(a.full_template.contains("已存在"));
        assert!(a.sparse_template.contains("提醒"));
    }

    #[test]
    fn reentry_notice_en() {
        let notice = plan_reentry_notice(None);
        assert!(notice.contains("re-entered Plan mode"));
    }

    #[test]
    fn reentry_notice_zh() {
        let notice = plan_reentry_notice(Some("zh-CN"));
        assert!(notice.contains("重新进入计划模式"));
    }

    #[test]
    fn zero_turns_between_always_skips_after_first() {
        let a = ModeAttachment {
            turns_between: 0,
            ..sample_attachment()
        };
        assert_eq!(a.decide(0), AttachmentDecision::Full);
        assert_eq!(a.decide(1), AttachmentDecision::Skip);
        assert_eq!(a.decide(100), AttachmentDecision::Skip);
    }

    #[test]
    fn full_schedule_over_30_turns() {
        let a = sample_attachment();
        let decisions: Vec<_> = (0..30).map(|t| a.decide(t)).collect();
        assert_eq!(decisions[0], AttachmentDecision::Full);
        assert_eq!(decisions[5], AttachmentDecision::Sparse);
        assert_eq!(decisions[10], AttachmentDecision::Sparse);
        assert_eq!(decisions[15], AttachmentDecision::Sparse);
        assert_eq!(decisions[20], AttachmentDecision::Sparse);
        assert_eq!(decisions[25], AttachmentDecision::Full);
    }
}
