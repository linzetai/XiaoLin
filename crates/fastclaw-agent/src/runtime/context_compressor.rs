use std::sync::Arc;

use fastclaw_core::types::{ChatMessage, Role};

use crate::llm::{CompletionParams, LlmProvider};

/// Fraction of context window at which LLM compression triggers.
/// Set to 0.50 — compressing at half-full gives the agent substantial
/// headroom and avoids the hard-truncation cliff that causes 100% stuck states.
pub const COMPRESSION_THRESHOLD: f32 = 0.50;

/// Fraction of recent history to preserve (the rest gets compressed).
const PRESERVE_FRACTION: f32 = 0.30;

/// Minimum fraction of history that must be compressible to justify an LLM call.
const MIN_COMPRESSIBLE_FRACTION: f32 = 0.05;

const COMPRESSION_SYSTEM_PROMPT: &str = r#"You are a conversation compression engine. Produce a CONCISE state snapshot (target: ≤800 tokens). This snapshot will be the agent's ONLY memory of the compressed portion. Preserve critical facts; omit verbose tool outputs, code listings, and conversational filler.

<state_snapshot>
<goal>One sentence: the user's objective.</goal>
<facts>Key constraints, decisions, tech stack. Bullet points, max 10.</facts>
<files>Files touched: path → status (created/modified/read). Only list files still relevant.</files>
<progress>Completed steps (one-liner each). Current step. Remaining TODO items.</progress>
<errors>Unresolved issues or blockers, if any.</errors>
</state_snapshot>

Rules: no code blocks, no raw tool output, no filler. Pure information density."#;

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

/// Attempt LLM-based compression of conversation history.
///
/// Triggers when estimated tokens exceed `COMPRESSION_THRESHOLD * context_window`.
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
) -> CompressionResult {
    let local_estimate = fastclaw_context::estimate_messages_tokens(messages);
    // Prefer API-reported tokens; fall back to local estimate.
    let estimated = if api_prompt_tokens > 0 { api_prompt_tokens } else { local_estimate };
    let threshold = (context_window as f32 * COMPRESSION_THRESHOLD) as usize;

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

    // Build the LLM compression request
    let mut compress_messages: Vec<ChatMessage> = Vec::new();
    compress_messages.push(ChatMessage {
        role: Role::System,
        content: Some(serde_json::Value::String(COMPRESSION_SYSTEM_PROMPT.to_string())),
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
            "First, reason in your scratchpad. Then, generate the <state_snapshot>.".to_string(),
        )),
        name: None,
        tool_calls: None,
        tool_call_id: None,
    });

    let params = CompletionParams {
        model,
        messages: &compress_messages,
        temperature: 0.0,
        max_tokens: Some(2048),
        tools: None,
    };

    let summary = match provider.chat_completion(&params).await {
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
        name: None,
        tool_calls: None,
        tool_call_id: None,
    });
    new_messages.push(ChatMessage {
        role: Role::Assistant,
        content: Some(serde_json::Value::String(
            "Got it. I have the full context from the previous conversation. Let me continue.".to_string(),
        )),
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

    // Save full pre-compression history to a file so the agent can search it later.
    let history_file = save_history_file(messages);
    if let Some(ref path) = history_file {
        tracing::info!(path, "saved pre-compression chat history to file");
    }

    // If we saved a history file, add a reference in the assistant message
    // so the agent knows it can search for details.
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

    /// Attempt compression if needed. Returns immediately with
    /// [`AutoCompactOutcome::CircuitBreakerOpen`] if too many consecutive
    /// failures have occurred.
    pub async fn compact_if_needed(
        &mut self,
        messages: &mut Vec<ChatMessage>,
        context_window: u32,
        provider: &Arc<dyn LlmProvider>,
        model: &str,
        api_prompt_tokens: usize,
    ) -> AutoCompactOutcome {
        if self.is_circuit_open() {
            tracing::warn!(
                consecutive_failures = self.consecutive_failures,
                "auto-compact circuit breaker open, skipping"
            );
            return AutoCompactOutcome::CircuitBreakerOpen;
        }

        let result = try_compress_chat(messages, context_window, provider, model, api_prompt_tokens).await;

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
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn asst(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::Assistant,
            content: Some(text.to_string().into()),
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
}
