//! Generate 2-3 suggested next actions at the end of an assistant turn.
//!
//! Uses a lightweight side-query to produce actionable follow-up suggestions
//! based on the current conversation context (recent messages, tool results,
//! and ongoing tasks). Can be disabled via config.

use std::sync::Arc;

use fastclaw_core::types::{ChatMessage, Role};

use super::side_query::{side_query, SideQueryOptions, SideQuerySource};
use crate::llm::LlmProvider;

/// Configuration for the suggestion feature.
#[derive(Debug, Clone)]
pub struct SuggestionConfig {
    pub enabled: bool,
    pub model: String,
    pub max_suggestions: usize,
}

impl Default for SuggestionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model: "default".into(),
            max_suggestions: 3,
        }
    }
}

const SUGGESTION_SYSTEM_PROMPT: &str = "\
Based on the conversation context, suggest 2-3 short, actionable next steps \
the user might want to take. Each suggestion should be a single sentence \
(imperative form, under 80 characters). Output one suggestion per line, \
no numbering, no bullet points, no extra formatting. Only output the suggestions.";

/// Generate follow-up suggestions based on conversation context.
///
/// Returns an empty vec if disabled, context is too short, or LLM fails.
pub async fn generate_suggestions(
    provider: &Arc<dyn LlmProvider>,
    recent_messages: &[ChatMessage],
    config: &SuggestionConfig,
) -> Vec<String> {
    if !config.enabled {
        return Vec::new();
    }

    if recent_messages.len() < 2 {
        return Vec::new();
    }

    let context = build_context_summary(recent_messages);
    if context.trim().is_empty() {
        return Vec::new();
    }

    let user_msg = format!(
        "Recent conversation context:\n\n{}\n\nSuggest 2-3 next actions.",
        context
    );

    let opts = SideQueryOptions {
        model: config.model.clone(),
        system: Some(SUGGESTION_SYSTEM_PROMPT.to_string()),
        messages: vec![ChatMessage {
            role: Role::User,
            content: Some(serde_json::Value::String(user_msg)),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }],
        max_tokens: Some(256),
        temperature: 0.7,
        max_retries: 1,
        query_source: SideQuerySource::Background,
        optional: true,
        abort: None,
    };

    match side_query(provider, opts).await {
        Ok(Some(result)) => parse_suggestions(&result.content, config.max_suggestions),
        _ => Vec::new(),
    }
}

fn build_context_summary(messages: &[ChatMessage]) -> String {
    let recent = if messages.len() > 6 {
        &messages[messages.len() - 6..]
    } else {
        messages
    };

    let mut summary = String::new();
    for msg in recent {
        let role = match msg.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
            Role::Tool => "Tool",
            Role::System => continue,
        };
        let text = msg.text_content().unwrap_or_default();
        let truncated: String = text.chars().take(200).collect();
        summary.push_str(&format!("[{}]: {}\n", role, truncated));
    }
    summary
}

fn parse_suggestions(response: &str, max: usize) -> Vec<String> {
    response
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .filter(|l| l.len() >= 5 && l.len() <= 120)
        .map(|l| {
            l.trim_start_matches(|c: char| {
                c == '-' || c == '*' || c == '•' || c.is_ascii_digit() || c == '.'
            })
            .trim()
            .to_string()
        })
        .filter(|l| !l.is_empty())
        .take(max)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_suggestions_from_clean_lines() {
        let response = "Run the test suite\nDeploy to staging\nUpdate the README";
        let result = parse_suggestions(response, 3);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "Run the test suite");
        assert_eq!(result[1], "Deploy to staging");
        assert_eq!(result[2], "Update the README");
    }

    #[test]
    fn parse_suggestions_strips_bullets_and_respects_max() {
        let response =
            "- Fix the failing test\n- Review PR comments\n- Merge to main\n- Clean up branches";
        let result = parse_suggestions(response, 3);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "Fix the failing test");
    }

    #[tokio::test]
    async fn disabled_config_returns_empty() {
        use crate::llm::CompletionParams;
        use async_trait::async_trait;
        use fastclaw_core::types::ChatResponse;
        use std::sync::Arc;

        struct NoopProvider;
        #[async_trait]
        impl LlmProvider for NoopProvider {
            async fn chat_completion(
                &self,
                _: &CompletionParams<'_>,
            ) -> anyhow::Result<ChatResponse> {
                panic!("should not be called");
            }
            async fn chat_completion_stream(
                &self,
                _: &CompletionParams<'_>,
            ) -> anyhow::Result<
                futures::stream::BoxStream<
                    'static,
                    anyhow::Result<fastclaw_core::types::StreamDelta>,
                >,
            > {
                panic!("should not be called");
            }
        }

        let provider: Arc<dyn LlmProvider> = Arc::new(NoopProvider);
        let config = SuggestionConfig {
            enabled: false,
            ..Default::default()
        };
        let messages = vec![
            ChatMessage {
                role: Role::User,
                content: Some(serde_json::Value::String("hi".into())),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: Role::Assistant,
                content: Some(serde_json::Value::String("hello".into())),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        let result = generate_suggestions(&provider, &messages, &config).await;
        assert!(result.is_empty());
    }
}
