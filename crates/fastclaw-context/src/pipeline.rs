use fastclaw_core::types::{ChatMessage, Role};

use crate::compressor::{
    estimate_messages_tokens, CompactionStrategy, ContextCompactor, DEFAULT_IMPORTANCE_MAX_MESSAGES,
    DEFAULT_IMPORTANCE_RECENT_WINDOW,
};
use crate::reactive::{ReactiveCompactResult, ReactiveCompactor, ReactiveCompactorConfig};
use crate::snip::{SnipCompactor, SnipCompactorConfig, SnipResult};

/// Per-layer compaction stats accumulated by [`ContextPipeline`].
#[derive(Debug, Clone, Default)]
pub struct CompactionMetadata {
    pub snip_tokens_freed: usize,
    pub snip_rounds_removed: usize,
    pub snip_applied: bool,

    pub micro_tokens_freed: usize,
    pub micro_evicted: usize,
    pub micro_applied: bool,

    pub collapse_applied: bool,

    pub auto_compact_applied: bool,
    pub auto_compact_original: usize,
    pub auto_compact_new: usize,

    /// Total tokens freed across all layers.
    pub total_tokens_freed: usize,
    /// Estimated token count after all pipeline stages.
    pub tokens_after: usize,
}

impl CompactionMetadata {
    /// Whether the caller should proceed with auto-compact (Layer 4).
    ///
    /// Collapse and auto-compact are mutually exclusive: when collapse is
    /// active, auto-compact must not run, because both are LLM-based
    /// summarizers that would fight over the same message range.
    pub fn should_auto_compact(&self, config: &PipelineConfig) -> bool {
        config.enable_auto_compact && !config.enable_collapse
    }
}

/// Configuration for which pipeline layers are enabled.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub enable_snip: bool,
    pub enable_micro_compact: bool,
    pub enable_collapse: bool,
    pub enable_auto_compact: bool,

    /// Max token budget for snip layer.
    pub snip_max_tokens: usize,
    pub snip_min_rounds: usize,

    /// Max messages for micro-compact (importance-based).
    pub micro_max_messages: usize,
    pub micro_recent_window: usize,

    /// Target tokens for reactive compaction.
    pub reactive_target_tokens: usize,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            enable_snip: true,
            enable_micro_compact: true,
            enable_collapse: false,
            enable_auto_compact: true,

            snip_max_tokens: 128_000,
            snip_min_rounds: 3,

            micro_max_messages: DEFAULT_IMPORTANCE_MAX_MESSAGES,
            micro_recent_window: DEFAULT_IMPORTANCE_RECENT_WINDOW,

            reactive_target_tokens: 128_000,
        }
    }
}

/// Orchestrates the multi-layer context compaction pipeline.
///
/// ```text
/// pre_query_compact():
///   1. Snip       — remove entire old API rounds
///   2. MicroCompact — importance-based message eviction
///   3. Collapse   — (reserved, currently None/no-op)
///   4. AutoCompact — LLM-based summary (caller-driven via callback)
///
/// reactive_compact():
///   Emergency recovery when prompt_too_long is returned
/// ```
pub struct ContextPipeline {
    config: PipelineConfig,
}

impl ContextPipeline {
    pub fn new(config: PipelineConfig) -> Self {
        Self { config }
    }

    /// Run the pre-query compaction pipeline (layers 1-3, synchronous).
    ///
    /// Layer 4 (AutoCompact) is not executed here because it requires an
    /// async LLM call. Callers should check `metadata.should_auto_compact(config)`
    /// before invoking auto-compact — when collapse is enabled, auto-compact
    /// is suppressed (mutual exclusion at the pipeline layer).
    pub fn pre_query_compact(&self, messages: &[ChatMessage]) -> (Vec<ChatMessage>, CompactionMetadata) {
        let tokens_before = estimate_messages_tokens(messages);
        let mut meta = CompactionMetadata::default();
        let mut current = messages.to_vec();

        let original_system: Vec<ChatMessage> = messages
            .iter()
            .filter(|m| m.role == Role::System)
            .cloned()
            .collect();

        // Layer 1: Snip
        if self.config.enable_snip {
            let result = self.run_snip(&current);
            if result.compacted {
                meta.snip_applied = true;
                meta.snip_tokens_freed = result.tokens_freed;
                meta.snip_rounds_removed = result.rounds_removed;
                current = Self::ensure_system_messages(&result.messages, &original_system);
            }
        }

        // Layer 2: MicroCompact (importance-based)
        if self.config.enable_micro_compact {
            let before = current.len();
            current = self.run_micro_compact(&current);
            let evicted = before.saturating_sub(current.len());
            if evicted > 0 {
                meta.micro_applied = true;
                meta.micro_evicted = evicted;
            }
        }

        // Layer 3: Collapse (marks active; actual summarization is async in CollapseEngine)
        // When collapse is enabled, auto-compact (Layer 4) is suppressed — see
        // `CompactionMetadata::should_auto_compact()`.
        if self.config.enable_collapse {
            meta.collapse_applied = true;
        }

        let tokens_after = estimate_messages_tokens(&current);
        meta.micro_tokens_freed = if meta.micro_applied {
            let after_snip = tokens_before.saturating_sub(meta.snip_tokens_freed);
            after_snip.saturating_sub(tokens_after)
        } else {
            0
        };
        meta.total_tokens_freed = tokens_before.saturating_sub(tokens_after);
        meta.tokens_after = tokens_after;

        (current, meta)
    }

    /// Run reactive compaction (emergency recovery from prompt_too_long).
    pub fn reactive_compact(&self, messages: &[ChatMessage]) -> ReactiveCompactResult {
        let compactor = ReactiveCompactor::new(ReactiveCompactorConfig {
            target_tokens: self.config.reactive_target_tokens,
            snip_min_rounds: self.config.snip_min_rounds,
            ..Default::default()
        });
        compactor.compact(messages)
    }

    /// Access the current config.
    pub fn config(&self) -> &PipelineConfig {
        &self.config
    }

    /// Re-insert any system messages removed by a compaction layer.
    fn ensure_system_messages(
        compacted: &[ChatMessage],
        original_system: &[ChatMessage],
    ) -> Vec<ChatMessage> {
        if original_system.is_empty() {
            return compacted.to_vec();
        }
        let existing_sys = compacted.iter().filter(|m| m.role == Role::System).count();
        if existing_sys >= original_system.len() {
            return compacted.to_vec();
        }
        let non_sys: Vec<ChatMessage> = compacted
            .iter()
            .filter(|m| m.role != Role::System)
            .cloned()
            .collect();
        let mut result = original_system.to_vec();
        result.extend(non_sys);
        result
    }

    fn run_snip(&self, messages: &[ChatMessage]) -> SnipResult {
        let compactor = SnipCompactor::new(SnipCompactorConfig {
            max_tokens: self.config.snip_max_tokens,
            min_rounds_to_keep: self.config.snip_min_rounds,
        });
        compactor.compact(messages)
    }

    fn run_micro_compact(&self, messages: &[ChatMessage]) -> Vec<ChatMessage> {
        let strategy = CompactionStrategy::ImportanceBased {
            max_messages: self.config.micro_max_messages,
            recent_window: self.config.micro_recent_window,
        };
        let compactor = ContextCompactor::new(strategy);
        let result = compactor.compact(messages);
        result.messages
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastclaw_core::types::Role;
    use serde_json::json;

    fn sys(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::System,
            content: Some(json!(text)),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn user(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::User,
            content: Some(json!(text)),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn assistant(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::Assistant,
            content: Some(json!(text)),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn long_text(n: usize) -> String {
        "x".repeat(n)
    }

    fn build_conversation(rounds: usize, chars_per_msg: usize) -> Vec<ChatMessage> {
        let mut msgs = vec![sys("You are a helpful assistant.")];
        for i in 0..rounds {
            msgs.push(user(&format!("q{i} {}", long_text(chars_per_msg))));
            msgs.push(assistant(&format!("a{i} {}", long_text(chars_per_msg))));
        }
        msgs
    }

    #[test]
    fn no_op_when_under_all_thresholds() {
        let msgs = vec![sys("system"), user("hi"), assistant("hello")];
        let pipeline = ContextPipeline::new(PipelineConfig::default());
        let (result, meta) = pipeline.pre_query_compact(&msgs);
        assert_eq!(result.len(), msgs.len());
        assert!(!meta.snip_applied);
        assert!(!meta.micro_applied);
        assert!(!meta.collapse_applied);
        assert_eq!(meta.total_tokens_freed, 0);
    }

    #[test]
    fn snip_layer_triggers_when_over_budget() {
        let msgs = build_conversation(20, 400);
        let total = estimate_messages_tokens(&msgs);
        let pipeline = ContextPipeline::new(PipelineConfig {
            snip_max_tokens: total * 80 / 100,
            enable_micro_compact: false,
            ..Default::default()
        });
        let (result, meta) = pipeline.pre_query_compact(&msgs);
        assert!(meta.snip_applied);
        assert!(meta.snip_tokens_freed > 0);
        assert!(meta.snip_rounds_removed > 0);
        assert!(result.len() < msgs.len());
    }

    #[test]
    fn micro_compact_triggers_when_too_many_messages() {
        let msgs = build_conversation(40, 50);
        let pipeline = ContextPipeline::new(PipelineConfig {
            enable_snip: false,
            micro_max_messages: 20,
            micro_recent_window: 10,
            ..Default::default()
        });
        let (result, meta) = pipeline.pre_query_compact(&msgs);
        assert!(meta.micro_applied);
        assert!(meta.micro_evicted > 0);
        assert!(result.len() < msgs.len());
    }

    #[test]
    fn collapse_layer_skipped_when_disabled() {
        let msgs = build_conversation(5, 100);
        let pipeline = ContextPipeline::new(PipelineConfig {
            enable_collapse: false,
            ..Default::default()
        });
        let (_, meta) = pipeline.pre_query_compact(&msgs);
        assert!(!meta.collapse_applied);
    }

    #[test]
    fn all_layers_disabled_is_no_op() {
        let msgs = build_conversation(20, 400);
        let pipeline = ContextPipeline::new(PipelineConfig {
            enable_snip: false,
            enable_micro_compact: false,
            enable_collapse: false,
            enable_auto_compact: false,
            ..Default::default()
        });
        let (result, meta) = pipeline.pre_query_compact(&msgs);
        assert_eq!(result.len(), msgs.len());
        assert_eq!(meta.total_tokens_freed, 0);
    }

    #[test]
    fn snip_tokens_freed_in_metadata() {
        let msgs = build_conversation(15, 600);
        let total = estimate_messages_tokens(&msgs);
        let pipeline = ContextPipeline::new(PipelineConfig {
            snip_max_tokens: total * 70 / 100,
            enable_micro_compact: false,
            ..Default::default()
        });
        let (_, meta) = pipeline.pre_query_compact(&msgs);
        assert!(meta.snip_applied);
        assert!(meta.snip_tokens_freed > 0);
        // total_tokens_freed may be slightly less than snip_tokens_freed when
        // system messages are restored after snip removes round 0.
        assert!(meta.total_tokens_freed <= meta.snip_tokens_freed);
        assert!(meta.total_tokens_freed > 0);
    }

    #[test]
    fn reactive_compact_independent_of_pre_query() {
        let msgs = build_conversation(20, 800);
        let total = estimate_messages_tokens(&msgs);
        let pipeline = ContextPipeline::new(PipelineConfig {
            reactive_target_tokens: total / 2,
            ..Default::default()
        });
        let result = pipeline.reactive_compact(&msgs);
        assert!(result.recovered);
        assert!(result.tokens_after <= total / 2);
    }

    #[test]
    fn metadata_tokens_after_is_accurate() {
        let msgs = build_conversation(20, 400);
        let total = estimate_messages_tokens(&msgs);
        let pipeline = ContextPipeline::new(PipelineConfig {
            snip_max_tokens: total * 75 / 100,
            ..Default::default()
        });
        let (result, meta) = pipeline.pre_query_compact(&msgs);
        let actual = estimate_messages_tokens(&result);
        assert_eq!(meta.tokens_after, actual);
    }

    // ====================================================================
    // Integration tests — P1-13
    // ====================================================================

    /// Build a large conversation: 200 rounds, ~4000 chars per message.
    /// Total ≈ 402k tokens (well above 128k context window).
    fn build_large_conversation() -> Vec<ChatMessage> {
        build_conversation(200, 4000)
    }

    #[test]
    fn integration_200_rounds_pre_query_fits_128k() {
        let context_window = 128_000;
        let msgs = build_large_conversation();
        let total = estimate_messages_tokens(&msgs);
        assert!(
            total > context_window,
            "precondition: raw 200-round conversation ({total} tokens) must exceed 128k"
        );

        let pipeline = ContextPipeline::new(PipelineConfig {
            snip_max_tokens: context_window,
            reactive_target_tokens: context_window,
            ..Default::default()
        });
        let (result, meta) = pipeline.pre_query_compact(&msgs);
        let result_tokens = estimate_messages_tokens(&result);

        assert!(
            result_tokens <= context_window,
            "post-compact tokens ({result_tokens}) must be <= {context_window}"
        );
        assert!(meta.snip_applied, "snip layer must activate for 200 rounds");
        assert_eq!(meta.tokens_after, result_tokens);
    }

    #[test]
    fn integration_200_rounds_compression_ratio_at_least_3x() {
        let context_window = 128_000;
        let msgs = build_large_conversation();
        let total = estimate_messages_tokens(&msgs);

        let pipeline = ContextPipeline::new(PipelineConfig {
            snip_max_tokens: context_window,
            reactive_target_tokens: context_window,
            ..Default::default()
        });
        let (result, meta) = pipeline.pre_query_compact(&msgs);
        let result_tokens = meta.tokens_after;

        let ratio = total as f64 / result_tokens.max(1) as f64;
        assert!(
            ratio >= 3.0,
            "compression ratio {ratio:.2}x (original {total}, after {result_tokens}) must be >= 3x"
        );
        assert!(result.len() < msgs.len());
    }

    #[test]
    fn integration_reactive_recovers_200k_to_128k() {
        let context_window = 128_000;
        let msgs = build_conversation(200, 2000);
        let total = estimate_messages_tokens(&msgs);
        assert!(
            total > context_window,
            "precondition: ~200k tokens ({total}) must exceed 128k"
        );

        let pipeline = ContextPipeline::new(PipelineConfig {
            reactive_target_tokens: context_window,
            ..Default::default()
        });
        let result = pipeline.reactive_compact(&msgs);

        assert!(result.recovered, "reactive compact must recover");
        assert!(
            result.tokens_after <= context_window,
            "post-reactive tokens ({}) must be <= {context_window}",
            result.tokens_after
        );
        assert!(result.level_used.is_some());
    }

    #[test]
    fn integration_system_messages_survive_all_compaction() {
        let context_window = 128_000;
        let mut msgs = vec![
            sys("You are FastClaw, a helpful AI assistant."),
            sys("Always respond in the user's language."),
        ];
        for i in 0..200 {
            msgs.push(user(&format!("question {i}: {}", long_text(4000))));
            msgs.push(assistant(&format!("answer {i}: {}", long_text(4000))));
        }

        let pipeline = ContextPipeline::new(PipelineConfig {
            snip_max_tokens: context_window,
            reactive_target_tokens: context_window,
            ..Default::default()
        });

        // pre_query_compact
        let (pre_result, _) = pipeline.pre_query_compact(&msgs);
        let sys_count = pre_result.iter().filter(|m| m.role == Role::System).count();
        assert!(
            sys_count >= 2,
            "pre_query must preserve both system messages, got {sys_count}"
        );

        // reactive_compact
        let reactive_result = pipeline.reactive_compact(&msgs);
        assert!(reactive_result.recovered);
        let sys_count_r = reactive_result
            .messages
            .iter()
            .filter(|m| m.role == Role::System)
            .count();
        assert!(
            sys_count_r >= 2,
            "reactive must preserve both system messages, got {sys_count_r}"
        );
    }

    // ═══════════════════════════════════════════════════════════════
    // Collapse ↔ AutoCompact mutual exclusion
    // ═══════════════════════════════════════════════════════════════

    #[test]
    fn collapse_enabled_suppresses_auto_compact() {
        let config = PipelineConfig {
            enable_collapse: true,
            enable_auto_compact: true,
            ..Default::default()
        };
        let msgs = build_conversation(5, 100);
        let pipeline = ContextPipeline::new(config.clone());
        let (_, meta) = pipeline.pre_query_compact(&msgs);

        assert!(meta.collapse_applied);
        assert!(
            !meta.should_auto_compact(&config),
            "auto_compact must be suppressed when collapse is enabled"
        );
    }

    #[test]
    fn collapse_disabled_allows_auto_compact() {
        let config = PipelineConfig {
            enable_collapse: false,
            enable_auto_compact: true,
            ..Default::default()
        };
        let msgs = build_conversation(5, 100);
        let pipeline = ContextPipeline::new(config.clone());
        let (_, meta) = pipeline.pre_query_compact(&msgs);

        assert!(!meta.collapse_applied);
        assert!(
            meta.should_auto_compact(&config),
            "auto_compact must be allowed when collapse is disabled"
        );
    }

    #[test]
    fn both_disabled_no_auto_compact() {
        let config = PipelineConfig {
            enable_collapse: false,
            enable_auto_compact: false,
            ..Default::default()
        };
        let msgs = build_conversation(5, 100);
        let pipeline = ContextPipeline::new(config.clone());
        let (_, meta) = pipeline.pre_query_compact(&msgs);

        assert!(!meta.should_auto_compact(&config));
    }

    #[test]
    fn integration_last_user_turn_survives_all_compaction() {
        let context_window = 128_000;
        let mut msgs = build_large_conversation();
        let sentinel = "UNIQUE_SENTINEL_LAST_USER_TURN_12345";
        msgs.push(user(sentinel));

        let pipeline = ContextPipeline::new(PipelineConfig {
            snip_max_tokens: context_window,
            reactive_target_tokens: context_window,
            ..Default::default()
        });

        // pre_query_compact
        let (pre_result, _) = pipeline.pre_query_compact(&msgs);
        let has_sentinel = pre_result.iter().any(|m| {
            m.text_content()
                .as_deref()
                .map_or(false, |t| t.contains(sentinel))
        });
        assert!(has_sentinel, "pre_query must preserve the last user turn");

        // reactive_compact
        let reactive_result = pipeline.reactive_compact(&msgs);
        assert!(reactive_result.recovered);
        let has_sentinel_r = reactive_result.messages.iter().any(|m| {
            m.text_content()
                .as_deref()
                .map_or(false, |t| t.contains(sentinel))
        });
        assert!(has_sentinel_r, "reactive must preserve the last user turn");
    }
}
