use std::sync::Arc;

use fastclaw_core::types::{ChatMessage, Role};

use crate::llm::{CompletionParams, LlmProvider};

/// Default fraction of context window at which LLM compression triggers.
/// Used as fallback when dynamic threshold computation is disabled.
pub const COMPRESSION_THRESHOLD: f32 = 0.50;

/// Compute a dynamic compression threshold based on the actual token distribution
/// in the current context. Adapts to workload characteristics:
/// - Large system prompts (>30% of window): lower threshold to 0.40
/// - Tool-result-heavy contexts (>50% of non-system): compress tools first (0.55)
/// - Active multi-step task: raise threshold to 0.60 to delay disruptive compression
pub fn compute_compression_threshold(
    system_tokens: usize,
    tool_tokens: usize,
    conversation_tokens: usize,
    context_window: u32,
    has_active_task: bool,
) -> f32 {
    let window = context_window as f32;
    if window < 1.0 {
        return COMPRESSION_THRESHOLD;
    }

    let system_ratio = system_tokens as f32 / window;
    let non_system_total = (tool_tokens + conversation_tokens) as f32;
    let tool_dominance = if non_system_total > 0.0 {
        tool_tokens as f32 / non_system_total
    } else {
        0.0
    };

    let mut threshold = COMPRESSION_THRESHOLD;

    if system_ratio > 0.30 {
        threshold = (threshold - 0.10).max(0.35);
    }

    if tool_dominance > 0.50 {
        threshold += 0.05;
    }

    if has_active_task {
        threshold = (threshold + 0.10).min(0.70);
    }

    threshold.clamp(0.35, 0.70)
}

/// Fraction of recent history to preserve (the rest gets compressed).
const PRESERVE_FRACTION: f32 = 0.30;

/// Minimum fraction of history that must be compressible to justify an LLM call.
const MIN_COMPRESSIBLE_FRACTION: f32 = 0.05;

const COMPRESSION_SYSTEM_PROMPT: &str = r#"You are a conversation summarizer. Your task is to create a detailed summary of the conversation so far. This summary will replace the conversation history as context for the assistant, so it is CRUCIAL that you include ALL information needed for the assistant to continue its work.

You MUST include:
1. Primary Request and Intent: What is the user fundamentally trying to accomplish?
2. Key Technical Concepts: Important technical details, patterns, APIs, data structures, algorithms, or design decisions discussed.
3. Files and Code Sections:
   - List ALL files created, modified, or referenced (with their paths)
   - For each file, note its importance and any changes made
   - Include relevant code snippets (function signatures, struct definitions, key logic). Do NOT omit code — the assistant may need exact signatures and line references.
4. Errors and fixes: Document any errors encountered and how they were resolved.
5. Problem Solving: Summarize the problem-solving process — what approaches were tried, what worked, what didn't, and why.
6. All user messages: List every distinct user message/instruction (including follow-up requests, corrections, and clarifications). Do NOT skip any.
7. Pending Tasks: List all incomplete tasks, TODOs, or follow-up items.
8. Current Work: Describe what the assistant was doing immediately before this summary — include the exact state of any in-progress operation.
9. Optional Next Step: If the conversation implies a clear next action, state it with a relevant quote from the original conversation to prevent drift.

### Transcript location:
  This is the full JSONL transcript of your past conversation with the user (pre- and post-summary): {{HISTORY_FILE_PATH}}

  If anything about the task or current state is unclear (missing context, ambiguous requirements, uncertain decisions, exact wording, IDs/paths, errors/logs), you should consult this transcript.

  How to use it:
  - Search first for relevant keywords (task name, filenames, IDs, errors, tool names).
  - Then read a small window around the matching lines to reconstruct intent and state.
  - Avoid reading linearly end-to-end; the file can be very large and some single lines can be huge.
  - Files contain one structured json event per line including user/assistant messages. Currently tool calls and results are excluded.

Format your response as follows:
1. First, write your analysis inside <analysis> tags. Think about what information is critical to preserve, which code snippets are essential, and what the assistant needs to continue seamlessly.
2. Then write the summary in plain text (NOT inside tags). The summary will be used directly as context.

IMPORTANT:
- Be thorough: the assistant will have NO access to the original conversation after compression.
- Include code snippets and file paths verbatim — do not paraphrase them.
- Quote user instructions exactly when they contain specific requirements.
- Preserve all numerical values, version numbers, configuration values, and IDs.
- If a TODO list or task queue was being maintained, reproduce it in full."#;

#[allow(dead_code)]
pub struct CompressionResult {
    pub compressed: bool,
    pub original_tokens: usize,
    pub new_tokens: usize,
    pub messages: Vec<ChatMessage>,
    pub history_file: Option<String>,
}

/// Save the pre-compression chat history to a file so the agent can search it
/// after compression to recover details lost in summarization.
/// Returns the file path on success.
fn save_history_file(messages: &[ChatMessage]) -> Option<String> {
    let dir = std::env::temp_dir().join("fastclaw_history");
    if std::fs::create_dir_all(&dir).is_err() {
        return None;
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("chat_history_{ts}.md"));

    let mut content = String::new();
    content.push_str("# Chat History (pre-compression snapshot)\n\n");
    for msg in messages {
        let role = match msg.role {
            Role::System => "SYSTEM",
            Role::User => "USER",
            Role::Assistant => "ASSISTANT",
            Role::Tool => "TOOL",
        };
        let name_suffix = msg.name.as_deref().map(|n| format!(" ({n})")).unwrap_or_default();
        content.push_str(&format!("## {role}{name_suffix}\n\n"));

        if let Some(ref c) = msg.content {
            match c {
                serde_json::Value::String(s) => {
                    content.push_str(s);
                }
                other => {
                    content.push_str(&serde_json::to_string_pretty(other).unwrap_or_default());
                }
            }
            content.push_str("\n\n");
        }

        if let Some(ref tcs) = msg.tool_calls {
            for tc in tcs {
                content.push_str(&format!(
                    "**Tool Call**: {} ({})\nArgs: {}\n\n",
                    tc.function.name, tc.id, tc.function.arguments
                ));
            }
        }
    }

    match std::fs::write(&path, &content) {
        Ok(()) => Some(path.to_string_lossy().to_string()),
        Err(_) => None,
    }
}

/// Find the split point: preserve the last `preserve_fraction` of non-system messages.
/// Split must land on a user message boundary.
fn find_split_point(non_system: &[&ChatMessage], preserve_fraction: f32) -> usize {
    if non_system.is_empty() {
        return 0;
    }

    let char_counts: Vec<usize> = non_system.iter().map(|m| {
        m.content.as_ref().map_or(0, |c| {
            serde_json::to_string(c).map(|s| s.len()).unwrap_or(0)
        }) + m.tool_calls.as_ref().map_or(0, |tc| {
            tc.iter().map(|t| t.function.name.len() + t.function.arguments.len()).sum()
        })
    }).collect();

    let total_chars: usize = char_counts.iter().sum();
    let target_chars = (total_chars as f32 * (1.0 - preserve_fraction)) as usize;

    let mut cumulative = 0usize;
    let mut last_user_boundary = 0usize;

    for (i, msg) in non_system.iter().enumerate() {
        if matches!(msg.role, Role::User) && !has_tool_response(msg) {
            if cumulative >= target_chars {
                return i;
            }
            last_user_boundary = i;
        }
        cumulative += char_counts[i];
    }

    last_user_boundary
}

fn has_tool_response(msg: &ChatMessage) -> bool {
    msg.tool_call_id.is_some()
}

/// Strip `<analysis>...</analysis>` blocks from LLM output (used for
/// chain-of-thought that improves summary quality but shouldn't be included
/// in the final compressed context).
fn strip_analysis_block(text: &str) -> String {
    let mut result = text.to_string();
    while let Some(start) = result.find("<analysis>") {
        if let Some(end) = result.find("</analysis>") {
            let block_end = end + "</analysis>".len();
            result = format!(
                "{}{}",
                &result[..start],
                result[block_end..].trim_start()
            );
        } else {
            result = result[..start].to_string();
            break;
        }
    }
    result.trim().to_string()
}

/// Attempt LLM-based compression of conversation history.
///
/// Triggers when estimated tokens exceed `threshold_fraction * context_window`.
/// Calls the LLM with a compression prompt to generate a state snapshot,
/// then replaces the compressed portion with the snapshot.
/// `api_prompt_tokens`: if >0, use the API-reported prompt token count
/// (from the previous LLM call's `usage.prompt_tokens`) as the authoritative
/// context size. Falls back to `estimate_messages_tokens` when 0.
pub async fn try_compress_chat(
    messages: &mut Vec<ChatMessage>,
    context_window: u32,
    provider: &Arc<dyn LlmProvider>,
    model: &str,
    api_prompt_tokens: usize,
    todo_store: Option<&crate::builtin_tools::TodoStore>,
) -> CompressionResult {
    try_compress_chat_with_threshold(
        messages,
        context_window,
        provider,
        model,
        api_prompt_tokens,
        todo_store,
        COMPRESSION_THRESHOLD,
    )
    .await
}

/// Like [`try_compress_chat`] but accepts a custom threshold fraction,
/// enabling dynamic threshold computation by the caller.
pub async fn try_compress_chat_with_threshold(
    messages: &mut Vec<ChatMessage>,
    context_window: u32,
    provider: &Arc<dyn LlmProvider>,
    model: &str,
    api_prompt_tokens: usize,
    todo_store: Option<&crate::builtin_tools::TodoStore>,
    threshold_fraction: f32,
) -> CompressionResult {
    let local_estimate = fastclaw_context::estimate_messages_tokens(messages);
    let estimated = if api_prompt_tokens > 0 { api_prompt_tokens } else { local_estimate };
    let threshold = (context_window as f32 * threshold_fraction) as usize;

    if estimated <= threshold {
        return CompressionResult {
            compressed: false,
            original_tokens: estimated,
            new_tokens: estimated,
            messages: messages.clone(),
            history_file: None,
        };
    }

    tracing::info!(
        estimated,
        local_estimate,
        api_prompt_tokens,
        threshold,
        context_window,
        "context compression triggered"
    );

    let mut system_indices: Vec<usize> = Vec::new();
    let mut non_system_indices: Vec<usize> = Vec::new();
    for (i, m) in messages.iter().enumerate() {
        if matches!(m.role, Role::System) {
            system_indices.push(i);
        } else {
            non_system_indices.push(i);
        }
    }

    let non_system_msgs: Vec<&ChatMessage> = non_system_indices.iter().map(|&i| &messages[i]).collect();

    if non_system_msgs.is_empty() {
        return CompressionResult {
            compressed: false,
            original_tokens: estimated,
            new_tokens: estimated,
            messages: messages.clone(),
            history_file: None,
        };
    }

    let split = find_split_point(&non_system_msgs, PRESERVE_FRACTION);
    if split == 0 {
        return CompressionResult {
            compressed: false,
            original_tokens: estimated,
            new_tokens: estimated,
            messages: messages.clone(),
            history_file: None,
        };
    }

    let to_compress = &non_system_msgs[..split];
    let to_keep = &non_system_msgs[split..];

    let compress_chars: usize = to_compress.iter().map(|m| {
        m.content.as_ref().map_or(0, |c| serde_json::to_string(c).map(|s| s.len()).unwrap_or(0))
    }).sum();
    let total_chars: usize = non_system_msgs.iter().map(|m| {
        m.content.as_ref().map_or(0, |c| serde_json::to_string(c).map(|s| s.len()).unwrap_or(0))
    }).sum();

    if total_chars > 0 && (compress_chars as f32 / total_chars as f32) < MIN_COMPRESSIBLE_FRACTION {
        tracing::info!("compressible fraction too small, skipping LLM compression");
        return CompressionResult {
            compressed: false,
            original_tokens: estimated,
            new_tokens: estimated,
            messages: messages.clone(),
            history_file: None,
        };
    }

    // Save history file before LLM call so we can reference it in the prompt.
    let history_file = save_history_file(messages);
    if let Some(ref path) = history_file {
        tracing::info!(path, "saved pre-compression chat history to file");
    }

    let mut system_prompt = match &history_file {
        Some(path) => COMPRESSION_SYSTEM_PROMPT.replace("{{HISTORY_FILE_PATH}}", path),
        None => COMPRESSION_SYSTEM_PROMPT.replace(
            "{{HISTORY_FILE_PATH}}",
            "(history file not available)",
        ),
    };

    if let Some(store) = todo_store {
        let items = store.snapshot().await;
        if !items.is_empty() {
            let todo_json = serde_json::to_string_pretty(&items).unwrap_or_default();
            system_prompt.push_str(&format!(
                "\n\n## MUST PRESERVE: Current Todo List\n\
                 The following todo list is actively being tracked. \
                 You MUST include it verbatim in your summary:\n\n```json\n{todo_json}\n```"
            ));
        }
    }

    // Build the LLM compression request
    let mut compress_messages: Vec<ChatMessage> = Vec::new();
    compress_messages.push(ChatMessage {
        role: Role::System,
        content: Some(serde_json::Value::String(system_prompt)),
        reasoning_content: None,
        name: None,
        tool_calls: None,
        tool_call_id: None,
    });
    for msg in to_compress {
        compress_messages.push((*msg).clone());
    }
    compress_messages.push(ChatMessage {
        role: Role::User,
        content: Some(serde_json::Value::String(
            "Summarize the conversation above. First, write your reasoning inside <analysis> tags, \
             then write the final summary in plain text."
                .to_string(),
        )),
        reasoning_content: None,
        name: None,
        tool_calls: None,
        tool_call_id: None,
    });

    let params = CompletionParams {
        model,
        messages: &compress_messages,
        temperature: 0.0,
        max_tokens: Some(4096),
        tools: None,
    };

    let raw_summary = match provider.chat_completion(&params).await {
        Ok(resp) => {
            resp.choices.first().and_then(|c| c.message.text_content()).unwrap_or_default()
        }
        Err(e) => {
            tracing::warn!(error = %e, "LLM compression failed, falling back to rule-based");
            return CompressionResult {
                compressed: false,
                original_tokens: estimated,
                new_tokens: estimated,
                messages: messages.clone(),
                history_file: None,
            };
        }
    };

    let summary = strip_analysis_block(&raw_summary);

    if summary.trim().is_empty() {
        tracing::warn!("LLM compression returned empty summary");
        return CompressionResult {
            compressed: false,
            original_tokens: estimated,
            new_tokens: estimated,
            messages: messages.clone(),
            history_file: None,
        };
    }

    // Rebuild messages: system msgs + summary as user/assistant pair + kept history
    let mut new_messages: Vec<ChatMessage> = Vec::new();

    for &idx in &system_indices {
        let msg: ChatMessage = messages[idx].clone();
        new_messages.push(msg);
    }

    new_messages.push(ChatMessage {
        role: Role::User,
        content: Some(serde_json::Value::String(summary)),
        reasoning_content: None,
        name: None,
        tool_calls: None,
        tool_call_id: None,
    });
    new_messages.push(ChatMessage {
        role: Role::Assistant,
        content: Some(serde_json::Value::String(
            "Got it. I have the full context from the previous conversation. Let me continue.".to_string(),
        )),
        reasoning_content: None,
        name: None,
        tool_calls: None,
        tool_call_id: None,
    });

    for msg in to_keep {
        new_messages.push((*msg).clone());
    }

    let new_estimated = fastclaw_context::estimate_messages_tokens(&new_messages);

    if new_estimated >= estimated {
        tracing::warn!(
            new_estimated,
            original = estimated,
            "compression inflated tokens, discarding"
        );
        return CompressionResult {
            compressed: false,
            original_tokens: estimated,
            new_tokens: estimated,
            messages: messages.clone(),
            history_file: None,
        };
    }

    // Add history file reference to assistant message so agent can recover details.
    if let Some(ref path) = history_file {
        if let Some(last_asst) = new_messages.iter_mut().rev().find(|m| matches!(m.role, Role::Assistant)) {
            if let Some(serde_json::Value::String(ref mut text)) = last_asst.content {
                text.push_str(&format!(
                    " Full conversation history saved to: {path} — use read_file or grep to recover any details."
                ));
            }
        }
    }

    tracing::info!(
        original_tokens = estimated,
        new_tokens = new_estimated,
        evicted_messages = to_compress.len(),
        kept_messages = to_keep.len(),
        history_file = ?history_file,
        "LLM compression successful"
    );

    *messages = new_messages.clone();

    CompressionResult {
        compressed: true,
        original_tokens: estimated,
        new_tokens: new_estimated,
        messages: new_messages,
        history_file,
    }
}

/// Maximum consecutive compression failures before the circuit breaker trips.
#[allow(dead_code)]
const MAX_CONSECUTIVE_AUTOCOMPACT_FAILURES: u32 = 3;

/// Outcome of an [`AutoCompactor::compact_if_needed`] call.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoCompactOutcome {
    /// Compression was not needed (below threshold).
    NotNeeded,
    /// Compression succeeded.
    Compressed { original_tokens: usize, new_tokens: usize },
    /// Compression failed (LLM error, inflated result, etc.).
    Failed,
    /// Skipped because the circuit breaker has tripped after too many failures.
    CircuitBreakerOpen,
}

/// Wraps [`try_compress_chat`] with a circuit breaker that stops retrying
/// after [`MAX_CONSECUTIVE_AUTOCOMPACT_FAILURES`] consecutive failures.
#[allow(dead_code)]
pub struct AutoCompactor {
    consecutive_failures: u32,
}

#[allow(dead_code)]
impl AutoCompactor {
    pub fn new() -> Self {
        Self {
            consecutive_failures: 0,
        }
    }

    /// Whether the circuit breaker is currently open (tripped).
    pub fn is_circuit_open(&self) -> bool {
        self.consecutive_failures >= MAX_CONSECUTIVE_AUTOCOMPACT_FAILURES
    }

    /// Current consecutive failure count.
    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    /// Check whether auto-compaction should trigger, accounting for tokens
    /// already freed by a prior snip pass.
    ///
    /// The effective token count is `estimated_tokens - snip_tokens_freed`.
    /// If this is below the threshold, compaction is skipped.
    pub fn should_compact(
        &self,
        estimated_tokens: usize,
        context_window: u32,
        snip_tokens_freed: usize,
    ) -> bool {
        let effective = estimated_tokens.saturating_sub(snip_tokens_freed);
        let threshold = (context_window as f32 * COMPRESSION_THRESHOLD) as usize;
        effective > threshold
    }

    /// Attempt compression if needed. Returns immediately with
    /// [`AutoCompactOutcome::CircuitBreakerOpen`] if too many consecutive
    /// failures have occurred.
    ///
    /// `snip_tokens_freed`: tokens already freed by a prior snip pass. The
    /// effective token count is reduced by this amount before checking the
    /// compression threshold.
    pub async fn compact_if_needed(
        &mut self,
        messages: &mut Vec<ChatMessage>,
        context_window: u32,
        provider: &Arc<dyn LlmProvider>,
        model: &str,
        api_prompt_tokens: usize,
        snip_tokens_freed: usize,
    ) -> AutoCompactOutcome {
        if self.is_circuit_open() {
            tracing::warn!(
                consecutive_failures = self.consecutive_failures,
                "auto-compact circuit breaker open, skipping"
            );
            return AutoCompactOutcome::CircuitBreakerOpen;
        }

        // Check threshold with snip awareness.
        let estimated = if api_prompt_tokens > 0 {
            api_prompt_tokens
        } else {
            fastclaw_context::estimate_messages_tokens(messages)
        };

        if !self.should_compact(estimated, context_window, snip_tokens_freed) {
            return AutoCompactOutcome::NotNeeded;
        }

        let result = try_compress_chat(messages, context_window, provider, model, api_prompt_tokens, None).await;

        if result.compressed {
            self.consecutive_failures = 0;
            AutoCompactOutcome::Compressed {
                original_tokens: result.original_tokens,
                new_tokens: result.new_tokens,
            }
        } else if result.original_tokens <= (context_window as f32 * COMPRESSION_THRESHOLD) as usize {
            AutoCompactOutcome::NotNeeded
        } else {
            self.consecutive_failures += 1;
            tracing::warn!(
                consecutive_failures = self.consecutive_failures,
                max = MAX_CONSECUTIVE_AUTOCOMPACT_FAILURES,
                "auto-compact failed"
            );
            AutoCompactOutcome::Failed
        }
    }

    /// Manually reset the circuit breaker (e.g. after a model change).
    pub fn reset(&mut self) {
        self.consecutive_failures = 0;
    }
}

impl Default for AutoCompactor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn asst(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::Assistant,
            content: Some(text.to_string().into()),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    #[test]
    fn split_point_preserves_recent_fraction() {
        let msgs = vec![
            user("old question 1"),
            asst("old answer 1"),
            user("old question 2"),
            asst("old answer 2"),
            user("recent question"),
            asst("recent answer"),
        ];
        let refs: Vec<&ChatMessage> = msgs.iter().collect();
        let split = find_split_point(&refs, 0.3);
        assert!(split > 0, "should split somewhere");
        assert!(split < msgs.len(), "should keep some messages");
    }

    #[test]
    fn split_point_empty_returns_zero() {
        let msgs: Vec<&ChatMessage> = vec![];
        assert_eq!(find_split_point(&msgs, 0.3), 0);
    }

    #[test]
    fn auto_compactor_starts_with_closed_breaker() {
        let ac = AutoCompactor::new();
        assert!(!ac.is_circuit_open());
        assert_eq!(ac.consecutive_failures(), 0);
    }

    #[test]
    fn circuit_breaker_trips_after_max_failures() {
        let mut ac = AutoCompactor::new();
        for _ in 0..MAX_CONSECUTIVE_AUTOCOMPACT_FAILURES {
            ac.consecutive_failures += 1;
        }
        assert!(ac.is_circuit_open());
        assert_eq!(ac.consecutive_failures(), MAX_CONSECUTIVE_AUTOCOMPACT_FAILURES);
    }

    #[test]
    fn circuit_breaker_does_not_trip_below_max() {
        let mut ac = AutoCompactor::new();
        ac.consecutive_failures = MAX_CONSECUTIVE_AUTOCOMPACT_FAILURES - 1;
        assert!(!ac.is_circuit_open());
    }

    #[test]
    fn reset_clears_circuit_breaker() {
        let mut ac = AutoCompactor::new();
        ac.consecutive_failures = MAX_CONSECUTIVE_AUTOCOMPACT_FAILURES;
        assert!(ac.is_circuit_open());

        ac.reset();
        assert!(!ac.is_circuit_open());
        assert_eq!(ac.consecutive_failures(), 0);
    }

    #[test]
    fn default_impl_matches_new() {
        let ac = AutoCompactor::default();
        assert!(!ac.is_circuit_open());
        assert_eq!(ac.consecutive_failures(), 0);
    }

    #[test]
    fn should_compact_without_snip() {
        let ac = AutoCompactor::new();
        // 60k tokens, 100k context window, threshold = 50% = 50k.
        // 60k > 50k, so should compact.
        assert!(ac.should_compact(60_000, 100_000, 0));
    }

    #[test]
    fn should_compact_under_threshold() {
        let ac = AutoCompactor::new();
        // 40k tokens, 100k context window, threshold = 50k.
        // 40k < 50k, should not compact.
        assert!(!ac.should_compact(40_000, 100_000, 0));
    }

    #[test]
    fn snip_freed_avoids_compact() {
        let ac = AutoCompactor::new();
        // 60k tokens, 100k context window, threshold = 50k.
        // snip freed 15k, so effective = 60k - 15k = 45k < 50k.
        assert!(!ac.should_compact(60_000, 100_000, 15_000));
    }

    #[test]
    fn snip_freed_partial_still_compacts() {
        let ac = AutoCompactor::new();
        // 60k tokens, 100k context window, threshold = 50k.
        // snip freed 5k, effective = 55k > 50k, still needs compact.
        assert!(ac.should_compact(60_000, 100_000, 5_000));
    }

    #[test]
    fn snip_freed_zero_same_as_no_snip() {
        let ac = AutoCompactor::new();
        assert_eq!(
            ac.should_compact(60_000, 100_000, 0),
            ac.should_compact(60_000, 100_000, 0)
        );
    }

    #[test]
    fn snip_freed_exceeds_estimated_saturates_to_zero() {
        let ac = AutoCompactor::new();
        // snip freed more than estimated — effective = 0, which is under threshold.
        assert!(!ac.should_compact(30_000, 100_000, 50_000));
    }

    #[test]
    fn strip_analysis_removes_block() {
        let input = "<analysis>This is reasoning that should be removed.</analysis>\n\nSummary:\n1. The user wants X.\n2. Key facts.";
        let result = super::strip_analysis_block(input);
        assert!(!result.contains("<analysis>"), "analysis tags should be removed");
        assert!(!result.contains("reasoning that should"), "analysis content should be removed");
        assert!(result.contains("Summary:"), "actual summary preserved");
        assert!(result.contains("Key facts"), "content after analysis preserved");
    }

    #[test]
    fn strip_analysis_handles_no_block() {
        let input = "Just a plain summary without analysis tags.";
        let result = super::strip_analysis_block(input);
        assert_eq!(result, input);
    }

    #[test]
    fn strip_analysis_handles_unclosed_tag() {
        let input = "<analysis>Unclosed reasoning block\nMore text here";
        let result = super::strip_analysis_block(input);
        assert!(!result.contains("<analysis>"));
        assert!(result.is_empty() || !result.contains("Unclosed"));
    }

    #[test]
    fn compression_prompt_has_9_sections() {
        let prompt = super::COMPRESSION_SYSTEM_PROMPT;
        assert!(prompt.contains("Primary Request"), "should include section 1");
        assert!(prompt.contains("Key Technical Concepts"), "should include section 2");
        assert!(prompt.contains("Files and Code Sections"), "should include section 3");
        assert!(prompt.contains("Errors and fixes"), "should include section 4");
        assert!(prompt.contains("Problem Solving"), "should include section 5");
        assert!(prompt.contains("All user messages"), "should include section 6");
        assert!(prompt.contains("Pending Tasks"), "should include section 7");
        assert!(prompt.contains("Current Work"), "should include section 8");
        assert!(prompt.contains("Next Step"), "should include section 9");
    }

    #[test]
    fn compression_prompt_allows_code() {
        let prompt = super::COMPRESSION_SYSTEM_PROMPT;
        assert!(!prompt.contains("no code blocks"), "should NOT prohibit code blocks");
        assert!(prompt.contains("code snippets"), "should encourage code preservation");
    }

    #[test]
    fn compression_prompt_requires_analysis_then_strip() {
        let prompt = super::COMPRESSION_SYSTEM_PROMPT;
        assert!(prompt.contains("<analysis>"), "should instruct model to use analysis tags");
    }

    #[test]
    fn dynamic_threshold_defaults_to_static_on_balanced_load() {
        let threshold = super::compute_compression_threshold(1000, 1000, 1000, 10_000, false);
        assert!((threshold - 0.50).abs() < 0.01, "balanced load should stay near default");
    }

    #[test]
    fn dynamic_threshold_lowers_with_large_system_prompt() {
        let threshold = super::compute_compression_threshold(4000, 500, 500, 10_000, false);
        assert!(threshold < 0.50, "large system prompt should lower threshold, got {threshold}");
    }

    #[test]
    fn dynamic_threshold_raises_with_active_task() {
        let threshold = super::compute_compression_threshold(1000, 1000, 1000, 10_000, true);
        assert!(threshold > 0.55, "active task should raise threshold, got {threshold}");
    }

    #[test]
    fn dynamic_threshold_clamped_to_valid_range() {
        let low = super::compute_compression_threshold(9000, 0, 0, 10_000, false);
        assert!(low >= 0.35, "threshold should not go below 0.35, got {low}");
        let high = super::compute_compression_threshold(100, 100, 100, 10_000, true);
        assert!(high <= 0.70, "threshold should not exceed 0.70, got {high}");
    }
}
