//! Unified query-loop state and transition types.
//!
//! Replaces the scattered state variables in `execute_stream` with a single
//! [`QueryLoopState`] struct, and provides type-safe [`LoopTransition`] to
//! drive loop flow instead of implicit `break`/`return`.

use std::collections::HashMap;

use super::post_compact_restore::RestorationState;
use super::stream_engine::ToolCallTrace;

/// Max attempts to recover from `max_output_tokens` (finish_reason=length)
/// by escalating the token limit before giving up.
pub(crate) const MAX_OUTPUT_TOKENS_RECOVERY_LIMIT: u32 = 3;

/// A boundary marker for the start of an agent iteration.
///
/// - `.0`: `messages.len()` at the time the boundary was pushed. May become
///   stale when later compaction steps delete messages — treat as a hint, not
///   as an authoritative index.
/// - `.1`: Wall-clock time when the boundary was pushed. Used by
///   `time_based_microcompact` to decide which iterations are outside the
///   cache window.
/// - `.2`: `tool_call_id` of the most recent Tool message at push time, if
///   any. Used by `compute_protected_indices` to re-resolve the boundary's
///   position in the current (possibly-compacted) Vec.
pub(crate) type IterationMsgBoundary = (usize, std::time::Instant, Option<String>);

/// When the same tool+args pair is called this many times, inject a
/// "change your approach" nudge into the system message.
const TOOL_REPEAT_WARN_THRESHOLD: u32 = 3;

/// Hard limit: force-terminate the tool loop when the same tool+args
/// pair is called this many times.
const TOOL_REPEAT_HARD_LIMIT: u32 = 5;

/// What action the runtime should take after recording a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolRepetitionAction {
    /// No repetition issue detected.
    None,
    /// Threshold reached — inject a guidance nudge and continue.
    Warn,
    /// Hard limit reached — terminate the tool loop immediately.
    ForceStop,
}

/// Token limit used when escalating after a max_output_tokens truncation.
/// 16 384 is a safe upper bound that most models support.
pub(crate) const ESCALATED_MAX_TOKENS: u32 = 16_384;

/// Check if an error string indicates a prompt_too_long / context_length_exceeded
/// condition from the LLM API.
pub(crate) fn is_prompt_too_long_error(err: &str) -> bool {
    let lower = err.to_lowercase();
    lower.contains("prompt_too_long")
        || lower.contains("context_length_exceeded")
        || lower.contains("maximum context length")
        || lower.contains("too many tokens")
}

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
    pub max_output_recovery_exhausted: bool,

    // ── Token accumulation (streaming path) ──────────────────────────
    pub acc_prompt_tokens: u32,
    pub acc_completion_tokens: u32,
    pub last_estimated_tokens: usize,

    // ── Warning deduplication ────────────────────────────────────────
    pub compact_warning_sent: bool,

    // ── Time-based microcompact tracking (6E-01) ─────────────────────
    /// Message count at the start of each iteration, paired with wall-clock time
    /// and an optional `tool_call_id` anchor for stable re-resolution after
    /// later compaction steps delete messages (BUG-023).
    ///
    /// The anchor is the `tool_call_id` of the most recent Tool message at the
    /// time the boundary was pushed. When compaction deletes earlier messages,
    /// `compute_protected_indices` looks the anchor up in the current Vec to
    /// re-resolve the boundary's true position, falling back to the (clamped)
    /// stored index if the anchor has also been evicted.
    pub iteration_msg_boundaries: Vec<IterationMsgBoundary>,

    // ── Tool repetition detection ────────────────────────────────────
    /// Per logical tool target call count within this query (path for read_file, etc.).
    tool_call_repetition_counts: HashMap<String, u32>,
    /// Tracks the highest escalation level we've already applied,
    /// so each level fires at most once.
    repetition_escalation: u32,
    /// Cumulative warn / force-stop triggers for this query (metrics snapshot).
    repetition_warn_count: u32,
    repetition_force_stop_count: u32,

    // ── Auto-fix loop state ───────────────────────────────────────────
    pub autofix: crate::autofix::AutoFixState,

    // ── Post-compact restoration state ────────────────────────────────
    /// Tracks recently read files, invoked skills, and plan content
    /// for restoration after context compaction.
    pub restoration_state: RestorationState,

    // ── Session memory (incremental) ──────────────────────────────────
    /// Accumulated session memory, updated incrementally on each compact.
    pub session_memory: Option<super::session_memory::SessionMemory>,

    // ── External goal cancellation ──────────────────────────────────
    /// When true, the loop should stop after the next LLM call completes
    /// (regardless of whether stop hooks say to continue).
    pub force_stop_after_next: bool,

    /// Consecutive agent iterations that invoked tools but made no write/progress.
    pub iterations_without_progress: u32,
    /// Whether the no-progress stall warning has been injected this turn.
    pub no_progress_stall_warn_sent: bool,
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
            max_output_recovery_exhausted: false,

            acc_prompt_tokens: 0,
            acc_completion_tokens: 0,
            last_estimated_tokens: 0,

            compact_warning_sent: false,

            iteration_msg_boundaries: Vec::new(),

            tool_call_repetition_counts: HashMap::new(),
            repetition_escalation: 0,
            repetition_warn_count: 0,
            repetition_force_stop_count: 0,

            autofix: crate::autofix::AutoFixState::default(),

            restoration_state: RestorationState::new(),

            session_memory: None,

            force_stop_after_next: false,
            iterations_without_progress: 0,
            no_progress_stall_warn_sent: false,
        }
    }

    /// Advance to the next iteration, handling grace-turn flag reset.
    pub fn begin_iteration(&mut self) {
        if self.grace_turn_active {
            self.grace_turn_active = false;
        }
        self.iteration += 1;
    }

    /// Record a tool call with its arguments and return what action the
    /// runtime should take.
    ///
    /// - `Warn` at [`TOOL_REPEAT_WARN_THRESHOLD`] repeated calls to the same target.
    /// - `ForceStop` at [`TOOL_REPEAT_HARD_LIMIT`] repeated calls.
    /// - `read_file` counts by file path (offset/limit variants are the same target).
    pub fn record_tool_call(&mut self, tool_name: &str, arguments: &str) -> ToolRepetitionAction {
        let key = super::tool_executor::tool_repetition_key(tool_name, arguments);
        let count = self.tool_call_repetition_counts.entry(key).or_insert(0);
        *count += 1;

        if *count >= TOOL_REPEAT_HARD_LIMIT && self.repetition_escalation < 2 {
            self.repetition_escalation = 2;
            self.repetition_force_stop_count += 1;
            return ToolRepetitionAction::ForceStop;
        }
        if *count >= TOOL_REPEAT_WARN_THRESHOLD && self.repetition_escalation < 1 {
            self.repetition_escalation = 1;
            self.repetition_warn_count += 1;
            return ToolRepetitionAction::Warn;
        }
        ToolRepetitionAction::None
    }

    /// Read-only snapshot of repetition-detection triggers in this query.
    pub fn repetition_stats(&self) -> (u32, u32) {
        (self.repetition_warn_count, self.repetition_force_stop_count)
    }

    /// Build a guidance message when exact tool call repetition is detected.
    ///
    /// The message adapts based on escalation level:
    /// - Level 1 (Warn): suggests changing approach.
    /// - Level 2 (ForceStop): explains the loop is being terminated.
    pub fn build_repetition_nudge(&self, force_stop: bool) -> Option<String> {
        let repeated: Vec<_> = self
            .tool_call_repetition_counts
            .iter()
            .filter(|(_, &count)| count >= TOOL_REPEAT_WARN_THRESHOLD)
            .map(|(key, &count)| (format_repetition_key(key), count))
            .collect();
        if repeated.is_empty() {
            return None;
        }

        let mut msg =
            String::from("[Tool loop detected] You have repeatedly called the same tool target:\n");
        for (label, count) in &repeated {
            msg.push_str(&format!("  - {label}: {count} calls\n"));
        }

        if force_stop {
            msg.push_str(
                "\nThe tool loop has been TERMINATED because you exceeded the repetition hard limit.\n\
                 You MUST stop calling these tools and instead:\n\
                 1. Summarize what you were trying to do and why it failed.\n\
                 2. Explain the situation to the user clearly.\n\
                 3. Suggest alternative approaches the user could try.\n\
                 Do NOT attempt any more tool calls for this failed approach."
            );
        } else {
            msg.push_str(
                "\nYou appear to be stuck in a loop. STOP and change your approach:\n\
                 - If a file was not found, use `list_directory` or `glob` with a partial name pattern to discover the correct path.\n\
                 - If permission was denied, ask the user to adjust the working directory or execution mode.\n\
                 - If a command keeps failing, explain the issue to the user instead of retrying.\n\
                 - Do NOT repeat the exact same tool call. Try a fundamentally different strategy.\n\
                 WARNING: If you continue repeating, the loop will be force-terminated."
            );
        }
        Some(msg)
    }

    /// Consecutive tool-only iterations without write/shell/subagent progress.
    pub fn record_iteration_progress(&mut self, had_tool_calls: bool, had_progress: bool) {
        if had_progress {
            self.iterations_without_progress = 0;
            self.no_progress_stall_warn_sent = false;
        } else if had_tool_calls {
            self.iterations_without_progress = self.iterations_without_progress.saturating_add(1);
        }
    }

    /// Check read-only stall and return guidance or force-stop flag.
    pub fn check_no_progress_stall(&mut self) -> NoProgressStallAction {
        const WARN_ITERATIONS: u32 = 12;
        const HARD_ITERATIONS: u32 = 25;

        if self.iterations_without_progress >= HARD_ITERATIONS {
            return NoProgressStallAction::ForceStop;
        }
        if self.iterations_without_progress >= WARN_ITERATIONS && !self.no_progress_stall_warn_sent
        {
            self.no_progress_stall_warn_sent = true;
            return NoProgressStallAction::Warn;
        }
        NoProgressStallAction::None
    }

    pub fn build_no_progress_stall_nudge(force_stop: bool) -> String {
        if force_stop {
            "[Read-only stall] You have spent many iterations reading/searching without \
             making changes (edit_file, write_file, shell_exec, etc.).\n\
             The loop is being terminated. Summarize what you learned, explain what is \
             blocking progress, and tell the user what you recommend next. \
             Do NOT call more read/search tools."
                .to_string()
        } else {
            "[Read-only stall warning] You have had many consecutive tool rounds with only \
             reads/searches and no edits or commands.\n\
             STOP re-reading the same files. Use the content already in context, make your \
             best edit with `edit_file`, or explain to the user what you are stuck on."
                .to_string()
        }
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
    pub fn build_usage(&self) -> Option<xiaolin_core::types::Usage> {
        let total = self.acc_prompt_tokens + self.acc_completion_tokens;
        if total > 0 {
            Some(xiaolin_core::types::Usage {
                prompt_tokens: self.acc_prompt_tokens,
                completion_tokens: self.acc_completion_tokens,
                total_tokens: total,
                ..Default::default()
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
        // Use Claude-Code style blocking limit: effective_window - 3K buffer
        // This is roughly 98% of context window (for 200K: 200K - 20K - 3K = 177K)
        let blocking_limit = super::context_compressor::compute_blocking_limit(context_window);
        if estimated_tokens >= blocking_limit {
            Some(LoopTransition::Terminal(TerminalReason::BlockingLimit))
        } else {
            None
        }
    }

    /// Attempt recovery from a `max_output_tokens` (finish_reason=length) truncation.
    ///
    /// Returns `Some(Continue(MaxOutputTokensRecovery))` if under the retry
    /// limit, or `None` if the limit is exhausted (caller should proceed
    /// normally, which typically means end-turn).
    pub fn try_max_output_tokens_recovery(&mut self) -> Option<LoopTransition> {
        if self.max_output_tokens_recovery_count < MAX_OUTPUT_TOKENS_RECOVERY_LIMIT {
            self.max_output_tokens_recovery_count += 1;
            Some(LoopTransition::Continue(
                ContinueReason::MaxOutputTokensRecovery,
            ))
        } else {
            self.max_output_recovery_exhausted = true;
            None
        }
    }

    /// After the LLM response: should the loop continue or terminate?
    /// When `max_iterations == 0` the iteration cap is disabled (unlimited).
    pub fn determine_post_llm_transition(&self, has_tool_calls: bool) -> LoopTransition {
        if !has_tool_calls {
            LoopTransition::Terminal(TerminalReason::EndTurn)
        } else if self.max_iterations > 0 && self.iteration >= self.max_iterations {
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

fn format_repetition_key(key: &str) -> String {
    if let Some(path) = key.strip_prefix("read_file\0path:") {
        return format!("read_file → {path}");
    }
    if let Some((tool, rest)) = key.split_once('\0') {
        return format!("{tool} ({rest})");
    }
    key.to_string()
}

/// Action to take when the agent stalls in read-only loops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoProgressStallAction {
    None,
    Warn,
    ForceStop,
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
    fn determine_post_llm_transition_unlimited_when_zero() {
        let mut s = QueryLoopState::new(0);
        s.iteration = 9999;
        assert_eq!(
            s.determine_post_llm_transition(true),
            LoopTransition::Continue(ContinueReason::ToolUse),
            "max_iterations=0 means unlimited"
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
    fn compact_warning_sent_defaults_false() {
        let s = QueryLoopState::new(10);
        assert!(!s.compact_warning_sent);
    }

    #[test]
    fn compact_warning_sent_flag_prevents_resend() {
        let mut s = QueryLoopState::new(10);
        assert!(!s.compact_warning_sent, "should start as false");
        s.compact_warning_sent = true;
        assert!(s.compact_warning_sent, "should stay true after setting");
    }

    #[test]
    fn is_prompt_too_long_detects_variants() {
        assert!(is_prompt_too_long_error(
            "Error: prompt_too_long — reduce input"
        ));
        assert!(is_prompt_too_long_error(
            "context_length_exceeded: 130000 > 128000"
        ));
        assert!(is_prompt_too_long_error(
            "This model's maximum context length is 128000"
        ));
        assert!(is_prompt_too_long_error("Too many tokens in the request"));
    }

    #[test]
    fn is_prompt_too_long_rejects_unrelated() {
        assert!(!is_prompt_too_long_error("rate_limit_exceeded"));
        assert!(!is_prompt_too_long_error("authentication_error"));
        assert!(!is_prompt_too_long_error("network timeout"));
        assert!(!is_prompt_too_long_error(""));
    }

    #[test]
    fn is_prompt_too_long_case_insensitive() {
        assert!(is_prompt_too_long_error("PROMPT_TOO_LONG"));
        assert!(is_prompt_too_long_error("Context_Length_Exceeded"));
    }

    #[test]
    fn max_output_tokens_recovery_succeeds_under_limit() {
        let mut s = QueryLoopState::new(10);
        for i in 0..MAX_OUTPUT_TOKENS_RECOVERY_LIMIT {
            let result = s.try_max_output_tokens_recovery();
            assert_eq!(
                result,
                Some(LoopTransition::Continue(
                    ContinueReason::MaxOutputTokensRecovery
                )),
                "attempt {} should succeed",
                i + 1
            );
            assert_eq!(s.max_output_tokens_recovery_count, i + 1);
        }
    }

    #[test]
    fn max_output_tokens_recovery_fails_at_limit() {
        let mut s = QueryLoopState::new(10);
        s.max_output_tokens_recovery_count = MAX_OUTPUT_TOKENS_RECOVERY_LIMIT;
        let result = s.try_max_output_tokens_recovery();
        assert!(result.is_none(), "should return None when limit exhausted");
        assert!(
            s.max_output_recovery_exhausted,
            "should set exhausted flag when limit reached"
        );
    }

    #[test]
    fn max_output_tokens_recovery_exhaustion_sequence() {
        let mut s = QueryLoopState::new(10);
        assert!(
            !s.max_output_recovery_exhausted,
            "should start as not exhausted"
        );
        for _ in 0..MAX_OUTPUT_TOKENS_RECOVERY_LIMIT {
            assert!(s.try_max_output_tokens_recovery().is_some());
            assert!(
                !s.max_output_recovery_exhausted,
                "should not be exhausted while under limit"
            );
        }
        assert!(
            s.try_max_output_tokens_recovery().is_none(),
            "4th attempt should fail after 3 successes"
        );
        assert!(
            s.max_output_recovery_exhausted,
            "should be exhausted after all retries consumed"
        );
    }

    #[test]
    fn blocking_limit_triggers_at_effective_minus_buffer() {
        let s = QueryLoopState::new(10);
        let context_window = 100_000_u32;
        // New formula: effective = 100K - 20K = 80K, blocking = 80K - 3K = 77K
        let blocking_limit =
            super::super::context_compressor::compute_blocking_limit(context_window);

        // At the blocking limit itself
        let result = s.check_blocking_limit(blocking_limit, context_window, false, false);
        assert_eq!(
            result,
            Some(LoopTransition::Terminal(TerminalReason::BlockingLimit))
        );

        // Above the blocking limit
        let result_above =
            s.check_blocking_limit(blocking_limit + 1000, context_window, false, false);
        assert_eq!(
            result_above,
            Some(LoopTransition::Terminal(TerminalReason::BlockingLimit))
        );
    }

    #[test]
    fn blocking_limit_skipped_when_below_threshold() {
        let s = QueryLoopState::new(10);
        let context_window = 100_000_u32;
        // blocking_limit = 77K, so 50K should be fine
        let result = s.check_blocking_limit(50_000, context_window, false, false);
        assert!(result.is_none());
    }

    #[test]
    fn blocking_limit_skipped_after_compact() {
        let s = QueryLoopState::new(10);
        let context_window = 100_000_u32;
        // Even above the blocking limit, should skip when just_compacted
        let result =
            s.check_blocking_limit(context_window as usize - 1, context_window, false, true);
        assert!(
            result.is_none(),
            "should not block after compaction just ran"
        );
    }

    #[test]
    fn blocking_limit_skipped_when_auto_compact_enabled() {
        let s = QueryLoopState::new(10);
        let context_window = 100_000_u32;
        // Even above the blocking limit, should skip when auto_compact handles it
        let result =
            s.check_blocking_limit(context_window as usize - 1, context_window, true, false);
        assert!(
            result.is_none(),
            "should not block when auto_compact handles it"
        );
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
        assert_eq!(
            TerminalReason::ConsecutiveErrors.to_string(),
            "consecutive_errors"
        );
    }

    #[test]
    fn continue_reason_display() {
        assert_eq!(ContinueReason::ToolUse.to_string(), "tool_use");
        assert_eq!(ContinueReason::StreamResume.to_string(), "stream_resume");
    }

    #[test]
    fn basic_recovery_guidance_empty_streak_returns_none() {
        use super::super::format_basic_recovery_guidance;
        assert!(format_basic_recovery_guidance(&[]).is_none());
    }

    #[test]
    fn basic_recovery_guidance_includes_tool_name_and_error() {
        use super::super::format_basic_recovery_guidance;
        let streak = vec![
            ToolCallTrace {
                tool_name: "read_file".into(),
                success: false,
                latency_ms: 0,
                error: Some("No such file: /tmp/missing.txt".into()),
            },
            ToolCallTrace {
                tool_name: "shell_exec".into(),
                success: false,
                latency_ms: 0,
                error: Some("command not found: foobar".into()),
            },
        ];
        let guidance = format_basic_recovery_guidance(&streak).unwrap();
        assert!(
            guidance.contains("read_file"),
            "should mention failing tool name"
        );
        assert!(
            guidance.contains("No such file"),
            "should include error message"
        );
        assert!(
            guidance.contains("shell_exec"),
            "should mention second failing tool"
        );
        assert!(
            guidance.contains("command not found"),
            "should include second error"
        );
        assert!(
            guidance.contains("File/path errors"),
            "should have file-specific suggestion"
        );
        assert!(
            guidance.contains("Command errors"),
            "should have shell-specific suggestion"
        );
        assert!(
            guidance.contains("Do NOT repeat"),
            "should warn against retrying"
        );
    }

    #[test]
    fn basic_recovery_guidance_truncates_long_errors() {
        use super::super::format_basic_recovery_guidance;
        let long_error = "x".repeat(300);
        let streak = vec![ToolCallTrace {
            tool_name: "grep".into(),
            success: false,
            latency_ms: 0,
            error: Some(long_error),
        }];
        let guidance = format_basic_recovery_guidance(&streak).unwrap();
        assert!(
            guidance.contains("..."),
            "should truncate long error with ellipsis"
        );
        assert!(
            guidance.contains("Search errors"),
            "should have grep-specific suggestion"
        );
    }

    #[test]
    fn inject_recovery_guidance_appends_to_last_user_msg() {
        use super::super::inject_tool_recovery_guidance;
        use xiaolin_core::types::{ChatMessage, Role};

        let mut messages = vec![
            ChatMessage {
                role: Role::System,
                content: Some(serde_json::Value::String(
                    "You are a helpful assistant.".into(),
                )),
                ..Default::default()
            },
            ChatMessage {
                role: Role::User,
                content: Some(serde_json::Value::String("hello".into())),
                ..Default::default()
            },
        ];

        inject_tool_recovery_guidance(&mut messages, "Try a different approach.");
        assert_eq!(messages.len(), 2, "should not insert new message");
        let sys = messages[0].text_content().unwrap();
        assert!(
            sys.contains("You are a helpful assistant"),
            "system message should be unchanged"
        );
        let user = messages[1].text_content().unwrap();
        assert!(user.contains("<system_context>"));
        assert!(
            user.contains("Tool execution recovery"),
            "should have recovery header"
        );
        assert!(
            user.contains("Try a different approach"),
            "should include guidance"
        );
    }

    #[test]
    fn record_tool_call_warns_at_threshold_same_args() {
        let mut s = QueryLoopState::new(10);
        let args = r#"{"file_path":"test.txt"}"#;
        assert_eq!(
            s.record_tool_call("read_file", args),
            ToolRepetitionAction::None
        );
        assert_eq!(
            s.record_tool_call("read_file", args),
            ToolRepetitionAction::None
        );
        assert_eq!(
            s.record_tool_call("read_file", args),
            ToolRepetitionAction::Warn,
            "3rd identical call should trigger Warn"
        );
        assert_eq!(s.repetition_stats(), (1, 0));
    }

    #[test]
    fn record_tool_call_force_stops_at_hard_limit() {
        let mut s = QueryLoopState::new(10);
        let args = r#"{"file_path":"test.txt"}"#;
        for _ in 0..(TOOL_REPEAT_HARD_LIMIT - 1) {
            s.record_tool_call("read_file", args);
        }
        assert_eq!(
            s.record_tool_call("read_file", args),
            ToolRepetitionAction::ForceStop,
            "5th identical call should trigger ForceStop"
        );
        assert_eq!(s.repetition_stats(), (1, 1));
    }

    #[test]
    fn record_tool_call_does_not_trigger_for_different_args() {
        let mut s = QueryLoopState::new(10);
        assert_eq!(
            s.record_tool_call("read_file", r#"{"file_path":"a.txt"}"#),
            ToolRepetitionAction::None
        );
        assert_eq!(
            s.record_tool_call("read_file", r#"{"file_path":"b.txt"}"#),
            ToolRepetitionAction::None
        );
        assert_eq!(
            s.record_tool_call("read_file", r#"{"file_path":"c.txt"}"#),
            ToolRepetitionAction::None
        );
        assert!(
            s.build_repetition_nudge(false).is_none(),
            "different args should not trigger"
        );
    }

    #[test]
    fn record_tool_call_warn_fires_once_then_force_stop() {
        let mut s = QueryLoopState::new(10);
        let args = r#"{"file_path":"test.txt"}"#;
        s.record_tool_call("read_file", args);
        s.record_tool_call("read_file", args);
        assert_eq!(
            s.record_tool_call("read_file", args),
            ToolRepetitionAction::Warn
        );
        // After Warn, escalation=1, so subsequent calls return None until hard limit
        assert_eq!(
            s.record_tool_call("read_file", args),
            ToolRepetitionAction::None
        );
        assert_eq!(
            s.record_tool_call("read_file", args),
            ToolRepetitionAction::ForceStop,
            "5th call should escalate to ForceStop"
        );
        // After ForceStop, escalation=2, no more actions
        assert_eq!(
            s.record_tool_call("read_file", args),
            ToolRepetitionAction::None
        );
    }

    #[test]
    fn build_repetition_nudge_warn_message() {
        let mut s = QueryLoopState::new(10);
        let args = r#"{"file_path":"test.txt"}"#;
        for _ in 0..4 {
            s.record_tool_call("read_file", args);
        }
        s.record_tool_call("write_file", r#"{"file_path":"out.txt"}"#);
        let nudge = s.build_repetition_nudge(false).expect("should build nudge");
        assert!(
            nudge.contains("read_file"),
            "should mention the repeated tool"
        );
        assert!(nudge.contains("4 calls"), "should show call count");
        assert!(
            !nudge.contains("`write_file`"),
            "write_file called only once, should not be listed"
        );
        assert!(
            nudge.contains("change your approach"),
            "should advise changing approach"
        );
        assert!(
            nudge.contains("force-terminated"),
            "should warn about escalation"
        );
    }

    #[test]
    fn build_repetition_nudge_force_stop_message() {
        let mut s = QueryLoopState::new(10);
        let args = r#"{"file_path":"test.txt"}"#;
        for _ in 0..5 {
            s.record_tool_call("read_file", args);
        }
        let nudge = s.build_repetition_nudge(true).expect("should build nudge");
        assert!(
            nudge.contains("TERMINATED"),
            "force_stop message should mention termination"
        );
        assert!(
            nudge.contains("Summarize"),
            "should advise summarizing the issue"
        );
    }

    #[test]
    fn record_tool_call_same_path_different_offset_counts_together() {
        let mut s = QueryLoopState::new(10);
        let path_a = r#"{"file_path":"src/foo.ts","offset":1,"limit":100}"#;
        let path_b = r#"{"file_path":"src/foo.ts","offset":101,"limit":100}"#;
        assert_eq!(
            s.record_tool_call("read_file", path_a),
            ToolRepetitionAction::None
        );
        assert_eq!(
            s.record_tool_call("read_file", path_b),
            ToolRepetitionAction::None
        );
        assert_eq!(
            s.record_tool_call("read_file", path_a),
            ToolRepetitionAction::Warn,
            "3rd read of same path (different offset) should trigger Warn"
        );
    }

    #[test]
    fn no_progress_stall_warns_then_force_stops() {
        let mut s = QueryLoopState::new(10);
        for _ in 0..11 {
            s.record_iteration_progress(true, false);
            assert_eq!(s.check_no_progress_stall(), NoProgressStallAction::None);
        }
        s.record_iteration_progress(true, false);
        assert_eq!(s.check_no_progress_stall(), NoProgressStallAction::Warn);
        assert_eq!(s.check_no_progress_stall(), NoProgressStallAction::None);
        for _ in 0..13 {
            s.record_iteration_progress(true, false);
        }
        assert_eq!(
            s.check_no_progress_stall(),
            NoProgressStallAction::ForceStop,
            "25 consecutive read-only iterations should force stop"
        );
    }

    #[test]
    fn no_progress_stall_resets_on_progress() {
        let mut s = QueryLoopState::new(10);
        for _ in 0..10 {
            s.record_iteration_progress(true, false);
        }
        s.record_iteration_progress(true, true);
        assert_eq!(s.iterations_without_progress, 0);
        assert_eq!(s.check_no_progress_stall(), NoProgressStallAction::None);
    }

    #[test]
    fn build_repetition_nudge_none_when_below_threshold() {
        let mut s = QueryLoopState::new(10);
        let args = r#"{"file_path":"test.txt"}"#;
        s.record_tool_call("read_file", args);
        s.record_tool_call("read_file", args);
        assert!(s.build_repetition_nudge(false).is_none());
    }

    #[test]
    fn inject_recovery_guidance_appends_user_context_when_no_user_exists() {
        use super::super::inject_tool_recovery_guidance;
        use xiaolin_core::types::{ChatMessage, Role};

        let mut messages = vec![ChatMessage {
            role: Role::System,
            content: Some(serde_json::Value::String("sys".into())),
            ..Default::default()
        }];

        inject_tool_recovery_guidance(&mut messages, "Check permissions.");
        assert_eq!(messages.len(), 2, "should append a user message");
        assert!(matches!(messages[1].role, Role::User));
        assert!(messages[1]
            .text_content()
            .unwrap()
            .contains("Check permissions"));
    }
}
