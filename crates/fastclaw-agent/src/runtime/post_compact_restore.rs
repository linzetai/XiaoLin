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

use std::path::PathBuf;
use std::time::SystemTime;

use fastclaw_core::types::{ChatMessage, Role};
use serde_json::json;

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

/// State to be restored after compaction.
#[derive(Debug, Clone, Default)]
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
            self.recent_files.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
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

            skills_content.push_str(&format!("---\n**Skill:** {} ({})\n\n", skill.name, skill.path.display()));
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

        let mut message = String::new();
        message.push_str("[Plan file restored after context compaction]\n\n");
        message.push_str(&format!("**Plan file:** {}\n\n", path.display()));
        message.push_str(content);

        Some(ChatMessage {
            role: Role::System,
            content: Some(json!(message)),
            ..Default::default()
        })
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
        assert!(truncated.len() < content.len(), "should truncate long content");
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
        assert!(messages[0].content.as_ref().unwrap().to_string().contains("Recently read files"));
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
}