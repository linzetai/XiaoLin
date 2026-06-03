//! Automatic task decomposition for complex user requests.
//!
//! When a user request appears complex (multi-step, multi-file, multi-domain),
//! this module uses a lightweight side-query to decompose it into an ordered
//! step list. The decomposition is injected into the system prompt so the LLM
//! can execute step-by-step without needing strong planning capabilities.
//!
//! This is a core part of the "model gap compensation" strategy: weak models
//! struggle with planning but can execute well when given explicit steps.

use std::sync::Arc;

use xiaolin_core::types::{ChatMessage, Role};

use super::side_query::{side_query, SideQueryOptions, SideQuerySource};
use crate::llm::LlmProvider;

/// Configuration for the task decomposition feature.
#[derive(Debug, Clone)]
pub struct TaskDecomposerConfig {
    pub enabled: bool,
    /// Model to use for decomposition (can be a cheaper model).
    pub model: String,
    /// Minimum message length (chars) to trigger decomposition.
    pub min_complexity_chars: usize,
    /// Maximum number of steps to generate.
    pub max_steps: usize,
    /// Maximum tokens for the decomposition response.
    pub max_tokens: u32,
}

impl Default for TaskDecomposerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model: String::new(),
            min_complexity_chars: 80,
            max_steps: 10,
            max_tokens: 512,
        }
    }
}

/// Result of task decomposition.
#[derive(Debug, Clone)]
pub struct Decomposition {
    /// The task category detected (coding, research, writing, data, workflow, etc.)
    pub task_type: TaskType,
    /// Ordered list of steps.
    pub steps: Vec<String>,
    /// Whether the task was deemed complex enough to warrant decomposition.
    #[allow(dead_code)] // TODO(integrate): expose in decomposition stream events
    pub was_decomposed: bool,
}

/// Broad task categories that affect decomposition strategy and downstream
/// validation/context assembly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskType {
    /// Code writing, debugging, refactoring.
    Coding,
    /// Information gathering, comparison, summarization.
    Research,
    /// Document writing, email, content creation.
    Writing,
    /// Data processing, analysis, visualization.
    DataAnalysis,
    /// Automation, scheduling, pipeline setup.
    Workflow,
    /// General Q&A, conversation.
    General,
}

impl TaskType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Coding => "coding",
            Self::Research => "research",
            Self::Writing => "writing",
            Self::DataAnalysis => "data_analysis",
            Self::Workflow => "workflow",
            Self::General => "general",
        }
    }

    pub fn from_str_loose_pub(s: &str) -> Self {
        Self::from_str_loose(s)
    }

    fn from_str_loose(s: &str) -> Self {
        let lower = s.to_lowercase();
        if lower.contains("cod")
            || lower.contains("debug")
            || lower.contains("refactor")
            || lower.contains("implement")
            || lower.contains("fix")
            || lower.contains("compil")
        {
            Self::Coding
        } else if lower.contains("research")
            || lower.contains("search")
            || lower.contains("find")
            || lower.contains("compar")
            || lower.contains("investig")
        {
            Self::Research
        } else if lower.contains("writ")
            || lower.contains("document")
            || lower.contains("email")
            || lower.contains("draft")
            || lower.contains("report")
        {
            Self::Writing
        } else if lower.contains("data")
            || lower.contains("analy")
            || lower.contains("chart")
            || lower.contains("statistic")
            || lower.contains("csv")
            || lower.contains("sql")
        {
            Self::DataAnalysis
        } else if lower.contains("automat")
            || lower.contains("workflow")
            || lower.contains("schedul")
            || lower.contains("pipeline")
            || lower.contains("cron")
        {
            Self::Workflow
        } else {
            Self::General
        }
    }
}

const DECOMPOSER_SYSTEM_PROMPT: &str = "\
You are a task planning assistant. Given a user request, output:
1. A single word on line 1: the task type (coding/research/writing/data_analysis/workflow/general)
2. A numbered step list (one step per line, 3-10 steps)

Rules:
- Each step must be concrete and actionable (not vague like \"understand the problem\")
- Steps should be in execution order with dependencies respected
- Keep each step under 100 characters
- If the task is simple (1-2 obvious steps), output just those steps
- Output ONLY the type line and numbered steps, nothing else

Example output:
coding
1. Read the existing auth middleware to understand the pattern
2. Create new file src/middleware/rateLimit.ts
3. Implement token bucket algorithm with configurable limits
4. Register middleware in the Express app router
5. Add unit tests for rate limiting logic
6. Run tests and fix any failures";

/// Attempt to decompose a complex user request into steps.
///
/// Returns `None` if:
/// - Feature is disabled
/// - The request is too short (below complexity threshold)
/// - The LLM call fails (optional/best-effort)
pub async fn decompose_task(
    provider: &Arc<dyn LlmProvider>,
    user_message: &str,
    config: &TaskDecomposerConfig,
) -> Option<Decomposition> {
    if !config.enabled {
        return None;
    }

    let trimmed = user_message.trim();
    if trimmed.len() < config.min_complexity_chars {
        return Some(Decomposition {
            task_type: TaskType::from_str_loose(trimmed),
            steps: Vec::new(),
            was_decomposed: false,
        });
    }

    let user_msg = format!(
        "Decompose this task into steps:\n\n{}",
        truncate_for_decomposition(trimmed, 2000)
    );

    let model = if config.model.is_empty() {
        "default".to_string()
    } else {
        config.model.clone()
    };

    let opts = SideQueryOptions {
        model,
        system: Some(DECOMPOSER_SYSTEM_PROMPT.to_string()),
        messages: vec![ChatMessage {
            role: Role::User,
            content: Some(serde_json::Value::String(user_msg)),
        ..Default::default()
        }],
        max_tokens: Some(config.max_tokens),
        temperature: 0.3,
        max_retries: 1,
        query_source: SideQuerySource::Background,
        optional: true,
        abort: None,
    };

    let result = side_query(provider, opts).await;
    match result {
        Ok(Some(r)) => Some(parse_decomposition(&r.content, config.max_steps)),
        _ => {
            tracing::debug!("task decomposition side-query failed, skipping");
            None
        }
    }
}

/// Format a decomposition as a prompt injection block.
///
/// Returns `None` if there are no steps (task was too simple).
pub fn format_decomposition_for_prompt(decomposition: &Decomposition) -> Option<String> {
    if decomposition.steps.is_empty() {
        return None;
    }

    let mut block = String::with_capacity(512);
    block.push_str("─── Task Plan ───────────────────────────────────────\n");
    block.push_str(&format!(
        "Task type: {}\n",
        decomposition.task_type.as_str()
    ));
    block.push_str("Execute these steps in order:\n");
    for (i, step) in decomposition.steps.iter().enumerate() {
        block.push_str(&format!("  {}. {}\n", i + 1, step));
    }
    block.push_str("After completing each step, verify the result before proceeding.\n");
    block.push_str("─────────────────────────────────────────────────────\n");
    Some(block)
}

fn parse_decomposition(response: &str, max_steps: usize) -> Decomposition {
    let mut lines = response.lines().map(|l| l.trim()).filter(|l| !l.is_empty());

    let first_line = lines.next().unwrap_or("general");
    let task_type = TaskType::from_str_loose(first_line);

    let steps: Vec<String> = lines
        .filter_map(|line| {
            let stripped = line.trim_start_matches(|c: char| {
                c.is_ascii_digit() || c == '.' || c == ')' || c == ' '
            });
            let stripped = stripped.trim_start_matches(['-', '*', ' ']);
            let stripped = stripped.trim();
            if stripped.len() >= 5 {
                Some(stripped.to_string())
            } else {
                None
            }
        })
        .take(max_steps)
        .collect();

    Decomposition {
        task_type,
        steps,
        was_decomposed: true,
    }
}

fn truncate_for_decomposition(text: &str, max_chars: usize) -> &str {
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
    fn parse_decomposition_basic() {
        let response = "coding\n1. Read the file\n2. Fix the bug\n3. Run tests";
        let result = parse_decomposition(response, 10);
        assert_eq!(result.task_type, TaskType::Coding);
        assert_eq!(result.steps.len(), 3);
        assert_eq!(result.steps[0], "Read the file");
        assert_eq!(result.steps[1], "Fix the bug");
        assert_eq!(result.steps[2], "Run tests");
    }

    #[test]
    fn parse_decomposition_respects_max_steps() {
        let response =
            "research\n1. Step one\n2. Step two\n3. Step three\n4. Step four\n5. Step five";
        let result = parse_decomposition(response, 3);
        assert_eq!(result.steps.len(), 3);
    }

    #[test]
    fn parse_decomposition_handles_various_bullet_formats() {
        let response = "writing\n- Draft the introduction section\n* Write the methodology\n3) Add the conclusion";
        let result = parse_decomposition(response, 10);
        assert_eq!(result.task_type, TaskType::Writing);
        assert_eq!(result.steps.len(), 3);
        assert_eq!(result.steps[0], "Draft the introduction section");
    }

    #[test]
    fn parse_decomposition_skips_short_lines() {
        let response = "coding\n1. ok\n2. Fix the authentication middleware\n3. hi";
        let result = parse_decomposition(response, 10);
        assert_eq!(result.steps.len(), 1);
        assert_eq!(result.steps[0], "Fix the authentication middleware");
    }

    #[test]
    fn task_type_detection() {
        assert_eq!(
            TaskType::from_str_loose("fix the compile error"),
            TaskType::Coding
        );
        assert_eq!(
            TaskType::from_str_loose("research AI trends"),
            TaskType::Research
        );
        assert_eq!(
            TaskType::from_str_loose("write a report"),
            TaskType::Writing
        );
        assert_eq!(
            TaskType::from_str_loose("analyze the CSV data"),
            TaskType::DataAnalysis
        );
        assert_eq!(
            TaskType::from_str_loose("automate daily backup"),
            TaskType::Workflow
        );
        assert_eq!(TaskType::from_str_loose("hello there"), TaskType::General);
    }

    #[test]
    fn format_decomposition_empty_steps_returns_none() {
        let d = Decomposition {
            task_type: TaskType::General,
            steps: Vec::new(),
            was_decomposed: false,
        };
        assert!(format_decomposition_for_prompt(&d).is_none());
    }

    #[test]
    fn format_decomposition_produces_readable_block() {
        let d = Decomposition {
            task_type: TaskType::Coding,
            steps: vec![
                "Read the existing code".into(),
                "Implement the fix".into(),
                "Run tests".into(),
            ],
            was_decomposed: true,
        };
        let block = format_decomposition_for_prompt(&d).unwrap();
        assert!(block.contains("Task type: coding"));
        assert!(block.contains("1. Read the existing code"));
        assert!(block.contains("2. Implement the fix"));
        assert!(block.contains("3. Run tests"));
        assert!(block.contains("verify the result"));
    }

    #[test]
    fn truncate_preserves_short_text() {
        assert_eq!(truncate_for_decomposition("hello", 100), "hello");
    }

    #[test]
    fn truncate_cuts_long_text() {
        let long = "a".repeat(500);
        let result = truncate_for_decomposition(&long, 100);
        assert_eq!(result.len(), 100);
    }

    #[tokio::test]
    async fn disabled_config_returns_none() {
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
        let config = TaskDecomposerConfig {
            enabled: false,
            ..Default::default()
        };
        let result =
            decompose_task(&provider, "do something complex with many steps", &config).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn short_message_skips_decomposition() {
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
        let config = TaskDecomposerConfig::default();
        let result = decompose_task(&provider, "fix typo", &config).await;
        assert!(result.is_some());
        let d = result.unwrap();
        assert!(!d.was_decomposed);
        assert!(d.steps.is_empty());
    }
}
