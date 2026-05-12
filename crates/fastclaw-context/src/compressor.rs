use fastclaw_core::types::{ChatMessage, Role};
use serde_json;

/// Tunings for tiered (recent / summary / archive) compression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressorConfig {
    /// Full-text rounds kept at the tail (a round starts at each user message).
    pub recent_window: usize,
    /// Rounds before the recent window that are stored as short summaries.
    pub summary_window: usize,
    /// When summarising, keep fenced code blocks.
    pub preserve_code_blocks: bool,
    /// When summarising, keep lines that look like identifiers / entities.
    pub preserve_entities: bool,
}

impl Default for CompressorConfig {
    fn default() -> Self {
        Self {
            recent_window: 10,
            summary_window: 20,
            preserve_code_blocks: true,
            preserve_entities: true,
        }
    }
}

/// Optional LLM-backed summarizer for mid / archive tiers (caller provides a small model).
pub trait LlmLayerSummarizer: Send + Sync {
    /// Summarise a block of plain text (already rule-cleaned).
    fn summarize_block(&self, text: &str) -> anyhow::Result<String>;
}

/// Importance tiers for [`CompactionStrategy::ImportanceBased`] (higher = retained longer).
/// System prompts are never scored here: they live in the `system_msgs` partition and are always kept.
pub const IMPORTANCE_SYSTEM: u32 = 100;
pub const IMPORTANCE_RECENT_MESSAGES: u32 = 80;
pub const IMPORTANCE_ASSISTANT_WITH_TOOL_CALLS: u32 = 60;
pub const IMPORTANCE_DEFAULT_CONVERSATION: u32 = 40;
pub const IMPORTANCE_TOOL_RESULTS: u32 = 30;

/// Max non-system messages kept after importance-based compaction.
/// The context engine’s default compaction trigger uses the same value so one pass usually suffices.
pub const DEFAULT_IMPORTANCE_MAX_MESSAGES: usize = 60;

const DEFAULT_CHARS_PER_TOKEN: usize = 4;
const PER_MESSAGE_OVERHEAD: usize = 4;

/// Estimate the total token count for a slice of messages using the chars/4 heuristic.
/// Includes per-message overhead (~4 tokens for role/separators) and counts content + tool_call JSON.
pub fn estimate_messages_tokens(messages: &[ChatMessage]) -> usize {
    messages.iter().map(estimate_single_message_tokens).sum()
}

fn estimate_single_message_tokens(msg: &ChatMessage) -> usize {
    let content_chars = msg.content.as_ref().map_or(0, |c| {
        serde_json::to_string(c).map(|s| s.len()).unwrap_or(0)
    });
    let tool_chars = msg.tool_calls.as_ref().map_or(0, |tc| {
        tc.iter()
            .map(|t| t.function.name.len() + t.function.arguments.len())
            .sum()
    });
    (content_chars + tool_chars) / DEFAULT_CHARS_PER_TOKEN + PER_MESSAGE_OVERHEAD
}

/// How many trailing conversational messages count as “recent” for [`IMPORTANCE_RECENT_MESSAGES`].
pub const DEFAULT_IMPORTANCE_RECENT_WINDOW: usize = 20;

/// Strategy for compacting conversation history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionStrategy {
    /// Keep the last N messages, summarize everything before that.
    SlidingWindow { keep_recent: usize },
    /// Score non-system messages by role/recency; evict lowest scores first until at most
    /// `max_messages` conversational messages remain.
    ImportanceBased {
        max_messages: usize,
        recent_window: usize,
    },
    /// Keep messages within an estimated token budget.
    TokenBudget { max_tokens: usize },
    /// Keep only system + last user/assistant pair + summary.
    Aggressive,
    /// Tiered: recent rounds verbatim, next band summarized, oldest band high-signal only.
    Layered(CompressorConfig),
}

impl Default for CompactionStrategy {
    fn default() -> Self {
        Self::ImportanceBased {
            max_messages: DEFAULT_IMPORTANCE_MAX_MESSAGES,
            recent_window: DEFAULT_IMPORTANCE_RECENT_WINDOW,
        }
    }
}

/// Result of a compaction operation.
#[derive(Debug, Clone)]
pub struct CompactionResult {
    pub messages: Vec<ChatMessage>,
    pub summary: Option<String>,
    pub original_count: usize,
    pub compacted_count: usize,
    pub evicted_count: usize,
}

/// Strip `image_url` content parts from all messages.
///
/// For models that don't support multimodal input (e.g. DeepSeek), `image_url`
/// parts in content arrays cause deserialization errors on the API side.
///
/// This function:
/// - Removes `image_url` parts from content arrays, keeping only text parts.
/// - If a content array becomes a single text part, collapses it back to a
///   plain string for cleaner serialization.
/// - If all parts were images (no text), replaces content with "[image omitted]".
pub fn strip_image_content(messages: &mut [ChatMessage]) {
    let mut count = 0usize;
    for msg in messages.iter_mut() {
        let arr = match &msg.content {
            Some(serde_json::Value::Array(a))
                if a.iter()
                    .any(|p| p.get("type").and_then(|v| v.as_str()) == Some("image_url")) =>
            {
                a.clone()
            }
            _ => continue,
        };

        let text_parts: Vec<&serde_json::Value> = arr
            .iter()
            .filter(|p| p.get("type").and_then(|v| v.as_str()) != Some("image_url"))
            .collect();

        msg.content = if text_parts.is_empty() {
            Some(serde_json::Value::String("[image omitted]".to_string()))
        } else if text_parts.len() == 1 {
            if let Some(t) = text_parts[0].get("text").and_then(|v| v.as_str()) {
                Some(serde_json::Value::String(t.to_string()))
            } else {
                Some(serde_json::Value::Array(
                    text_parts.into_iter().cloned().collect(),
                ))
            }
        } else {
            Some(serde_json::Value::Array(
                text_parts.into_iter().cloned().collect(),
            ))
        };
        count += 1;
    }
    if count > 0 {
        tracing::debug!(
            count,
            "strip_image_content: removed image_url parts from messages"
        );
    }
}

/// Repair broken `Assistant(tool_calls) → Tool` pairing in a message sequence.
///
/// Some APIs (notably DeepSeek) strictly require every assistant message that
/// carries `tool_calls` to be followed by `Tool` messages responding to each
/// call. After compaction/eviction this invariant may be violated.
///
/// Repairs applied:
/// - An `Assistant` message whose `tool_calls` reference IDs not present in the
///   immediately following `Tool` messages has its `tool_calls` stripped (the
///   content text is preserved).
/// - Orphan `Tool` messages that have no preceding `Assistant` with a matching
///   `tool_calls` entry are removed entirely.
pub fn sanitize_tool_call_pairing(messages: &mut Vec<ChatMessage>) {
    use std::collections::HashSet;

    // Pass 1: collect (index, set_of_tool_call_ids) for each assistant with tool_calls.
    let mut asst_indices: Vec<(usize, HashSet<String>)> = Vec::new();
    for (i, msg) in messages.iter().enumerate() {
        if msg.role == Role::Assistant {
            if let Some(ref tc) = msg.tool_calls {
                if !tc.is_empty() {
                    let ids: HashSet<String> = tc.iter().map(|c| c.id.clone()).collect();
                    asst_indices.push((i, ids));
                }
            }
        }
    }

    if asst_indices.is_empty() {
        return;
    }

    // Pass 2: for each assistant(tool_calls), check the immediately following
    // contiguous Tool messages. Build a set of tool_call_ids that are answered.
    let mut strip_tool_calls_at: Vec<usize> = Vec::new();
    for &(asst_idx, ref expected_ids) in &asst_indices {
        let mut answered: HashSet<&str> = HashSet::new();
        for msg in &messages[(asst_idx + 1)..] {
            if msg.role != Role::Tool {
                break;
            }
            if let Some(ref tcid) = msg.tool_call_id {
                answered.insert(tcid.as_str());
            }
        }
        if !expected_ids.iter().all(|id| answered.contains(id.as_str())) {
            strip_tool_calls_at.push(asst_idx);
        }
    }

    for &idx in &strip_tool_calls_at {
        messages[idx].tool_calls = None;
        tracing::debug!(
            idx,
            "sanitize_tool_call_pairing: stripped orphaned tool_calls from assistant message"
        );
    }

    // Pass 2b: Remove assistant messages that became empty after stripping tool_calls.
    // LLM APIs require assistant messages to have `content` or `tool_calls`;
    // `reasoning_content` alone does NOT satisfy this constraint (it's metadata,
    // not a valid substitute), so we only check content and tool_calls here.
    if !strip_tool_calls_at.is_empty() {
        let before = messages.len();
        messages.retain(|m| {
            if m.role != Role::Assistant {
                return true;
            }
            let has_tool_calls = m.tool_calls.as_ref().is_some_and(|tc| !tc.is_empty());
            let has_content = m.content.as_ref().is_some_and(|c| match c {
                serde_json::Value::String(s) => !s.trim().is_empty(),
                serde_json::Value::Array(arr) => !arr.is_empty(),
                _ => false,
            });
            has_tool_calls || has_content
        });
        let removed = before - messages.len();
        if removed > 0 {
            tracing::debug!(
                removed,
                "sanitize_tool_call_pairing: removed empty assistant messages after tool_call strip"
            );
        }
    }

    // Pass 3: remove orphan Tool messages whose tool_call_id doesn't match
    // any preceding assistant's tool_calls.
    let valid_ids: HashSet<String> = messages
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .filter_map(|m| m.tool_calls.as_ref())
        .flat_map(|tc| tc.iter().map(|c| c.id.clone()))
        .collect();

    let before_len = messages.len();
    messages.retain(|m| {
        if m.role != Role::Tool {
            return true;
        }
        match &m.tool_call_id {
            Some(id) => valid_ids.contains(id),
            None => false,
        }
    });
    let removed = before_len - messages.len();
    if removed > 0 {
        tracing::debug!(
            removed,
            "sanitize_tool_call_pairing: removed orphan tool messages"
        );
    }
}

/// Defence-in-depth: remove any assistant message that has neither `content`
/// nor `tool_calls`.  Most LLM APIs (OpenAI, DeepSeek, etc.) return 400 for
/// such messages.  This function is cheap (single pass, no allocations when
/// nothing is removed) and should be called right before the LLM request.
pub fn ensure_valid_assistant_messages(messages: &mut Vec<ChatMessage>) {
    let before = messages.len();
    messages.retain(|m| {
        if m.role != Role::Assistant {
            return true;
        }
        let has_content = m.content.as_ref().is_some_and(|c| match c {
            serde_json::Value::String(s) => !s.trim().is_empty(),
            serde_json::Value::Array(arr) => !arr.is_empty(),
            _ => false,
        });
        let has_tool_calls = m.tool_calls.as_ref().is_some_and(|tc| !tc.is_empty());
        has_content || has_tool_calls
    });
    let removed = before - messages.len();
    if removed > 0 {
        tracing::warn!(
            removed,
            "ensure_valid_assistant_messages: dropped invalid assistant messages \
             (no content, no tool_calls)"
        );
    }
}

/// Compacts conversation history to stay within context window limits.
///
/// This is a rule-based compactor that doesn't require LLM calls.
/// It uses heuristic summarization of older messages.
pub struct ContextCompactor {
    strategy: CompactionStrategy,
    chars_per_token: usize,
}

impl ContextCompactor {
    pub fn new(strategy: CompactionStrategy) -> Self {
        Self {
            strategy,
            chars_per_token: 4,
        }
    }

    pub fn with_chars_per_token(mut self, ratio: usize) -> Self {
        self.chars_per_token = ratio;
        self
    }

    /// Compact a list of messages according to the configured strategy.
    ///
    /// Returns the compacted messages plus an optional summary of evicted content.
    pub fn compact(&self, messages: &[ChatMessage]) -> CompactionResult {
        let original_count = messages.len();

        if messages.is_empty() {
            return CompactionResult {
                messages: Vec::new(),
                summary: None,
                original_count: 0,
                compacted_count: 0,
                evicted_count: 0,
            };
        }

        let (system_msgs, conversation): (Vec<_>, Vec<_>) = messages
            .iter()
            .partition(|m| matches!(m.role, Role::System));

        let mut result = match self.strategy {
            CompactionStrategy::SlidingWindow { keep_recent } => self.compact_sliding_window(
                &system_msgs,
                &conversation,
                keep_recent,
                original_count,
            ),
            CompactionStrategy::ImportanceBased {
                max_messages,
                recent_window,
            } => self.compact_importance_based(
                &system_msgs,
                &conversation,
                max_messages,
                recent_window,
                original_count,
            ),
            CompactionStrategy::TokenBudget { max_tokens } => {
                self.compact_token_budget(&system_msgs, &conversation, max_tokens, original_count)
            }
            CompactionStrategy::Aggressive => {
                self.compact_aggressive(&system_msgs, &conversation, original_count)
            }
            CompactionStrategy::Layered(ref cfg) => {
                self.compact_layered(&system_msgs, &conversation, cfg, None, original_count)
            }
        };
        sanitize_tool_call_pairing(&mut result.messages);
        result.compacted_count = result.messages.len();
        result
    }

    /// Same as [`Self::compact`], but mid / archive tiers may call `llm` when provided.
    pub fn compact_with_optional_llm(
        &self,
        messages: &[ChatMessage],
        llm: Option<&dyn LlmLayerSummarizer>,
    ) -> CompactionResult {
        let original_count = messages.len();
        if messages.is_empty() {
            return CompactionResult {
                messages: Vec::new(),
                summary: None,
                original_count: 0,
                compacted_count: 0,
                evicted_count: 0,
            };
        }
        let (system_msgs, conversation): (Vec<_>, Vec<_>) = messages
            .iter()
            .partition(|m| matches!(m.role, Role::System));
        let mut result = match &self.strategy {
            CompactionStrategy::Layered(cfg) => {
                self.compact_layered(&system_msgs, &conversation, cfg, llm, original_count)
            }
            _ => return self.compact(messages),
        };
        sanitize_tool_call_pairing(&mut result.messages);
        result.compacted_count = result.messages.len();
        result
    }

    fn importance_score(msg: &ChatMessage, idx: usize, n: usize, recent_window: usize) -> u32 {
        let pos_from_end = n.saturating_sub(1).saturating_sub(idx);
        if pos_from_end < recent_window {
            return IMPORTANCE_RECENT_MESSAGES;
        }
        match msg.role {
            Role::Tool => IMPORTANCE_TOOL_RESULTS,
            Role::Assistant if msg.tool_calls.as_ref().is_some_and(|t| !t.is_empty()) => {
                IMPORTANCE_ASSISTANT_WITH_TOOL_CALLS
            }
            Role::User | Role::Assistant => IMPORTANCE_DEFAULT_CONVERSATION,
            Role::System => IMPORTANCE_SYSTEM,
        }
    }

    fn compact_importance_based(
        &self,
        system_msgs: &[&ChatMessage],
        conversation: &[&ChatMessage],
        max_messages: usize,
        recent_window: usize,
        original_count: usize,
    ) -> CompactionResult {
        let max_messages = max_messages.max(1);
        let n = conversation.len();
        if n <= max_messages {
            let mut result: Vec<ChatMessage> = system_msgs.iter().copied().cloned().collect();
            result.extend(conversation.iter().copied().cloned());
            return CompactionResult {
                compacted_count: result.len(),
                messages: result,
                summary: None,
                original_count,
                evicted_count: 0,
            };
        }

        let mut scored: Vec<(usize, u32)> = (0..n)
            .map(|i| {
                let s = Self::importance_score(conversation[i], i, n, recent_window);
                (i, s)
            })
            .collect();
        // Evict lowest importance first; tie-break: earlier message (smaller index) first.
        scored.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));

        let remove_count = n - max_messages;
        let remove_idx: std::collections::HashSet<usize> =
            scored.iter().take(remove_count).map(|(i, _)| *i).collect();

        let mut kept: Vec<ChatMessage> = Vec::new();
        let mut evicted: Vec<&ChatMessage> = Vec::new();
        for (i, msg) in conversation.iter().enumerate().take(n) {
            if remove_idx.contains(&i) {
                evicted.push(*msg);
            } else {
                kept.push((**msg).clone());
            }
        }

        let summary = self.summarize_messages(&evicted);
        let mut result: Vec<ChatMessage> = system_msgs.iter().copied().cloned().collect();
        if !summary.is_empty() {
            result.push(ChatMessage {
                role: Role::System,
                content: Some(format!("[Conversation history summary]\n{summary}").into()),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
            });
        }
        result.extend(kept);

        CompactionResult {
            compacted_count: result.len(),
            messages: result,
            summary: Some(summary),
            original_count,
            evicted_count: evicted.len(),
        }
    }

    fn compact_sliding_window(
        &self,
        system_msgs: &[&ChatMessage],
        conversation: &[&ChatMessage],
        keep_recent: usize,
        original_count: usize,
    ) -> CompactionResult {
        if conversation.len() <= keep_recent {
            let mut result: Vec<ChatMessage> = system_msgs.iter().cloned().cloned().collect();
            result.extend(conversation.iter().cloned().cloned());
            return CompactionResult {
                compacted_count: result.len(),
                messages: result,
                summary: None,
                original_count,
                evicted_count: 0,
            };
        }

        let split = conversation.len() - keep_recent;
        let evicted = &conversation[..split];
        let kept = &conversation[split..];

        let summary = self.summarize_messages(evicted);

        let mut result: Vec<ChatMessage> = system_msgs.iter().cloned().cloned().collect();
        result.push(ChatMessage {
            role: Role::System,
            content: Some(format!("[Conversation history summary]\n{summary}").into()),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        });
        result.extend(kept.iter().cloned().cloned());

        CompactionResult {
            compacted_count: result.len(),
            messages: result,
            summary: Some(summary),
            original_count,
            evicted_count: evicted.len(),
        }
    }

    fn compact_token_budget(
        &self,
        system_msgs: &[&ChatMessage],
        conversation: &[&ChatMessage],
        max_tokens: usize,
        original_count: usize,
    ) -> CompactionResult {
        let system_tokens: usize = system_msgs.iter().map(|m| self.estimate_tokens(m)).sum();
        let remaining_budget = max_tokens.saturating_sub(system_tokens);

        let mut kept = Vec::new();
        let mut used_tokens = 0;
        for msg in conversation.iter().rev() {
            let tokens = self.estimate_tokens(msg);
            if used_tokens + tokens > remaining_budget && !kept.is_empty() {
                break;
            }
            kept.push(*msg);
            used_tokens += tokens;
        }
        kept.reverse();

        let evicted_count = conversation.len() - kept.len();
        let evicted = &conversation[..evicted_count];
        let summary = if evicted.is_empty() {
            None
        } else {
            Some(self.summarize_messages(evicted))
        };

        let mut result: Vec<ChatMessage> = system_msgs.iter().cloned().cloned().collect();
        if let Some(ref s) = summary {
            result.push(ChatMessage {
                role: Role::System,
                content: Some(format!("[Conversation history summary]\n{s}").into()),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
            });
        }
        result.extend(kept.into_iter().cloned());

        CompactionResult {
            compacted_count: result.len(),
            messages: result,
            summary,
            original_count,
            evicted_count,
        }
    }

    fn compact_aggressive(
        &self,
        system_msgs: &[&ChatMessage],
        conversation: &[&ChatMessage],
        original_count: usize,
    ) -> CompactionResult {
        let summary = self.summarize_messages(conversation);

        let last_pair: Vec<_> = conversation.iter().rev().take(2).cloned().collect();
        let mut last_pair_ordered: Vec<_> = last_pair.into_iter().rev().cloned().collect();

        let mut result: Vec<ChatMessage> = system_msgs.iter().cloned().cloned().collect();
        result.push(ChatMessage {
            role: Role::System,
            content: Some(format!("[Full conversation summary]\n{summary}").into()),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        });
        result.append(&mut last_pair_ordered);

        let evicted_count = conversation.len().saturating_sub(2);

        CompactionResult {
            compacted_count: result.len(),
            messages: result,
            summary: Some(summary),
            original_count,
            evicted_count,
        }
    }

    fn compact_layered(
        &self,
        system_msgs: &[&ChatMessage],
        conversation: &[&ChatMessage],
        cfg: &CompressorConfig,
        llm: Option<&dyn LlmLayerSummarizer>,
        original_count: usize,
    ) -> CompactionResult {
        let rounds = split_into_rounds(conversation);
        if rounds.is_empty() {
            let result: Vec<ChatMessage> = system_msgs.iter().copied().cloned().collect();
            return CompactionResult {
                compacted_count: result.len(),
                messages: result,
                summary: None,
                original_count,
                evicted_count: 0,
            };
        }

        let total = rounds.len();
        let n = cfg.recent_window.max(1);
        let m = cfg.summary_window;

        let recent_start = total.saturating_sub(n);
        let summary_start = total.saturating_sub(n.saturating_add(m));

        let archive_rounds = &rounds[..summary_start];
        let summary_rounds = &rounds[summary_start..recent_start];
        let recent_rounds = &rounds[recent_start..];

        let mut summary_parts = Vec::new();

        for r in archive_rounds {
            if let Some(line) = summarize_round_archive(r, cfg, self.chars_per_token) {
                summary_parts.push(line);
            }
        }

        for r in summary_rounds {
            let block = flatten_round_text(r);
            let cleaned = rule_compress_text(&block, cfg);
            let piece = if let Some(llm) = llm {
                llm.summarize_block(&cleaned)
                    .unwrap_or_else(|_| summarize_round_rule(r, cfg))
            } else {
                summarize_round_rule(r, cfg)
            };
            if !piece.is_empty() {
                summary_parts.push(piece);
            }
        }

        let mut recent_msgs: Vec<ChatMessage> = Vec::new();
        for r in recent_rounds {
            for m in r {
                recent_msgs.push((*m).clone());
            }
        }

        let combined_summary = if summary_parts.is_empty() {
            None
        } else {
            Some(summary_parts.join("\n"))
        };

        let evicted = conversation.len().saturating_sub(recent_msgs.len());

        let mut result: Vec<ChatMessage> = system_msgs.iter().copied().cloned().collect();
        if let Some(ref s) = combined_summary {
            result.push(ChatMessage {
                role: Role::System,
                content: Some(
                    format!("[Layered conversation summary — older & mid bands]\n{s}").into(),
                ),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
            });
        }
        result.extend(recent_msgs);

        CompactionResult {
            compacted_count: result.len(),
            messages: result,
            summary: combined_summary,
            original_count,
            evicted_count: evicted,
        }
    }

    fn estimate_tokens(&self, msg: &ChatMessage) -> usize {
        let content_chars = msg.content.as_ref().map_or(0, |c| {
            serde_json::to_string(c)
                .map(|s| s.chars().count())
                .unwrap_or(0)
        });
        let tool_chars = msg.tool_calls.as_ref().map_or(0, |tc| {
            tc.iter()
                .map(|t| t.function.name.len() + t.function.arguments.chars().count())
                .sum()
        });
        let overhead = 4;
        (content_chars + tool_chars) / self.chars_per_token + overhead
    }

    /// Heuristic summarization without requiring an LLM call.
    /// Extracts key information from evicted messages.
    fn summarize_messages(&self, messages: &[&ChatMessage]) -> String {
        if messages.is_empty() {
            return String::new();
        }

        let mut topics = Vec::new();
        let mut tool_calls_seen = Vec::new();
        let mut user_questions = 0;
        let mut assistant_responses = 0;

        for msg in messages {
            match msg.role {
                Role::User => {
                    user_questions += 1;
                    if let Some(content) = msg.text_content() {
                        let preview = if content.len() > 80 {
                            let end = content
                                .char_indices()
                                .map(|(i, _)| i)
                                .take_while(|&i| i <= 77)
                                .last()
                                .unwrap_or(0);
                            format!("{}...", &content[..end])
                        } else {
                            content.clone()
                        };
                        topics.push(format!("- User: {preview}"));
                    }
                }
                Role::Assistant => {
                    assistant_responses += 1;
                    if let Some(tc) = &msg.tool_calls {
                        for call in tc {
                            let args_preview = if call.function.arguments.len() > 60 {
                                let end = call
                                    .function
                                    .arguments
                                    .char_indices()
                                    .map(|(i, _)| i)
                                    .take_while(|&i| i <= 57)
                                    .last()
                                    .unwrap_or(0);
                                format!("{}...", &call.function.arguments[..end])
                            } else {
                                call.function.arguments.clone()
                            };
                            tool_calls_seen
                                .push(format!("{}({})", call.function.name, args_preview));
                        }
                    }
                    if let Some(content) = msg.text_content() {
                        if content.len() > 200 {
                            let end = content
                                .char_indices()
                                .map(|(i, _)| i)
                                .take_while(|&i| i <= 197)
                                .last()
                                .unwrap_or(0);
                            let preview = &content[..end];
                            topics.push(format!("- Assistant: {preview}..."));
                        }
                    }
                }
                Role::Tool => {
                    // tool results are context, skip in summary
                }
                Role::System => {
                    // system messages preserved separately
                }
            }
        }

        let mut summary = format!(
            "Previous conversation ({user_questions} user messages, {assistant_responses} assistant responses)."
        );

        if !tool_calls_seen.is_empty() {
            let unique: Vec<_> = {
                let mut seen = std::collections::HashSet::new();
                tool_calls_seen
                    .into_iter()
                    .filter(|t| seen.insert(t.clone()))
                    .collect()
            };
            let max_tool_entries = 15;
            let tool_display: Vec<_> = if unique.len() > max_tool_entries {
                let mut v = unique[..max_tool_entries].to_vec();
                v.push(format!("...+{} more", unique.len() - max_tool_entries));
                v
            } else {
                unique
            };
            summary.push_str("\nTool calls:\n");
            for tc in &tool_display {
                summary.push_str(&format!("  - {tc}\n"));
            }
        }

        if !topics.is_empty() {
            let max_topics = 10;
            let shown: Vec<_> = if topics.len() > max_topics {
                let mut t = topics[..max_topics].to_vec();
                t.push(format!(
                    "... and {} more exchanges",
                    topics.len() - max_topics
                ));
                t
            } else {
                topics
            };
            summary.push_str("\nKey exchanges:\n");
            summary.push_str(&shown.join("\n"));
        }

        summary
    }
}

fn split_into_rounds<'a>(conversation: &'a [&'a ChatMessage]) -> Vec<Vec<&'a ChatMessage>> {
    let mut rounds: Vec<Vec<&ChatMessage>> = Vec::new();
    let mut i = 0usize;
    while i < conversation.len() {
        let start = i;
        i += 1;
        while i < conversation.len() && !matches!(conversation[i].role, Role::User) {
            i += 1;
        }
        rounds.push(conversation[start..i].to_vec());
    }
    rounds
}

fn flatten_round_text(r: &[&ChatMessage]) -> String {
    let mut s = String::new();
    for m in r {
        let header = match m.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
            Role::Tool => "Tool",
            Role::System => "System",
        };
        s.push_str(header);
        s.push_str(": ");
        if let Some(c) = m.text_content() {
            s.push_str(&c);
        }
        if let Some(tc) = &m.tool_calls {
            for t in tc {
                s.push_str(&format!(" [tool {}]", t.function.name));
            }
        }
        s.push('\n');
    }
    s
}

fn message_importance(msg: &ChatMessage) -> u32 {
    let mut score = 0u32;
    if let Some(ref c) = msg.text_content() {
        if c.contains("```") {
            score += 4;
        }
        if c.contains("http://") || c.contains("https://") {
            score += 2;
        }
        if c.chars().count() > 350 {
            score += 1;
        }
    }
    if msg.tool_calls.as_ref().is_some_and(|t| !t.is_empty()) {
        score += 3;
    }
    score
}

fn round_importance(round: &[&ChatMessage]) -> u32 {
    round.iter().map(|m| message_importance(m)).sum()
}

fn is_small_talk_line(line: &str) -> bool {
    let t = line.trim();
    if t.len() <= 2 {
        return true;
    }
    let lower = t.to_lowercase();
    const GREETINGS: &[&str] = &[
        "thanks",
        "thank you",
        "谢谢",
        "你好",
        "您好",
        "hi",
        "hello",
        "好的",
        "收到",
        "没问题",
        "ok",
        "okay",
    ];
    GREETINGS
        .iter()
        .any(|g| lower.contains(*g) && lower.len() < 28)
}

/// Rule-only cleanup: drop light-weight lines, dedupe, cap length, optionally keep entity-like tokens.
fn rule_compress_text(s: &str, cfg: &CompressorConfig) -> String {
    let lines: Vec<&str> = s.lines().filter(|l| !is_small_talk_line(l)).collect();
    let mut deduped: Vec<String> = Vec::new();
    for line in lines {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if deduped.last().map(|x: &String| x.as_str()) != Some(t) {
            deduped.push(t.to_string());
        }
    }
    let mut joined = deduped.join("\n");
    if cfg.preserve_entities {
        joined = append_entity_hints(&joined);
    }
    if cfg.preserve_code_blocks {
        let blocks = extract_code_blocks(s);
        for b in blocks {
            if !joined.contains(&b[..b.len().min(40)]) {
                joined.push_str("\n```\n");
                joined.push_str(&b);
                joined.push_str("\n```\n");
            }
        }
    }
    if joined.chars().count() > 520 {
        joined.chars().take(520).collect::<String>() + "…"
    } else {
        joined
    }
}

fn append_entity_hints(s: &str) -> String {
    let mut out = s.to_string();
    let re = regex::Regex::new(r"\b[A-Z][a-z][a-zA-Z0-9]{2,}\b").ok();
    if let Some(re) = re {
        let mut seen = std::collections::HashSet::new();
        let mut hints = Vec::new();
        for cap in re.find_iter(s) {
            let t = cap.as_str();
            if seen.insert(t.to_string()) {
                hints.push(t.to_string());
            }
            if hints.len() >= 12 {
                break;
            }
        }
        if !hints.is_empty() {
            out.push_str("\n[entities] ");
            out.push_str(&hints.join(", "));
        }
    }
    out
}

fn extract_code_blocks(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_block = false;
    let mut buf = String::new();
    for line in s.lines() {
        if line.trim_start().starts_with("```") {
            if in_block && !buf.is_empty() {
                out.push(buf.trim_end().to_string());
                buf.clear();
            }
            in_block = !in_block;
            continue;
        }
        if in_block {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    out
}

fn summarize_round_rule(r: &[&ChatMessage], cfg: &CompressorConfig) -> String {
    let flat = flatten_round_text(r);
    rule_compress_text(&flat, cfg)
}

fn summarize_round_archive(
    round: &[&ChatMessage],
    cfg: &CompressorConfig,
    _chars_per_token: usize,
) -> Option<String> {
    let imp = round_importance(round);
    if imp < 2 {
        return None;
    }
    Some(summarize_round_rule(round, cfg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastclaw_core::types::Role;

    fn user(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::User,
            content: Some(text.to_string().into()),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn assistant(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::Assistant,
            content: Some(text.to_string().into()),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn system(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::System,
            content: Some(text.to_string().into()),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    #[test]
    fn no_compaction_needed() {
        let compactor =
            ContextCompactor::new(CompactionStrategy::SlidingWindow { keep_recent: 10 });
        let msgs = vec![user("hi"), assistant("hello")];
        let result = compactor.compact(&msgs);
        assert_eq!(result.evicted_count, 0);
        assert_eq!(result.compacted_count, 2);
        assert!(result.summary.is_none());
    }

    #[test]
    fn sliding_window_compaction() {
        let compactor = ContextCompactor::new(CompactionStrategy::SlidingWindow { keep_recent: 2 });
        let msgs = vec![
            user("first question"),
            assistant("first answer"),
            user("second question"),
            assistant("second answer"),
            user("third question"),
            assistant("third answer"),
        ];
        let result = compactor.compact(&msgs);
        assert_eq!(result.evicted_count, 4);
        assert_eq!(result.compacted_count, 3); // summary + 2 kept
        assert!(result.summary.is_some());
        assert!(result.summary.unwrap().contains("user messages"));
        assert_eq!(
            result.messages.last().unwrap().text_content().as_deref(),
            Some("third answer")
        );
    }

    #[test]
    fn preserves_system_prompt() {
        let compactor = ContextCompactor::new(CompactionStrategy::SlidingWindow { keep_recent: 1 });
        let msgs = vec![
            system("You are helpful"),
            user("old question"),
            assistant("old answer"),
            user("new question"),
        ];
        let result = compactor.compact(&msgs);
        assert!(matches!(result.messages[0].role, Role::System));
        assert_eq!(
            result.messages[0].text_content().as_deref(),
            Some("You are helpful")
        );
        assert!(result.messages[1]
            .text_content()
            .as_deref()
            .unwrap()
            .contains("summary"));
    }

    #[test]
    fn aggressive_compaction() {
        let compactor = ContextCompactor::new(CompactionStrategy::Aggressive);
        let msgs = vec![
            user("q1"),
            assistant("a1"),
            user("q2"),
            assistant("a2"),
            user("q3"),
            assistant("a3"),
            user("q4"),
            assistant("a4"),
        ];
        let result = compactor.compact(&msgs);
        assert!(result.summary.is_some());
        // Should keep summary + last 2 messages
        assert!(result.compacted_count <= 3);
        assert!(result.evicted_count >= 6);
    }

    #[test]
    fn token_budget_compaction() {
        let compactor = ContextCompactor::new(CompactionStrategy::TokenBudget { max_tokens: 50 });
        let msgs = vec![
            user("a very long question that takes many tokens to represent in the context"),
            assistant("a very long answer that also takes many tokens to represent properly"),
            user("short q"),
            assistant("short a"),
        ];
        let result = compactor.compact(&msgs);
        assert!(result.compacted_count <= msgs.len());
        assert!(result.messages.last().unwrap().text_content().as_deref() == Some("short a"));
    }

    #[test]
    fn empty_input() {
        let compactor = ContextCompactor::new(CompactionStrategy::default());
        let result = compactor.compact(&[]);
        assert_eq!(result.original_count, 0);
        assert_eq!(result.compacted_count, 0);
        assert!(result.messages.is_empty());
    }

    #[test]
    fn importance_based_evicts_lower_scores_first() {
        fn tool(out: &str, id: &str) -> ChatMessage {
            ChatMessage {
                role: Role::Tool,
                content: Some(out.to_string().into()),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: Some(id.to_string()),
            }
        }
        fn asst_tools() -> ChatMessage {
            ChatMessage {
                role: Role::Assistant,
                content: None,
                reasoning_content: None,
                name: None,
                tool_calls: Some(vec![fastclaw_core::types::ToolCall {
                    id: "1".into(),
                    call_type: "function".into(),
                    function: fastclaw_core::types::FunctionCall {
                        name: "x".into(),
                        arguments: "{}".into(),
                    },
                    output: None,
                    success: None,
                    duration_ms: None,
                }]),
                tool_call_id: None,
            }
        }
        let compactor = ContextCompactor::new(CompactionStrategy::ImportanceBased {
            max_messages: 4,
            recent_window: 2,
        });
        // Old band: user + tool (low score) + assistant with tools; tail: user + user (recent).
        let msgs = vec![
            user("old"),
            tool("blob", "a"),
            asst_tools(),
            user("q3"),
            user("q4"),
        ];
        let result = compactor.compact(&msgs);
        assert!(result.evicted_count >= 1);
        assert!(result.compacted_count <= msgs.len() + 2);
        assert!(
            result
                .messages
                .iter()
                .any(|m| { m.text_content().as_deref().unwrap_or("").contains("q4") }),
            "recent user message should survive"
        );
    }

    #[test]
    fn layered_keeps_recent_verbatim() {
        let cfg = CompressorConfig {
            recent_window: 1,
            summary_window: 1,
            ..Default::default()
        };
        let compactor = ContextCompactor::new(CompactionStrategy::Layered(cfg));
        let msgs = vec![
            user("first"),
            assistant("a1"),
            user("second"),
            assistant("a2"),
            user("third"),
        ];
        let result = compactor.compact(&msgs);
        assert!(result.summary.is_some());
        assert_eq!(
            result.messages.last().unwrap().text_content().as_deref(),
            Some("third")
        );
    }

    fn assistant_with_tool_calls(
        content: Option<&str>,
        reasoning: Option<&str>,
        tc_ids: &[&str],
    ) -> ChatMessage {
        use fastclaw_core::types::{FunctionCall, ToolCall};
        ChatMessage {
            role: Role::Assistant,
            content: content.map(|s| serde_json::Value::String(s.to_string())),
            reasoning_content: reasoning.map(|s| s.to_string()),
            name: None,
            tool_calls: if tc_ids.is_empty() {
                None
            } else {
                Some(
                    tc_ids
                        .iter()
                        .map(|id| ToolCall {
                            id: id.to_string(),
                            call_type: "function".to_string(),
                            function: FunctionCall {
                                name: "test_tool".to_string(),
                                arguments: "{}".to_string(),
                            },
                            output: None,
                            success: None,
                            duration_ms: None,
                        })
                        .collect(),
                )
            },
            tool_call_id: None,
        }
    }

    fn tool_result(call_id: &str, output: &str) -> ChatMessage {
        ChatMessage {
            role: Role::Tool,
            content: Some(serde_json::Value::String(output.to_string())),
            reasoning_content: None,
            name: Some("test_tool".to_string()),
            tool_calls: None,
            tool_call_id: Some(call_id.to_string()),
        }
    }

    #[test]
    fn sanitize_removes_reasoning_only_assistant_after_strip() {
        let mut messages = vec![
            user("do something"),
            assistant_with_tool_calls(None, Some("thinking about tools..."), &["tc1"]),
            user("next question"),
        ];
        sanitize_tool_call_pairing(&mut messages);
        assert_eq!(
            messages.len(),
            2,
            "assistant with only reasoning_content should be removed"
        );
        assert!(messages.iter().all(|m| m.role != Role::Assistant));
    }

    #[test]
    fn sanitize_keeps_assistant_with_content_after_strip() {
        let mut messages = vec![
            user("do something"),
            assistant_with_tool_calls(Some("I'll help"), Some("thinking..."), &["tc1"]),
            user("next question"),
        ];
        sanitize_tool_call_pairing(&mut messages);
        assert_eq!(messages.len(), 3, "assistant with content should be kept");
        let asst = messages.iter().find(|m| m.role == Role::Assistant).unwrap();
        assert!(
            asst.tool_calls.is_none(),
            "orphaned tool_calls should be stripped"
        );
        assert_eq!(asst.text_content().as_deref(), Some("I'll help"));
    }

    #[test]
    fn sanitize_keeps_paired_tool_calls_with_reasoning() {
        let mut messages = vec![
            user("do something"),
            assistant_with_tool_calls(None, Some("thinking..."), &["tc1"]),
            tool_result("tc1", "result"),
            user("next question"),
        ];
        sanitize_tool_call_pairing(&mut messages);
        assert_eq!(messages.len(), 4, "fully paired tool calls should be kept");
        let asst = messages.iter().find(|m| m.role == Role::Assistant).unwrap();
        assert!(asst.tool_calls.is_some());
    }

    #[test]
    fn ensure_valid_drops_reasoning_only_assistant() {
        let mut messages = vec![
            user("hi"),
            ChatMessage {
                role: Role::Assistant,
                content: None,
                reasoning_content: Some("deep thoughts".to_string()),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
            user("next"),
        ];
        ensure_valid_assistant_messages(&mut messages);
        assert_eq!(messages.len(), 2);
        assert!(messages.iter().all(|m| m.role != Role::Assistant));
    }

    #[test]
    fn ensure_valid_keeps_assistant_with_content() {
        let mut messages = vec![user("hi"), assistant("hello"), user("next")];
        ensure_valid_assistant_messages(&mut messages);
        assert_eq!(messages.len(), 3);
    }

    #[test]
    fn ensure_valid_keeps_assistant_with_tool_calls() {
        let mut messages = vec![
            user("do something"),
            assistant_with_tool_calls(None, None, &["tc1"]),
            tool_result("tc1", "done"),
        ];
        ensure_valid_assistant_messages(&mut messages);
        assert_eq!(messages.len(), 3);
    }
}
