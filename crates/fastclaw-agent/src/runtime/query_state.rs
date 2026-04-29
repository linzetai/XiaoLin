//! Unified query-loop state and transition types.
//!
//! Replaces the scattered state variables in `execute_stream` with a single
//! [`QueryLoopState`] struct, and provides type-safe [`LoopTransition`] to
//! drive loop flow instead of implicit `break`/`return`.

use super::stream_engine::ToolCallTrace;

/// Unified mutable state tracked across iterations of the agent query loop.
///
/// Subsumes the old `LoopState` and the scattered token-accumulation variables
/// (`acc_prompt_tokens`, `acc_completion_tokens`, `last_estimated_tokens`).
#[allow(dead_code)]
pub(crate) struct QueryLoopState {
    // ── Iteration control ────────────────────────────────────────────
    pub iteration: u32,
    pub max_iterations: u32,
    pub turn_count: u32,

    // ── Tool & error tracking ────────────────────────────────────────
    pub total_tool_calls: u32,
    pub consecutive_errors: u32,
    pub failure_streak_traces: Vec<ToolCallTrace>,
    pub self_iter_recovery_used: u32,
    pub error_limit_reached: bool,

    // ── Grace turn: one final LLM call to explain failures ───────────
    pub grace_turn_active: bool,
    pub grace_turn_used: bool,

    // ── Recovery tracking (used by 6C-02 max_output_tokens, 6C-04 reactive compact)
    pub max_output_tokens_recovery_count: u32,
    pub has_attempted_reactive_compact: bool,

    // ── Token accumulation (streaming path) ──────────────────────────
    pub acc_prompt_tokens: u32,
    pub acc_completion_tokens: u32,
    pub last_estimated_tokens: usize,
}

/// The outcome of one loop iteration: continue or terminate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LoopTransition {
    Continue(ContinueReason),
    Terminal(TerminalReason),
}

/// Why the loop should continue for another iteration.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum ContinueReason {
    /// LLM returned tool_calls to execute.
    ToolUse,
    /// Recovered from `prompt_too_long` via reactive compaction.
    ReactiveCompactRecovery,
    /// Recovered from `max_output_tokens` by escalating token limit.
    MaxOutputTokensRecovery,
    /// A stop-hook evaluated to "should continue".
    StopHookContinuation,
    /// SSE stream was interrupted; resuming with partial context.
    StreamResume,
}

/// Why the loop should terminate.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum TerminalReason {
    /// LLM finished with no tool_calls — natural end of turn.
    EndTurn,
    /// Context tokens >= 95% of context_window and auto_compact is off.
    BlockingLimit,
    /// Reached `max_tool_calls_per_turn`.
    MaxIterations,
    /// User or system requested abort.
    Aborted,
    /// Too many consecutive tool errors (after grace turn exhausted).
    ConsecutiveErrors,
    /// API cost budget exhausted.
    BudgetExhausted,
    /// Detected diminishing returns in tool-call loop.
    DiminishingReturns,
}

impl QueryLoopState {
    pub fn new(max_iterations: u32) -> Self {
        Self {
            iteration: 0,
            max_iterations,
            turn_count: 0,

            total_tool_calls: 0,
            consecutive_errors: 0,
            failure_streak_traces: Vec::new(),
            self_iter_recovery_used: 0,
            error_limit_reached: false,

            grace_turn_active: false,
            grace_turn_used: false,

            max_output_tokens_recovery_count: 0,
            has_attempted_reactive_compact: false,

            acc_prompt_tokens: 0,
            acc_completion_tokens: 0,
            last_estimated_tokens: 0,
        }
    }

    /// Advance to the next iteration, handling grace-turn flag reset.
    pub fn begin_iteration(&mut self) {
        if self.grace_turn_active {
            self.grace_turn_active = false;
        }
        self.iteration += 1;
    }

    pub fn record_tool_error(&mut self, tool_name: &str, error_output: &str) {
        self.consecutive_errors += 1;
        self.failure_streak_traces.push(ToolCallTrace {
            tool_name: tool_name.to_string(),
            success: false,
            latency_ms: 0,
            error: Some(error_output.to_string()),
        });
    }

    pub fn clear_error_streak(&mut self) {
        self.consecutive_errors = 0;
        self.failure_streak_traces.clear();
    }

    pub fn format_failure_summary(&self) -> String {
        if self.failure_streak_traces.is_empty() {
            return String::new();
        }
        let mut lines = Vec::new();
        for (i, trace) in self.failure_streak_traces.iter().enumerate() {
            let err_msg = trace.error.as_deref().unwrap_or("unknown error");
            let truncated = if err_msg.len() > 200 {
                let end = err_msg.floor_char_boundary(200);
                format!("{}...", &err_msg[..end])
            } else {
                err_msg.to_string()
            };
            lines.push(format!("  {}. `{}`: {}", i + 1, trace.tool_name, truncated));
        }
        lines.join("\n")
    }

    /// Build a Usage summary from accumulated token counts. Returns `None` if
    /// no tokens have been tracked (e.g. non-streaming path or first iteration).
    pub fn build_usage(&self) -> Option<fastclaw_core::types::Usage> {
        let total = self.acc_prompt_tokens + self.acc_completion_tokens;
        if total > 0 {
            Some(fastclaw_core::types::Usage {
                prompt_tokens: self.acc_prompt_tokens,
                completion_tokens: self.acc_completion_tokens,
                total_tokens: total,
            })
        } else {
            None
        }
    }

    // ── Transition determination ─────────────────────────────────────

    /// Pre-iteration check: should the loop stop before making an LLM call?
    pub fn check_pre_iteration(&self) -> Option<LoopTransition> {
        if self.error_limit_reached {
            return Some(LoopTransition::Terminal(TerminalReason::ConsecutiveErrors));
        }
        None
    }

    /// Check if estimated tokens have reached the blocking limit (95% of
    /// context window).  When auto-compact is enabled the pipeline will handle
    /// reduction automatically, so we only block when manual `/compact` is the
    /// only remedy.
    ///
    /// `just_compacted` should be true when a compression step ran earlier in
    /// this iteration and freed tokens — in that case we skip the check to
    /// avoid immediately re-blocking on the freshly compacted messages.
    pub fn check_blocking_limit(
        &self,
        estimated_tokens: usize,
        context_window: u32,
        auto_compact_enabled: bool,
        just_compacted: bool,
    ) -> Option<LoopTransition> {
        if just_compacted || auto_compact_enabled {
            return None;
        }
        let blocking_limit = (context_window as f64 * 0.95) as usize;
        if estimated_tokens >= blocking_limit {
            Some(LoopTransition::Terminal(TerminalReason::BlockingLimit))
        } else {
            None
        }
    }

    /// After the LLM response: should the loop continue or terminate?
    pub fn determine_post_llm_transition(&self, has_tool_calls: bool) -> LoopTransition {
        if !has_tool_calls {
            LoopTransition::Terminal(TerminalReason::EndTurn)
        } else if self.iteration >= self.max_iterations {
            LoopTransition::Terminal(TerminalReason::MaxIterations)
        } else {
            LoopTransition::Continue(ContinueReason::ToolUse)
        }
    }

    /// After processing tool errors: should the loop enter grace mode or stop?
    ///
    /// Returns `None` if error count is below the limit.
    /// Returns `Some(Terminal(ConsecutiveErrors))` when grace is already used.
    /// When grace is available, sets grace flags and resets errors — the caller
    /// should inject a guidance message and `break` to re-enter the outer loop.
    pub fn check_error_limit(&mut self, max_errors: u32) -> Option<LoopTransition> {
        if self.consecutive_errors < max_errors {
            return None;
        }
        if !self.grace_turn_used {
            self.grace_turn_active = true;
            self.grace_turn_used = true;
            let summary = self.format_failure_summary();
            let errors = self.consecutive_errors;
            self.consecutive_errors = 0;
            self.failure_streak_traces.clear();
            tracing::info!(
                consecutive_errors = errors,
                "consecutive error limit reached — entering grace turn"
            );
            // Caller should inject the guidance message using `grace_guidance_message(errors, &summary)`
            // and then break to the outer loop.
            // We store the summary temporarily — the caller reads it before we clear.
            let _ = summary; // consumed by caller via format_failure_summary() before calling this
            None
        } else {
            self.error_limit_reached = true;
            Some(LoopTransition::Terminal(TerminalReason::ConsecutiveErrors))
        }
    }
}

impl std::fmt::Display for TerminalReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EndTurn => write!(f, "end_turn"),
            Self::BlockingLimit => write!(f, "blocking_limit"),
            Self::MaxIterations => write!(f, "max_iterations"),
            Self::Aborted => write!(f, "aborted"),
            Self::ConsecutiveErrors => write!(f, "consecutive_errors"),
            Self::BudgetExhausted => write!(f, "budget_exhausted"),
            Self::DiminishingReturns => write!(f, "diminishing_returns"),
        }
    }
}

impl std::fmt::Display for ContinueReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ToolUse => write!(f, "tool_use"),
            Self::ReactiveCompactRecovery => write!(f, "reactive_compact_recovery"),
            Self::MaxOutputTokensRecovery => write!(f, "max_output_tokens_recovery"),
            Self::StopHookContinuation => write!(f, "stop_hook_continuation"),
            Self::StreamResume => write!(f, "stream_resume"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_has_correct_defaults() {
        let s = QueryLoopState::new(50);
        assert_eq!(s.iteration, 0);
        assert_eq!(s.max_iterations, 50);
        assert_eq!(s.total_tool_calls, 0);
        assert_eq!(s.consecutive_errors, 0);
        assert!(!s.error_limit_reached);
        assert!(!s.grace_turn_active);
        assert!(!s.grace_turn_used);
        assert_eq!(s.max_output_tokens_recovery_count, 0);
        assert_eq!(s.acc_prompt_tokens, 0);
        assert_eq!(s.last_estimated_tokens, 0);
    }

    #[test]
    fn begin_iteration_increments_and_clears_grace() {
        let mut s = QueryLoopState::new(10);
        s.grace_turn_active = true;
        s.begin_iteration();
        assert!(!s.grace_turn_active);
        assert_eq!(s.iteration, 1);

        s.begin_iteration();
        assert_eq!(s.iteration, 2);
    }

    #[test]
    fn record_and_clear_error_streak() {
        let mut s = QueryLoopState::new(10);
        s.record_tool_error("shell", "permission denied");
        s.record_tool_error("shell", "file not found");
        assert_eq!(s.consecutive_errors, 2);
        assert_eq!(s.failure_streak_traces.len(), 2);

        s.clear_error_streak();
        assert_eq!(s.consecutive_errors, 0);
        assert!(s.failure_streak_traces.is_empty());
    }

    #[test]
    fn format_failure_summary_truncates_long_errors() {
        let mut s = QueryLoopState::new(10);
        let long_err = "x".repeat(300);
        s.record_tool_error("shell", &long_err);
        let summary = s.format_failure_summary();
        assert!(summary.contains("..."));
        assert!(summary.len() < 300);
    }

    #[test]
    fn check_pre_iteration_when_error_limit_reached() {
        let mut s = QueryLoopState::new(10);
        assert!(s.check_pre_iteration().is_none());

        s.error_limit_reached = true;
        assert_eq!(
            s.check_pre_iteration(),
            Some(LoopTransition::Terminal(TerminalReason::ConsecutiveErrors))
        );
    }

    #[test]
    fn determine_post_llm_transition_end_turn() {
        let s = QueryLoopState::new(10);
        assert_eq!(
            s.determine_post_llm_transition(false),
            LoopTransition::Terminal(TerminalReason::EndTurn)
        );
    }

    #[test]
    fn determine_post_llm_transition_max_iterations() {
        let mut s = QueryLoopState::new(5);
        s.iteration = 5;
        assert_eq!(
            s.determine_post_llm_transition(true),
            LoopTransition::Terminal(TerminalReason::MaxIterations)
        );
    }

    #[test]
    fn determine_post_llm_transition_tool_use() {
        let mut s = QueryLoopState::new(10);
        s.iteration = 3;
        assert_eq!(
            s.determine_post_llm_transition(true),
            LoopTransition::Continue(ContinueReason::ToolUse)
        );
    }

    #[test]
    fn blocking_limit_triggers_at_95_percent() {
        let s = QueryLoopState::new(10);
        let context_window = 100_000_u32;
        let at_95 = (context_window as f64 * 0.95) as usize; // 95000

        let result = s.check_blocking_limit(at_95, context_window, false, false);
        assert_eq!(
            result,
            Some(LoopTransition::Terminal(TerminalReason::BlockingLimit))
        );

        let result_above = s.check_blocking_limit(at_95 + 1000, context_window, false, false);
        assert_eq!(
            result_above,
            Some(LoopTransition::Terminal(TerminalReason::BlockingLimit))
        );
    }

    #[test]
    fn blocking_limit_skipped_when_below_threshold() {
        let s = QueryLoopState::new(10);
        let result = s.check_blocking_limit(90_000, 100_000, false, false);
        assert!(result.is_none());
    }

    #[test]
    fn blocking_limit_skipped_after_compact() {
        let s = QueryLoopState::new(10);
        let result = s.check_blocking_limit(96_000, 100_000, false, true);
        assert!(result.is_none(), "should not block after compaction just ran");
    }

    #[test]
    fn blocking_limit_skipped_when_auto_compact_enabled() {
        let s = QueryLoopState::new(10);
        let result = s.check_blocking_limit(96_000, 100_000, true, false);
        assert!(result.is_none(), "should not block when auto_compact handles it");
    }

    #[test]
    fn check_error_limit_below_threshold() {
        let mut s = QueryLoopState::new(10);
        s.consecutive_errors = 2;
        assert!(s.check_error_limit(5).is_none());
    }

    #[test]
    fn check_error_limit_grace_turn() {
        let mut s = QueryLoopState::new(10);
        s.consecutive_errors = 3;
        s.record_tool_error("x", "e1");
        s.record_tool_error("y", "e2");
        // errors are now 5 (2 from record + 3 manual set)
        s.consecutive_errors = 5;

        let result = s.check_error_limit(5);
        assert!(result.is_none()); // grace turn activated, not terminal
        assert!(s.grace_turn_active);
        assert!(s.grace_turn_used);
        assert_eq!(s.consecutive_errors, 0);
    }

    #[test]
    fn check_error_limit_after_grace_exhausted() {
        let mut s = QueryLoopState::new(10);
        s.grace_turn_used = true;
        s.consecutive_errors = 5;

        let result = s.check_error_limit(5);
        assert_eq!(
            result,
            Some(LoopTransition::Terminal(TerminalReason::ConsecutiveErrors))
        );
        assert!(s.error_limit_reached);
    }

    #[test]
    fn terminal_reason_display() {
        assert_eq!(TerminalReason::EndTurn.to_string(), "end_turn");
        assert_eq!(TerminalReason::MaxIterations.to_string(), "max_iterations");
        assert_eq!(TerminalReason::ConsecutiveErrors.to_string(), "consecutive_errors");
    }

    #[test]
    fn continue_reason_display() {
        assert_eq!(ContinueReason::ToolUse.to_string(), "tool_use");
        assert_eq!(ContinueReason::StreamResume.to_string(), "stream_resume");
    }
}
