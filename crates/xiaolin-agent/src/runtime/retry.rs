//! Enhanced retry logic for LLM API calls.
//!
//! Wraps API calls with exponential backoff, jitter, and source-aware retry
//! strategies. Key behaviors:
//!
//! - **429 (Rate Limited)**: Always retry, respecting the `retry-after` header.
//! - **529 (Overloaded)**: Only retry for foreground queries (MainThread/Agent);
//!   Background sources give up immediately to avoid gateway amplification.
//! - **401 (Auth Expired)**: Retry once after credential refresh attempt.
//! - **Timeout/Reset**: Retry with exponential backoff.
//! - **PromptTooLong/InvalidApiKey/BudgetExhausted**: Never retry (terminal).

use std::future::Future;
use std::time::Duration;

use super::api_errors::{ApiErrorClassifier, ApiErrorKind};

/// Where the query originates — affects retry aggressiveness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuerySource {
    /// User-facing main loop query.
    #[allow(dead_code)] // used in tests; production paths use Agent/Background
    MainThread,
    /// SubAgent or delegated task.
    Agent,
    /// Background side-query (memory consolidation, skill extraction, etc.).
    Background,
}

impl QuerySource {
    pub fn is_foreground(self) -> bool {
        matches!(self, Self::MainThread | Self::Agent)
    }
}

/// Configuration for the retry wrapper.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum total retry attempts before giving up.
    pub max_retries: u32,
    /// Base delay for exponential backoff (doubles each attempt).
    pub base_delay: Duration,
    /// Maximum delay cap (backoff won't exceed this).
    pub max_delay: Duration,
    /// Maximum 529 retries for foreground queries.
    pub max_529_retries: u32,
    /// Whether to attempt credential refresh on 401.
    pub allow_credential_refresh: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 10,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(60),
            max_529_retries: 3,
            allow_credential_refresh: true,
        }
    }
}

/// Mutable state tracked across retry attempts.
#[derive(Debug, Clone)]
pub struct RetryState {
    pub attempt: u32,
    pub consecutive_529: u32,
    pub auth_refresh_attempted: bool,
    pub total_wait: Duration,
}

impl Default for RetryState {
    fn default() -> Self {
        Self::new()
    }
}

impl RetryState {
    pub fn new() -> Self {
        Self {
            attempt: 0,
            consecutive_529: 0,
            auth_refresh_attempted: false,
            total_wait: Duration::ZERO,
        }
    }
}

/// Outcome of a single retry decision evaluation.
#[derive(Debug, Clone, PartialEq)]
pub enum RetryDecision {
    /// Retry after the specified delay.
    Retry { delay: Duration },
    /// Give up — the error is terminal or retries are exhausted.
    GiveUp { reason: String },
    /// Retry with escalated output tokens (max_tokens increase).
    SwitchStrategy {
        delay: Duration,
        escalated_max_tokens: u32,
    },
}

/// Error context passed to the retry decision logic.
#[derive(Debug)]
pub struct RetryError {
    pub status: Option<u16>,
    pub message: String,
    pub response_body: Option<String>,
}

impl RetryError {
    pub fn from_anyhow(err: &anyhow::Error) -> Self {
        let msg = err.to_string();
        let status = extract_status_from_error(&msg);
        Self {
            status,
            message: msg,
            response_body: None,
        }
    }

    #[allow(dead_code)] // test helpers for constructing RetryError
    pub fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    #[allow(dead_code)] // test helpers for constructing RetryError
    pub fn with_body(mut self, body: String) -> Self {
        self.response_body = Some(body);
        self
    }
}

/// Evaluate whether an error should be retried given current state and config.
pub fn evaluate_retry(
    error: &RetryError,
    state: &RetryState,
    config: &RetryConfig,
    source: QuerySource,
) -> RetryDecision {
    if state.attempt >= config.max_retries {
        return RetryDecision::GiveUp {
            reason: format!("max retries ({}) exhausted", config.max_retries),
        };
    }

    let kind =
        ApiErrorClassifier::classify(error.status, &error.message, error.response_body.as_deref());

    match &kind {
        // Terminal errors — never retry
        ApiErrorKind::PromptTooLong { .. } => RetryDecision::GiveUp {
            reason: "prompt_too_long is not retryable (requires compaction)".into(),
        },
        ApiErrorKind::InvalidApiKey => RetryDecision::GiveUp {
            reason: "invalid API key — check configuration".into(),
        },
        ApiErrorKind::BudgetExhausted => RetryDecision::GiveUp {
            reason: "budget exhausted — cannot retry".into(),
        },
        ApiErrorKind::ModelNotAvailable { model } => RetryDecision::GiveUp {
            reason: format!("model '{}' not available — cannot retry", model),
        },
        ApiErrorKind::ImageTooLarge { .. } | ApiErrorKind::PdfTooLarge { .. } => {
            RetryDecision::GiveUp {
                reason: "input too large — reduce size before retrying".into(),
            }
        }

        // 429 Rate Limited — always retry, respect retry-after
        ApiErrorKind::RateLimited { retry_after } => {
            let delay = retry_after.unwrap_or_else(|| backoff_delay(state.attempt, config));
            RetryDecision::Retry { delay }
        }

        // 529 Overloaded — only foreground retries
        ApiErrorKind::Overloaded => {
            if !source.is_foreground() {
                return RetryDecision::GiveUp {
                    reason: "background query — skip 529 retry to avoid amplification".into(),
                };
            }
            if state.consecutive_529 >= config.max_529_retries {
                return RetryDecision::GiveUp {
                    reason: format!("max 529 retries ({}) exhausted", config.max_529_retries),
                };
            }
            let delay = backoff_delay(state.attempt, config);
            RetryDecision::Retry { delay }
        }

        // 401 Auth Expired — retry once after refresh
        ApiErrorKind::AuthExpired => {
            if state.auth_refresh_attempted || !config.allow_credential_refresh {
                return RetryDecision::GiveUp {
                    reason: "auth refresh already attempted or disabled".into(),
                };
            }
            RetryDecision::Retry {
                delay: Duration::from_millis(100),
            }
        }

        // MaxOutputTokens — escalate token limit
        ApiErrorKind::MaxOutputTokens => {
            let escalated = compute_escalated_max_tokens(state.attempt);
            let delay = Duration::from_millis(100);
            RetryDecision::SwitchStrategy {
                delay,
                escalated_max_tokens: escalated,
            }
        }

        // Transient network errors — retry with backoff
        ApiErrorKind::ConnectionTimeout | ApiErrorKind::ConnectionReset => {
            let delay = backoff_delay(state.attempt, config);
            RetryDecision::Retry { delay }
        }

        // Stream interrupted — retry with backoff (may resume)
        ApiErrorKind::StreamInterrupted => {
            let delay = backoff_delay(state.attempt, config);
            RetryDecision::Retry { delay }
        }

        // Unknown errors — retry with backoff (may be transient)
        ApiErrorKind::Unknown(_) => {
            let delay = backoff_delay(state.attempt, config);
            RetryDecision::Retry { delay }
        }
    }
}

/// Execute an async operation with retry logic.
///
/// `op` is called repeatedly until it succeeds or retry is exhausted.
/// `on_retry` is an optional callback invoked before each retry sleep (for logging/metrics).
pub async fn with_retry<F, Fut, T, R>(
    config: &RetryConfig,
    source: QuerySource,
    mut op: F,
    mut on_retry: R,
) -> Result<T, anyhow::Error>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, anyhow::Error>>,
    R: FnMut(&RetryState, &RetryDecision, &ApiErrorKind),
{
    let mut state = RetryState::new();

    loop {
        match op().await {
            Ok(val) => return Ok(val),
            Err(e) => {
                let retry_error = RetryError::from_anyhow(&e);
                let kind = ApiErrorClassifier::classify(
                    retry_error.status,
                    &retry_error.message,
                    retry_error.response_body.as_deref(),
                );
                let decision = evaluate_retry(&retry_error, &state, config, source);

                on_retry(&state, &decision, &kind);

                match decision {
                    RetryDecision::GiveUp { .. } => return Err(e),
                    RetryDecision::Retry { delay } => {
                        if matches!(kind, ApiErrorKind::Overloaded) {
                            state.consecutive_529 += 1;
                        } else {
                            state.consecutive_529 = 0;
                        }
                        if matches!(kind, ApiErrorKind::AuthExpired) {
                            state.auth_refresh_attempted = true;
                        }
                        state.attempt += 1;
                        state.total_wait += delay;
                        tokio::time::sleep(delay).await;
                    }
                    RetryDecision::SwitchStrategy { delay, .. } => {
                        state.attempt += 1;
                        state.total_wait += delay;
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }
    }
}

/// Compute exponential backoff delay with jitter.
///
/// Formula: min(base * 2^attempt + jitter, max_delay)
/// Jitter is ±25% of the computed delay to spread out retry storms.
pub fn backoff_delay(attempt: u32, config: &RetryConfig) -> Duration {
    let base_ms = config.base_delay.as_millis() as u64;
    let exp_ms = base_ms.saturating_mul(1u64 << attempt.min(10));
    let max_ms = config.max_delay.as_millis() as u64;
    let capped_ms = exp_ms.min(max_ms);

    let jitter = jitter_offset(capped_ms);
    let final_ms = (capped_ms as i64 + jitter).max(50) as u64;

    Duration::from_millis(final_ms.min(max_ms))
}

/// Generate a pseudo-random jitter offset (±25% of the base value).
/// Uses a simple hash-based approach to avoid pulling in a full RNG crate.
fn jitter_offset(base_ms: u64) -> i64 {
    let quarter = (base_ms / 4) as i64;
    if quarter == 0 {
        return 0;
    }
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as i64;
    (seed % (2 * quarter + 1)) - quarter
}

/// Compute escalated max_tokens for output token recovery.
///
/// Escalation tiers: 8192 → 16384 → 32768
fn compute_escalated_max_tokens(attempt: u32) -> u32 {
    match attempt {
        0 => 8192,
        1 => 16_384,
        _ => 32_768,
    }
}

/// Try to extract an HTTP status code from an error message string.
fn extract_status_from_error(msg: &str) -> Option<u16> {
    let patterns = ["status: ", "status=", "HTTP ", "http "];
    for pat in &patterns {
        if let Some(pos) = msg.find(pat) {
            let after = &msg[pos + pat.len()..];
            let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(code) = num_str.parse::<u16>() {
                if (100..=599).contains(&code) {
                    return Some(code);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> RetryConfig {
        RetryConfig::default()
    }

    fn state_at_attempt(attempt: u32) -> RetryState {
        let mut s = RetryState::new();
        s.attempt = attempt;
        s
    }

    // ── evaluate_retry tests ─────────────────────────────────────────

    #[test]
    fn rate_limited_always_retries() {
        let err = RetryError {
            status: Some(429),
            message: "Too many requests".into(),
            response_body: None,
        };
        let decision = evaluate_retry(
            &err,
            &RetryState::new(),
            &default_config(),
            QuerySource::MainThread,
        );
        assert!(matches!(decision, RetryDecision::Retry { .. }));
    }

    #[test]
    fn rate_limited_respects_retry_after() {
        let err = RetryError {
            status: Some(429),
            message: "Rate limited. Retry-After: 45".into(),
            response_body: None,
        };
        let decision = evaluate_retry(
            &err,
            &RetryState::new(),
            &default_config(),
            QuerySource::MainThread,
        );
        match decision {
            RetryDecision::Retry { delay } => {
                assert_eq!(delay, Duration::from_secs(45));
            }
            _ => panic!("expected Retry"),
        }
    }

    #[test]
    fn overloaded_retries_foreground() {
        let err = RetryError {
            status: Some(529),
            message: "overloaded".into(),
            response_body: None,
        };
        let decision = evaluate_retry(
            &err,
            &RetryState::new(),
            &default_config(),
            QuerySource::MainThread,
        );
        assert!(matches!(decision, RetryDecision::Retry { .. }));
    }

    #[test]
    fn overloaded_gives_up_background() {
        let err = RetryError {
            status: Some(529),
            message: "overloaded".into(),
            response_body: None,
        };
        let decision = evaluate_retry(
            &err,
            &RetryState::new(),
            &default_config(),
            QuerySource::Background,
        );
        assert!(matches!(decision, RetryDecision::GiveUp { .. }));
    }

    #[test]
    fn overloaded_gives_up_after_max_529_retries() {
        let err = RetryError {
            status: Some(529),
            message: "overloaded".into(),
            response_body: None,
        };
        let mut state = RetryState::new();
        state.consecutive_529 = 3; // default max
        let decision = evaluate_retry(&err, &state, &default_config(), QuerySource::MainThread);
        assert!(matches!(decision, RetryDecision::GiveUp { .. }));
    }

    #[test]
    fn prompt_too_long_never_retries() {
        let err = RetryError {
            status: Some(400),
            message: "prompt_too_long: 150000 > 128000".into(),
            response_body: None,
        };
        let decision = evaluate_retry(
            &err,
            &RetryState::new(),
            &default_config(),
            QuerySource::MainThread,
        );
        assert!(matches!(decision, RetryDecision::GiveUp { .. }));
    }

    #[test]
    fn invalid_api_key_never_retries() {
        let err = RetryError {
            status: Some(401),
            message: "Invalid API key provided".into(),
            response_body: None,
        };
        let decision = evaluate_retry(
            &err,
            &RetryState::new(),
            &default_config(),
            QuerySource::MainThread,
        );
        assert!(matches!(decision, RetryDecision::GiveUp { .. }));
    }

    #[test]
    fn budget_exhausted_never_retries() {
        let err = RetryError {
            status: None,
            message: "Token budget exhausted".into(),
            response_body: None,
        };
        let decision = evaluate_retry(
            &err,
            &RetryState::new(),
            &default_config(),
            QuerySource::MainThread,
        );
        assert!(matches!(decision, RetryDecision::GiveUp { .. }));
    }

    #[test]
    fn auth_expired_retries_once() {
        let err = RetryError {
            status: Some(401),
            message: "Unauthorized".into(),
            response_body: None,
        };
        let state = RetryState::new();
        let decision = evaluate_retry(&err, &state, &default_config(), QuerySource::MainThread);
        assert!(matches!(decision, RetryDecision::Retry { .. }));
    }

    #[test]
    fn auth_expired_gives_up_after_refresh() {
        let err = RetryError {
            status: Some(401),
            message: "Unauthorized".into(),
            response_body: None,
        };
        let mut state = RetryState::new();
        state.auth_refresh_attempted = true;
        let decision = evaluate_retry(&err, &state, &default_config(), QuerySource::MainThread);
        assert!(matches!(decision, RetryDecision::GiveUp { .. }));
    }

    #[test]
    fn max_output_tokens_returns_switch_strategy() {
        let err = RetryError {
            status: None,
            message: "max_tokens output limit reached".into(),
            response_body: None,
        };
        let decision = evaluate_retry(
            &err,
            &RetryState::new(),
            &default_config(),
            QuerySource::MainThread,
        );
        match decision {
            RetryDecision::SwitchStrategy {
                escalated_max_tokens,
                ..
            } => {
                assert_eq!(escalated_max_tokens, 8192);
            }
            _ => panic!("expected SwitchStrategy"),
        }
    }

    #[test]
    fn max_output_tokens_escalates_per_attempt() {
        let err = RetryError {
            status: None,
            message: "max_tokens output limit reached".into(),
            response_body: None,
        };
        let state1 = state_at_attempt(1);
        let decision = evaluate_retry(&err, &state1, &default_config(), QuerySource::MainThread);
        match decision {
            RetryDecision::SwitchStrategy {
                escalated_max_tokens,
                ..
            } => {
                assert_eq!(escalated_max_tokens, 16_384);
            }
            _ => panic!("expected SwitchStrategy"),
        }

        let state2 = state_at_attempt(2);
        let decision = evaluate_retry(&err, &state2, &default_config(), QuerySource::MainThread);
        match decision {
            RetryDecision::SwitchStrategy {
                escalated_max_tokens,
                ..
            } => {
                assert_eq!(escalated_max_tokens, 32_768);
            }
            _ => panic!("expected SwitchStrategy"),
        }
    }

    #[test]
    fn connection_timeout_retries_with_backoff() {
        let err = RetryError {
            status: None,
            message: "Request timed out".into(),
            response_body: None,
        };
        let decision = evaluate_retry(
            &err,
            &RetryState::new(),
            &default_config(),
            QuerySource::MainThread,
        );
        assert!(matches!(decision, RetryDecision::Retry { .. }));
    }

    #[test]
    fn connection_reset_retries_with_backoff() {
        let err = RetryError {
            status: None,
            message: "Connection reset by peer".into(),
            response_body: None,
        };
        let decision = evaluate_retry(
            &err,
            &RetryState::new(),
            &default_config(),
            QuerySource::MainThread,
        );
        assert!(matches!(decision, RetryDecision::Retry { .. }));
    }

    #[test]
    fn max_retries_exhausted_gives_up() {
        let err = RetryError {
            status: Some(429),
            message: "rate limited".into(),
            response_body: None,
        };
        let state = state_at_attempt(10); // default max is 10
        let decision = evaluate_retry(&err, &state, &default_config(), QuerySource::MainThread);
        assert!(matches!(decision, RetryDecision::GiveUp { .. }));
    }

    #[test]
    fn unknown_error_retries() {
        let err = RetryError {
            status: Some(500),
            message: "Internal server error".into(),
            response_body: None,
        };
        let decision = evaluate_retry(
            &err,
            &RetryState::new(),
            &default_config(),
            QuerySource::MainThread,
        );
        assert!(matches!(decision, RetryDecision::Retry { .. }));
    }

    // ── backoff_delay tests ──────────────────────────────────────────

    #[test]
    fn backoff_increases_exponentially() {
        let config = RetryConfig {
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(60),
            ..Default::default()
        };

        let d0 = backoff_delay(0, &config);
        let d1 = backoff_delay(1, &config);
        let d2 = backoff_delay(2, &config);

        // With jitter the values won't be exact, but d1 should be roughly 2x d0
        assert!(d1.as_millis() > d0.as_millis(), "d1 should be > d0");
        assert!(d2.as_millis() > d1.as_millis(), "d2 should be > d1");
    }

    #[test]
    fn backoff_respects_max_delay() {
        let config = RetryConfig {
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(5),
            ..Default::default()
        };

        let d10 = backoff_delay(10, &config);
        assert!(d10 <= Duration::from_secs(5));
    }

    // ── compute_escalated_max_tokens tests ───────────────────────────

    #[test]
    fn escalation_tiers() {
        assert_eq!(compute_escalated_max_tokens(0), 8192);
        assert_eq!(compute_escalated_max_tokens(1), 16_384);
        assert_eq!(compute_escalated_max_tokens(2), 32_768);
        assert_eq!(compute_escalated_max_tokens(5), 32_768);
    }

    // ── extract_status_from_error tests ──────────────────────────────

    #[test]
    fn extract_status_from_various_formats() {
        assert_eq!(
            extract_status_from_error("status: 429 Too Many Requests"),
            Some(429)
        );
        assert_eq!(
            extract_status_from_error("HTTP 503 Service Unavailable"),
            Some(503)
        );
        assert_eq!(extract_status_from_error("status=401"), Some(401));
        assert_eq!(extract_status_from_error("no status here"), None);
    }

    // ── with_retry integration test ──────────────────────────────────

    #[tokio::test]
    async fn with_retry_succeeds_on_first_try() {
        let config = default_config();
        let result = with_retry(
            &config,
            QuerySource::MainThread,
            || async { Ok::<_, anyhow::Error>(42) },
            |_, _, _| {},
        )
        .await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn with_retry_succeeds_after_transient_failure() {
        let config = RetryConfig {
            base_delay: Duration::from_millis(10),
            ..Default::default()
        };
        let attempt = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let attempt_clone = attempt.clone();

        let result = with_retry(
            &config,
            QuerySource::MainThread,
            move || {
                let a = attempt_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                async move {
                    if a < 2 {
                        Err(anyhow::anyhow!("Connection reset by peer"))
                    } else {
                        Ok(99)
                    }
                }
            },
            |_, _, _| {},
        )
        .await;

        assert_eq!(result.unwrap(), 99);
        assert_eq!(attempt.load(std::sync::atomic::Ordering::Relaxed), 3);
    }

    #[tokio::test]
    async fn with_retry_gives_up_on_terminal_error() {
        let config = RetryConfig {
            base_delay: Duration::from_millis(10),
            ..Default::default()
        };

        let result: Result<i32, _> = with_retry(
            &config,
            QuerySource::MainThread,
            || async { Err::<i32, _>(anyhow::anyhow!("Invalid API key provided")) },
            |_, _, _| {},
        )
        .await;

        assert!(result.is_err());
    }

    // ── QuerySource tests ────────────────────────────────────────────

    #[test]
    fn query_source_foreground_classification() {
        assert!(QuerySource::MainThread.is_foreground());
        assert!(QuerySource::Agent.is_foreground());
        assert!(!QuerySource::Background.is_foreground());
    }
}
