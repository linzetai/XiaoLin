use fastclaw_core::types::{ChatMessage, Role};

use crate::compressor::{CompactionResult, CompactionStrategy, ContextCompactor};
use crate::engine::{assemble_context, AssembledContext, ContextBudget, ContextLayers};
use crate::user_profile::UserProfile;

fn is_session_summary_block(c: &str) -> bool {
    c.starts_with("[Conversation history summary]")
        || c.starts_with("[Layered conversation summary")
        || c.starts_with("[Full conversation summary]")
}

fn is_recall_block(c: &str) -> bool {
    c.starts_with("[Relevant memories]")
}

/// Split engine-managed system blocks from the conversational tail.
fn partition_messages(messages: &[ChatMessage]) -> (String, String, String, Vec<ChatMessage>) {
    let mut static_sys = Vec::new();
    let mut recall = Vec::new();
    let mut session = Vec::new();
    let mut conv = Vec::new();
    for m in messages {
        if matches!(m.role, Role::System) {
            let c = m.text_content().unwrap_or_default();
            if is_recall_block(&c) {
                recall.push(c.to_string());
            } else if is_session_summary_block(&c) {
                session.push(c.to_string());
            } else {
                static_sys.push(c.to_string());
            }
        } else {
            conv.push(m.clone());
        }
    }
    (
        static_sys.join("\n\n"),
        recall.join("\n\n"),
        session.join("\n\n"),
        conv,
    )
}

fn split_tail_user(mut conv: Vec<ChatMessage>) -> (Vec<ChatMessage>, Option<ChatMessage>) {
    if matches!(conv.last().map(|m| &m.role), Some(Role::User)) {
        let u = conv.pop();
        (conv, u)
    } else {
        (conv, None)
    }
}

/// High-level context manager that decides when and how to compact.
///
/// Tracks conversation length and triggers automatic compaction
/// when the context exceeds configured thresholds. Coordinates
/// [`UserProfile`] updates, compression, and six-layer assembly.
pub struct ContextManager {
    compactor: ContextCompactor,
    compaction_threshold: usize,
    profile: UserProfile,
    budget: ContextBudget,
    last_summary: Option<String>,
    compaction_count: u32,
}

impl ContextManager {
    /// Create a new context manager.
    ///
    /// - `strategy`: How to compact (sliding window, token budget, aggressive, or layered).
    /// - `compaction_threshold`: Trigger compaction when non-system message count exceeds this.
    pub fn new(strategy: CompactionStrategy, compaction_threshold: usize) -> Self {
        Self {
            compactor: ContextCompactor::new(strategy),
            compaction_threshold,
            profile: UserProfile::default(),
            budget: ContextBudget::default(),
            last_summary: None,
            compaction_count: 0,
        }
    }

    pub fn user_profile(&self) -> &UserProfile {
        &self.profile
    }

    pub fn user_profile_mut(&mut self) -> &mut UserProfile {
        &mut self.profile
    }

    pub fn budget(&self) -> &ContextBudget {
        &self.budget
    }

    pub fn budget_mut(&mut self) -> &mut ContextBudget {
        &mut self.budget
    }

    pub fn set_compaction_strategy(&mut self, strategy: CompactionStrategy) {
        self.compactor = ContextCompactor::new(strategy);
    }

    /// Rule-based profile refresh from all user turns in `messages`.
    pub fn sync_user_profile_from_messages(&mut self, messages: &[ChatMessage]) {
        for m in messages {
            if matches!(m.role, Role::User) {
                if let Some(t) = m.text_content() {
                    self.profile.extract_from_message(&t);
                }
            }
        }
    }

    /// Assemble using the configured [`ContextBudget`] without mutating profile or compacting.
    pub fn assemble_layers(&self, layers: &ContextLayers) -> AssembledContext {
        assemble_context(&self.budget, layers)
    }

    /// End-to-end: profile sync → optional compaction → six-layer trim → messages.
    ///
    /// `agent_system_prompt` is treated as layer-1 base; static system messages from hooks
    /// (SOUL, USER.md, etc.) are merged into layer 1. `extra_recall` is appended to memory
    /// blocks parsed from `messages`.
    pub fn build_assembled_context(
        &mut self,
        agent_system_prompt: &str,
        extra_recall: &str,
        messages: &[ChatMessage],
    ) -> AssembledContext {
        self.sync_user_profile_from_messages(messages);

        let work = if self.should_compact(messages) {
            tracing::info!(
                msg_count = messages.len(),
                threshold = self.compaction_threshold,
                "context manager: compacting before assembly"
            );
            let result = self.compactor.compact(messages);
            self.compaction_count += 1;
            if let Some(ref s) = result.summary {
                self.last_summary = Some(s.clone());
            }
            result.messages
        } else {
            messages.to_vec()
        };

        let (static_sys, recall_sys, session_sys, conv) = partition_messages(&work);
        let (recent, current) = split_tail_user(conv);

        let system_prompt = [agent_system_prompt.trim(), static_sys.trim()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");

        let session_summary = if !session_sys.is_empty() {
            session_sys
        } else {
            self.last_summary.clone().unwrap_or_default()
        };

        let recall_text = match (
            !extra_recall.trim().is_empty(),
            !recall_sys.trim().is_empty(),
        ) {
            (true, true) => format!("{extra_recall}\n\n{recall_sys}"),
            (true, false) => extra_recall.to_string(),
            (false, true) => recall_sys,
            _ => String::new(),
        };

        let layers = ContextLayers {
            system_prompt: system_prompt,
            profile_text: self.profile.to_prompt_text(),
            session_summary,
            recall_text,
            recent_messages: recent,
            current_input: current,
        };

        assemble_context(&self.budget, &layers)
    }

    /// Check whether compaction should be triggered for the given messages.
    pub fn should_compact(&self, messages: &[ChatMessage]) -> bool {
        let non_system = messages
            .iter()
            .filter(|m| !matches!(m.role, Role::System))
            .count();
        non_system > self.compaction_threshold
    }

    /// Process messages: compact if needed, return the (possibly compacted) message list.
    pub fn process(&mut self, messages: &[ChatMessage]) -> Vec<ChatMessage> {
        if !self.should_compact(messages) {
            return messages.to_vec();
        }

        tracing::info!(
            msg_count = messages.len(),
            threshold = self.compaction_threshold,
            "triggering context compaction"
        );

        let result = self.compactor.compact(messages);
        self.compaction_count += 1;

        if let Some(ref summary) = result.summary {
            tracing::info!(
                evicted = result.evicted_count,
                kept = result.compacted_count,
                compaction_num = self.compaction_count,
                "compaction complete"
            );
            self.last_summary = Some(summary.clone());
        }

        result.messages
    }

    /// Run compaction unconditionally and return the full result.
    pub fn force_compact(&mut self, messages: &[ChatMessage]) -> CompactionResult {
        let result = self.compactor.compact(messages);
        self.compaction_count += 1;
        if let Some(ref s) = result.summary {
            self.last_summary = Some(s.clone());
        }
        result
    }

    /// Optional LLM path for layered strategy only.
    pub fn force_compact_with_optional_llm(
        &mut self,
        messages: &[ChatMessage],
        llm: Option<&dyn crate::compressor::LlmLayerSummarizer>,
    ) -> CompactionResult {
        let result = self.compactor.compact_with_optional_llm(messages, llm);
        self.compaction_count += 1;
        if let Some(ref s) = result.summary {
            self.last_summary = Some(s.clone());
        }
        result
    }

    pub fn last_summary(&self) -> Option<&str> {
        self.last_summary.as_deref()
    }

    pub fn compaction_count(&self) -> u32 {
        self.compaction_count
    }

    pub fn compaction_threshold(&self) -> usize {
        self.compaction_threshold
    }

    pub fn set_compaction_threshold(&mut self, threshold: usize) {
        self.compaction_threshold = threshold;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compressor::{CompactionStrategy, CompressorConfig};

    fn user(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::User,
            content: Some(text.to_string().into()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn assistant(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::Assistant,
            content: Some(text.to_string().into()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    #[test]
    fn no_compaction_under_threshold() {
        let mut mgr = ContextManager::new(CompactionStrategy::SlidingWindow { keep_recent: 4 }, 10);
        let msgs = vec![user("hi"), assistant("hello")];
        assert!(!mgr.should_compact(&msgs));
        let result = mgr.process(&msgs);
        assert_eq!(result.len(), 2);
        assert_eq!(mgr.compaction_count(), 0);
    }

    #[test]
    fn auto_compact_above_threshold() {
        let mut mgr = ContextManager::new(CompactionStrategy::SlidingWindow { keep_recent: 2 }, 4);
        let mut msgs = Vec::new();
        for i in 0..6 {
            msgs.push(user(&format!("q{i}")));
            msgs.push(assistant(&format!("a{i}")));
        }
        assert!(mgr.should_compact(&msgs));
        let result = mgr.process(&msgs);
        assert!(result.len() < msgs.len());
        assert_eq!(mgr.compaction_count(), 1);
        assert!(mgr.last_summary().is_some());
    }

    #[test]
    fn force_compact_returns_result() {
        let mut mgr = ContextManager::new(CompactionStrategy::Aggressive, 100);
        let msgs = vec![user("q1"), assistant("a1"), user("q2"), assistant("a2")];
        let result = mgr.force_compact(&msgs);
        assert!(result.summary.is_some());
        assert_eq!(mgr.compaction_count(), 1);
    }

    #[test]
    fn build_assembled_context_orders_layers() {
        let mut mgr = ContextManager::new(
            CompactionStrategy::Layered(CompressorConfig {
                recent_window: 1,
                summary_window: 1,
                ..Default::default()
            }),
            2,
        );
        let msgs = vec![
            user("I love Rust and tokio"),
            assistant("Great!"),
            user("Follow up on async"),
            assistant("Sure."),
            user("Final question"),
        ];
        let out = mgr.build_assembled_context("You are FastClaw.", "", &msgs);
        assert!(!out.messages.is_empty());
        assert!(matches!(out.messages[0].role, Role::System));
        assert!(
            out.messages.last().unwrap().text_content().as_deref() == Some("Final question")
        );
    }
}
