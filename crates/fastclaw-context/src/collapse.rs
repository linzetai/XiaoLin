//! Context Collapse — out-of-band storage for API round summaries.
//!
//! Instead of mutating the original message array, `CollapseStore` keeps
//! a side table of `CollapseSpan`s. Each span records which API rounds
//! have been collapsed and what the LLM-generated summary text is.
//!
//! At query time, [`project`] merges these summaries into the message
//! list, replacing collapsed rounds with their summary while leaving
//! uncollapsed messages intact. The original messages are never modified.
//!
//! The [`CollapseEngine`] monitors context usage and triggers LLM-based
//! summarization at configurable thresholds (90% async, 95% blocking).

use std::collections::BTreeMap;

use fastclaw_core::types::{ChatMessage, Role};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::compressor::estimate_messages_tokens;
use crate::snip::{group_by_api_round, ApiRound};

/// A collapsed range of API rounds together with its summary.
///
/// `start_round..=end_round` identifies the rounds (0-indexed, matching
/// [`ApiRound::index`] from `snip.rs`) that this summary replaces.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CollapseSpan {
    /// First API round index (inclusive) covered by this span.
    pub start_round: usize,
    /// Last API round index (inclusive) covered by this span.
    pub end_round: usize,
    /// LLM-generated summary that replaces the original messages.
    pub summary: String,
    /// Estimated token count of the summary text.
    pub summary_tokens: usize,
    /// Total token count of the original messages before collapse.
    pub original_tokens: usize,
    /// Unix-millis timestamp when this collapse was created.
    pub created_at: u64,
}

impl CollapseSpan {
    /// How many tokens this collapse saves (positive = net win).
    pub fn tokens_saved(&self) -> usize {
        self.original_tokens.saturating_sub(self.summary_tokens)
    }

    /// Number of rounds covered.
    pub fn round_count(&self) -> usize {
        self.end_round - self.start_round + 1
    }
}

/// Persistent, non-destructive storage for collapsed API round summaries.
///
/// Keyed by the `start_round` of each span for O(log n) lookups. Spans
/// must not overlap — `add` rejects overlapping entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CollapseStore {
    spans: BTreeMap<usize, CollapseSpan>,
}

impl CollapseStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a collapse span. Returns `Err` if it overlaps an existing span.
    pub fn add(&mut self, span: CollapseSpan) -> Result<(), CollapseOverlapError> {
        if span.start_round > span.end_round {
            return Err(CollapseOverlapError {
                message: format!(
                    "invalid range: start_round ({}) > end_round ({})",
                    span.start_round, span.end_round
                ),
            });
        }
        for existing in self.spans.values() {
            if ranges_overlap(
                span.start_round,
                span.end_round,
                existing.start_round,
                existing.end_round,
            ) {
                return Err(CollapseOverlapError {
                    message: format!(
                        "new span [{}..={}] overlaps existing [{}..={}]",
                        span.start_round, span.end_round, existing.start_round, existing.end_round,
                    ),
                });
            }
        }
        self.spans.insert(span.start_round, span);
        Ok(())
    }

    /// Look up the collapse span that covers `round_index`, if any.
    pub fn get_for_round(&self, round_index: usize) -> Option<&CollapseSpan> {
        self.spans
            .values()
            .find(|span| round_index >= span.start_round && round_index <= span.end_round)
    }

    /// Return all collapse spans in ascending round order.
    pub fn all(&self) -> Vec<&CollapseSpan> {
        self.spans.values().collect()
    }

    /// Remove the span that starts at `start_round`.
    pub fn remove(&mut self, start_round: usize) -> Option<CollapseSpan> {
        self.spans.remove(&start_round)
    }

    /// Number of stored spans.
    pub fn len(&self) -> usize {
        self.spans.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// Total tokens saved across all spans.
    pub fn total_tokens_saved(&self) -> usize {
        self.spans.values().map(|s| s.tokens_saved()).sum()
    }

    /// Check if a given round is collapsed.
    pub fn is_round_collapsed(&self, round_index: usize) -> bool {
        self.get_for_round(round_index).is_some()
    }

    /// Clear all spans.
    pub fn clear(&mut self) {
        self.spans.clear();
    }
}

fn ranges_overlap(a_start: usize, a_end: usize, b_start: usize, b_end: usize) -> bool {
    a_start <= b_end && b_start <= a_end
}

/// Returned when a new span overlaps an existing one.
#[derive(Debug, Clone)]
pub struct CollapseOverlapError {
    pub message: String,
}

impl std::fmt::Display for CollapseOverlapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "collapse overlap: {}", self.message)
    }
}

impl std::error::Error for CollapseOverlapError {}

// ─── Collapse Engine — threshold-based summary generation ────────────

/// Async trait for generating summaries via LLM.
#[async_trait::async_trait]
pub trait CollapseSummarizer: Send + Sync {
    /// Summarize a set of messages into a concise text.
    /// Returns `(summary_text, summary_token_estimate)`.
    async fn summarize(&self, messages: &[ChatMessage]) -> anyhow::Result<String>;
}

/// How the collapse engine should act based on context usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollapseMode {
    /// Context usage is below thresholds — do nothing.
    NoOp,
    /// Usage >= async_threshold — spawn background summarization.
    Async,
    /// Usage >= blocking_threshold — block until summarization completes.
    Blocking,
}

/// Configuration for the collapse engine.
#[derive(Debug, Clone)]
pub struct CollapseEngineConfig {
    /// Context usage ratio (0.0–1.0) at which async summarization triggers.
    pub async_threshold: f64,
    /// Context usage ratio (0.0–1.0) at which blocking summarization triggers.
    pub blocking_threshold: f64,
    /// Minimum number of recent rounds to preserve (never collapse).
    pub preserve_recent_rounds: usize,
    /// Minimum rounds in a batch to justify collapsing.
    pub min_collapse_batch: usize,
}

impl Default for CollapseEngineConfig {
    fn default() -> Self {
        Self {
            async_threshold: 0.75,
            blocking_threshold: 0.90,
            preserve_recent_rounds: 5,
            min_collapse_batch: 3,
        }
    }
}

/// Result of a collapse attempt.
#[derive(Debug)]
pub struct CollapseResult {
    /// The collapse span that was created, if any.
    pub span: Option<CollapseSpan>,
    /// Which mode was used.
    pub mode: CollapseMode,
}

/// Engine that monitors context usage and triggers LLM-based summarization.
pub struct CollapseEngine {
    config: CollapseEngineConfig,
}

impl CollapseEngine {
    pub fn new(config: CollapseEngineConfig) -> Self {
        Self { config }
    }

    /// Determine the collapse mode based on current context usage.
    pub fn evaluate_mode(&self, current_tokens: usize, context_window: usize) -> CollapseMode {
        if context_window == 0 {
            return CollapseMode::NoOp;
        }
        let ratio = current_tokens as f64 / context_window as f64;
        if ratio >= self.config.blocking_threshold {
            CollapseMode::Blocking
        } else if ratio >= self.config.async_threshold {
            CollapseMode::Async
        } else {
            CollapseMode::NoOp
        }
    }

    /// Select rounds eligible for collapsing.
    ///
    /// Returns the range `(start_round, end_round)` of rounds to collapse,
    /// or `None` if no eligible rounds exist. Skips rounds already collapsed
    /// and preserves the most recent `preserve_recent_rounds`.
    pub fn select_rounds_to_collapse(
        &self,
        rounds: &[ApiRound],
        store: &CollapseStore,
    ) -> Option<(usize, usize)> {
        if rounds.len() <= self.config.preserve_recent_rounds {
            return None;
        }

        let eligible_end = rounds
            .len()
            .saturating_sub(self.config.preserve_recent_rounds);

        // Find the first contiguous batch of uncollapsed rounds.
        let mut batch_start: Option<usize> = None;
        let mut batch_end: usize = 0;

        for round in rounds.iter().take(eligible_end) {
            if store.is_round_collapsed(round.index) {
                // If we have accumulated a batch, check if it's large enough.
                if let Some(start) = batch_start {
                    if batch_end - start + 1 >= self.config.min_collapse_batch {
                        return Some((start, batch_end));
                    }
                }
                batch_start = None;
            } else {
                if batch_start.is_none() {
                    batch_start = Some(round.index);
                }
                batch_end = round.index;
            }
        }

        // Check trailing batch.
        if let Some(start) = batch_start {
            if batch_end - start + 1 >= self.config.min_collapse_batch {
                return Some((start, batch_end));
            }
        }

        None
    }

    /// Perform the collapse: select rounds, call LLM summarizer, store result.
    ///
    /// Returns `Ok(CollapseResult)` with `span = None` if no rounds qualify
    /// or if the LLM call fails (graceful degradation).
    pub async fn collapse(
        &self,
        messages: &[ChatMessage],
        context_window: usize,
        store: &mut CollapseStore,
        summarizer: &dyn CollapseSummarizer,
    ) -> anyhow::Result<CollapseResult> {
        let current_tokens = estimate_messages_tokens(messages);
        let mode = self.evaluate_mode(current_tokens, context_window);

        if mode == CollapseMode::NoOp {
            return Ok(CollapseResult { span: None, mode });
        }

        let rounds = group_by_api_round(messages);
        let range = self.select_rounds_to_collapse(&rounds, store);

        let Some((start, end)) = range else {
            return Ok(CollapseResult { span: None, mode });
        };

        // Collect messages for the selected rounds.
        let collapse_messages: Vec<ChatMessage> = rounds
            .iter()
            .filter(|r| r.index >= start && r.index <= end)
            .flat_map(|r| r.messages.iter().cloned())
            .collect();

        let original_tokens = estimate_messages_tokens(&collapse_messages);

        // Call LLM summarizer — graceful failure.
        let summary = match summarizer.summarize(&collapse_messages).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    start_round = start,
                    end_round = end,
                    "Collapse summarization failed, skipping gracefully"
                );
                return Ok(CollapseResult { span: None, mode });
            }
        };

        let summary_tokens = summary.len() / 4; // chars/4 heuristic
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let span = CollapseSpan {
            start_round: start,
            end_round: end,
            summary,
            summary_tokens,
            original_tokens,
            created_at: now,
        };

        // Attempt to insert — if overlap somehow occurs, treat as no-op.
        match store.add(span.clone()) {
            Ok(()) => Ok(CollapseResult {
                span: Some(span),
                mode,
            }),
            Err(e) => {
                tracing::warn!(error = %e, "Collapse span overlaps existing, skipping");
                Ok(CollapseResult { span: None, mode })
            }
        }
    }

    pub fn config(&self) -> &CollapseEngineConfig {
        &self.config
    }
}

// ─── Project collapsed summaries into a message list ─────────────────

/// Non-destructively project collapsed summaries into the message list.
///
/// For each API round that is collapsed, replace its messages with a
/// single system message containing the summary. Uncollapsed rounds
/// pass through unchanged. The original `messages` slice is not modified.
pub fn project(messages: &[ChatMessage], store: &CollapseStore) -> Vec<ChatMessage> {
    if store.is_empty() {
        return messages.to_vec();
    }
    let rounds = group_by_api_round(messages);
    let mut result: Vec<ChatMessage> = Vec::new();
    let mut emitted_spans = std::collections::HashSet::new();

    for round in &rounds {
        if let Some(span) = store.get_for_round(round.index) {
            if emitted_spans.insert(span.start_round) {
                result.push(ChatMessage {
                    role: Role::System,
                    content: Some(json!(format!(
                        "[Summary of rounds {}–{}]: {}",
                        span.start_round, span.end_round, span.summary
                    ))),
                    ..Default::default()
                });
            }
        } else {
            result.extend(round.messages.iter().cloned());
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastclaw_core::types::Role;
    use serde_json::json;

    fn now_millis() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    fn make_span(start: usize, end: usize, summary: &str) -> CollapseSpan {
        CollapseSpan {
            start_round: start,
            end_round: end,
            summary: summary.to_string(),
            summary_tokens: summary.len() / 4,
            original_tokens: 1000,
            created_at: now_millis(),
        }
    }

    fn msg(role: Role, text: &str) -> ChatMessage {
        ChatMessage {
            role,
            content: Some(json!(text)),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
            compact_metadata: None,
        }
    }

    // ═══════════════════════════════════════════════════════════════
    // Basic CRUD
    // ═══════════════════════════════════════════════════════════════

    #[test]
    fn add_and_retrieve() {
        let mut store = CollapseStore::new();
        assert!(store.is_empty());

        store
            .add(make_span(0, 2, "First three rounds discussed setup."))
            .unwrap();
        assert_eq!(store.len(), 1);

        let span = store.get_for_round(1).unwrap();
        assert_eq!(span.start_round, 0);
        assert_eq!(span.end_round, 2);
        assert!(span.summary.contains("setup"));
    }

    #[test]
    fn remove_span() {
        let mut store = CollapseStore::new();
        store.add(make_span(0, 2, "rounds 0-2")).unwrap();
        store.add(make_span(5, 7, "rounds 5-7")).unwrap();
        assert_eq!(store.len(), 2);

        let removed = store.remove(0).unwrap();
        assert_eq!(removed.start_round, 0);
        assert_eq!(store.len(), 1);
        assert!(store.get_for_round(1).is_none());
        assert!(store.get_for_round(5).is_some());
    }

    #[test]
    fn remove_nonexistent_returns_none() {
        let mut store = CollapseStore::new();
        assert!(store.remove(42).is_none());
    }

    #[test]
    fn clear_empties_store() {
        let mut store = CollapseStore::new();
        store.add(make_span(0, 1, "a")).unwrap();
        store.add(make_span(3, 4, "b")).unwrap();
        store.clear();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
    }

    // ═══════════════════════════════════════════════════════════════
    // Multiple rounds collapsed simultaneously
    // ═══════════════════════════════════════════════════════════════

    #[test]
    fn multiple_non_overlapping_spans() {
        let mut store = CollapseStore::new();
        store.add(make_span(0, 2, "early context")).unwrap();
        store.add(make_span(5, 8, "middle context")).unwrap();
        store.add(make_span(12, 15, "later context")).unwrap();
        assert_eq!(store.len(), 3);

        assert!(store.is_round_collapsed(0));
        assert!(store.is_round_collapsed(2));
        assert!(!store.is_round_collapsed(3));
        assert!(store.is_round_collapsed(6));
        assert!(!store.is_round_collapsed(10));
        assert!(store.is_round_collapsed(14));
    }

    #[test]
    fn overlapping_spans_rejected() {
        let mut store = CollapseStore::new();
        store.add(make_span(2, 5, "original")).unwrap();

        let result = store.add(make_span(4, 7, "overlap"));
        assert!(result.is_err());
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn exact_overlap_rejected() {
        let mut store = CollapseStore::new();
        store.add(make_span(3, 6, "first")).unwrap();
        assert!(store.add(make_span(3, 6, "duplicate")).is_err());
    }

    #[test]
    fn adjacent_spans_allowed() {
        let mut store = CollapseStore::new();
        store.add(make_span(0, 2, "first")).unwrap();
        store.add(make_span(3, 5, "second")).unwrap();
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn invalid_range_rejected() {
        let mut store = CollapseStore::new();
        let result = store.add(make_span(5, 3, "bad range"));
        assert!(result.is_err());
    }

    // ═══════════════════════════════════════════════════════════════
    // Serialization / deserialization
    // ═══════════════════════════════════════════════════════════════

    #[test]
    fn serde_round_trip() {
        let mut store = CollapseStore::new();
        store.add(make_span(0, 3, "setup & config")).unwrap();
        store.add(make_span(6, 9, "debugging session")).unwrap();

        let json = serde_json::to_string(&store).unwrap();
        let restored: CollapseStore = serde_json::from_str(&json).unwrap();

        assert_eq!(store, restored);
        assert_eq!(restored.len(), 2);
        assert!(restored.is_round_collapsed(1));
        assert!(restored.is_round_collapsed(7));
    }

    #[test]
    fn span_serde_round_trip() {
        let span = make_span(10, 15, "complex analysis");
        let json = serde_json::to_string(&span).unwrap();
        let restored: CollapseSpan = serde_json::from_str(&json).unwrap();
        assert_eq!(span, restored);
    }

    // ═══════════════════════════════════════════════════════════════
    // Token accounting
    // ═══════════════════════════════════════════════════════════════

    #[test]
    fn total_tokens_saved() {
        let mut store = CollapseStore::new();
        // original_tokens=1000, summary_tokens ≈ len/4
        store.add(make_span(0, 2, "short")).unwrap(); // saves ~999
        store.add(make_span(5, 7, "another short")).unwrap(); // saves ~997
        assert!(store.total_tokens_saved() > 1900);
    }

    #[test]
    fn tokens_saved_per_span() {
        let span = CollapseSpan {
            start_round: 0,
            end_round: 3,
            summary: "x".repeat(400),
            summary_tokens: 100,
            original_tokens: 5000,
            created_at: now_millis(),
        };
        assert_eq!(span.tokens_saved(), 4900);
        assert_eq!(span.round_count(), 4);
    }

    // ═══════════════════════════════════════════════════════════════
    // Projection — non-destructive message replacement
    // ═══════════════════════════════════════════════════════════════

    #[test]
    fn project_empty_store_returns_original() {
        let messages = vec![
            msg(Role::System, "sys"),
            msg(Role::User, "q1"),
            msg(Role::Assistant, "a1"),
        ];
        let store = CollapseStore::new();
        let projected = project(&messages, &store);
        assert_eq!(projected.len(), messages.len());
    }

    #[test]
    fn project_replaces_collapsed_rounds() {
        let messages = vec![
            msg(Role::System, "system prompt"),
            msg(Role::User, "question 0"),
            msg(Role::Assistant, "answer 0"),
            msg(Role::User, "question 1"),
            msg(Role::Assistant, "answer 1"),
            msg(Role::User, "question 2"),
            msg(Role::Assistant, "answer 2"),
        ];

        let mut store = CollapseStore::new();
        // Collapse round 0 (sys + user-q0 + assistant-a0)
        store
            .add(make_span(0, 0, "Initial setup discussion"))
            .unwrap();

        let projected = project(&messages, &store);

        // Round 0 replaced by 1 summary message, rounds 1-2 remain intact (4 msgs)
        assert_eq!(projected.len(), 5); // 1 summary + 2 user + 2 assistant
        let summary_text = projected[0].text_content().unwrap();
        assert!(summary_text.contains("Initial setup discussion"));
        assert_eq!(projected[1].role, Role::User);
    }

    #[test]
    fn project_preserves_original_messages() {
        let messages = vec![
            msg(Role::System, "system"),
            msg(Role::User, "q0"),
            msg(Role::Assistant, "a0"),
            msg(Role::User, "q1"),
            msg(Role::Assistant, "a1"),
        ];
        let original_clone = messages.clone();

        let mut store = CollapseStore::new();
        store.add(make_span(0, 0, "summary")).unwrap();
        let _projected = project(&messages, &store);

        // Original messages must be unchanged
        assert_eq!(messages.len(), original_clone.len());
        for (a, b) in messages.iter().zip(original_clone.iter()) {
            assert_eq!(a.role, b.role);
            assert_eq!(a.content, b.content);
        }
    }

    #[test]
    fn project_multiple_collapsed_spans() {
        let mut messages = vec![msg(Role::System, "sys")];
        for i in 0..10 {
            messages.push(msg(Role::User, &format!("q{i}")));
            messages.push(msg(Role::Assistant, &format!("a{i}")));
        }

        let mut store = CollapseStore::new();
        store.add(make_span(0, 2, "rounds 0-2 summary")).unwrap();
        store.add(make_span(5, 7, "rounds 5-7 summary")).unwrap();

        let projected = project(&messages, &store);

        // Rounds 0-2 (3 rounds) → 1 summary
        // Rounds 3-4 (2 rounds) → 4 messages (2 user + 2 assistant)
        // Rounds 5-7 (3 rounds) → 1 summary
        // Rounds 8-9 (2 rounds) → 4 messages (2 user + 2 assistant)
        // Total: 1 + 4 + 1 + 4 = 10
        assert_eq!(projected.len(), 10);

        let summary_count = projected
            .iter()
            .filter(|m| {
                m.role == Role::System
                    && m.text_content()
                        .as_deref()
                        .is_some_and(|t| t.contains("[Summary"))
            })
            .count();
        assert_eq!(summary_count, 2);
    }

    #[test]
    fn project_reduces_token_count() {
        let mut messages = vec![msg(Role::System, "sys")];
        for i in 0..20 {
            messages.push(msg(Role::User, &format!("q{i} {}", "x".repeat(500))));
            messages.push(msg(Role::Assistant, &format!("a{i} {}", "x".repeat(500))));
        }

        let tokens_before = crate::compressor::estimate_messages_tokens(&messages);

        let mut store = CollapseStore::new();
        store
            .add(make_span(0, 5, "Early discussion summary"))
            .unwrap();
        store
            .add(make_span(8, 12, "Middle discussion summary"))
            .unwrap();

        let projected = project(&messages, &store);
        let tokens_after = crate::compressor::estimate_messages_tokens(&projected);

        assert!(
            tokens_after < tokens_before,
            "projected tokens ({tokens_after}) should be less than original ({tokens_before})"
        );
    }

    // ═══════════════════════════════════════════════════════════════
    // all() ordering
    // ═══════════════════════════════════════════════════════════════

    #[test]
    fn all_returns_spans_in_order() {
        let mut store = CollapseStore::new();
        store.add(make_span(10, 12, "later")).unwrap();
        store.add(make_span(0, 2, "earlier")).unwrap();
        store.add(make_span(5, 7, "middle")).unwrap();

        let all = store.all();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].start_round, 0);
        assert_eq!(all[1].start_round, 5);
        assert_eq!(all[2].start_round, 10);
    }

    // ═══════════════════════════════════════════════════════════════
    // CollapseEngine tests
    // ═══════════════════════════════════════════════════════════════

    struct MockSummarizer {
        response: Result<String, String>,
    }

    impl MockSummarizer {
        fn ok(text: &str) -> Self {
            Self {
                response: Ok(text.to_string()),
            }
        }
        fn failing(err: &str) -> Self {
            Self {
                response: Err(err.to_string()),
            }
        }
    }

    #[async_trait::async_trait]
    impl CollapseSummarizer for MockSummarizer {
        async fn summarize(&self, _messages: &[ChatMessage]) -> anyhow::Result<String> {
            match &self.response {
                Ok(text) => Ok(text.clone()),
                Err(e) => Err(anyhow::anyhow!("{}", e)),
            }
        }
    }

    fn make_conversation(num_rounds: usize, chars_per_msg: usize) -> Vec<ChatMessage> {
        let mut messages = vec![msg(Role::System, "system prompt")];
        for i in 0..num_rounds {
            let content = format!("q{i} {}", "x".repeat(chars_per_msg));
            messages.push(msg(Role::User, &content));
            let answer = format!("a{i} {}", "y".repeat(chars_per_msg));
            messages.push(msg(Role::Assistant, &answer));
        }
        messages
    }

    #[test]
    fn evaluate_mode_noop_below_75() {
        let engine = CollapseEngine::new(CollapseEngineConfig::default());
        let mode = engine.evaluate_mode(70_000, 100_000);
        assert_eq!(mode, CollapseMode::NoOp);
    }

    #[test]
    fn evaluate_mode_async_at_75() {
        let engine = CollapseEngine::new(CollapseEngineConfig::default());
        let mode = engine.evaluate_mode(75_000, 100_000);
        assert_eq!(mode, CollapseMode::Async);
    }

    #[test]
    fn evaluate_mode_async_at_85() {
        let engine = CollapseEngine::new(CollapseEngineConfig::default());
        let mode = engine.evaluate_mode(85_000, 100_000);
        assert_eq!(mode, CollapseMode::Async);
    }

    #[test]
    fn evaluate_mode_blocking_at_90() {
        let engine = CollapseEngine::new(CollapseEngineConfig::default());
        let mode = engine.evaluate_mode(90_000, 100_000);
        assert_eq!(mode, CollapseMode::Blocking);
    }

    #[test]
    fn evaluate_mode_blocking_at_100() {
        let engine = CollapseEngine::new(CollapseEngineConfig::default());
        let mode = engine.evaluate_mode(100_000, 100_000);
        assert_eq!(mode, CollapseMode::Blocking);
    }

    #[test]
    fn evaluate_mode_zero_context_window() {
        let engine = CollapseEngine::new(CollapseEngineConfig::default());
        let mode = engine.evaluate_mode(50_000, 0);
        assert_eq!(mode, CollapseMode::NoOp);
    }

    #[test]
    fn select_rounds_preserves_recent() {
        let engine = CollapseEngine::new(CollapseEngineConfig {
            preserve_recent_rounds: 3,
            min_collapse_batch: 2,
            ..Default::default()
        });
        let messages = make_conversation(5, 100);
        let rounds = group_by_api_round(&messages);
        let store = CollapseStore::new();

        let range = engine.select_rounds_to_collapse(&rounds, &store);
        // 5 rounds, preserve last 3, eligible = rounds 0..2
        assert!(range.is_some());
        let (start, end) = range.unwrap();
        assert!(end < 5 - 3);
        assert!(start <= end);
    }

    #[test]
    fn select_rounds_too_few_rounds() {
        let engine = CollapseEngine::new(CollapseEngineConfig {
            preserve_recent_rounds: 5,
            min_collapse_batch: 3,
            ..Default::default()
        });
        let messages = make_conversation(4, 100);
        let rounds = group_by_api_round(&messages);
        let store = CollapseStore::new();

        let range = engine.select_rounds_to_collapse(&rounds, &store);
        assert!(range.is_none());
    }

    #[test]
    fn select_rounds_skips_already_collapsed() {
        let engine = CollapseEngine::new(CollapseEngineConfig {
            preserve_recent_rounds: 2,
            min_collapse_batch: 2,
            ..Default::default()
        });
        let messages = make_conversation(8, 100);
        let rounds = group_by_api_round(&messages);
        let mut store = CollapseStore::new();
        // Collapse rounds 0-2 already.
        store.add(make_span(0, 2, "already collapsed")).unwrap();

        let range = engine.select_rounds_to_collapse(&rounds, &store);
        // Should skip 0-2 and find 3+ as eligible.
        assert!(range.is_some());
        let (start, _end) = range.unwrap();
        assert!(start >= 3);
    }

    #[tokio::test]
    async fn collapse_noop_below_threshold() {
        let engine = CollapseEngine::new(CollapseEngineConfig::default());
        // Small messages, large context window → NoOp.
        let messages = make_conversation(3, 10);
        let mut store = CollapseStore::new();
        let summarizer = MockSummarizer::ok("summary");

        let result = engine
            .collapse(&messages, 1_000_000, &mut store, &summarizer)
            .await
            .unwrap();
        assert_eq!(result.mode, CollapseMode::NoOp);
        assert!(result.span.is_none());
        assert!(store.is_empty());
    }

    #[tokio::test]
    async fn collapse_triggers_and_stores_span() {
        let engine = CollapseEngine::new(CollapseEngineConfig {
            async_threshold: 0.90,
            blocking_threshold: 0.95,
            preserve_recent_rounds: 2,
            min_collapse_batch: 2,
        });
        // Create enough messages to exceed 90% of a small context window.
        let messages = make_conversation(10, 200);
        let tokens = estimate_messages_tokens(&messages);
        // Set context_window slightly above tokens so we're at ~92%.
        let context_window = (tokens as f64 / 0.92) as usize;

        let mut store = CollapseStore::new();
        let summarizer = MockSummarizer::ok("This is the LLM summary.");

        let result = engine
            .collapse(&messages, context_window, &mut store, &summarizer)
            .await
            .unwrap();

        assert!(matches!(
            result.mode,
            CollapseMode::Async | CollapseMode::Blocking
        ));
        assert!(result.span.is_some());
        let span = result.span.unwrap();
        assert_eq!(span.summary, "This is the LLM summary.");
        assert!(!store.is_empty());
        assert!(store.is_round_collapsed(span.start_round));
    }

    #[tokio::test]
    async fn collapse_graceful_on_llm_failure() {
        let engine = CollapseEngine::new(CollapseEngineConfig {
            async_threshold: 0.90,
            blocking_threshold: 0.95,
            preserve_recent_rounds: 2,
            min_collapse_batch: 2,
        });
        let messages = make_conversation(10, 200);
        let tokens = estimate_messages_tokens(&messages);
        let context_window = (tokens as f64 / 0.92) as usize;

        let mut store = CollapseStore::new();
        let summarizer = MockSummarizer::failing("LLM rate limited");

        let result = engine
            .collapse(&messages, context_window, &mut store, &summarizer)
            .await
            .unwrap();

        // Should succeed but with no span created.
        assert!(result.span.is_none());
        assert!(store.is_empty());
    }

    #[tokio::test]
    async fn collapse_blocking_mode_at_95() {
        let engine = CollapseEngine::new(CollapseEngineConfig {
            async_threshold: 0.90,
            blocking_threshold: 0.95,
            preserve_recent_rounds: 2,
            min_collapse_batch: 2,
        });
        let messages = make_conversation(10, 200);
        let tokens = estimate_messages_tokens(&messages);
        // Set context window so usage is ~96%.
        let context_window = (tokens as f64 / 0.96) as usize;

        let mut store = CollapseStore::new();
        let summarizer = MockSummarizer::ok("Blocking summary.");

        let result = engine
            .collapse(&messages, context_window, &mut store, &summarizer)
            .await
            .unwrap();

        assert_eq!(result.mode, CollapseMode::Blocking);
        assert!(result.span.is_some());
    }

    #[tokio::test]
    async fn collapse_no_eligible_rounds_returns_none() {
        let engine = CollapseEngine::new(CollapseEngineConfig {
            async_threshold: 0.50, // Very low threshold to ensure trigger.
            blocking_threshold: 0.95,
            preserve_recent_rounds: 10, // Preserve more than available.
            min_collapse_batch: 2,
        });
        let messages = make_conversation(5, 200);
        let tokens = estimate_messages_tokens(&messages);
        let context_window = tokens; // 100% usage.

        let mut store = CollapseStore::new();
        let summarizer = MockSummarizer::ok("summary");

        let result = engine
            .collapse(&messages, context_window, &mut store, &summarizer)
            .await
            .unwrap();

        // Triggered but no rounds eligible (all preserved).
        assert!(result.span.is_none());
        assert!(store.is_empty());
    }
}
