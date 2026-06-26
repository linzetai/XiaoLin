//! Critic/verifier side-query for model gap compensation.
//!
//! After the agent produces a key output (code, document, analysis), a separate
//! side-query reviews the output for correctness. Issues found are fed back for
//! immediate correction.
//!
//! Task-complexity routing lives in `xiaolin-model-router`.

use std::sync::Arc;

use xiaolin_core::types::{ChatMessage, Role};

use super::side_query::{side_query, SideQueryOptions, SideQuerySource};
use super::task_decomposer::TaskType;
use crate::llm::LlmProvider;

/// Configuration for the critic/verifier.
#[derive(Debug, Clone)]
pub struct CriticConfig {
    pub enabled: bool,
    /// Model to use for criticism (can be same as main or different).
    pub model: String,
    /// Maximum tokens for critic response.
    pub max_tokens: u32,
    /// Minimum output length (chars) to trigger critic review.
    pub min_output_chars: usize,
}

impl Default for CriticConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: String::new(),
            max_tokens: 512,
            min_output_chars: 200,
        }
    }
}

/// Result of a critic review.
#[derive(Debug, Clone)]
pub struct CriticReview {
    /// Whether the output passed review.
    pub approved: bool,
    /// Issues found (empty if approved).
    pub issues: Vec<String>,
    /// Suggested improvements (optional).
    pub suggestions: Vec<String>,
}

impl CriticReview {
    /// Format the review as guidance to inject into the next LLM turn.
    pub fn format_for_injection(&self) -> Option<String> {
        if self.approved {
            return None;
        }

        let mut block = String::with_capacity(256);
        block.push_str("─── Review Feedback ────────────────────────────────\n");
        block.push_str("Your output has issues that need correction:\n\n");

        for (i, issue) in self.issues.iter().enumerate() {
            block.push_str(&format!("{}. {}\n", i + 1, issue));
        }

        if !self.suggestions.is_empty() {
            block.push_str("\nSuggestions:\n");
            for suggestion in &self.suggestions {
                block.push_str(&format!("  • {}\n", suggestion));
            }
        }

        block.push_str("\nPlease fix the issues above before proceeding.\n");
        block.push_str("────────────────────────────────────────────────────\n");
        Some(block)
    }
}

/// Run a critic review on the agent's output.
///
/// Returns `None` if the critic is disabled, output is too short, or the
/// LLM call fails.
pub async fn run_critic(
    provider: &Arc<dyn LlmProvider>,
    task_type: TaskType,
    agent_output: &str,
    config: &CriticConfig,
) -> Option<CriticReview> {
    if !config.enabled {
        return None;
    }

    if agent_output.len() < config.min_output_chars {
        return None;
    }

    let system_prompt = critic_system_prompt(task_type);
    let user_msg = format!(
        "Review this output for correctness and quality:\n\n{}",
        truncate_for_review(agent_output, 3000)
    );

    let model = if config.model.is_empty() {
        "default".to_string()
    } else {
        config.model.clone()
    };

    let opts = SideQueryOptions {
        model,
        system: Some(system_prompt.to_string()),
        messages: vec![ChatMessage {
            role: Role::User,
            content: Some(serde_json::Value::String(user_msg)),
            ..Default::default()
        }],
        max_tokens: Some(config.max_tokens),
        temperature: 0.2,
        max_retries: 1,
        query_source: SideQuerySource::Background,
        optional: true,
        abort: None,
    };

    match side_query(provider, opts).await {
        Ok(Some(result)) => Some(parse_critic_response(&result.content)),
        _ => None,
    }
}

fn critic_system_prompt(task_type: TaskType) -> &'static str {
    match task_type {
        TaskType::Coding => CODE_CRITIC_PROMPT,
        TaskType::Writing => WRITING_CRITIC_PROMPT,
        TaskType::DataAnalysis => DATA_CRITIC_PROMPT,
        _ => GENERAL_CRITIC_PROMPT,
    }
}

const CODE_CRITIC_PROMPT: &str = "\
You are a code reviewer. Check the output for:
1. Logic errors or bugs
2. Missing error handling
3. Security vulnerabilities
4. API misuse or wrong assumptions

Output format (exactly):
APPROVED
or
ISSUES:
- issue 1
- issue 2
SUGGESTIONS:
- suggestion 1";

const WRITING_CRITIC_PROMPT: &str = "\
You are an editor. Check the output for:
1. Logical consistency and coherence
2. Missing key points or arguments
3. Factual accuracy (flag anything questionable)
4. Appropriate tone and structure

Output format (exactly):
APPROVED
or
ISSUES:
- issue 1
SUGGESTIONS:
- suggestion 1";

const DATA_CRITIC_PROMPT: &str = "\
You are a data analyst reviewer. Check the output for:
1. Statistical methodology correctness
2. Whether conclusions are supported by the data shown
3. Calculation errors or misinterpretations
4. Missing caveats or limitations

Output format (exactly):
APPROVED
or
ISSUES:
- issue 1
SUGGESTIONS:
- suggestion 1";

const GENERAL_CRITIC_PROMPT: &str = "\
You are a quality reviewer. Check the output for:
1. Correctness and accuracy
2. Completeness (are key points addressed?)
3. Clarity and coherence
4. Any obvious errors

Output format (exactly):
APPROVED
or
ISSUES:
- issue 1
SUGGESTIONS:
- suggestion 1";

fn parse_critic_response(response: &str) -> CriticReview {
    let trimmed = response.trim();

    if trimmed.starts_with("APPROVED") || trimmed.to_uppercase().starts_with("APPROVED") {
        return CriticReview {
            approved: true,
            issues: Vec::new(),
            suggestions: Vec::new(),
        };
    }

    let mut issues = Vec::new();
    let mut suggestions = Vec::new();
    let mut in_issues = false;
    let mut in_suggestions = false;

    for line in trimmed.lines() {
        let line = line.trim();
        if line.starts_with("ISSUES:") || line.starts_with("Issues:") {
            in_issues = true;
            in_suggestions = false;
            continue;
        }
        if line.starts_with("SUGGESTIONS:") || line.starts_with("Suggestions:") {
            in_suggestions = true;
            in_issues = false;
            continue;
        }

        if line.starts_with('-') || line.starts_with('•') || line.starts_with('*') {
            let content = line
                .trim_start_matches(['-', '•', '*', ' '])
                .trim()
                .to_string();
            if content.len() >= 5 {
                if in_issues {
                    issues.push(content);
                } else if in_suggestions {
                    suggestions.push(content);
                } else {
                    issues.push(content);
                }
            }
        }
    }

    CriticReview {
        approved: issues.is_empty(),
        issues,
        suggestions,
    }
}

fn truncate_for_review(text: &str, max_chars: usize) -> &str {
    if text.len() <= max_chars {
        text
    } else {
        let boundary = text
            .char_indices()
            .nth(max_chars)
            .map(|(i, _)| i)
            .unwrap_or(text.len());
        &text[..boundary]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_critic_approved() {
        let review = parse_critic_response("APPROVED");
        assert!(review.approved);
        assert!(review.issues.is_empty());
    }

    #[test]
    fn parse_critic_with_issues() {
        let response = "ISSUES:\n- Missing null check on line 42\n- SQL injection vulnerability in query builder\nSUGGESTIONS:\n- Use parameterized queries instead";
        let review = parse_critic_response(response);
        assert!(!review.approved);
        assert_eq!(review.issues.len(), 2);
        assert_eq!(review.suggestions.len(), 1);
        assert!(review.issues[0].contains("null check"));
        assert!(review.issues[1].contains("SQL injection"));
        assert!(review.suggestions[0].contains("parameterized"));
    }

    #[test]
    fn parse_critic_empty_issues_means_approved() {
        let response = "ISSUES:\nSUGGESTIONS:\n- Could improve readability";
        let review = parse_critic_response(response);
        assert!(review.approved);
        assert_eq!(review.suggestions.len(), 1);
    }

    #[test]
    fn critic_review_format_approved_returns_none() {
        let review = CriticReview {
            approved: true,
            issues: Vec::new(),
            suggestions: Vec::new(),
        };
        assert!(review.format_for_injection().is_none());
    }

    #[test]
    fn critic_review_format_with_issues() {
        let review = CriticReview {
            approved: false,
            issues: vec!["Missing error handling".into(), "Race condition".into()],
            suggestions: vec!["Add mutex lock".into()],
        };
        let formatted = review.format_for_injection().unwrap();
        assert!(formatted.contains("Missing error handling"));
        assert!(formatted.contains("Race condition"));
        assert!(formatted.contains("Add mutex lock"));
        assert!(formatted.contains("fix the issues"));
    }

    #[tokio::test]
    async fn critic_disabled_returns_none() {
        use async_trait::async_trait;
        use xiaolin_core::types::ChatResponse;

        struct PanicProvider;
        #[async_trait]
        impl LlmProvider for PanicProvider {
            async fn chat_completion(
                &self,
                _: &crate::llm::CompletionParams<'_>,
            ) -> anyhow::Result<ChatResponse> {
                panic!("should not be called");
            }
            async fn chat_completion_stream(
                &self,
                _: &crate::llm::CompletionParams<'_>,
            ) -> anyhow::Result<
                futures::stream::BoxStream<
                    'static,
                    anyhow::Result<xiaolin_core::types::StreamDelta>,
                >,
            > {
                panic!("should not be called");
            }
        }

        let provider: Arc<dyn LlmProvider> = Arc::new(PanicProvider);
        let config = CriticConfig::default(); // disabled by default
        let result = run_critic(&provider, TaskType::Coding, "some long output here that is definitely more than 200 characters so it would trigger the critic if enabled but it won't because disabled", &config).await;
        assert!(result.is_none());
    }
}
