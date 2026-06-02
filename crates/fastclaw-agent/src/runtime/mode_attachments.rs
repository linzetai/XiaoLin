//! Mode-specific per-turn attachment injection.
//!
//! Instead of embedding mode instructions in the system prompt (which costs
//! ~800 tokens every turn), this module injects them as user-role messages
//! with a throttling schedule: full on the first turn, nothing for a few
//! turns, then a sparse reminder, cycling back to full periodically.
//!
//! Inspired by Claude Code's per-turn attachment pattern and Codex CLI's
//! contextual fragments.

use fastclaw_core::types::ExecutionMode;

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
        None => "Use `todo_write` to record your plan steps.".to_string(),
    };

    format!(
        "<mode_attachment type=\"full\">\n\
         ## Plan Mode Active (Read-Only)\n\n\
         All edit and execute tools are blocked except writing to the plan file.\n\n\
         ### Plan File\n\
         {plan_file_info}\n\
         This is the ONLY file you may write to.\n\n\
         ### Workflow\n\
         1. Explore: read files, search patterns, understand architecture\n\
         2. Investigate: trace call chains, find reusable utilities\n\
         3. Clarify: use ask_question if requirements are ambiguous\n\
         4. Write plan: context, approach, files to change, verification steps\n\
         5. Submit: call exit_plan_mode to present plan for approval\n\n\
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
        None => "使用 `todo_write` 记录你的计划步骤。".to_string(),
    };

    format!(
        "<mode_attachment type=\"full\">\n\
         ## 计划模式已激活（只读）\n\n\
         所有编辑和执行工具均被阻塞，唯一例外是写入计划文件。\n\n\
         ### 计划文件\n\
         {plan_file_info}\n\
         这是你唯一可以写入的文件。\n\n\
         ### 工作流\n\
         1. 探索：读取文件、搜索模式、理解架构\n\
         2. 调查：追踪调用链、找到可复用工具\n\
         3. 澄清：需求模糊时使用 ask_question\n\
         4. 写计划：背景、方案、修改文件、验证步骤\n\
         5. 提交：调用 exit_plan_mode 提交审批\n\n\
         除计划文件外，不要写入或编辑任何文件。\n\
         </mode_attachment>"
    )
}

fn plan_sparse_en() -> String {
    "<mode_attachment type=\"sparse\">\n\
     Reminder: Plan mode is active. Read-only — no edits except plan file. \
     Call exit_plan_mode when your plan is ready.\n\
     </mode_attachment>"
        .to_string()
}

fn plan_sparse_zh() -> String {
    "<mode_attachment type=\"sparse\">\n\
     提醒：计划模式激活中。只读 — 除计划文件外不可编辑。\
     计划就绪后调用 exit_plan_mode。\n\
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
