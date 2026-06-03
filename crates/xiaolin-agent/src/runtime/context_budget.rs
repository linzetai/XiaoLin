use xiaolin_core::types::{ChatMessage, Role};

use super::tool_executor::{
    build_cleared_with_recall, classify_retention_tier, summarize_tool_result, RetentionTier,
    RECALL_HINT_MARKER,
};

/// Semantic importance score for a tool result.
///
/// Higher scores mean the result is more important to retain.
/// The score combines multiple signals beyond just tool name / retention tier.
#[derive(Debug, Clone)]
pub(crate) struct ToolResultImportance {
    pub score: f32,
}

impl ToolResultImportance {
    /// Score a tool result based on content signals, recency, and context.
    ///
    /// Factors:
    /// - Base score from retention tier (0.2 Ephemeral, 0.5 Summarize, 0.7 FullRetain)
    /// - Content complexity (code blocks, error messages, struct definitions)
    /// - Length bonus for substantial results (diminishing returns)
    /// - Recency bonus (recently generated results score higher)
    /// - Reference bonus (results whose content appears referenced in assistant msgs)
    pub fn score_tool_result(
        msg: &ChatMessage,
        msg_index: usize,
        total_messages: usize,
        assistant_messages: &[&ChatMessage],
    ) -> Self {
        let tool_name = msg.name.as_deref().unwrap_or("");
        let content = msg.text_content().unwrap_or_default();
        let tier = classify_retention_tier(tool_name);

        let base_score = match tier {
            RetentionTier::Ephemeral => 0.2,
            RetentionTier::Summarize => 0.5,
            RetentionTier::FullRetain => 0.7,
        };

        let content_signal_bonus = Self::content_signals(&content);
        let length_bonus = Self::length_bonus(content.len());
        let recency_bonus = Self::recency_bonus(msg_index, total_messages);
        let reference_bonus = Self::reference_bonus(&content, assistant_messages);

        let score =
            (base_score + content_signal_bonus + length_bonus + recency_bonus + reference_bonus)
                .clamp(0.0, 1.0);

        Self { score }
    }

    fn content_signals(content: &str) -> f32 {
        let mut bonus = 0.0_f32;

        if content.contains("```")
            || content.contains("fn ")
            || content.contains("def ")
            || content.contains("class ")
            || content.contains("impl ")
        {
            bonus += 0.10;
        }

        if content.contains("error")
            || content.contains("Error")
            || content.contains("FAILED")
            || content.contains("panic")
            || content.contains("exception")
        {
            bonus += 0.08;
        }

        if content.contains("struct ")
            || content.contains("enum ")
            || content.contains("interface ")
            || content.contains("type ")
            || content.contains("trait ")
        {
            bonus += 0.05;
        }

        if content.contains(".rs")
            || content.contains(".ts")
            || content.contains(".py")
            || content.contains(".go")
            || content.contains("src/")
        {
            bonus += 0.03;
        }

        bonus.min(0.20)
    }

    fn length_bonus(char_count: usize) -> f32 {
        match char_count {
            0..=100 => 0.0,
            101..=500 => 0.02,
            501..=2000 => 0.05,
            2001..=5000 => 0.07,
            _ => 0.08,
        }
    }

    fn recency_bonus(msg_index: usize, total_messages: usize) -> f32 {
        if total_messages == 0 {
            return 0.0;
        }
        let position_ratio = msg_index as f32 / total_messages as f32;
        (position_ratio * 0.15).min(0.15)
    }

    fn reference_bonus(content: &str, assistant_messages: &[&ChatMessage]) -> f32 {
        if content.len() < 10 || assistant_messages.is_empty() {
            return 0.0;
        }

        let key_fragments: Vec<&str> = content
            .lines()
            .take(5)
            .filter(|l| l.len() > 15)
            .take(3)
            .collect();

        if key_fragments.is_empty() {
            return 0.0;
        }

        let referenced = assistant_messages.iter().any(|asst_msg| {
            let asst_text = asst_msg.text_content().unwrap_or_default();
            key_fragments.iter().any(|frag| {
                let check = if frag.len() > 40 {
                    let end = frag.floor_char_boundary(40);
                    &frag[..end]
                } else {
                    frag
                };
                asst_text.contains(check)
            })
        });

        if referenced {
            0.12
        } else {
            0.0
        }
    }
}

/// Budget allocation fractions for the context window.
///
/// These fractions determine how the context window is divided among
/// different content categories.
#[derive(Debug, Clone)]
pub(crate) struct BudgetConfig {
    /// Fraction for system prompts + user messages (non-compressible).
    pub system_user_fraction: f32,
    /// Fraction for the most recent tool call results (sliding window).
    pub recent_tool_fraction: f32,
    /// Fraction for older tool call results (summary retention).
    pub older_tool_fraction: f32,
    /// Fraction for historical summaries (heavily compressed).
    #[allow(dead_code)]
    pub history_fraction: f32,
    /// Number of most recent tool results considered "recent".
    pub recent_tool_window: usize,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            system_user_fraction: 0.30,
            recent_tool_fraction: 0.40,
            older_tool_fraction: 0.20,
            history_fraction: 0.10,
            recent_tool_window: 6,
        }
    }
}

/// Result of applying the token budget.
#[derive(Debug, Default)]
#[allow(dead_code)]
pub(crate) struct BudgetResult {
    pub recent_tools_trimmed: usize,
    pub older_tools_summarized: usize,
    pub history_compressed: usize,
    pub total_tokens_freed: usize,
}

/// Classify messages into budget categories.
struct MessageClassification {
    system_indices: Vec<usize>,
    user_indices: Vec<usize>,
    recent_tool_indices: Vec<usize>,
    older_tool_indices: Vec<usize>,
    #[allow(dead_code)]
    assistant_indices: Vec<usize>,
}

fn classify_messages(messages: &[ChatMessage], recent_window: usize) -> MessageClassification {
    let mut system_indices = Vec::new();
    let mut user_indices = Vec::new();
    let mut tool_indices = Vec::new();
    let mut assistant_indices = Vec::new();

    for (i, msg) in messages.iter().enumerate() {
        match msg.role {
            Role::System => system_indices.push(i),
            Role::User => user_indices.push(i),
            Role::Tool => tool_indices.push(i),
            Role::Assistant => assistant_indices.push(i),
        }
    }

    let split = tool_indices.len().saturating_sub(recent_window);
    let older_tool_indices = tool_indices[..split].to_vec();
    let recent_tool_indices = tool_indices[split..].to_vec();

    MessageClassification {
        system_indices,
        user_indices,
        recent_tool_indices,
        older_tool_indices,
        assistant_indices,
    }
}

fn estimate_tokens_for_indices(messages: &[ChatMessage], indices: &[usize]) -> usize {
    indices
        .iter()
        .map(|&i| xiaolin_context::estimate_messages_tokens(std::slice::from_ref(&messages[i])))
        .sum()
}

/// Apply the token budget to conversation messages.
///
/// This is a soft enforcement: it compresses older tool results down to
/// summaries when they exceed their allocated budget, but never touches
/// system/user messages or the most recent tool results.
pub(crate) fn apply_token_budget(
    messages: &mut [ChatMessage],
    context_window: u32,
    config: &BudgetConfig,
) -> BudgetResult {
    let total_budget = context_window as usize;
    let classified = classify_messages(messages, config.recent_tool_window);

    let system_user_budget = (total_budget as f32 * config.system_user_fraction) as usize;
    let recent_tool_budget = (total_budget as f32 * config.recent_tool_fraction) as usize;
    let older_tool_budget = (total_budget as f32 * config.older_tool_fraction) as usize;

    let system_user_tokens = estimate_tokens_for_indices(messages, &classified.system_indices)
        + estimate_tokens_for_indices(messages, &classified.user_indices);

    let mut older_tools_summarized = 0;
    let mut recent_tools_trimmed = 0;
    let mut total_tokens_freed = 0;

    // Phase 1: Compress older tool results to fit their budget.
    let older_tool_tokens = estimate_tokens_for_indices(messages, &classified.older_tool_indices);
    if older_tool_tokens > older_tool_budget {
        let overshoot = older_tool_tokens - older_tool_budget;
        let mut freed = 0;

        // Collect assistant messages for reference scoring.
        let assistant_msgs: Vec<&ChatMessage> = messages
            .iter()
            .filter(|m| matches!(m.role, Role::Assistant))
            .collect();
        let total_msgs = messages.len();

        // Sort older tool indices by semantic importance (lowest score = evicted first).
        let mut sorted_older: Vec<(usize, RetentionTier)> = classified
            .older_tool_indices
            .iter()
            .filter_map(|&i| {
                let name = messages[i].name.as_deref()?;
                Some((i, classify_retention_tier(name)))
            })
            .collect();
        sorted_older.sort_by(|(idx_a, _), (idx_b, _)| {
            let score_a = ToolResultImportance::score_tool_result(
                &messages[*idx_a],
                *idx_a,
                total_msgs,
                &assistant_msgs,
            )
            .score;
            let score_b = ToolResultImportance::score_tool_result(
                &messages[*idx_b],
                *idx_b,
                total_msgs,
                &assistant_msgs,
            )
            .score;
            score_a
                .partial_cmp(&score_b)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for (idx, tier) in sorted_older {
            if freed >= overshoot {
                break;
            }

            let msg = &messages[idx];
            let text = match msg.text_content() {
                Some(t) => t,
                None => continue,
            };

            if text.starts_with("[summarized]")
                || text.starts_with(RECALL_HINT_MARKER)
                || text.starts_with("[faded]")
                || text.starts_with("[oneliner]")
                || text.starts_with("[time-compacted]")
                || text == "[Old tool result content cleared]"
            {
                continue;
            }

            let tool_name = messages[idx]
                .name
                .as_deref()
                .unwrap_or("unknown")
                .to_string();
            let before_tokens =
                xiaolin_context::estimate_messages_tokens(std::slice::from_ref(&messages[idx]));

            let replacement = match tier {
                RetentionTier::Ephemeral => {
                    build_cleared_with_recall(&tool_name, tier, &text, None)
                }
                RetentionTier::Summarize => {
                    let summary = summarize_tool_result(&tool_name, &text, 300);
                    format!("[summarized] {summary}")
                }
                RetentionTier::FullRetain => {
                    let summary = summarize_tool_result(&tool_name, &text, 500);
                    format!("[summarized] {summary}")
                }
            };

            messages[idx].content = Some(serde_json::Value::String(replacement));
            let after_tokens =
                xiaolin_context::estimate_messages_tokens(std::slice::from_ref(&messages[idx]));

            let delta = before_tokens.saturating_sub(after_tokens);
            freed += delta;
            total_tokens_freed += delta;
            older_tools_summarized += 1;
        }
    }

    // Phase 2: If system+user messages eat into the recent tool budget, apply
    // light compression to the oldest recent tool results.
    let system_user_overflow = system_user_tokens.saturating_sub(system_user_budget);
    let effective_recent_budget = recent_tool_budget.saturating_sub(system_user_overflow);
    let recent_tool_tokens = estimate_tokens_for_indices(messages, &classified.recent_tool_indices);

    if recent_tool_tokens > effective_recent_budget {
        let overshoot = recent_tool_tokens - effective_recent_budget;
        let mut freed = 0;

        // Compress from the oldest of the "recent" window.
        for &idx in &classified.recent_tool_indices {
            if freed >= overshoot {
                break;
            }

            let msg = &messages[idx];
            let text = match msg.text_content() {
                Some(t) => t,
                None => continue,
            };

            if text.starts_with("[summarized]")
                || text.starts_with(RECALL_HINT_MARKER)
                || text.starts_with("[faded]")
                || text.starts_with("[time-compacted]")
                || text == "[Old tool result content cleared]"
            {
                continue;
            }

            let tool_name = messages[idx]
                .name
                .as_deref()
                .unwrap_or("unknown")
                .to_string();
            let tier = classify_retention_tier(&tool_name);
            let before_tokens =
                xiaolin_context::estimate_messages_tokens(std::slice::from_ref(&messages[idx]));

            // For recent results, only fade — don't fully clear.
            let max_chars = match tier {
                RetentionTier::FullRetain => 600,
                RetentionTier::Summarize => 400,
                RetentionTier::Ephemeral => 150,
            };
            let summary = summarize_tool_result(&tool_name, &text, max_chars);
            let replacement = format!("[summarized] {summary}");

            messages[idx].content = Some(serde_json::Value::String(replacement));
            let after_tokens =
                xiaolin_context::estimate_messages_tokens(std::slice::from_ref(&messages[idx]));

            let delta = before_tokens.saturating_sub(after_tokens);
            freed += delta;
            total_tokens_freed += delta;
            recent_tools_trimmed += 1;
        }
    }

    BudgetResult {
        recent_tools_trimmed,
        older_tools_summarized,
        history_compressed: 0,
        total_tokens_freed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system_msg(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::System,
            content: Some(serde_json::Value::String(text.to_string())),
            ..Default::default()
        }
    }

    fn user_msg(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::User,
            content: Some(serde_json::Value::String(text.to_string())),
            ..Default::default()
        }
    }

    fn tool_msg(name: &str, text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::Tool,
            content: Some(serde_json::Value::String(text.to_string())),
            name: Some(name.to_string()),
            tool_call_id: Some(format!("call_{name}")),
            ..Default::default()
        }
    }

    fn asst_msg(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::Assistant,
            content: Some(serde_json::Value::String(text.to_string())),
            ..Default::default()
        }
    }

    #[test]
    fn budget_config_default_sums_to_one() {
        let config = BudgetConfig::default();
        let total = config.system_user_fraction
            + config.recent_tool_fraction
            + config.older_tool_fraction
            + config.history_fraction;
        assert!((total - 1.0).abs() < 0.001);
    }

    #[test]
    fn classify_messages_splits_correctly() {
        let msgs = vec![
            system_msg("sys"),
            user_msg("hello"),
            tool_msg("read_file", "content 1"),
            tool_msg("read_file", "content 2"),
            tool_msg("grep", "matches"),
            tool_msg("list_dir", "files"),
            asst_msg("reply"),
        ];
        let result = classify_messages(&msgs, 2);
        assert_eq!(result.system_indices.len(), 1);
        assert_eq!(result.user_indices.len(), 1);
        assert_eq!(result.older_tool_indices.len(), 2);
        assert_eq!(result.recent_tool_indices.len(), 2);
        assert_eq!(result.assistant_indices.len(), 1);
    }

    #[test]
    fn budget_no_compression_when_under_limit() {
        let mut msgs = vec![
            system_msg("system prompt"),
            user_msg("question"),
            tool_msg("read_file", "short content"),
            asst_msg("answer"),
        ];
        let result = apply_token_budget(&mut msgs, 100_000, &BudgetConfig::default());
        assert_eq!(result.older_tools_summarized, 0);
        assert_eq!(result.recent_tools_trimmed, 0);
    }

    #[test]
    fn budget_compresses_older_tools_when_over_limit() {
        let big_content = "x".repeat(5000);
        let mut msgs = vec![
            system_msg("sys"),
            user_msg("q"),
            // These will be "older" (beyond recent_window of 2)
            tool_msg("read_file", &big_content),
            tool_msg("grep", &big_content),
            tool_msg("list_dir", &big_content),
            // These are "recent" (last 2)
            tool_msg("read_file", "recent1"),
            tool_msg("read_file", "recent2"),
            asst_msg("a"),
        ];

        let config = BudgetConfig {
            older_tool_fraction: 0.01, // Very tight budget for older tools
            recent_tool_window: 2,
            ..Default::default()
        };

        let result = apply_token_budget(&mut msgs, 1000, &config);
        assert!(
            result.older_tools_summarized > 0,
            "should summarize older tools"
        );
        assert!(result.total_tokens_freed > 0, "should free tokens");
    }

    #[test]
    fn ephemeral_tools_compressed_first() {
        let big = "x".repeat(3000);
        let mut msgs = vec![
            system_msg("sys"),
            user_msg("q"),
            tool_msg("list_dir", &big),  // Ephemeral
            tool_msg("read_file", &big), // FullRetain
            tool_msg("grep", &big),      // Summarize
            tool_msg("read_file", "recent"),
            asst_msg("a"),
        ];

        let config = BudgetConfig {
            older_tool_fraction: 0.01,
            recent_tool_window: 1,
            ..Default::default()
        };

        apply_token_budget(&mut msgs, 1000, &config);

        // Ephemeral (list_dir) should be fully cleared first
        let list_dir_text = msgs[2].text_content().unwrap();
        assert!(
            list_dir_text.starts_with(RECALL_HINT_MARKER),
            "list_dir should be recall-cleared, got: {list_dir_text}"
        );
    }

    #[test]
    fn retention_tier_ordering() {
        assert!(RetentionTier::Ephemeral < RetentionTier::Summarize);
        assert!(RetentionTier::Summarize < RetentionTier::FullRetain);
    }
}
