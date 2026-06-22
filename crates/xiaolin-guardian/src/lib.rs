mod prompt;

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Guardian configuration.
#[derive(Debug, Clone)]
pub struct GuardianConfig {
    /// Whether the Guardian is enabled.
    pub enabled: bool,
    /// When `enabled` is false, only auto-allow if the user explicitly opted out.
    /// If false, a missing or failed config load is treated as fail-closed deny.
    pub explicitly_disabled: bool,
    /// Timeout for LLM review calls.
    pub timeout: Duration,
    /// Model to use for reviews.
    pub model: String,
    /// Maximum tokens for the intent transcript.
    pub max_transcript_tokens: usize,
}

impl Default for GuardianConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            explicitly_disabled: true,
            timeout: Duration::from_secs(60),
            model: "deepseek/deepseek-v4-flash".to_string(),
            max_transcript_tokens: 10000,
        }
    }
}

impl GuardianConfig {
    /// Config successfully loaded with Guardian enabled.
    pub fn enabled_from_config(timeout: Duration, model: String, max_transcript_tokens: usize) -> Self {
        Self {
            enabled: true,
            explicitly_disabled: false,
            timeout,
            model,
            max_transcript_tokens,
        }
    }

    /// Config successfully loaded with Guardian explicitly disabled by the user.
    pub fn explicitly_disabled_from_config() -> Self {
        Self {
            enabled: false,
            explicitly_disabled: true,
            ..Default::default()
        }
    }

    /// Config missing or failed to parse — fail-closed.
    pub fn unavailable() -> Self {
        Self {
            enabled: false,
            explicitly_disabled: false,
            ..Default::default()
        }
    }
}

/// Decision from Guardian review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuardianDecision {
    Allow,
    Deny,
}

/// Risk level assessment (aligned with Codex: includes Critical).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuardianRiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// How much authorization the user has provided for this action.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuardianUserAuthorization {
    #[default]
    Unknown,
    Low,
    Medium,
    High,
}

/// Structured assessment from Guardian LLM review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardianAssessment {
    pub decision: GuardianDecision,
    pub risk_level: GuardianRiskLevel,
    #[serde(default)]
    pub user_authorization: GuardianUserAuthorization,
    pub rationale: String,
}

impl GuardianAssessment {
    /// Create a deny assessment for error/timeout cases (fail-closed).
    pub fn deny(reason: &str) -> Self {
        Self {
            decision: GuardianDecision::Deny,
            risk_level: GuardianRiskLevel::High,
            user_authorization: GuardianUserAuthorization::Unknown,
            rationale: reason.to_string(),
        }
    }

    pub fn is_allowed(&self) -> bool {
        self.decision == GuardianDecision::Allow
    }
}

/// Operation to be reviewed by the Guardian.
#[derive(Debug, Clone)]
pub struct ReviewOperation {
    /// The command to be executed.
    pub command: String,
    /// Working directory.
    pub working_dir: Option<String>,
    /// Type of operation (e.g., "shell_exec", "file_write").
    pub operation_type: String,
}

/// Context for Guardian review, including user intent.
#[derive(Debug, Clone)]
pub struct ReviewContext {
    /// Compact transcript of recent user messages.
    pub intent_transcript: String,
    /// Number of tokens in the transcript (approximate).
    pub transcript_tokens: usize,
}

/// Trait for LLM invocation, allowing dependency injection.
/// This avoids circular dependency with xiaolin-agent.
#[async_trait::async_trait]
pub trait GuardianLlm: Send + Sync {
    /// Send a prompt and get a text response.
    async fn complete(&self, prompt: &str, model: &str) -> anyhow::Result<String>;
}

// ---------------------------------------------------------------------------
// Circuit Breaker
// ---------------------------------------------------------------------------

pub const MAX_CONSECUTIVE_DENIALS_PER_TURN: u32 = 3;
pub const MAX_RECENT_DENIALS_PER_TURN: u32 = 10;
pub const DENIAL_WINDOW_SIZE: usize = 50;
/// Max distinct turn IDs tracked by the circuit breaker before evicting oldest.
pub const MAX_CIRCUIT_BREAKER_TURNS: usize = 100;

/// Action to take after recording a denial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CircuitBreakerAction {
    Continue,
    InterruptTurn {
        consecutive_denials: u32,
        recent_denials: u32,
    },
}

#[derive(Debug, Default)]
struct CircuitBreakerTurnState {
    consecutive_denials: u32,
    recent_outcomes: VecDeque<bool>,
    interrupt_triggered: bool,
}

/// Tracks per-turn denial counts to detect runaway denial loops.
///
/// Each turn is identified by a string key. When the agent's actions are
/// repeatedly denied by the Guardian, the circuit breaker returns
/// `InterruptTurn` to stop the current turn instead of letting the agent
/// keep retrying.
pub struct CircuitBreaker {
    turns: HashMap<String, CircuitBreakerTurnState>,
}

impl CircuitBreaker {
    pub fn new() -> Self {
        Self {
            turns: HashMap::new(),
        }
    }

    fn evict_oldest_turn_if_needed(&mut self) {
        if self.turns.len() < MAX_CIRCUIT_BREAKER_TURNS {
            return;
        }
        if let Some(oldest) = self.turns.keys().next().cloned() {
            self.turns.remove(&oldest);
            tracing::warn!(
                max = MAX_CIRCUIT_BREAKER_TURNS,
                removed = %oldest,
                remaining = self.turns.len(),
                "circuit breaker turns map at capacity; evicted oldest entry"
            );
        }
    }

    fn turn_state(&mut self, turn_id: &str) -> &mut CircuitBreakerTurnState {
        if !self.turns.contains_key(turn_id) {
            self.evict_oldest_turn_if_needed();
        }
        self.turns.entry(turn_id.to_string()).or_default()
    }

    /// Record a denial and check whether the turn should be interrupted.
    pub fn record_denial(&mut self, turn_id: &str) -> CircuitBreakerAction {
        let state = self.turn_state(turn_id);

        if state.interrupt_triggered {
            state.consecutive_denials = 0;
            state.interrupt_triggered = false;
        }

        state.consecutive_denials += 1;
        push_outcome(&mut state.recent_outcomes, true);

        if state.consecutive_denials >= MAX_CONSECUTIVE_DENIALS_PER_TURN {
            let consecutive = state.consecutive_denials;
            let recent = state.recent_outcomes.iter().filter(|&&d| d).count() as u32;
            tracing::warn!(
                consecutive,
                "circuit breaker: consecutive denial threshold reached"
            );
            state.interrupt_triggered = true;
            return CircuitBreakerAction::InterruptTurn {
                consecutive_denials: consecutive,
                recent_denials: recent,
            };
        }

        let recent = state.recent_outcomes.iter().filter(|&&d| d).count() as u32;
        if recent >= MAX_RECENT_DENIALS_PER_TURN {
            let consecutive = state.consecutive_denials;
            tracing::warn!(
                recent,
                window = DENIAL_WINDOW_SIZE,
                "circuit breaker: recent denial threshold reached"
            );
            state.interrupt_triggered = true;
            return CircuitBreakerAction::InterruptTurn {
                consecutive_denials: consecutive,
                recent_denials: recent,
            };
        }

        CircuitBreakerAction::Continue
    }

    /// Record a non-denial (allow), resetting the consecutive counter.
    pub fn record_non_denial(&mut self, turn_id: &str) {
        let state = self.turn_state(turn_id);

        if state.interrupt_triggered {
            state.consecutive_denials = 0;
            state.interrupt_triggered = false;
        }

        state.consecutive_denials = 0;
        push_outcome(&mut state.recent_outcomes, false);
    }

    /// Clear state for a specific turn.
    pub fn clear_turn(&mut self, turn_id: &str) {
        self.turns.remove(turn_id);
    }
}

fn push_outcome(outcomes: &mut VecDeque<bool>, is_denial: bool) {
    if outcomes.len() >= DENIAL_WINDOW_SIZE {
        outcomes.pop_front();
    }
    outcomes.push_back(is_denial);
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Guardian Agent
// ---------------------------------------------------------------------------

/// The Guardian agent: reviews operations for safety using LLM.
pub struct Guardian {
    config: GuardianConfig,
    llm: Arc<dyn GuardianLlm>,
}

impl Guardian {
    pub fn new(config: GuardianConfig, llm: Arc<dyn GuardianLlm>) -> Self {
        Self { config, llm }
    }

    /// Review an operation. Returns assessment or deny on failure (fail-closed).
    pub async fn review(
        &self,
        operation: &ReviewOperation,
        context: &ReviewContext,
    ) -> GuardianAssessment {
        if !self.config.enabled {
            if self.config.explicitly_disabled {
                tracing::info!("Guardian explicitly disabled by user; auto-allowing operation");
                return GuardianAssessment {
                    decision: GuardianDecision::Allow,
                    risk_level: GuardianRiskLevel::Low,
                    user_authorization: GuardianUserAuthorization::Unknown,
                    rationale: "Guardian explicitly disabled by user; auto-allow".to_string(),
                };
            }
            tracing::warn!(
                "Guardian configuration unavailable or invalid; denying operation (fail-closed)"
            );
            return GuardianAssessment::deny(
                "Guardian configuration unavailable; fail-closed deny",
            );
        }

        let review_prompt = prompt::build_review_prompt(operation, context);

        match tokio::time::timeout(
            self.config.timeout,
            self.llm.complete(&review_prompt, &self.config.model),
        )
        .await
        {
            Ok(Ok(response)) => self.parse_response(&response),
            Ok(Err(e)) => {
                tracing::error!(error = %e, "Guardian LLM call failed");
                GuardianAssessment::deny(&format!("LLM call failed: {e}"))
            }
            Err(_) => {
                tracing::error!(
                    timeout_secs = self.config.timeout.as_secs(),
                    "Guardian review timed out"
                );
                GuardianAssessment::deny(&format!(
                    "Review timed out after {}s",
                    self.config.timeout.as_secs()
                ))
            }
        }
    }

    fn parse_response(&self, response: &str) -> GuardianAssessment {
        let _ = &self.config;

        if let Some(assessment) = parse_validated_assessment(response) {
            return validate_assessment(assessment);
        }

        if let Some(start) = response.find('{') {
            if let Some(end) = response.rfind('}') {
                if let Some(assessment) = parse_validated_assessment(&response[start..=end]) {
                    return validate_assessment(assessment);
                }
            }
        }

        tracing::error!("Failed to parse Guardian response as JSON");
        GuardianAssessment::deny("Failed to parse LLM response as valid assessment JSON")
    }
}

fn parse_validated_assessment(json_str: &str) -> Option<GuardianAssessment> {
    let mut val: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let decision_raw = val.get("decision")?.as_str()?.to_string();
    let normalized = match decision_raw.as_str() {
        "allow" | "deny" => decision_raw,
        "needs_confirmation" => {
            tracing::info!(
                decision = decision_raw,
                "Guardian LLM returned needs_confirmation; treating as deny pending user review"
            );
            "deny".to_string()
        }
        other => {
            tracing::warn!(decision = %other, "Guardian response has invalid decision value");
            return None;
        }
    };
    val.as_object_mut()?
        .insert("decision".to_string(), serde_json::Value::String(normalized));
    serde_json::from_value(val).ok()
}

/// Enforce consistency invariants on a parsed Guardian assessment.
fn validate_assessment(mut assessment: GuardianAssessment) -> GuardianAssessment {
    if assessment.decision == GuardianDecision::Allow
        && matches!(
            assessment.risk_level,
            GuardianRiskLevel::High | GuardianRiskLevel::Critical
        )
    {
        tracing::warn!(
            risk_level = ?assessment.risk_level,
            rationale = %assessment.rationale,
            "Guardian assessment inconsistent: allow with high/critical risk; downgrading to deny"
        );
        let original = assessment.rationale.clone();
        assessment.decision = GuardianDecision::Deny;
        assessment.rationale = format!(
            "[auto-corrected] inconsistent allow with {:?} risk: {original}",
            assessment.risk_level
        );
    }
    assessment
}

/// Kind of transcript entry, used for budget allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryKind {
    User,
    Assistant,
    Tool(String),
}

/// A structured transcript entry for two-budget selection.
#[derive(Debug, Clone)]
pub struct TranscriptEntry {
    pub kind: EntryKind,
    pub text: String,
}

const GUARDIAN_MAX_TOOL_TOKENS: usize = 10_000;
const CHARS_PER_TOKEN: usize = 4;

/// Build a compact intent transcript using a two-budget model:
/// - `message_budget`: for User and Assistant entries
/// - `tool_budget`: for Tool entries
///
/// Strategy (aligned with Codex):
/// 1. Always include first and last user turns as anchors
/// 2. Fill remaining user/assistant turns from most recent to oldest
/// 3. Fill tool entries from most recent to oldest with a separate budget
pub fn build_intent_transcript_v2(
    entries: &[TranscriptEntry],
    max_message_tokens: usize,
) -> ReviewContext {
    let msg_budget_chars = max_message_tokens * CHARS_PER_TOKEN;
    let tool_budget_chars = GUARDIAN_MAX_TOOL_TOKENS * CHARS_PER_TOKEN;

    let mut message_slots: Vec<(usize, String)> = Vec::new();
    let mut tool_slots: Vec<(usize, String)> = Vec::new();

    let mut msg_used = 0usize;

    // Phase 1: anchor first and last user entries
    let first_user = entries
        .iter()
        .enumerate()
        .find(|(_, e)| e.kind == EntryKind::User);
    let last_user = entries
        .iter()
        .enumerate()
        .rev()
        .find(|(_, e)| e.kind == EntryKind::User);

    let mut anchored: std::collections::HashSet<usize> = std::collections::HashSet::new();

    if let Some((idx, entry)) = first_user {
        let line = format!("[user]: {}\n", entry.text);
        if msg_used + line.len() <= msg_budget_chars {
            msg_used += line.len();
            message_slots.push((idx, line));
            anchored.insert(idx);
        }
    }
    if let Some((idx, entry)) = last_user {
        if !anchored.contains(&idx) {
            let line = format!("[user]: {}\n", entry.text);
            if msg_used + line.len() <= msg_budget_chars {
                msg_used += line.len();
                message_slots.push((idx, line));
                anchored.insert(idx);
            }
        }
    }

    // Phase 2: fill remaining message entries (most recent first)
    for (idx, entry) in entries.iter().enumerate().rev() {
        if anchored.contains(&idx) {
            continue;
        }
        match &entry.kind {
            EntryKind::User | EntryKind::Assistant => {
                let role = if entry.kind == EntryKind::User {
                    "user"
                } else {
                    "assistant"
                };
                let line = format!("[{role}]: {}\n", entry.text);
                if msg_used + line.len() > msg_budget_chars {
                    continue;
                }
                msg_used += line.len();
                message_slots.push((idx, line));
            }
            EntryKind::Tool(_) => {}
        }
    }

    // Phase 3: fill tool entries (most recent first) with separate budget
    let mut tool_chars_used = 0usize;
    for (idx, entry) in entries.iter().enumerate().rev() {
        if let EntryKind::Tool(name) = &entry.kind {
            let line = format!("[tool:{name}]: {}\n", entry.text);
            if tool_chars_used + line.len() > tool_budget_chars {
                continue;
            }
            tool_chars_used += line.len();
            tool_slots.push((idx, line));
        }
    }
    let tool_used = tool_chars_used;

    // Merge and sort by original index to preserve chronological order
    let mut all_slots = message_slots;
    all_slots.extend(tool_slots);
    all_slots.sort_by_key(|(idx, _)| *idx);

    let transcript: String = all_slots.into_iter().map(|(_, line)| line).collect();
    let total_chars = msg_used + tool_used;

    ReviewContext {
        intent_transcript: transcript,
        transcript_tokens: total_chars / CHARS_PER_TOKEN,
    }
}

/// Simple transcript builder (backward-compatible convenience wrapper).
pub fn build_intent_transcript(
    messages: &[(String, String)], // (role, content) pairs
    max_tokens: usize,
) -> ReviewContext {
    let entries: Vec<TranscriptEntry> = messages
        .iter()
        .map(|(role, content)| TranscriptEntry {
            kind: match role.as_str() {
                "user" => EntryKind::User,
                "assistant" => EntryKind::Assistant,
                other => EntryKind::Tool(other.to_string()),
            },
            text: content.clone(),
        })
        .collect();

    build_intent_transcript_v2(&entries, max_tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockLlm {
        response: String,
    }

    #[async_trait::async_trait]
    impl GuardianLlm for MockLlm {
        async fn complete(&self, _prompt: &str, _model: &str) -> anyhow::Result<String> {
            Ok(self.response.clone())
        }
    }

    struct FailingLlm;

    #[async_trait::async_trait]
    impl GuardianLlm for FailingLlm {
        async fn complete(&self, _prompt: &str, _model: &str) -> anyhow::Result<String> {
            Err(anyhow::anyhow!("API error"))
        }
    }

    struct SlowLlm;

    #[async_trait::async_trait]
    impl GuardianLlm for SlowLlm {
        async fn complete(&self, _prompt: &str, _model: &str) -> anyhow::Result<String> {
            tokio::time::sleep(Duration::from_secs(120)).await;
            Ok("never reached".into())
        }
    }

    fn test_operation() -> ReviewOperation {
        ReviewOperation {
            command: "rm -rf /tmp/test".to_string(),
            working_dir: Some("/home/user/project".to_string()),
            operation_type: "shell_exec".to_string(),
        }
    }

    fn test_context() -> ReviewContext {
        ReviewContext {
            intent_transcript: "[user]: Clean up temp files".to_string(),
            transcript_tokens: 10,
        }
    }

    // --- Guardian review tests ---

    #[tokio::test]
    async fn review_allow_response() {
        let llm = Arc::new(MockLlm {
            response: r#"{"decision":"allow","risk_level":"low","rationale":"Safe temp cleanup"}"#
                .to_string(),
        });
        let guardian = Guardian::new(
            GuardianConfig {
                enabled: true,
                ..Default::default()
            },
            llm,
        );

        let result = guardian.review(&test_operation(), &test_context()).await;
        assert!(result.is_allowed());
        assert_eq!(result.risk_level, GuardianRiskLevel::Low);
        assert_eq!(result.user_authorization, GuardianUserAuthorization::Unknown);
    }

    #[tokio::test]
    async fn review_deny_response() {
        let llm = Arc::new(MockLlm {
            response: r#"{"decision":"deny","risk_level":"high","rationale":"Dangerous"}"#
                .to_string(),
        });
        let guardian = Guardian::new(
            GuardianConfig {
                enabled: true,
                ..Default::default()
            },
            llm,
        );

        let result = guardian.review(&test_operation(), &test_context()).await;
        assert!(!result.is_allowed());
        assert_eq!(result.risk_level, GuardianRiskLevel::High);
    }

    #[tokio::test]
    async fn review_critical_risk_level() {
        let llm = Arc::new(MockLlm {
            response: r#"{"decision":"deny","risk_level":"critical","user_authorization":"low","rationale":"Extremely dangerous"}"#
                .to_string(),
        });
        let guardian = Guardian::new(
            GuardianConfig {
                enabled: true,
                ..Default::default()
            },
            llm,
        );

        let result = guardian.review(&test_operation(), &test_context()).await;
        assert!(!result.is_allowed());
        assert_eq!(result.risk_level, GuardianRiskLevel::Critical);
        assert_eq!(result.user_authorization, GuardianUserAuthorization::Low);
    }

    #[tokio::test]
    async fn review_with_user_authorization() {
        let llm = Arc::new(MockLlm {
            response: r#"{"decision":"allow","risk_level":"medium","user_authorization":"high","rationale":"User explicitly requested"}"#
                .to_string(),
        });
        let guardian = Guardian::new(
            GuardianConfig {
                enabled: true,
                ..Default::default()
            },
            llm,
        );

        let result = guardian.review(&test_operation(), &test_context()).await;
        assert!(result.is_allowed());
        assert_eq!(result.user_authorization, GuardianUserAuthorization::High);
    }

    #[tokio::test]
    async fn fail_closed_on_llm_error() {
        let llm = Arc::new(FailingLlm);
        let guardian = Guardian::new(
            GuardianConfig {
                enabled: true,
                ..Default::default()
            },
            llm,
        );

        let result = guardian.review(&test_operation(), &test_context()).await;
        assert!(!result.is_allowed());
        assert!(result.rationale.contains("failed"));
    }

    #[tokio::test]
    async fn fail_closed_on_timeout() {
        let llm = Arc::new(SlowLlm);
        let guardian = Guardian::new(
            GuardianConfig {
                enabled: true,
                timeout: Duration::from_millis(50),
                ..Default::default()
            },
            llm,
        );

        let result = guardian.review(&test_operation(), &test_context()).await;
        assert!(!result.is_allowed());
        assert!(result.rationale.contains("timed out"));
    }

    #[tokio::test]
    async fn fail_closed_on_invalid_json() {
        let llm = Arc::new(MockLlm {
            response: "not valid json at all".to_string(),
        });
        let guardian = Guardian::new(
            GuardianConfig {
                enabled: true,
                ..Default::default()
            },
            llm,
        );

        let result = guardian.review(&test_operation(), &test_context()).await;
        assert!(!result.is_allowed());
        assert!(result.rationale.contains("parse"));
    }

    #[tokio::test]
    async fn disabled_guardian_auto_allows() {
        let llm = Arc::new(FailingLlm);
        let guardian = Guardian::new(GuardianConfig::default(), llm);

        let result = guardian.review(&test_operation(), &test_context()).await;
        assert!(result.is_allowed());
        assert!(result.rationale.contains("explicitly disabled"));
    }

    #[tokio::test]
    async fn unavailable_guardian_config_denies() {
        let llm = Arc::new(FailingLlm);
        let guardian = Guardian::new(GuardianConfig::unavailable(), llm);

        let result = guardian.review(&test_operation(), &test_context()).await;
        assert!(!result.is_allowed());
        assert!(result.rationale.contains("unavailable"));
    }

    #[tokio::test]
    async fn review_downgrades_allow_with_critical_risk() {
        let llm = Arc::new(MockLlm {
            response: r#"{"decision":"allow","risk_level":"critical","rationale":"Trust me"}"#
                .to_string(),
        });
        let guardian = Guardian::new(
            GuardianConfig {
                enabled: true,
                explicitly_disabled: false,
                ..Default::default()
            },
            llm,
        );

        let result = guardian.review(&test_operation(), &test_context()).await;
        assert!(!result.is_allowed());
        assert!(result.rationale.contains("auto-corrected"));
    }

    #[test]
    fn rejects_invalid_decision_value() {
        let json = r#"{"decision":"approve","risk_level":"low","rationale":"bad"}"#;
        assert!(parse_validated_assessment(json).is_none());
    }

    #[tokio::test]
    async fn json_in_markdown_code_block() {
        let llm = Arc::new(MockLlm {
            response: "Here is my assessment:\n```json\n{\"decision\":\"allow\",\"risk_level\":\"medium\",\"rationale\":\"OK\"}\n```".to_string(),
        });
        let guardian = Guardian::new(
            GuardianConfig {
                enabled: true,
                ..Default::default()
            },
            llm,
        );

        let result = guardian.review(&test_operation(), &test_context()).await;
        assert!(result.is_allowed());
    }

    // --- Circuit breaker tests ---

    #[test]
    fn circuit_breaker_consecutive_threshold() {
        let mut cb = CircuitBreaker::new();
        let turn = "turn-1";

        assert_eq!(cb.record_denial(turn), CircuitBreakerAction::Continue);
        assert_eq!(cb.record_denial(turn), CircuitBreakerAction::Continue);
        assert!(matches!(
            cb.record_denial(turn),
            CircuitBreakerAction::InterruptTurn {
                consecutive_denials: 3,
                ..
            }
        ));
    }

    #[test]
    fn circuit_breaker_resets_on_allow() {
        let mut cb = CircuitBreaker::new();
        let turn = "turn-1";

        assert_eq!(cb.record_denial(turn), CircuitBreakerAction::Continue);
        assert_eq!(cb.record_denial(turn), CircuitBreakerAction::Continue);
        cb.record_non_denial(turn);
        assert_eq!(cb.record_denial(turn), CircuitBreakerAction::Continue);
        assert_eq!(cb.record_denial(turn), CircuitBreakerAction::Continue);
        assert!(matches!(
            cb.record_denial(turn),
            CircuitBreakerAction::InterruptTurn { .. }
        ));
    }

    #[test]
    fn circuit_breaker_separate_turns_are_independent() {
        let mut cb = CircuitBreaker::new();

        assert_eq!(cb.record_denial("turn-1"), CircuitBreakerAction::Continue);
        assert_eq!(cb.record_denial("turn-1"), CircuitBreakerAction::Continue);
        // Different turn: independent state
        assert_eq!(cb.record_denial("turn-2"), CircuitBreakerAction::Continue);
        assert_eq!(cb.record_denial("turn-2"), CircuitBreakerAction::Continue);
        assert!(matches!(
            cb.record_denial("turn-2"),
            CircuitBreakerAction::InterruptTurn { .. }
        ));
    }

    #[test]
    fn circuit_breaker_recent_window_threshold() {
        let mut cb = CircuitBreaker::new();
        let turn = "turn-1";

        for _ in 0..MAX_RECENT_DENIALS_PER_TURN {
            cb.record_non_denial(turn);
            let _ = cb.record_denial(turn);
        }

        assert!(matches!(
            cb.record_denial(turn),
            CircuitBreakerAction::InterruptTurn { .. }
        ));
    }

    #[test]
    fn circuit_breaker_clear_turn() {
        let mut cb = CircuitBreaker::new();
        cb.record_denial("turn-1");
        cb.record_denial("turn-1");
        cb.clear_turn("turn-1");
        assert_eq!(cb.record_denial("turn-1"), CircuitBreakerAction::Continue);
    }

    #[test]
    fn circuit_breaker_interrupt_resets_consecutive_on_next_denial() {
        let mut cb = CircuitBreaker::new();
        let turn = "turn-1";

        cb.record_denial(turn);
        cb.record_denial(turn);
        let action = cb.record_denial(turn);
        assert!(matches!(action, CircuitBreakerAction::InterruptTurn { .. }));

        assert_eq!(cb.record_denial(turn), CircuitBreakerAction::Continue);
        assert_eq!(cb.record_denial(turn), CircuitBreakerAction::Continue);
    }

    #[test]
    fn circuit_breaker_interrupt_details() {
        let mut cb = CircuitBreaker::new();
        let turn = "turn-x";

        cb.record_denial(turn);
        cb.record_denial(turn);
        match cb.record_denial(turn) {
            CircuitBreakerAction::InterruptTurn {
                consecutive_denials,
                recent_denials,
            } => {
                assert_eq!(consecutive_denials, 3);
                assert_eq!(recent_denials, 3);
            }
            CircuitBreakerAction::Continue => panic!("expected InterruptTurn"),
        }
    }

    // --- Transcript tests ---

    #[test]
    fn build_transcript_respects_limit() {
        let messages: Vec<(String, String)> = (0..100)
            .map(|i| ("user".to_string(), format!("Message {i} with some content")))
            .collect();

        let ctx = build_intent_transcript(&messages, 100);
        assert!(ctx.transcript_tokens <= 100);
        assert!(!ctx.intent_transcript.is_empty());
    }

    #[test]
    fn build_transcript_preserves_order() {
        let messages = vec![
            ("user".to_string(), "first".to_string()),
            ("assistant".to_string(), "second".to_string()),
            ("user".to_string(), "third".to_string()),
        ];

        let ctx = build_intent_transcript(&messages, 10000);
        let first_pos = ctx.intent_transcript.find("first").unwrap();
        let third_pos = ctx.intent_transcript.find("third").unwrap();
        assert!(first_pos < third_pos);
    }

    #[test]
    fn v2_transcript_anchors_first_and_last_user() {
        let entries = vec![
            TranscriptEntry {
                kind: EntryKind::User,
                text: "initial request".into(),
            },
            TranscriptEntry {
                kind: EntryKind::Assistant,
                text: "working on it".into(),
            },
            TranscriptEntry {
                kind: EntryKind::Tool("shell".into()),
                text: "echo hello".into(),
            },
            TranscriptEntry {
                kind: EntryKind::User,
                text: "final clarification".into(),
            },
        ];

        let ctx = build_intent_transcript_v2(&entries, 10_000);
        assert!(ctx.intent_transcript.contains("initial request"));
        assert!(ctx.intent_transcript.contains("final clarification"));

        let first_pos = ctx.intent_transcript.find("initial request").unwrap();
        let last_pos = ctx.intent_transcript.find("final clarification").unwrap();
        assert!(first_pos < last_pos);
    }

    #[test]
    fn v2_transcript_separates_tool_budget() {
        let entries = vec![
            TranscriptEntry {
                kind: EntryKind::User,
                text: "do something".into(),
            },
            TranscriptEntry {
                kind: EntryKind::Tool("shell".into()),
                text: "ls -la".into(),
            },
            TranscriptEntry {
                kind: EntryKind::Tool("file_read".into()),
                text: "cat main.rs".into(),
            },
        ];

        let ctx = build_intent_transcript_v2(&entries, 10_000);
        assert!(ctx.intent_transcript.contains("[tool:shell]"));
        assert!(ctx.intent_transcript.contains("[tool:file_read]"));
    }

    #[test]
    fn v2_transcript_preserves_chronological_order() {
        let entries = vec![
            TranscriptEntry {
                kind: EntryKind::User,
                text: "step 1".into(),
            },
            TranscriptEntry {
                kind: EntryKind::Tool("a".into()),
                text: "tool a".into(),
            },
            TranscriptEntry {
                kind: EntryKind::Assistant,
                text: "reply".into(),
            },
            TranscriptEntry {
                kind: EntryKind::Tool("b".into()),
                text: "tool b".into(),
            },
            TranscriptEntry {
                kind: EntryKind::User,
                text: "step 2".into(),
            },
        ];

        let ctx = build_intent_transcript_v2(&entries, 10_000);
        let pos_1 = ctx.intent_transcript.find("step 1").unwrap();
        let pos_a = ctx.intent_transcript.find("tool a").unwrap();
        let pos_reply = ctx.intent_transcript.find("reply").unwrap();
        let pos_b = ctx.intent_transcript.find("tool b").unwrap();
        let pos_2 = ctx.intent_transcript.find("step 2").unwrap();

        assert!(pos_1 < pos_a);
        assert!(pos_a < pos_reply);
        assert!(pos_reply < pos_b);
        assert!(pos_b < pos_2);
    }

    #[test]
    fn assessment_deny_helper() {
        let a = GuardianAssessment::deny("test reason");
        assert!(!a.is_allowed());
        assert_eq!(a.risk_level, GuardianRiskLevel::High);
        assert_eq!(a.user_authorization, GuardianUserAuthorization::Unknown);
        assert_eq!(a.rationale, "test reason");
    }
}
