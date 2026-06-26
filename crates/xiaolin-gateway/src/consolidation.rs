use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use xiaolin_agent::{CompletionParams, LlmProvider};
use xiaolin_context::ContextHook;
use xiaolin_core::types::{ChatMessage, Role};
use xiaolin_memory::{
    EmbeddingProvider, Episode, EpisodicMemory, Fact, FactCategory, ImportanceScorer,
    SemanticMemory,
};

const CONSOLIDATION_PROMPT: &str = "\
Summarize this conversation in 2-3 sentences focusing on:
1. Key decisions made and their reasoning
2. User preferences or corrections expressed
3. Important facts learned

Also extract any user preferences as \"FACT: subject | predicate | object\" lines.
Reply with the summary first, then FACT lines (if any). Nothing else.";

const MAX_CONTEXT_MESSAGES: usize = 15;
const MAX_TOOL_RESULT_CHARS: usize = 100;

pub struct MemoryConsolidationHook {
    llm: Arc<dyn LlmProvider>,
    episodic_map: HashMap<String, Arc<EpisodicMemory>>,
    semantic_map: HashMap<String, Arc<SemanticMemory>>,
    embedder: Option<Arc<dyn EmbeddingProvider>>,
    scorer: ImportanceScorer,
    min_messages: usize,
    model: String,
}

impl MemoryConsolidationHook {
    pub fn new(
        llm: Arc<dyn LlmProvider>,
        episodic_map: HashMap<String, Arc<EpisodicMemory>>,
        semantic_map: HashMap<String, Arc<SemanticMemory>>,
        embedder: Option<Arc<dyn EmbeddingProvider>>,
        scorer: ImportanceScorer,
        min_messages: usize,
        model: String,
    ) -> Self {
        Self {
            llm,
            episodic_map,
            semantic_map,
            embedder,
            scorer,
            min_messages,
            model,
        }
    }
}

#[async_trait]
impl ContextHook for MemoryConsolidationHook {
    fn name(&self) -> &str {
        "memory_consolidation"
    }

    async fn on_after_turn(
        &self,
        messages: &[ChatMessage],
        agent_id: &str,
        session_id: &str,
    ) -> anyhow::Result<()> {
        let non_system_count = messages
            .iter()
            .filter(|m| !matches!(m.role, Role::System))
            .count();
        if non_system_count < self.min_messages {
            return Ok(());
        }

        let score = self.scorer.score(messages);
        if score < self.scorer.min_threshold {
            return Ok(());
        }

        let Some(episodic) = self.episodic_map.get(agent_id).cloned() else {
            return Ok(());
        };
        let semantic = self.semantic_map.get(agent_id).cloned();

        let compact = build_compact_context(messages);
        let model = self.model.clone();
        let llm = self.llm.clone();
        let embedder = self.embedder.clone();
        let session_id = session_id.to_string();
        let agent_id = agent_id.to_string();

        tokio::spawn(async move {
            if let Err(e) = run_consolidation(
                llm.as_ref(),
                &model,
                &compact,
                score,
                &episodic,
                semantic.as_deref(),
                embedder.as_deref(),
                &session_id,
                &agent_id,
            )
            .await
            {
                tracing::warn!(
                    agent_id = %agent_id,
                    session_id = %session_id,
                    error = %e,
                    "memory consolidation failed"
                );
            }
        });

        Ok(())
    }
}

fn build_compact_context(messages: &[ChatMessage]) -> String {
    let recent: Vec<&ChatMessage> = messages
        .iter()
        .filter(|m| !matches!(m.role, Role::System))
        .rev()
        .take(MAX_CONTEXT_MESSAGES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    let mut buf = String::new();
    for m in recent {
        let role_label = match m.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
            Role::Tool => "Tool",
            Role::System => "System",
        };
        let text = m.text_content().unwrap_or_default();
        let text = if matches!(m.role, Role::Tool) && text.len() > MAX_TOOL_RESULT_CHARS {
            let end = text.floor_char_boundary(MAX_TOOL_RESULT_CHARS);
            format!("{}...", &text[..end])
        } else {
            text.into_owned()
        };
        buf.push_str(&format!("{role_label}: {text}\n"));
    }
    buf
}

#[allow(clippy::too_many_arguments)]
async fn run_consolidation(
    llm: &dyn LlmProvider,
    model: &str,
    compact_context: &str,
    importance: f32,
    episodic: &EpisodicMemory,
    semantic: Option<&SemanticMemory>,
    embedder: Option<&dyn EmbeddingProvider>,
    session_id: &str,
    agent_id: &str,
) -> anyhow::Result<()> {
    let prompt_text = format!("{CONSOLIDATION_PROMPT}\n\nConversation:\n{compact_context}");
    let messages = vec![ChatMessage {
        role: Role::User,
        content: Some(serde_json::Value::String(prompt_text)),
        ..Default::default()
    }];

    let params = CompletionParams {
        model,
        messages: &messages,
        temperature: 0.3,
        max_tokens: Some(300),
        tools: None,
    };

    let resp = llm.chat_completion(&params).await?;
    let reply = resp
        .choices
        .first()
        .and_then(|c| c.message.text_content())
        .unwrap_or_default();

    if reply.trim().is_empty() {
        return Ok(());
    }

    let (summary, facts) = parse_consolidation_reply(&reply);

    if summary.split_whitespace().count() < 5 && facts.is_empty() {
        tracing::debug!(
            agent_id = %agent_id,
            summary_len = summary.len(),
            "consolidation reply too short or empty, skipping storage"
        );
        return Ok(());
    }

    let facts: Vec<_> = facts
        .into_iter()
        .filter(|(s, p, o)| !s.trim().is_empty() && !p.trim().is_empty() && !o.trim().is_empty())
        .collect();

    if !summary.is_empty() {
        let ep = Episode {
            id: format!("cons_{}", &uuid::Uuid::new_v4().to_string()[..8]),
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            summary: summary.clone(),
            importance,
            tags: "consolidation".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            dreamed_at: None,
        };
        episodic.record_auto(&ep, embedder).await?;
        tracing::info!(
            agent_id = %agent_id,
            session_id = %session_id,
            importance = importance,
            "consolidated episode stored"
        );
    }

    if let Some(sem) = semantic {
        for (subj, pred, obj) in &facts {
            let now = chrono::Utc::now().to_rfc3339();
            let fact = Fact {
                id: format!(
                    "cons_{}_{}",
                    subj.replace(' ', "_"),
                    &uuid::Uuid::new_v4().to_string()[..8]
                ),
                subject: subj.clone(),
                predicate: pred.clone(),
                object: obj.clone(),
                category: FactCategory::UserPreference.as_str().to_string(),
                confidence: importance.clamp(0.0, 1.0),
                source_session: Some(session_id.to_string()),
                created_at: now.clone(),
                updated_at: now,
            };
            if let Err(e) = sem.upsert_auto(&fact, embedder).await {
                tracing::warn!(error = %e, "consolidation: failed to store fact");
            }
        }
        if !facts.is_empty() {
            tracing::info!(
                agent_id = %agent_id,
                count = facts.len(),
                "consolidated facts stored"
            );
        }
    }

    Ok(())
}

fn parse_consolidation_reply(reply: &str) -> (String, Vec<(String, String, String)>) {
    let mut summary_lines = Vec::new();
    let mut facts = Vec::new();

    for line in reply.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("FACT:") {
            let parts: Vec<&str> = rest.split('|').map(|s| s.trim()).collect();
            if parts.len() == 3 && parts.iter().all(|p| !p.is_empty()) {
                facts.push((
                    parts[0].to_string(),
                    parts[1].to_string(),
                    parts[2].to_string(),
                ));
            }
        } else if !trimmed.is_empty() {
            summary_lines.push(trimmed);
        }
    }

    (summary_lines.join(" "), facts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reply_extracts_summary_and_facts() {
        let reply = "\
The user decided to use Postgres for the database. They prefer dark mode in the editor.

FACT: user | prefers_db | Postgres
FACT: user | prefers_theme | dark mode";

        let (summary, facts) = parse_consolidation_reply(reply);
        assert!(summary.contains("Postgres"));
        assert_eq!(facts.len(), 2);
        assert_eq!(
            facts[0],
            ("user".into(), "prefers_db".into(), "Postgres".into())
        );
        assert_eq!(
            facts[1],
            ("user".into(), "prefers_theme".into(), "dark mode".into())
        );
    }

    #[test]
    fn parse_reply_no_facts() {
        let reply = "Just a summary without any facts.";
        let (summary, facts) = parse_consolidation_reply(reply);
        assert_eq!(summary, "Just a summary without any facts.");
        assert!(facts.is_empty());
    }

    #[test]
    fn parse_reply_malformed_fact_ignored() {
        let reply = "Summary here.\nFACT: only two | parts\nFACT: | |";
        let (summary, facts) = parse_consolidation_reply(reply);
        assert_eq!(summary, "Summary here.");
        assert!(facts.is_empty());
    }

    #[test]
    fn build_compact_context_truncates_tool_output() {
        let msgs = vec![
            ChatMessage {
                role: Role::User,
                content: Some("hello".into()),
                ..Default::default()
            },
            ChatMessage {
                role: Role::Tool,
                content: Some(serde_json::Value::String("x".repeat(500))),
                tool_call_id: Some("tc1".into()),
                ..Default::default()
            },
        ];
        let ctx = build_compact_context(&msgs);
        assert!(ctx.contains("Tool: xxxx"));
        assert!(ctx.contains("..."));
        assert!(ctx.len() < 500);
    }
}
