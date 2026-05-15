use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;

use fastclaw_core::types::{ChatMessage, Role};
use fastclaw_memory::{EmbeddingProvider, Fact, FactCategory, SemanticMemory};

use crate::engine::{ContextHook, IngestInput};

/// Scans the last user message for explicit memory-trigger keywords and
/// auto-stores the captured content as a [`SemanticMemory`] fact.
///
/// A system hint is injected so the LLM knows the content was captured.
pub struct MemoryKeywordInterceptor {
    semantic_map: HashMap<String, Arc<SemanticMemory>>,
    embedder: Option<Arc<dyn EmbeddingProvider>>,
    patterns: Vec<Regex>,
}

impl MemoryKeywordInterceptor {
    pub fn new(
        semantic_map: HashMap<String, Arc<SemanticMemory>>,
        embedder: Option<Arc<dyn EmbeddingProvider>>,
    ) -> Self {
        Self {
            semantic_map,
            embedder,
            patterns: build_patterns(),
        }
    }
}

fn build_patterns() -> Vec<Regex> {
    let raw = [
        // English
        r"(?i)remember\s+(?:that\s+)?(.{4,})",
        r"(?i)note\s+(?:this|that)[:\s]+(.{4,})",
        r"(?i)keep\s+in\s+mind[:\s]+(.{4,})",
        r"(?i)don'?t\s+forget[:\s]+(.{4,})",
        r"(?i)my\s+preference\s+is[:\s]+(.{4,})",
        // Chinese
        r"记住(.{2,})",
        r"记一下(.{2,})",
        r"别忘了(.{2,})",
        r"我的偏好是(.{2,})",
        r"以后注意(.{2,})",
    ];
    raw.iter()
        .filter_map(|p| match Regex::new(p) {
            Ok(r) => Some(r),
            Err(e) => {
                tracing::warn!(pattern = p, error = %e, "failed to compile keyword pattern");
                None
            }
        })
        .collect()
}

fn extract_keyword_content(text: &str, patterns: &[Regex]) -> Option<String> {
    for pat in patterns {
        if let Some(caps) = pat.captures(text) {
            if let Some(m) = caps.get(1) {
                let content = m.as_str().trim();
                if !content.is_empty() {
                    return Some(content.to_string());
                }
            }
        }
    }
    None
}

#[async_trait]
impl ContextHook for MemoryKeywordInterceptor {
    fn name(&self) -> &str {
        "memory_keyword_interceptor"
    }

    async fn on_ingest(
        &self,
        input: &IngestInput,
        messages: &mut Vec<ChatMessage>,
    ) -> anyhow::Result<()> {
        let Some(semantic) = self.semantic_map.get(&input.agent_id) else {
            return Ok(());
        };

        let last_user_text = input
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, Role::User))
            .and_then(ChatMessage::text_content);

        let text = match last_user_text.as_deref() {
            Some(t) if !t.is_empty() => t,
            _ => return Ok(()),
        };

        let Some(content) = extract_keyword_content(text, &self.patterns) else {
            return Ok(());
        };

        let id = format!("kw_{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let now = chrono::Utc::now().to_rfc3339();
        let fact = Fact {
            id,
            subject: "user".to_string(),
            predicate: "stated".to_string(),
            object: content.clone(),
            category: FactCategory::UserPreference.as_str().to_string(),
            confidence: 1.0,
            source_session: Some(input.session_id.clone()),
            created_at: now.clone(),
            updated_at: now,
        };

        if let Err(e) = semantic.upsert_auto(&fact, self.embedder.as_deref()).await {
            tracing::warn!(error = %e, "keyword interceptor: failed to auto-store fact");
        } else {
            tracing::info!(
                content = %content,
                agent_id = %input.agent_id,
                "keyword interceptor: auto-stored user statement"
            );
        }

        let hint = format!(
            "[Auto-captured] User explicitly asked to remember: \"{content}\". Acknowledge this."
        );
        let pos = messages
            .iter()
            .position(|m| !matches!(m.role, Role::System))
            .unwrap_or(messages.len());
        messages.insert(
            pos,
            ChatMessage {
                role: Role::System,
                content: Some(hint.into()),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
            compact_metadata: None,
            },
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_remember_captures_content() {
        let patterns = build_patterns();
        assert_eq!(
            extract_keyword_content("remember that I use fish shell", &patterns),
            Some("I use fish shell".to_string())
        );
    }

    #[test]
    fn english_note_this() {
        let patterns = build_patterns();
        assert_eq!(
            extract_keyword_content("note this: always use tabs", &patterns),
            Some("always use tabs".to_string())
        );
    }

    #[test]
    fn english_dont_forget() {
        let patterns = build_patterns();
        assert_eq!(
            extract_keyword_content("don't forget: deploy on Fridays", &patterns),
            Some("deploy on Fridays".to_string())
        );
    }

    #[test]
    fn english_keep_in_mind() {
        let patterns = build_patterns();
        assert_eq!(
            extract_keyword_content("keep in mind: I prefer dark mode", &patterns),
            Some("I prefer dark mode".to_string())
        );
    }

    #[test]
    fn english_preference() {
        let patterns = build_patterns();
        assert_eq!(
            extract_keyword_content("my preference is Rust over Go", &patterns),
            Some("Rust over Go".to_string())
        );
    }

    #[test]
    fn chinese_remember() {
        let patterns = build_patterns();
        assert_eq!(
            extract_keyword_content("记住我喜欢用 Vim", &patterns),
            Some("我喜欢用 Vim".to_string())
        );
    }

    #[test]
    fn chinese_note() {
        let patterns = build_patterns();
        assert_eq!(
            extract_keyword_content("记一下项目用 pnpm", &patterns),
            Some("项目用 pnpm".to_string())
        );
    }

    #[test]
    fn chinese_dont_forget() {
        let patterns = build_patterns();
        assert_eq!(
            extract_keyword_content("别忘了数据库用的是 MySQL", &patterns),
            Some("数据库用的是 MySQL".to_string())
        );
    }

    #[test]
    fn no_match_returns_none() {
        let patterns = build_patterns();
        assert_eq!(
            extract_keyword_content("just a normal message", &patterns),
            None
        );
    }

    #[test]
    fn too_short_content_ignored() {
        let patterns = build_patterns();
        // "remember X" — captured group "X" is only 1 char, below the 4-char minimum
        assert_eq!(extract_keyword_content("remember X", &patterns), None);
    }
}
