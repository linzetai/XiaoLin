use std::sync::Arc;

use fastclaw_core::types::{ChatMessage, Role};
use serde::{Deserialize, Serialize};

use crate::llm::{CompletionParams, LlmProvider};

/// Extracted session memory — the essential state of a conversation.
///
/// When the context window is nearing capacity, this snapshot preserves
/// critical information so that more aggressive compression can safely
/// discard the raw history.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct SessionMemory {
    pub key_facts: Vec<String>,
    pub decisions_made: Vec<String>,
    pub current_task_state: String,
    pub files_modified: Vec<String>,
}

/// Outcome of a session memory extraction attempt.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct ExtractionResult {
    pub memory: Option<SessionMemory>,
    pub token_estimate: usize,
}

const EXTRACTION_SYSTEM_PROMPT: &str = r#"You are a session memory extractor. Given a conversation between a user and an AI assistant, extract the essential state into a structured JSON object.

Output ONLY a valid JSON object with these fields:
{
  "key_facts": ["fact1", "fact2", ...],
  "decisions_made": ["decision1", "decision2", ...],
  "current_task_state": "one sentence describing what is being worked on right now",
  "files_modified": ["path/to/file1", "path/to/file2", ...]
}

Rules:
- key_facts: Important constraints, preferences, technical details. Max 15 items.
- decisions_made: Architectural or implementation choices. Max 10 items.
- current_task_state: What the user is working on RIGHT NOW. One sentence.
- files_modified: Only files that were created, edited, or are actively being discussed.
- Be extremely concise. Each item should be one short sentence.
- Output ONLY the JSON object, no markdown fences, no explanation."#;

/// Minimum number of non-system messages required before extraction is attempted.
const MIN_MESSAGES_FOR_EXTRACTION: usize = 10;

/// Fraction of context window at which session memory extraction triggers.
/// Set below the LLM compression threshold (0.50) so that session memory
/// is extracted first, enabling more aggressive compression afterward.
/// Lowered from 0.40 to 0.30 to ensure extraction happens earlier for
/// large context windows (e.g., 1M tokens for qwen3.5-plus).
pub(crate) const SESSION_MEMORY_THRESHOLD: f32 = 0.30;

/// Try to extract session memory from the conversation.
///
/// Returns `Some(SessionMemory)` on success, `None` if extraction is
/// skipped (too few messages, below threshold) or fails (LLM error,
/// parse error). Failures are logged but never propagate — the caller
/// falls back to normal compression.
pub(crate) async fn extract_session_memory(
    messages: &[ChatMessage],
    provider: &Arc<dyn LlmProvider>,
    model: &str,
    context_window: u32,
    estimated_tokens: usize,
) -> ExtractionResult {
    let non_system_count = messages
        .iter()
        .filter(|m| !matches!(m.role, Role::System))
        .count();

    if non_system_count < MIN_MESSAGES_FOR_EXTRACTION {
        return ExtractionResult {
            memory: None,
            token_estimate: estimated_tokens,
        };
    }

    let threshold = (context_window as f32 * SESSION_MEMORY_THRESHOLD) as usize;
    if estimated_tokens < threshold {
        return ExtractionResult {
            memory: None,
            token_estimate: estimated_tokens,
        };
    }

    let conversation_text = build_conversation_summary(messages);

    let extraction_messages = vec![
        ChatMessage {
            role: Role::System,
            content: Some(serde_json::Value::String(
                EXTRACTION_SYSTEM_PROMPT.to_string(),
            )),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        },
        ChatMessage {
            role: Role::User,
            content: Some(serde_json::Value::String(conversation_text)),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        },
    ];

    let params = CompletionParams {
        model,
        messages: &extraction_messages,
        temperature: 0.0,
        max_tokens: Some(1024),
        tools: None,
    };

    let raw_output = match provider.chat_completion(&params).await {
        Ok(resp) => resp
            .choices
            .first()
            .and_then(|c| c.message.text_content())
            .unwrap_or_default(),
        Err(e) => {
            tracing::warn!(error = %e, "session memory extraction LLM call failed");
            return ExtractionResult {
                memory: None,
                token_estimate: estimated_tokens,
            };
        }
    };

    match parse_session_memory(&raw_output) {
        Some(mem) => {
            tracing::info!(
                facts = mem.key_facts.len(),
                decisions = mem.decisions_made.len(),
                files = mem.files_modified.len(),
                "session memory extracted"
            );
            ExtractionResult {
                memory: Some(mem),
                token_estimate: estimated_tokens,
            }
        }
        None => {
            tracing::warn!("failed to parse session memory from LLM output");
            ExtractionResult {
                memory: None,
                token_estimate: estimated_tokens,
            }
        }
    }
}

/// Build a condensed conversation text for the extraction prompt.
///
/// Includes role labels and truncates individual messages to avoid
/// blowing up the side-query context.
fn build_conversation_summary(messages: &[ChatMessage]) -> String {
    const MAX_MSG_CHARS: usize = 500;
    let mut out = String::with_capacity(4096);

    for msg in messages {
        if matches!(msg.role, Role::System) {
            continue;
        }

        let role = match msg.role {
            Role::User => "USER",
            Role::Assistant => "ASSISTANT",
            Role::Tool => "TOOL",
            Role::System => unreachable!(),
        };

        let name_suffix = msg
            .name
            .as_deref()
            .map(|n| format!(" ({n})"))
            .unwrap_or_default();

        out.push_str(&format!("[{role}{name_suffix}] "));

        if let Some(text) = msg.text_content() {
            if text.len() > MAX_MSG_CHARS {
                out.push_str(&text[..MAX_MSG_CHARS]);
                out.push_str("...(truncated)");
            } else {
                out.push_str(&text);
            }
        }

        if let Some(ref tcs) = msg.tool_calls {
            for tc in tcs {
                out.push_str(&format!(" [call:{}]", tc.function.name));
            }
        }

        out.push('\n');
    }

    out
}

/// Parse the LLM's JSON output into a `SessionMemory`.
///
/// Handles common issues: markdown code fences, leading/trailing whitespace,
/// partial JSON. Returns `None` on parse failure rather than propagating errors.
fn parse_session_memory(raw: &str) -> Option<SessionMemory> {
    let trimmed = raw.trim();

    let json_str = if trimmed.starts_with("```") {
        trimmed
            .strip_prefix("```json")
            .or_else(|| trimmed.strip_prefix("```"))
            .and_then(|s| s.strip_suffix("```"))
            .unwrap_or(trimmed)
            .trim()
    } else {
        trimmed
    };

    serde_json::from_str::<SessionMemory>(json_str).ok()
}

/// Inject extracted session memory into the system prompt as an additional
/// context block. This preserves the memory across compression rounds.
pub(crate) fn inject_session_memory(messages: &mut [ChatMessage], memory: &SessionMemory) {
    let mut block = String::from("<session_memory>\n");

    if !memory.key_facts.is_empty() {
        block.push_str("<key_facts>\n");
        for fact in &memory.key_facts {
            block.push_str(&format!("- {fact}\n"));
        }
        block.push_str("</key_facts>\n");
    }

    if !memory.decisions_made.is_empty() {
        block.push_str("<decisions>\n");
        for dec in &memory.decisions_made {
            block.push_str(&format!("- {dec}\n"));
        }
        block.push_str("</decisions>\n");
    }

    if !memory.current_task_state.is_empty() {
        block.push_str(&format!(
            "<current_task>{}</current_task>\n",
            memory.current_task_state
        ));
    }

    if !memory.files_modified.is_empty() {
        block.push_str("<files_modified>\n");
        for f in &memory.files_modified {
            block.push_str(&format!("- {f}\n"));
        }
        block.push_str("</files_modified>\n");
    }

    block.push_str("</session_memory>");

    if let Some(sys_msg) = messages.iter_mut().find(|m| matches!(m.role, Role::System)) {
        let existing = sys_msg.text_content().unwrap_or_default();
        if !existing.contains("<session_memory>") {
            sys_msg.content = Some(serde_json::Value::String(format!("{existing}\n\n{block}")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_json() {
        let raw = r#"{
            "key_facts": ["uses Rust", "project is FastClaw"],
            "decisions_made": ["chose SQLite over Postgres"],
            "current_task_state": "implementing session memory",
            "files_modified": ["src/runtime/session_memory.rs"]
        }"#;

        let mem = parse_session_memory(raw).expect("should parse");
        assert_eq!(mem.key_facts.len(), 2);
        assert_eq!(mem.decisions_made.len(), 1);
        assert_eq!(mem.current_task_state, "implementing session memory");
        assert_eq!(mem.files_modified.len(), 1);
    }

    #[test]
    fn parse_json_with_markdown_fences() {
        let raw = r#"```json
{
    "key_facts": ["fact1"],
    "decisions_made": [],
    "current_task_state": "working",
    "files_modified": []
}
```"#;

        let mem = parse_session_memory(raw).expect("should strip fences");
        assert_eq!(mem.key_facts, vec!["fact1"]);
    }

    #[test]
    fn parse_invalid_json_returns_none() {
        assert!(parse_session_memory("not json at all").is_none());
        assert!(parse_session_memory("").is_none());
        assert!(parse_session_memory("{broken").is_none());
    }

    #[test]
    fn build_summary_truncates_long_messages() {
        let long_content = "x".repeat(1000);
        let messages = vec![ChatMessage {
            role: Role::User,
            content: Some(serde_json::Value::String(long_content)),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }];

        let summary = build_conversation_summary(&messages);
        assert!(summary.contains("...(truncated)"));
        assert!(summary.len() < 600);
    }

    #[test]
    fn inject_memory_appends_to_system_prompt() {
        let mut messages = vec![
            ChatMessage {
                role: Role::System,
                content: Some(serde_json::Value::String("You are helpful.".into())),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: Role::User,
                content: Some(serde_json::Value::String("hello".into())),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
        ];

        let memory = SessionMemory {
            key_facts: vec!["uses Rust".into()],
            decisions_made: vec!["chose SQLite".into()],
            current_task_state: "implementing feature X".into(),
            files_modified: vec!["src/main.rs".into()],
        };

        inject_session_memory(&mut messages, &memory);

        let sys = messages[0].text_content().unwrap();
        assert!(sys.contains("<session_memory>"), "should inject block");
        assert!(sys.contains("uses Rust"), "should include facts");
        assert!(sys.contains("chose SQLite"), "should include decisions");
        assert!(
            sys.contains("implementing feature X"),
            "should include task state"
        );
        assert!(sys.contains("src/main.rs"), "should include files");
        assert!(
            sys.starts_with("You are helpful."),
            "should preserve original prompt"
        );
    }
}
