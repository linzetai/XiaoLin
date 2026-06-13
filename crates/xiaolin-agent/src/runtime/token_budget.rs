//! Token budget system: allows users to specify a token budget (e.g., "+500k")
//! as a safety CEILING — the turn is stopped if output exceeds the budget.
//!
//! The budget does NOT force continuation. When the model says "done", we trust
//! it. The budget only acts as an upper-bound safety valve (enforced in
//! iteration_check.rs via soft nudge at 100% and hard stop at 120%).
//!
//! Cross-turn inheritance: if a user message does NOT contain a budget spec,
//! the previous session budget is inherited. The budget is cleared when a turn
//! hits the ceiling or the user explicitly starts a new context.

use dashmap::DashMap;
use regex::Regex;
use std::sync::{Arc, LazyLock};

/// Global per-session budget registry. Survives across turns within the same
/// gateway process. Keyed by session_id → target_tokens.
static SESSION_BUDGETS: LazyLock<Arc<DashMap<String, u64>>> =
    LazyLock::new(|| Arc::new(DashMap::new()));

/// Store a token budget for a session (persists across turns).
pub fn set_session_budget(session_id: &str, target_tokens: u64) {
    SESSION_BUDGETS.insert(session_id.to_string(), target_tokens);
}

/// Get the inherited budget for a session (if any).
pub fn get_session_budget(session_id: &str) -> Option<u64> {
    SESSION_BUDGETS.get(session_id).map(|v| *v)
}

/// Clear the budget for a session (called when budget is reached or user
/// sends a message with no budget in a new context).
pub fn clear_session_budget(session_id: &str) {
    SESSION_BUDGETS.remove(session_id);
}

/// Resolve the effective budget for a turn:
/// 1. If user message contains a budget spec → use it (and persist)
/// 2. Otherwise → inherit from session registry
pub fn resolve_turn_budget(user_message: &str, session_id: Option<&str>) -> Option<TokenBudget> {
    if let Some(budget) = parse_token_budget(user_message) {
        if let Some(sid) = session_id {
            set_session_budget(sid, budget.target_tokens);
        }
        return Some(budget);
    }
    if let Some(sid) = session_id {
        get_session_budget(sid).map(|t| TokenBudget { target_tokens: t })
    } else {
        None
    }
}

/// Parsed token budget from user input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenBudget {
    pub target_tokens: u64,
}

static SHORTHAND_START_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*\+(\d+(?:\.\d+)?)\s*(k|m|b)\b").unwrap());

static SHORTHAND_END_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\s\+(\d+(?:\.\d+)?)\s*(k|m|b)\s*[.!?]?\s*$").unwrap());

static VERBOSE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:use|spend|花费?|消耗|用)\s*(\d+(?:\.\d+)?)\s*(k|m|b)\s*tokens?\b")
        .unwrap()
});

fn multiplier(suffix: &str) -> u64 {
    match suffix.to_lowercase().as_str() {
        "k" => 1_000,
        "m" => 1_000_000,
        "b" => 1_000_000_000,
        _ => 1,
    }
}

fn parse_match(value: &str, suffix: &str) -> u64 {
    let num: f64 = value.parse().unwrap_or(0.0);
    (num * multiplier(suffix) as f64) as u64
}

/// Try to extract a token budget from a user message.
///
/// Recognizes patterns like:
/// - `+500k` (at start or end of message)
/// - `use 2M tokens`
/// - `spend 1.5m tokens`
/// - `花 500k tokens`
pub fn parse_token_budget(text: &str) -> Option<TokenBudget> {
    if let Some(caps) = SHORTHAND_START_RE.captures(text) {
        return Some(TokenBudget {
            target_tokens: parse_match(&caps[1], &caps[2]),
        });
    }
    if let Some(caps) = SHORTHAND_END_RE.captures(text) {
        return Some(TokenBudget {
            target_tokens: parse_match(&caps[1], &caps[2]),
        });
    }
    if let Some(caps) = VERBOSE_RE.captures(text) {
        return Some(TokenBudget {
            target_tokens: parse_match(&caps[1], &caps[2]),
        });
    }
    None
}

/// Tracks token budget state within a turn.
/// Used by iteration_check.rs to enforce the CEILING (soft nudge + hard stop).
#[derive(Debug, Clone)]
pub struct BudgetTracker {
    pub budget: TokenBudget,
    /// Whether the per-iteration soft nudge (at 100%) has already been injected.
    pub soft_nudge_sent: bool,
}

impl BudgetTracker {
    pub fn new(budget: TokenBudget) -> Self {
        Self {
            budget,
            soft_nudge_sent: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_shorthand_start() {
        assert_eq!(
            parse_token_budget("+500k do the task").map(|b| b.target_tokens),
            Some(500_000)
        );
        assert_eq!(
            parse_token_budget("+2M").map(|b| b.target_tokens),
            Some(2_000_000)
        );
        assert_eq!(
            parse_token_budget("+1.5m tokens").map(|b| b.target_tokens),
            Some(1_500_000)
        );
    }

    #[test]
    fn parse_shorthand_end() {
        assert_eq!(
            parse_token_budget("implement the feature +500k").map(|b| b.target_tokens),
            Some(500_000)
        );
    }

    #[test]
    fn parse_verbose() {
        assert_eq!(
            parse_token_budget("use 2m tokens to implement this").map(|b| b.target_tokens),
            Some(2_000_000)
        );
        assert_eq!(
            parse_token_budget("spend 500k tokens on refactoring").map(|b| b.target_tokens),
            Some(500_000)
        );
    }

    #[test]
    fn parse_chinese() {
        assert_eq!(
            parse_token_budget("用 1m tokens 完成任务").map(|b| b.target_tokens),
            Some(1_000_000)
        );
    }

    #[test]
    fn parse_no_match() {
        assert!(parse_token_budget("hello world").is_none());
        assert!(parse_token_budget("implement markdown viewer").is_none());
    }

    #[test]
    fn resolve_with_explicit_budget() {
        let result = resolve_turn_budget("+200k implement feature", Some("sess_1"));
        assert_eq!(result.map(|b| b.target_tokens), Some(200_000));
        // Should be persisted
        assert_eq!(get_session_budget("sess_1"), Some(200_000));
        clear_session_budget("sess_1");
    }

    #[test]
    fn resolve_inherits_from_session() {
        set_session_budget("sess_2", 750_000);
        // No budget in message → inherits
        let result = resolve_turn_budget("continue working on the task", Some("sess_2"));
        assert_eq!(result.map(|b| b.target_tokens), Some(750_000));
        clear_session_budget("sess_2");
    }

    #[test]
    fn resolve_no_session_no_budget() {
        let result = resolve_turn_budget("hello", None);
        assert!(result.is_none());
    }

    #[test]
    fn clear_removes_budget() {
        set_session_budget("sess_3", 500_000);
        clear_session_budget("sess_3");
        assert_eq!(get_session_budget("sess_3"), None);
    }
}
