use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use xiaolin_core::types::{ChatMessage, ChatResponse, Role};
use xiaolin_protocol::{GuardianOutcome, RiskLevel};
use tokio::sync::Mutex;

use crate::llm::{CompletionParams, LlmProvider};

/// Assessment result produced by the Guardian reviewer.
#[derive(Debug, Clone)]
pub struct GuardianAssessment {
    pub review_id: String,
    pub risk_level: RiskLevel,
    pub outcome: GuardianOutcome,
    pub rationale: String,
}

/// Circuit breaker state to halt turns after repeated denials.
struct CircuitBreaker {
    consecutive_denials: u32,
    recent_outcomes: VecDeque<bool>,
    max_consecutive: u32,
    window_size: usize,
    window_threshold: u32,
}

impl CircuitBreaker {
    fn new() -> Self {
        Self {
            consecutive_denials: 0,
            recent_outcomes: VecDeque::with_capacity(50),
            max_consecutive: 3,
            window_size: 50,
            window_threshold: 10,
        }
    }

    fn record(&mut self, allowed: bool) {
        if allowed {
            self.consecutive_denials = 0;
        } else {
            self.consecutive_denials += 1;
        }
        if self.recent_outcomes.len() >= self.window_size {
            self.recent_outcomes.pop_front();
        }
        self.recent_outcomes.push_back(allowed);
    }

    fn is_tripped(&self) -> bool {
        if self.consecutive_denials >= self.max_consecutive {
            return true;
        }
        let denial_count = self.recent_outcomes.iter().filter(|&&ok| !ok).count() as u32;
        denial_count >= self.window_threshold
    }
}

/// Reviews high-risk tool actions via a secondary LLM call.
pub struct GuardianReviewer {
    llm: Arc<dyn LlmProvider>,
    timeout: Duration,
    circuit_breaker: Mutex<CircuitBreaker>,
    model: String,
}

impl GuardianReviewer {
    pub fn new(llm: Arc<dyn LlmProvider>, model: String) -> Self {
        Self {
            llm,
            timeout: Duration::from_secs(60),
            circuit_breaker: Mutex::new(CircuitBreaker::new()),
            model,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Returns true if the circuit breaker is tripped (too many denials).
    pub async fn is_circuit_tripped(&self) -> bool {
        self.circuit_breaker.lock().await.is_tripped()
    }

    /// Review an action description and return an assessment.
    pub async fn review(&self, action_description: &str) -> anyhow::Result<GuardianAssessment> {
        let review_id = uuid::Uuid::new_v4().to_string();

        let system_prompt = concat!(
            "You are a security reviewer for an AI coding agent. ",
            "Evaluate the following tool action for safety risks. ",
            "Respond with a JSON object containing:\n",
            "- \"risk_level\": one of \"low\", \"medium\", \"high\", \"critical\"\n",
            "- \"outcome\": \"allow\" or \"deny\"\n",
            "- \"rationale\": a brief explanation\n\n",
            "Be conservative: deny destructive or irreversible operations ",
            "(e.g. force push, recursive delete, dropping databases). ",
            "Allow standard development operations (read files, run tests, git commit)."
        );

        let messages = vec![
            ChatMessage {
                role: Role::System,
                content: Some(serde_json::Value::String(system_prompt.to_string())),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
                compact_metadata: None,
            },
            ChatMessage {
                role: Role::User,
                content: Some(serde_json::Value::String(format!(
                    "Review this action:\n{}",
                    action_description
                ))),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
                compact_metadata: None,
            },
        ];

        let params = CompletionParams {
            model: &self.model,
            messages: &messages,
            temperature: 0.0,
            max_tokens: Some(256),
            tools: None,
        };

        let result = tokio::time::timeout(self.timeout, self.llm.chat_completion(&params)).await;

        let assessment = match result {
            Ok(Ok(response)) => parse_guardian_response(&review_id, &response),
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "guardian LLM call failed, defaulting to deny");
                GuardianAssessment {
                    review_id: review_id.clone(),
                    risk_level: RiskLevel::High,
                    outcome: GuardianOutcome::Deny,
                    rationale: format!("Guardian LLM error: {e}"),
                }
            }
            Err(_) => {
                tracing::warn!("guardian LLM call timed out, defaulting to deny");
                GuardianAssessment {
                    review_id: review_id.clone(),
                    risk_level: RiskLevel::High,
                    outcome: GuardianOutcome::Deny,
                    rationale: "Guardian review timed out".to_string(),
                }
            }
        };

        let allowed = assessment.outcome == GuardianOutcome::Allow;
        self.circuit_breaker.lock().await.record(allowed);

        Ok(assessment)
    }
}

fn parse_guardian_response(review_id: &str, response: &ChatResponse) -> GuardianAssessment {
    let text = response
        .choices
        .first()
        .and_then(|c| c.message.text_content())
        .unwrap_or_default();

    let trimmed = text.trim();
    let json_str = if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            &trimmed[start..=end]
        } else {
            trimmed
        }
    } else {
        trimmed
    };

    if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
        let risk_level = match val
            .get("risk_level")
            .and_then(|v| v.as_str())
            .unwrap_or("high")
        {
            "low" => RiskLevel::Low,
            "medium" => RiskLevel::Medium,
            "high" => RiskLevel::High,
            "critical" => RiskLevel::Critical,
            _ => RiskLevel::High,
        };

        let outcome = match val
            .get("outcome")
            .and_then(|v| v.as_str())
            .unwrap_or("deny")
        {
            "allow" => GuardianOutcome::Allow,
            _ => GuardianOutcome::Deny,
        };

        let rationale = val
            .get("rationale")
            .and_then(|v| v.as_str())
            .unwrap_or("No rationale provided")
            .to_string();

        GuardianAssessment {
            review_id: review_id.to_string(),
            risk_level,
            outcome,
            rationale,
        }
    } else {
        GuardianAssessment {
            review_id: review_id.to_string(),
            risk_level: RiskLevel::High,
            outcome: GuardianOutcome::Deny,
            rationale: format!("Failed to parse guardian response: {text}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circuit_breaker_trips_on_consecutive_denials() {
        let mut cb = CircuitBreaker::new();
        cb.record(false);
        cb.record(false);
        assert!(!cb.is_tripped());
        cb.record(false);
        assert!(cb.is_tripped());
    }

    #[test]
    fn circuit_breaker_resets_on_allow() {
        let mut cb = CircuitBreaker::new();
        cb.record(false);
        cb.record(false);
        cb.record(true);
        assert!(!cb.is_tripped());
    }

    #[test]
    fn circuit_breaker_trips_on_window_threshold() {
        let mut cb = CircuitBreaker::new();
        for _ in 0..10 {
            cb.record(false);
            cb.record(true);
        }
        assert!(cb.is_tripped());
    }

    fn make_test_response(content: &str) -> ChatResponse {
        use xiaolin_core::types::ChatChoice;
        ChatResponse {
            id: "test".into(),
            object: "chat.completion".into(),
            created: 0,
            model: "gpt-4".into(),
            choices: vec![ChatChoice {
                index: 0,
                message: ChatMessage {
                    role: Role::Assistant,
                    content: Some(serde_json::Value::String(content.to_string())),
                    reasoning_content: None,
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                    compact_metadata: None,
                },
                finish_reason: Some("stop".into()),
            }],
            usage: None,
        }
    }

    #[test]
    fn parse_guardian_response_valid_json() {
        let json_str = r#"{"risk_level": "low", "outcome": "allow", "rationale": "Safe read operation"}"#;
        let response = make_test_response(json_str);
        let assessment = parse_guardian_response("r1", &response);
        assert_eq!(assessment.risk_level, RiskLevel::Low);
        assert_eq!(assessment.outcome, GuardianOutcome::Allow);
    }

    #[test]
    fn parse_guardian_response_invalid_json_defaults_to_deny() {
        let response = make_test_response("not valid json");
        let assessment = parse_guardian_response("r2", &response);
        assert_eq!(assessment.outcome, GuardianOutcome::Deny);
    }
}
