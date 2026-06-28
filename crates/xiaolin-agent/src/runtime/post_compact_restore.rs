//! Post-compaction state restoration.
//!
//! When context is compressed, essential state must be preserved:
//! 1. Recently read files (for continuity)
//! 2. Invoked skill instructions (for behavior consistency)
//! 3. Plan file content (for task tracking)
//! 4. Plan mode marker (for mode persistence)
//!
//! This module creates system messages that inject these back into context
//! after compression, matching Claude-Code's post-compact restoration approach.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::SystemTime;

use serde_json::json;
use xiaolin_core::types::{ChatMessage, Role};

/// Maximum number of recent files to restore after compaction.
/// Matches Claude-Code's POST_COMPACT_MAX_FILES_TO_RESTORE.
pub const MAX_FILES_TO_RESTORE: usize = 5;

/// Token budget for post-compact file attachments.
/// Matches Claude-Code's POST_COMPACT_TOKEN_BUDGET.
pub const FILES_TOKEN_BUDGET: usize = 50_000;

/// Maximum tokens per file in post-compact restoration.
/// Matches Claude-Code's POST_COMPACT_MAX_TOKENS_PER_FILE.
pub const MAX_TOKENS_PER_FILE: usize = 5_000;

/// Maximum tokens per skill in post-compact restoration.
/// Matches Claude-Code's POST_COMPACT_MAX_TOKENS_PER_SKILL.
pub const MAX_TOKENS_PER_SKILL: usize = 5_000;

/// Token budget for skill attachments.
/// Matches Claude-Code's POST_COMPACT_SKILLS_TOKEN_BUDGET.
pub const SKILLS_TOKEN_BUDGET: usize = 25_000;

pub const MAX_OUTPUT_HANDLES_TO_RESTORE: usize = 20;
pub const OUTPUT_HANDLES_TOKEN_BUDGET: usize = 2_000;

/// A recently read file entry for restoration.
#[derive(Debug, Clone)]
pub struct RecentFile {
    pub path: PathBuf,
    pub content: String,
    pub timestamp: SystemTime,
}

/// Skill invocation record for restoration.
#[derive(Debug, Clone)]
pub struct InvokedSkill {
    pub name: String,
    pub path: PathBuf,
    pub content: String,
    pub invoked_at: SystemTime,
}

/// A deferred tool that was activated during this session.
#[derive(Debug, Clone)]
pub struct ActivatedTool {
    pub name: String,
    pub description: String,
}

/// Record of an output handle that should be preserved across compaction.
#[derive(Debug, Clone)]
pub struct OutputHandleRecord {
    pub handle: String,
    pub tool_name: String,
    pub arguments_summary: String,
    /// SHA-256 digest for Phase 8.4 correlation with repeated tool calls.
    pub arguments_digest: String,
}

/// State to be restored after compaction.
#[derive(Debug, Clone)]
pub struct RestorationState {
    /// Recently read files (path -> (content, timestamp)).
    pub recent_files: Vec<RecentFile>,
    /// Invoked skills in this session.
    pub invoked_skills: Vec<InvokedSkill>,
    /// Plan file content, if any.
    pub plan_content: Option<String>,
    /// Plan file path, if any.
    pub plan_path: Option<PathBuf>,
    /// Whether we're in plan mode.
    pub is_plan_mode: bool,
    /// Deferred tools that were activated during this session.
    /// After compaction, the activation context may be lost;
    /// this list ensures the LLM knows they are still available.
    pub activated_tools: Vec<ActivatedTool>,
    pub output_handles: VecDeque<OutputHandleRecord>,
}

impl Default for RestorationState {
    fn default() -> Self {
        Self {
            recent_files: Vec::new(), invoked_skills: Vec::new(),
            plan_content: None, plan_path: None, is_plan_mode: false,
            activated_tools: Vec::new(), output_handles: VecDeque::new(),
        }
    }
}

impl RestorationState {
    /// Create a new empty restoration state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a recently read file, maintaining the max limit.
    pub fn add_file(&mut self, path: PathBuf, content: String) {
        // Remove existing entry for this path
        self.recent_files.retain(|f| f.path != path);

        self.recent_files.push(RecentFile {
            path,
            content,
            timestamp: SystemTime::now(),
        });

        // Keep only most recent MAX_FILES_TO_RESTORE files
        if self.recent_files.len() > MAX_FILES_TO_RESTORE {
            // Sort by timestamp descending, keep most recent
            self.recent_files
                .sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
            self.recent_files.truncate(MAX_FILES_TO_RESTORE);
        }
    }

    /// Add or update an invoked skill.
    pub fn add_skill(&mut self, name: String, path: PathBuf, content: String) {
        // Remove existing entry for this skill
        self.invoked_skills.retain(|s| s.name != name);

        self.invoked_skills.push(InvokedSkill {
            name,
            path,
            content,
            invoked_at: SystemTime::now(),
        });
    }

    /// Set plan state.
    pub fn set_plan(&mut self, path: PathBuf, content: String) {
        self.plan_path = Some(path);
        self.plan_content = Some(content);
    }

    /// Clear plan state.
    pub fn clear_plan(&mut self) {
        self.plan_path = None;
        self.plan_content = None;
    }

    /// Record a deferred tool activation so it can be restored after compaction.
    pub fn record_tool_activation(&mut self, name: String, description: String) {
        if !self.activated_tools.iter().any(|t| t.name == name) {
            self.activated_tools
                .push(ActivatedTool { name, description });
        }
    }

    pub fn add_output_handle(&mut self, handle: String, tool_name: String, arguments_summary: String, arguments_digest: String) {
        if self.output_handles.iter().any(|h| h.handle == handle) { return; }
        if self.output_handles.len() >= MAX_OUTPUT_HANDLES_TO_RESTORE { self.output_handles.pop_front(); }
        self.output_handles.push_back(OutputHandleRecord { handle, tool_name, arguments_summary, arguments_digest });
    }

    /// Phase 8.4: Check whether a repeated tool call has a prior output handle
    /// with the same arguments digest, indicating a potentially unnecessary re-run.
    pub fn find_handle_for_tool_call(&self, tool_name: &str, arguments_digest: &str) -> Option<&OutputHandleRecord> {
        self.output_handles.iter()
            .filter(|h| h.tool_name == tool_name && h.arguments_digest == arguments_digest)
            .last() // most recent match
    }

    /// Generate restoration messages to inject after compaction.
    /// Returns system messages that restore the preserved state.
    pub fn generate_restoration_messages(&self) -> Vec<ChatMessage> {
        let mut messages = Vec::new();

        // 1. Restore recent files
        let files_message = self.build_files_message();
        if let Some(msg) = files_message {
            messages.push(msg);
        }

        // 2. Restore invoked skills
        let skills_message = self.build_skills_message();
        if let Some(msg) = skills_message {
            messages.push(msg);
        }

        // 3. Restore plan content
        let plan_message = self.build_plan_message();
        if let Some(msg) = plan_message {
            messages.push(msg);
        }

        // 4. Restore plan mode marker
        if self.is_plan_mode {
            messages.push(self.build_plan_mode_message());
        }

        // 5. Restore activated deferred tools reminder
        if let Some(msg) = self.build_activated_tools_message() {
            messages.push(msg);
        }

        // 6. Restore output handles
        if let Some(msg) = self.build_output_handles_message() {
            messages.push(msg);
        }

        messages
    }

    fn build_files_message(&self) -> Option<ChatMessage> {
        if self.recent_files.is_empty() {
            return None;
        }

        let mut used_tokens = 0usize;
        let mut files_content = String::new();
        files_content.push_str("[Recently read files restored after context compaction]\n\n");

        for file in &self.recent_files {
            // Truncate content if needed
            let content = truncate_to_tokens(&file.content, MAX_TOKENS_PER_FILE);

            // Estimate tokens for this file
            let file_tokens = rough_token_estimate(&content);

            // Check budget
            if used_tokens + file_tokens > FILES_TOKEN_BUDGET {
                break;
            }

            used_tokens += file_tokens;

            files_content.push_str(&format!("---\n**File:** {}\n\n```\n", file.path.display()));
            files_content.push_str(&content);
            if content.len() < file.content.len() {
                files_content.push_str("\n... (truncated)");
            }
            files_content.push_str("\n```\n\n");
        }

        Some(ChatMessage {
            role: Role::System,
            content: Some(json!(files_content)),
            ..Default::default()
        })
    }

    fn build_skills_message(&self) -> Option<ChatMessage> {
        if self.invoked_skills.is_empty() {
            return None;
        }

        // Sort by invocation time, most recent first
        let mut sorted_skills = self.invoked_skills.clone();
        sorted_skills.sort_by(|a, b| b.invoked_at.cmp(&a.invoked_at));

        let mut used_tokens = 0usize;
        let mut skills_content = String::new();
        skills_content.push_str("[Invoked skills restored after context compaction]\n\n");

        for skill in &sorted_skills {
            let content = truncate_to_tokens(&skill.content, MAX_TOKENS_PER_SKILL);
            let skill_tokens = rough_token_estimate(&content);

            if used_tokens + skill_tokens > SKILLS_TOKEN_BUDGET {
                break;
            }

            used_tokens += skill_tokens;

            skills_content.push_str(&format!(
                "---\n**Skill:** {} ({})\n\n",
                skill.name,
                skill.path.display()
            ));
            skills_content.push_str(&content);
            if content.len() < skill.content.len() {
                skills_content.push_str("\n... (truncated)");
            }
            skills_content.push_str("\n\n");
        }

        Some(ChatMessage {
            role: Role::System,
            content: Some(json!(skills_content)),
            ..Default::default()
        })
    }

    fn build_plan_message(&self) -> Option<ChatMessage> {
        let content = self.plan_content.as_ref()?;
        let path = self.plan_path.as_ref()?;

        let progress = parse_plan_progress(content);

        let mut message = String::new();
        message.push_str("[Plan file restored after context compaction]\n\n");
        message.push_str(&format!("**Plan file:** {}\n", path.display()));
        if let Some(ref p) = progress {
            message.push_str(&format!(
                "**Progress:** {}/{} completed, {} in progress\n",
                p.completed, p.total, p.in_progress,
            ));
            if let Some(ref next) = p.next_step {
                message.push_str(&format!("**Next step:** {next}\n"));
            }
        }
        message.push('\n');
        message.push_str(content);
        if progress.is_some() {
            message.push_str(
                "\n\nContinue implementing the plan. Use `update_plan` to track step progress.",
            );
        }

        Some(ChatMessage {
            role: Role::System,
            content: Some(json!(message)),
            ..Default::default()
        })
    }

    fn build_activated_tools_message(&self) -> Option<ChatMessage> {
        if self.activated_tools.is_empty() {
            return None;
        }
        let mut text =
            String::from("[Previously activated tools — still available after compaction]\n\n");
        for tool in &self.activated_tools {
            text.push_str(&format!("- **{}**: {}\n", tool.name, tool.description));
        }
        Some(ChatMessage {
            role: Role::System,
            content: Some(json!(text)),
            ..Default::default()
        })
    }

    fn build_output_handles_message(&self) -> Option<ChatMessage> {
        if self.output_handles.is_empty() { return None; }
        let header = "[Output handles preserved after context compaction]\n\n             The following tool outputs were stored as recoverable assets before compaction.              Use the recall tools to recover exact content when needed:\n\n";
        let footer = "To recover content from a handle, use:\n             - output_read for line/byte/page ranges\n             - output_search for pattern matching within the output\n             - output_tail for the last N lines\n             - output_summary for a typed summary";
        let mut used_tokens = rough_token_estimate(header) + rough_token_estimate(footer);
        let mut handles_text = String::new();
        let mut included = 0usize;
        for h in &self.output_handles {
            let relevance = if h.arguments_summary.is_empty() { format!("{} output", h.tool_name) } else { format!("{} output for {}", h.tool_name, h.arguments_summary) };
            let entry = format!("- Handle: **{handle}**\n  Tool: {tool}\n  Args: {args}\n  Relevance: {relevance}\n\n", handle = h.handle, tool = h.tool_name, args = h.arguments_summary, relevance = relevance);
            let entry_tokens = rough_token_estimate(&entry);
            if used_tokens + entry_tokens > OUTPUT_HANDLES_TOKEN_BUDGET { break; }
            used_tokens += entry_tokens;
            handles_text.push_str(&entry);
            included += 1;
        }
        if included == 0 { tracing::warn!(included=0, total_handles=self.output_handles.len(), budget=OUTPUT_HANDLES_TOKEN_BUDGET, "output handles budget exhausted"); return None; }
        let mut text = String::from(header);
        text.push_str(&handles_text);
        if included < self.output_handles.len() { text.push_str(&format!("... and {} more handles (use output_summary to list all)\n\n", self.output_handles.len() - included)); }
        text.push_str(footer);
        Some(ChatMessage { role: Role::System, content: Some(json!(text)), ..Default::default() })
    }

    fn build_plan_mode_message(&self) -> ChatMessage {
        ChatMessage {
            role: Role::System,
            content: Some(json!(
                "[Plan mode active] The user is currently in plan mode. \
                 You should continue operating in plan mode, presenting plans \
                 for approval rather than making direct changes. \
                 Use ExitPlanMode when ready for user approval."
            )),
            ..Default::default()
        }
    }
}

/// Parsed progress from plan markdown step markers.
pub struct PlanProgress {
    pub total: usize,
    pub completed: usize,
    pub in_progress: usize,
    pub pending: usize,
    pub next_step: Option<String>,
}

/// Parse `[ ]`, `[~]`, `[x]` step markers from plan markdown content.
/// Returns `None` if no step markers are found.
pub fn parse_plan_progress(content: &str) -> Option<PlanProgress> {
    let mut total = 0usize;
    let mut completed = 0usize;
    let mut in_progress = 0usize;
    let mut next_step: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim_start_matches(['-', ' ', '*']);
        if let Some(rest) = trimmed
            .strip_prefix("[x] ")
            .or_else(|| trimmed.strip_prefix("[X] "))
        {
            total += 1;
            completed += 1;
            let _ = rest;
        } else if let Some(rest) = trimmed.strip_prefix("[~] ") {
            total += 1;
            in_progress += 1;
            if next_step.is_none() {
                next_step = Some(rest.trim().to_string());
            }
        } else if let Some(rest) = trimmed.strip_prefix("[ ] ") {
            total += 1;
            if next_step.is_none() {
                next_step = Some(rest.trim().to_string());
            }
        }
    }

    if total == 0 {
        return None;
    }

    Some(PlanProgress {
        total,
        completed,
        in_progress,
        pending: total - completed - in_progress,
        next_step,
    })
}

/// Truncate content to approximately `max_tokens` tokens.
/// Uses a rough 4 chars per token estimate.
fn truncate_to_tokens(content: &str, max_tokens: usize) -> String {
    let max_chars = max_tokens.saturating_mul(4);
    if content.len() <= max_chars {
        return content.to_string();
    }

    // Find a good break point
    let end = content
        .char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(content.len());

    // Try to break at a line boundary
    let truncated = &content[..end];
    if let Some(last_newline) = truncated.rfind('\n') {
        content[..=last_newline].to_string()
    } else {
        truncated.to_string()
    }
}

/// Rough token count estimate (4 chars per token).
fn rough_token_estimate(content: &str) -> usize {
    content.len() / 4 + 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_respects_token_limit() {
        let content = "x".repeat(1000);
        let truncated = truncate_to_tokens(&content, 50);
        // 50 tokens * 4 chars = 200 chars max
        assert!(truncated.len() <= 204); // Allow for rounding
    }

    #[test]
    fn truncation_breaks_at_line_boundary() {
        // Use content longer than max_tokens (10 tokens * 4 chars = 40 chars)
        let content = "line one\nline two\nline three\nline four\nline five";
        let truncated = truncate_to_tokens(content, 10);
        // Should break at a line boundary and be shorter than original
        assert!(truncated.ends_with('\n') || truncated.len() < content.len());
        assert!(
            truncated.len() < content.len(),
            "should truncate long content"
        );
    }

    #[test]
    fn restoration_state_limits_files() {
        let mut state = RestorationState::new();
        for i in 0..10 {
            state.add_file(
                PathBuf::from(format!("/file{}.txt", i)),
                format!("content {}", i),
            );
        }
        assert_eq!(state.recent_files.len(), MAX_FILES_TO_RESTORE);
    }

    #[test]
    fn restoration_state_keeps_most_recent_files() {
        let mut state = RestorationState::new();
        // Add files in order
        state.add_file(PathBuf::from("/old.txt"), "old".to_string());
        std::thread::sleep(std::time::Duration::from_millis(10));
        state.add_file(PathBuf::from("/new.txt"), "new".to_string());

        // Both should be present (under limit)
        assert_eq!(state.recent_files.len(), 2);
    }

    #[test]
    fn generate_restoration_messages_empty_state() {
        let state = RestorationState::new();
        let messages = state.generate_restoration_messages();
        assert!(messages.is_empty());
    }

    #[test]
    fn generate_restoration_messages_with_files() {
        let mut state = RestorationState::new();
        state.add_file(PathBuf::from("/test.txt"), "test content".to_string());
        let messages = state.generate_restoration_messages();
        assert!(!messages.is_empty());
        assert!(messages[0]
            .content
            .as_ref()
            .unwrap()
            .to_string()
            .contains("Recently read files"));
    }

    #[test]
    fn generate_restoration_messages_with_plan_mode() {
        let mut state = RestorationState::new();
        state.is_plan_mode = true;
        let messages = state.generate_restoration_messages();
        assert!(messages.iter().any(|m| {
            m.content
                .as_ref()
                .map(|c| c.to_string().contains("Plan mode active"))
                .unwrap_or(false)
        }));
    }

    #[test]
    fn generate_restoration_messages_with_plan_content() {
        let mut state = RestorationState::new();
        state.set_plan(
            PathBuf::from("/plans/test-plan.md"),
            "# My Plan\n\n- [x] Step 1\n- [~] Step 2\n- [ ] Step 3".to_string(),
        );
        let messages = state.generate_restoration_messages();
        assert!(messages.iter().any(|m| {
            m.content
                .as_ref()
                .map(|c| {
                    let s = c.to_string();
                    s.contains("Plan file restored")
                        && s.contains("test-plan.md")
                        && s.contains("My Plan")
                        && s.contains("1/3 completed")
                })
                .unwrap_or(false)
        }));
    }

    #[test]
    fn parse_plan_progress_basic() {
        let content = "# Plan\n- [x] Done\n- [~] Working\n- [ ] Todo";
        let p = parse_plan_progress(content).unwrap();
        assert_eq!(p.total, 3);
        assert_eq!(p.completed, 1);
        assert_eq!(p.in_progress, 1);
        assert_eq!(p.pending, 1);
        assert_eq!(p.next_step.as_deref(), Some("Working"));
    }

    #[test]
    fn parse_plan_progress_no_markers() {
        let content = "# Plan\n\nJust a description.";
        assert!(parse_plan_progress(content).is_none());
    }

    #[test]
    fn parse_plan_progress_all_completed() {
        let content = "- [x] First\n- [x] Second";
        let p = parse_plan_progress(content).unwrap();
        assert_eq!(p.total, 2);
        assert_eq!(p.completed, 2);
        assert!(p.next_step.is_none());
    }

    #[test]
    fn clear_plan_removes_plan_content() {
        let mut state = RestorationState::new();
        state.set_plan(PathBuf::from("/plans/test.md"), "content".to_string());
        assert!(state.plan_content.is_some());
        state.clear_plan();
        assert!(state.plan_content.is_none());
    }

    #[test] fn add_output_handle_deduplicates() {
        let mut state = RestorationState::new();
        state.add_output_handle("out_a1b2".into(), "read_file".into(), "src/main.rs".into(), "dummy_digest".into());
        state.add_output_handle("out_a1b2".into(), "read_file".into(), "src/main.rs".into(), "dummy_digest".into());
        assert_eq!(state.output_handles.len(), 1);
    }
    #[test] fn add_output_handle_multiple_unique() {
        let mut state = RestorationState::new();
        state.add_output_handle("out_a1".into(), "search_in_files".into(), r#"pattern: "fn main""#.into(), "dd".into());
        state.add_output_handle("out_b2".into(), "shell_exec".into(), "cargo test".into(), "dd".into());
        assert_eq!(state.output_handles.len(), 2);
    }
    #[test] fn output_handles_message_with_computed_relevance() {
        let mut state = RestorationState::new();
        state.add_output_handle("out_abc123".into(), "read_file".into(), "src/lib.rs:1-200".into(), "dd".into());
        let msg = state.generate_restoration_messages().into_iter().find(|m| m.content.as_ref().map(|c| c.to_string().contains("Output handles preserved")).unwrap_or(false)).expect("should have handle msg");
        let text = msg.content.as_ref().unwrap().to_string();
        assert!(text.contains("out_abc123"));
        assert!(text.contains("read_file output for src/lib.rs"), "got: {text}");
        assert!(text.contains("output_read")); assert!(text.contains("output_search"));
        assert!(text.contains("output_tail")); assert!(text.contains("output_summary"));
    }
    #[test] fn output_handles_message_empty_when_no_handles() {
        assert!(!RestorationState::new().generate_restoration_messages().iter().any(|m| m.content.as_ref().map(|c| c.to_string().contains("Output handles preserved")).unwrap_or(false)));
    }
    #[test] fn output_handles_coexist_with_files_and_plan() {
        let mut state = RestorationState::new();
        state.add_file(PathBuf::from("/src/main.rs"), "fn main() {}".into());
        state.add_output_handle("out_test123".into(), "shell_exec".into(), "cargo test --lib".into(), "dd".into());
        state.is_plan_mode = true;
        let msgs = state.generate_restoration_messages();
        assert!(msgs.iter().any(|m| m.content.as_ref().map(|c| c.to_string().contains("Recently read files")).unwrap_or(false)));
        assert!(msgs.iter().any(|m| m.content.as_ref().map(|c| c.to_string().contains("Output handles preserved")).unwrap_or(false)));
        assert!(msgs.iter().any(|m| m.content.as_ref().map(|c| c.to_string().contains("Plan mode active")).unwrap_or(false)));
    }
    #[test] fn output_handle_cap_evicts_oldest() {
        let mut state = RestorationState::new();
        for i in 0..100u32 { state.add_output_handle(format!("out_{i:06x}"), "read_file".into(), format!("file_{i}.rs"), "dd".into()); }
        assert_eq!(state.output_handles.len(), MAX_OUTPUT_HANDLES_TO_RESTORE);
        assert_eq!(state.output_handles[0].handle, "out_000050");
        assert_eq!(state.output_handles.back().unwrap().handle, "out_000063");
    }
    #[test] fn output_handle_budget_fits_all_capped_handles() {
        let mut state = RestorationState::new();
        for i in 0..MAX_OUTPUT_HANDLES_TO_RESTORE { state.add_output_handle(format!("out_handle_{i:04}"), "shell_exec".into(), format!("command_line_{i}: cargo test --lib -- --test-threads=1"), "dd".into()); }
        let msg = state.generate_restoration_messages().into_iter().find(|m| m.content.as_ref().map(|c| c.to_string().contains("Output handles preserved")).unwrap_or(false)).expect("should have handle msg");
        let text = msg.content.as_ref().unwrap().to_string();
        assert_eq!(text.matches("Handle:").count(), MAX_OUTPUT_HANDLES_TO_RESTORE);
        assert!(text.contains("output_read"));
    }
}
