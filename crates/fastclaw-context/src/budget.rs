/// Decision from [`TokenBudgetTracker::record`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetDecision {
    /// Budget available; keep executing.
    Continue {
        /// Non-empty when consumption >= 90% — a nudge to wrap up soon.
        nudge_message: Option<String>,
    },
    /// Stop execution.
    Stop {
        /// Why we stopped.
        reason: StopReason,
    },
}

/// Reason for a [`BudgetDecision::Stop`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// No budget was configured (budget_tokens == 0).
    NoBudget,
    /// Total consumption reached or exceeded the budget.
    Exhausted,
    /// The last N rounds each consumed fewer than `diminishing_threshold`
    /// tokens, indicating the agent is no longer making meaningful progress.
    DiminishingReturns,
}

const NUDGE_THRESHOLD: f64 = 0.90;
const DEFAULT_DIMINISHING_THRESHOLD: usize = 500;
const DEFAULT_DIMINISHING_WINDOW: usize = 3;

/// Tracks cumulative token consumption across turns and emits
/// Continue / Stop decisions with optional nudge messages.
pub struct TokenBudgetTracker {
    budget_tokens: usize,
    consumed: usize,
    /// Per-turn token deltas for diminishing-returns detection.
    deltas: Vec<usize>,
    diminishing_threshold: usize,
    diminishing_window: usize,
}

impl TokenBudgetTracker {
    /// Create a tracker with the given total budget (in tokens).
    /// Pass `0` to indicate "no budget" — `record` will always return `Stop`.
    pub fn new(budget_tokens: usize) -> Self {
        Self {
            budget_tokens,
            consumed: 0,
            deltas: Vec::new(),
            diminishing_threshold: DEFAULT_DIMINISHING_THRESHOLD,
            diminishing_window: DEFAULT_DIMINISHING_WINDOW,
        }
    }

    /// Override the diminishing-returns detection parameters.
    pub fn with_diminishing(mut self, threshold: usize, window: usize) -> Self {
        self.diminishing_threshold = threshold;
        self.diminishing_window = window.max(1);
        self
    }

    /// Record `tokens_this_turn` consumed in this turn and return a decision.
    pub fn record(&mut self, tokens_this_turn: usize) -> BudgetDecision {
        if self.budget_tokens == 0 {
            return BudgetDecision::Stop {
                reason: StopReason::NoBudget,
            };
        }

        self.consumed += tokens_this_turn;
        self.deltas.push(tokens_this_turn);

        if self.consumed >= self.budget_tokens {
            return BudgetDecision::Stop {
                reason: StopReason::Exhausted,
            };
        }

        if self.is_diminishing() {
            return BudgetDecision::Stop {
                reason: StopReason::DiminishingReturns,
            };
        }

        let ratio = self.consumed as f64 / self.budget_tokens as f64;
        if ratio >= NUDGE_THRESHOLD {
            let remaining = self.budget_tokens - self.consumed;
            let pct = (ratio * 100.0).round() as u32;
            BudgetDecision::Continue {
                nudge_message: Some(format!(
                    "[Budget {pct}% used — ~{remaining} tokens remaining. Please wrap up soon.]"
                )),
            }
        } else {
            BudgetDecision::Continue {
                nudge_message: None,
            }
        }
    }

    /// Current total consumed tokens.
    pub fn consumed(&self) -> usize {
        self.consumed
    }

    /// Remaining token budget.
    pub fn remaining(&self) -> usize {
        self.budget_tokens.saturating_sub(self.consumed)
    }

    /// Configured budget.
    pub fn budget(&self) -> usize {
        self.budget_tokens
    }

    fn is_diminishing(&self) -> bool {
        let w = self.diminishing_window;
        if self.deltas.len() < w {
            return false;
        }
        self.deltas[self.deltas.len() - w..]
            .iter()
            .all(|&d| d < self.diminishing_threshold)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_budget_always_stops() {
        let mut tracker = TokenBudgetTracker::new(0);
        let d = tracker.record(100);
        assert_eq!(
            d,
            BudgetDecision::Stop {
                reason: StopReason::NoBudget
            }
        );
        let d2 = tracker.record(0);
        assert_eq!(
            d2,
            BudgetDecision::Stop {
                reason: StopReason::NoBudget
            }
        );
    }

    #[test]
    fn under_90_percent_continue_no_nudge() {
        let mut tracker = TokenBudgetTracker::new(10_000);
        let d = tracker.record(5_000); // 50%
        assert_eq!(d, BudgetDecision::Continue { nudge_message: None });
        assert_eq!(tracker.consumed(), 5_000);
        assert_eq!(tracker.remaining(), 5_000);
    }

    #[test]
    fn at_90_percent_continue_with_nudge() {
        let mut tracker = TokenBudgetTracker::new(10_000);
        let d = tracker.record(9_000); // 90%
        match d {
            BudgetDecision::Continue { nudge_message } => {
                assert!(nudge_message.is_some(), "should have nudge at 90%");
                let msg = nudge_message.unwrap();
                assert!(msg.contains("1000"), "should mention ~1000 remaining");
                assert!(msg.contains("90%"), "should mention 90%");
            }
            other => panic!("expected Continue with nudge, got {other:?}"),
        }
    }

    #[test]
    fn budget_exhausted_stops() {
        let mut tracker = TokenBudgetTracker::new(10_000);
        tracker.record(5_000);
        let d = tracker.record(5_000); // exactly 100%
        assert_eq!(
            d,
            BudgetDecision::Stop {
                reason: StopReason::Exhausted
            }
        );
    }

    #[test]
    fn budget_exceeded_stops() {
        let mut tracker = TokenBudgetTracker::new(10_000);
        let d = tracker.record(12_000); // over
        assert_eq!(
            d,
            BudgetDecision::Stop {
                reason: StopReason::Exhausted
            }
        );
    }

    #[test]
    fn diminishing_returns_after_window() {
        let mut tracker = TokenBudgetTracker::new(100_000);
        // Three rounds each below 500 tokens
        assert!(matches!(
            tracker.record(200),
            BudgetDecision::Continue { .. }
        ));
        assert!(matches!(
            tracker.record(150),
            BudgetDecision::Continue { .. }
        ));
        let d = tracker.record(100);
        assert_eq!(
            d,
            BudgetDecision::Stop {
                reason: StopReason::DiminishingReturns
            }
        );
    }

    #[test]
    fn large_turn_resets_diminishing_window() {
        let mut tracker = TokenBudgetTracker::new(100_000);
        tracker.record(100); // small
        tracker.record(100); // small
        tracker.record(5_000); // large — resets the window
        // Need 3 consecutive small turns again.
        assert!(matches!(
            tracker.record(100),
            BudgetDecision::Continue { .. }
        ));
        assert!(matches!(
            tracker.record(100),
            BudgetDecision::Continue { .. }
        ));
        let d = tracker.record(100);
        assert_eq!(
            d,
            BudgetDecision::Stop {
                reason: StopReason::DiminishingReturns
            }
        );
    }

    #[test]
    fn custom_diminishing_window() {
        let mut tracker = TokenBudgetTracker::new(100_000).with_diminishing(1000, 5);
        for _ in 0..4 {
            assert!(matches!(
                tracker.record(500),
                BudgetDecision::Continue { .. }
            ));
        }
        let d = tracker.record(500);
        assert_eq!(
            d,
            BudgetDecision::Stop {
                reason: StopReason::DiminishingReturns
            },
            "should trigger after window=5"
        );
    }

    #[test]
    fn nudge_at_95_percent() {
        let mut tracker = TokenBudgetTracker::new(20_000);
        let d = tracker.record(19_000); // 95%
        match d {
            BudgetDecision::Continue { nudge_message } => {
                assert!(nudge_message.is_some());
                let msg = nudge_message.unwrap();
                assert!(msg.contains("95%"));
            }
            other => panic!("expected Continue with nudge, got {other:?}"),
        }
    }
}
