//! LLM-driven memory relevance selection.
//!
//! Two-stage pipeline: vector recall (top-20 candidates) → LLM rerank (top-5).
//! Falls back to vector top-5 when the LLM rerank fails or is unavailable.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::side_query::{side_query, SideQueryOptions, SideQuerySource};
use crate::llm::LlmProvider;

const CANDIDATE_LIMIT: usize = 20;
const SELECT_LIMIT: usize = 5;

/// A memory candidate with its metadata and relevance score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryCandidate {
    pub id: String,
    pub content: String,
    pub relevance: f32,
}

/// Result of the memory selection pipeline.
#[derive(Debug, Clone)]
pub struct SelectionResult {
    pub selected: Vec<MemoryCandidate>,
    pub method: SelectionMethod,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionMethod {
    LlmRerank,
    VectorFallback,
    Empty,
}

/// Build the LLM rerank prompt from candidate memories.
pub fn build_rerank_prompt(query: &str, candidates: &[MemoryCandidate]) -> String {
    let mut prompt = format!(
        "You are selecting the most relevant memories for a user's current query.\n\n\
         Current query: {query}\n\n\
         Below are candidate memories (numbered). Select up to {SELECT_LIMIT} that are most \
         relevant to the query. Return ONLY the numbers of selected memories, \
         one per line, in order of relevance (most relevant first).\n\n"
    );

    for (i, c) in candidates.iter().enumerate() {
        let preview = if c.content.len() > 200 {
            format!("{}...", &c.content[..200])
        } else {
            c.content.clone()
        };
        prompt.push_str(&format!("{}. {}\n", i + 1, preview));
    }

    prompt.push_str("\nSelected numbers (one per line):");
    prompt
}

/// Parse the LLM response into selected indices (0-based).
pub fn parse_rerank_response(response: &str, candidate_count: usize) -> Vec<usize> {
    let mut indices = Vec::new();
    for line in response.lines() {
        let trimmed = line.trim().trim_end_matches('.');
        if let Ok(n) = trimmed.parse::<usize>() {
            if n >= 1 && n <= candidate_count && !indices.contains(&(n - 1)) {
                indices.push(n - 1);
            }
        }
        if indices.len() >= SELECT_LIMIT {
            break;
        }
    }
    indices
}

/// Run the full memory selection pipeline.
///
/// 1. Recall top-20 candidates from the memory manager
/// 2. If candidates are empty, return immediately
/// 3. Attempt LLM rerank via side_query (optional mode)
/// 4. Fallback to vector top-5 if LLM fails
pub async fn select_relevant_memories(
    candidates: Vec<MemoryCandidate>,
    query: &str,
    provider: &Arc<dyn LlmProvider>,
    model: &str,
) -> SelectionResult {
    if candidates.is_empty() {
        return SelectionResult {
            selected: Vec::new(),
            method: SelectionMethod::Empty,
        };
    }

    let capped: Vec<_> = candidates.into_iter().take(CANDIDATE_LIMIT).collect();

    let prompt = build_rerank_prompt(query, &capped);
    let opts = SideQueryOptions {
        model: model.to_string(),
        messages: vec![fastclaw_core::types::ChatMessage {
            role: fastclaw_core::types::Role::User,
            content: Some(serde_json::Value::String(prompt)),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
            compact_metadata: None,
        }],
        max_tokens: Some(100),
        temperature: 0.0,
        max_retries: 1,
        query_source: SideQuerySource::Foreground,
        optional: true,
        abort: None,
        ..Default::default()
    };

    if let Ok(Some(result)) = side_query(provider, opts).await {
        let indices = parse_rerank_response(&result.content, capped.len());
        if !indices.is_empty() {
            let selected: Vec<_> = indices
                .into_iter()
                .filter_map(|i| capped.get(i).cloned())
                .collect();
            return SelectionResult {
                selected,
                method: SelectionMethod::LlmRerank,
            };
        }
    }

    let selected: Vec<_> = capped.into_iter().take(SELECT_LIMIT).collect();
    SelectionResult {
        selected,
        method: SelectionMethod::VectorFallback,
    }
}

/// Convert recalled memories from the memory manager into candidates.
pub fn recalled_to_candidates(
    recalled: Vec<fastclaw_memory::RecalledMemory>,
) -> Vec<MemoryCandidate> {
    recalled
        .into_iter()
        .map(|r| MemoryCandidate {
            id: r.id,
            content: r.content,
            relevance: r.relevance,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rerank_valid_numbers() {
        let response = "1\n3\n5\n";
        let indices = parse_rerank_response(response, 10);
        assert_eq!(indices, vec![0, 2, 4]);
    }

    #[test]
    fn parse_rerank_ignores_out_of_range() {
        let response = "0\n1\n99\n2\n";
        let indices = parse_rerank_response(response, 5);
        assert_eq!(indices, vec![0, 1]);
    }

    #[test]
    fn parse_rerank_deduplicates() {
        let response = "1\n1\n2\n2\n3\n";
        let indices = parse_rerank_response(response, 5);
        assert_eq!(indices, vec![0, 1, 2]);
    }

    #[test]
    fn parse_rerank_respects_limit() {
        let response = "1\n2\n3\n4\n5\n6\n7\n";
        let indices = parse_rerank_response(response, 10);
        assert_eq!(indices.len(), SELECT_LIMIT);
    }

    #[test]
    fn build_rerank_prompt_truncates_long_content() {
        let candidates = vec![MemoryCandidate {
            id: "1".into(),
            content: "x".repeat(300),
            relevance: 0.9,
        }];
        let prompt = build_rerank_prompt("test query", &candidates);
        assert!(prompt.contains("..."));
        assert!(prompt.len() < 600);
    }
}
