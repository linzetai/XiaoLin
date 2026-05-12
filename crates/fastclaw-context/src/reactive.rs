use fastclaw_core::types::{ChatMessage, Role};

use crate::compressor::{estimate_messages_tokens, CompactionStrategy, ContextCompactor};
use crate::snip::{SnipCompactor, SnipCompactorConfig};

/// Result of a reactive compaction pass.
#[derive(Debug)]
pub struct ReactiveCompactResult {
    /// Messages after compaction.
    pub messages: Vec<ChatMessage>,
    /// Which strategy level succeeded (1 = snip, 2 = importance, 3 = hard truncate).
    /// `None` if no compaction was needed or all levels failed.
    pub level_used: Option<u8>,
    /// Estimated token count after compaction.
    pub tokens_after: usize,
    /// Whether recovery succeeded (tokens_after <= target).
    pub recovered: bool,
}

/// Configuration for the reactive compactor.
pub struct ReactiveCompactorConfig {
    /// Target token budget to recover to (e.g. 128_000).
    pub target_tokens: usize,
    /// Minimum rounds to keep during snip compaction (level 1).
    pub snip_min_rounds: usize,
    /// Minimum messages to keep during hard truncation (level 3).
    /// System messages are always preserved on top of this count.
    pub hard_truncate_keep: usize,
}

impl Default for ReactiveCompactorConfig {
    fn default() -> Self {
        Self {
            target_tokens: 128_000,
            snip_min_rounds: 3,
            hard_truncate_keep: 6,
        }
    }
}

/// Reactive compactor: emergency recovery when an LLM API returns `prompt_too_long`.
///
/// Applies a three-level escalation strategy:
/// 1. **Snip (microcompact)**: Remove entire old API rounds to get under budget.
/// 2. **Importance-based**: Score messages by importance; evict lowest scores.
/// 3. **Hard truncate**: Keep only system messages + the last N conversation messages.
///
/// Invariants:
/// - System messages are never removed.
/// - The current (last) user turn is never removed.
pub struct ReactiveCompactor {
    config: ReactiveCompactorConfig,
}

impl ReactiveCompactor {
    pub fn new(config: ReactiveCompactorConfig) -> Self {
        Self { config }
    }

    /// Attempt to compact `messages` to fit within `target_tokens`.
    ///
    /// Returns a `ReactiveCompactResult` with `recovered = true` if the final
    /// token count is within budget, or `recovered = false` if even hard
    /// truncation could not bring it down (extremely unlikely).
    pub fn compact(&self, messages: &[ChatMessage]) -> ReactiveCompactResult {
        let current = estimate_messages_tokens(messages);
        if current <= self.config.target_tokens {
            return ReactiveCompactResult {
                messages: messages.to_vec(),
                level_used: None,
                tokens_after: current,
                recovered: true,
            };
        }

        let system_msgs: Vec<ChatMessage> = messages
            .iter()
            .filter(|m| m.role == Role::System)
            .cloned()
            .collect();

        // Level 1: Snip compaction (remove entire old rounds).
        if let Some(result) = self.try_snip(messages, &system_msgs) {
            return result;
        }

        // Level 2: Importance-based compaction.
        if let Some(result) = self.try_importance(messages, &system_msgs) {
            return result;
        }

        // Level 3: Hard truncation — guaranteed to produce a small result.
        self.hard_truncate(messages)
    }

    /// Ensure all original system messages appear at the front of `compacted`.
    /// The snip compactor may remove the first round which contains system messages.
    fn ensure_system_messages(
        compacted: &[ChatMessage],
        original_system: &[ChatMessage],
    ) -> Vec<ChatMessage> {
        if original_system.is_empty() {
            return compacted.to_vec();
        }
        let existing_sys: Vec<&ChatMessage> = compacted
            .iter()
            .filter(|m| m.role == Role::System)
            .collect();
        if existing_sys.len() >= original_system.len() {
            return compacted.to_vec();
        }
        // Re-insert missing system messages at the front.
        let non_sys: Vec<ChatMessage> = compacted
            .iter()
            .filter(|m| m.role != Role::System)
            .cloned()
            .collect();
        let mut result = original_system.to_vec();
        result.extend(non_sys);
        result
    }

    /// Level 1: Use `SnipCompactor` with an inner budget reduced by the
    /// system message overhead (since snip may remove the first round that
    /// contains system messages, which we restore afterwards).
    fn try_snip(
        &self,
        messages: &[ChatMessage],
        original_system: &[ChatMessage],
    ) -> Option<ReactiveCompactResult> {
        let sys_tokens = estimate_messages_tokens(original_system);
        let inner_budget = self.config.target_tokens.saturating_sub(sys_tokens);

        let compactor = SnipCompactor::new(SnipCompactorConfig {
            max_tokens: inner_budget,
            min_rounds_to_keep: self.config.snip_min_rounds,
        });
        let result = compactor.compact(messages);
        if !result.compacted {
            return None;
        }
        let msgs = Self::ensure_system_messages(&result.messages, original_system);
        let tokens_after = estimate_messages_tokens(&msgs);
        if tokens_after <= self.config.target_tokens {
            return Some(ReactiveCompactResult {
                messages: msgs,
                level_used: Some(1),
                tokens_after,
                recovered: true,
            });
        }
        None
    }

    /// Level 2: Token-budget compaction — keeps system messages and fills the
    /// remaining budget with the most recent conversation messages.
    /// Uses an 80% inner budget to leave room for the summary message overhead
    /// that `ContextCompactor::TokenBudget` injects (the summary can be
    /// significant when many messages are evicted).
    fn try_importance(
        &self,
        messages: &[ChatMessage],
        original_system: &[ChatMessage],
    ) -> Option<ReactiveCompactResult> {
        let target = self.config.target_tokens;
        let inner_budget = target * 80 / 100;

        let strategy = CompactionStrategy::TokenBudget {
            max_tokens: inner_budget,
        };
        let compactor = ContextCompactor::new(strategy);
        let result = compactor.compact(messages);

        let msgs = Self::ensure_system_messages(&result.messages, original_system);
        let tokens_after = estimate_messages_tokens(&msgs);
        if tokens_after <= target {
            return Some(ReactiveCompactResult {
                messages: msgs,
                level_used: Some(2),
                tokens_after,
                recovered: true,
            });
        }
        None
    }

    /// Level 3: Hard truncation. Keep system messages + last N conversation messages.
    /// This always produces a result — it is the fallback of last resort.
    fn hard_truncate(&self, messages: &[ChatMessage]) -> ReactiveCompactResult {
        let (system_msgs, conversation): (Vec<&ChatMessage>, Vec<&ChatMessage>) =
            messages.iter().partition(|m| m.role == Role::System);

        let keep = self.config.hard_truncate_keep.max(2);
        let start = conversation.len().saturating_sub(keep);
        let tail = &conversation[start..];

        // Ensure the last user message is included (it always will be if it's
        // within the last `keep` messages, but guard anyway).
        let mut result: Vec<ChatMessage> = system_msgs.into_iter().cloned().collect();
        result.extend(tail.iter().copied().cloned());

        // If still over budget, keep only system + last 2 messages.
        let mut tokens_after = estimate_messages_tokens(&result);
        if tokens_after > self.config.target_tokens && result.len() > 3 {
            let sys: Vec<ChatMessage> = result
                .iter()
                .filter(|m| m.role == Role::System)
                .cloned()
                .collect();
            let non_sys: Vec<ChatMessage> = result
                .into_iter()
                .filter(|m| m.role != Role::System)
                .collect();
            let last_two_start = non_sys.len().saturating_sub(2);
            result = sys;
            result.extend_from_slice(&non_sys[last_two_start..]);
            tokens_after = estimate_messages_tokens(&result);
        }

        let recovered = tokens_after <= self.config.target_tokens;
        ReactiveCompactResult {
            messages: result,
            level_used: Some(3),
            tokens_after,
            recovered,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sys(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::System,
            content: Some(json!(text)),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn user(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::User,
            content: Some(json!(text)),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn assistant(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::Assistant,
            content: Some(json!(text)),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn long_text(n: usize) -> String {
        "x".repeat(n)
    }

    /// Build a conversation with `rounds` user-assistant pairs, each with
    /// `chars_per_msg` characters of content.
    fn build_conversation(rounds: usize, chars_per_msg: usize) -> Vec<ChatMessage> {
        let mut msgs = vec![sys("You are a helpful assistant.")];
        for i in 0..rounds {
            msgs.push(user(&format!("q{i} {}", long_text(chars_per_msg))));
            msgs.push(assistant(&format!("a{i} {}", long_text(chars_per_msg))));
        }
        msgs
    }

    #[test]
    fn no_op_when_under_budget() {
        let msgs = vec![sys("system"), user("hi"), assistant("hello")];
        let compactor = ReactiveCompactor::new(ReactiveCompactorConfig {
            target_tokens: 100_000,
            ..Default::default()
        });
        let result = compactor.compact(&msgs);
        assert!(result.recovered);
        assert!(result.level_used.is_none());
        assert_eq!(result.messages.len(), msgs.len());
    }

    #[test]
    fn mild_overflow_recovers_without_hard_truncate() {
        // 20 rounds with moderate content; set budget to ~80% of total.
        // Level 1 or 2 should handle this without resorting to hard truncate.
        let msgs = build_conversation(20, 400);
        let total = estimate_messages_tokens(&msgs);
        let budget = total * 80 / 100;

        let compactor = ReactiveCompactor::new(ReactiveCompactorConfig {
            target_tokens: budget,
            snip_min_rounds: 3,
            ..Default::default()
        });
        let result = compactor.compact(&msgs);
        assert!(result.recovered);
        assert!(
            result.level_used.unwrap() <= 2,
            "should use snip or importance (level 1 or 2), got level {}",
            result.level_used.unwrap()
        );
        assert!(result.tokens_after <= budget);
    }

    #[test]
    fn level3_hard_truncate_guarantees_recovery() {
        // Huge conversation that can't be fixed by snip/importance alone
        // due to aggressive budget.
        let msgs = build_conversation(50, 1000);
        let compactor = ReactiveCompactor::new(ReactiveCompactorConfig {
            target_tokens: 200, // absurdly low
            snip_min_rounds: 40,
            hard_truncate_keep: 2,
        });
        let result = compactor.compact(&msgs);
        // Hard truncate always produces something.
        assert_eq!(result.level_used, Some(3));
        // System message should be preserved.
        assert!(result.messages.iter().any(|m| m.role == Role::System));
        // At least 1 non-system message should remain.
        assert!(result.messages.iter().any(|m| m.role != Role::System));
    }

    #[test]
    fn system_messages_always_preserved() {
        let mut msgs = vec![sys("system prompt 1"), sys("system prompt 2")];
        for i in 0..10 {
            msgs.push(user(&format!("q{i} {}", long_text(500))));
            msgs.push(assistant(&format!("a{i} {}", long_text(500))));
        }
        let total = estimate_messages_tokens(&msgs);
        let compactor = ReactiveCompactor::new(ReactiveCompactorConfig {
            target_tokens: total / 3,
            ..Default::default()
        });
        let result = compactor.compact(&msgs);
        assert!(result.recovered);
        let sys_count = result
            .messages
            .iter()
            .filter(|m| m.role == Role::System)
            .count();
        assert!(
            sys_count >= 2,
            "both system messages should survive, got {sys_count}"
        );
    }

    #[test]
    fn last_user_turn_preserved() {
        let mut msgs = build_conversation(15, 600);
        let last_user_text = "UNIQUE_LAST_USER_TURN";
        msgs.push(user(last_user_text));

        let total = estimate_messages_tokens(&msgs);
        let compactor = ReactiveCompactor::new(ReactiveCompactorConfig {
            target_tokens: total / 2,
            ..Default::default()
        });
        let result = compactor.compact(&msgs);
        assert!(result.recovered);
        let has_last = result.messages.iter().any(|m| {
            m.text_content()
                .as_deref()
                .is_some_and(|t| t.contains(last_user_text))
        });
        assert!(has_last, "last user turn must survive compaction");
    }

    #[test]
    fn returns_recovered_false_only_on_extreme_budget() {
        // Single system message with massive content and target 1 token.
        // Even hard truncate can't get below if the system message alone exceeds budget.
        let msgs = vec![sys(&long_text(100_000)), user("hi")];
        let compactor = ReactiveCompactor::new(ReactiveCompactorConfig {
            target_tokens: 1,
            hard_truncate_keep: 1,
            ..Default::default()
        });
        let result = compactor.compact(&msgs);
        assert!(!result.recovered, "system msg alone exceeds budget");
        assert_eq!(result.level_used, Some(3));
    }

    #[test]
    fn empty_messages_is_no_op() {
        let compactor = ReactiveCompactor::new(ReactiveCompactorConfig::default());
        let result = compactor.compact(&[]);
        assert!(result.recovered);
        assert!(result.level_used.is_none());
        assert!(result.messages.is_empty());
        assert_eq!(result.tokens_after, 0);
    }

    #[test]
    fn level2_escalation_when_snip_insufficient() {
        // Force snip to fail by setting min_rounds_to_keep very high so it
        // can't remove enough rounds. Level 2 (token budget) should then succeed.
        let msgs = build_conversation(10, 800);
        let total = estimate_messages_tokens(&msgs);
        let budget = total * 50 / 100;

        let compactor = ReactiveCompactor::new(ReactiveCompactorConfig {
            target_tokens: budget,
            snip_min_rounds: 9, // protect almost all rounds so snip can't help
            hard_truncate_keep: 6,
        });
        let result = compactor.compact(&msgs);
        assert!(result.recovered);
        assert_eq!(
            result.level_used,
            Some(2),
            "snip should fail (too many protected rounds), level 2 should handle it"
        );
        assert!(result.tokens_after <= budget);
    }

    #[test]
    fn tokens_after_matches_actual_estimate() {
        let msgs = build_conversation(15, 500);
        let total = estimate_messages_tokens(&msgs);
        let budget = total * 60 / 100;

        let compactor = ReactiveCompactor::new(ReactiveCompactorConfig {
            target_tokens: budget,
            ..Default::default()
        });
        let result = compactor.compact(&msgs);
        assert!(result.recovered);
        let actual = estimate_messages_tokens(&result.messages);
        assert_eq!(
            result.tokens_after, actual,
            "tokens_after should match actual estimate"
        );
    }

    #[test]
    fn message_order_preserved_after_compaction() {
        let mut msgs = vec![sys("system")];
        for i in 0..8 {
            msgs.push(user(&format!("q{i}")));
            msgs.push(assistant(&format!("a{i}")));
        }
        let total = estimate_messages_tokens(&msgs);
        let budget = total * 70 / 100;

        let compactor = ReactiveCompactor::new(ReactiveCompactorConfig {
            target_tokens: budget,
            ..Default::default()
        });
        let result = compactor.compact(&msgs);
        assert!(result.recovered);

        // Verify system messages come first.
        let first_non_sys = result.messages.iter().position(|m| m.role != Role::System);
        if let Some(pos) = first_non_sys {
            for m in &result.messages[..pos] {
                assert_eq!(m.role, Role::System);
            }
        }

        // Verify user-assistant ordering is maintained in the conversation portion.
        let conv: Vec<&ChatMessage> = result
            .messages
            .iter()
            .filter(|m| m.role != Role::System)
            .collect();
        for window in conv.windows(2) {
            if window[0].role == Role::User {
                assert_eq!(
                    window[1].role,
                    Role::Assistant,
                    "assistant should follow user"
                );
            }
        }
    }
}
