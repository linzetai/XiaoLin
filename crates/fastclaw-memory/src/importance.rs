use fastclaw_core::config::ImportanceScoringConfig;
use fastclaw_core::types::{ChatMessage, Role};
use serde::{Deserialize, Serialize};

/// Configurable weights for the importance scoring heuristic.
///
/// Each signal is normalised to `[0.0, 1.0]` and the weighted sum is clamped
/// to the same range. A conversation that scores below `min_threshold` can be
/// skipped by callers (e.g. consolidation, auto-record).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportanceScorer {
    #[serde(default = "default_weight_length")]
    pub weight_length: f32,
    #[serde(default = "default_weight_tool_calls")]
    pub weight_tool_calls: f32,
    #[serde(default = "default_weight_keywords")]
    pub weight_keywords: f32,
    #[serde(default = "default_weight_depth")]
    pub weight_depth: f32,
    #[serde(default = "default_weight_corrections")]
    pub weight_corrections: f32,
    #[serde(default = "default_min_threshold")]
    pub min_threshold: f32,
}

fn default_weight_length() -> f32 {
    0.15
}
fn default_weight_tool_calls() -> f32 {
    0.25
}
fn default_weight_keywords() -> f32 {
    0.30
}
fn default_weight_depth() -> f32 {
    0.15
}
fn default_weight_corrections() -> f32 {
    0.15
}
fn default_min_threshold() -> f32 {
    0.3
}

impl Default for ImportanceScorer {
    fn default() -> Self {
        Self {
            weight_length: default_weight_length(),
            weight_tool_calls: default_weight_tool_calls(),
            weight_keywords: default_weight_keywords(),
            weight_depth: default_weight_depth(),
            weight_corrections: default_weight_corrections(),
            min_threshold: default_min_threshold(),
        }
    }
}

impl From<ImportanceScoringConfig> for ImportanceScorer {
    fn from(cfg: ImportanceScoringConfig) -> Self {
        Self {
            weight_length: cfg.weight_length,
            weight_tool_calls: cfg.weight_tool_calls,
            weight_keywords: cfg.weight_keywords,
            weight_depth: cfg.weight_depth,
            weight_corrections: cfg.weight_corrections,
            min_threshold: cfg.min_threshold,
        }
    }
}

const DECISION_KEYWORDS: &[&str] = &[
    "decided",
    "chose",
    "prefer",
    "decided against",
    "went with",
    "selected",
    "记住",
    "决定",
    "选择",
    "偏好",
    "采用",
];

const CORRECTION_KEYWORDS: &[&str] = &[
    "actually",
    "no,",
    "wrong",
    "incorrect",
    "not right",
    "应该是",
    "不对",
    "错了",
    "其实",
    "纠正",
];

impl ImportanceScorer {
    /// Score a full conversation's importance based on multiple weighted signals.
    ///
    /// Returns a value in `[0.0, 1.0]`.
    pub fn score(&self, messages: &[ChatMessage]) -> f32 {
        let non_system: Vec<&ChatMessage> = messages
            .iter()
            .filter(|m| !matches!(m.role, Role::System))
            .collect();

        let length_signal = (non_system.len() as f32 / 20.0).min(1.0);

        let tool_count = messages
            .iter()
            .filter(|m| {
                m.tool_calls.as_ref().is_some_and(|tc| !tc.is_empty()) || m.tool_call_id.is_some()
            })
            .count();
        let tool_signal = (tool_count as f32 / 10.0).min(1.0);

        let all_text = collect_text(messages);
        let keyword_signal = keyword_presence(&all_text, DECISION_KEYWORDS);
        let correction_signal = keyword_presence(&all_text, CORRECTION_KEYWORDS);

        let user_turns = messages
            .iter()
            .filter(|m| matches!(m.role, Role::User))
            .count();
        let depth_signal = (user_turns as f32 / 10.0).min(1.0);

        let raw = self.weight_length * length_signal
            + self.weight_tool_calls * tool_signal
            + self.weight_keywords * keyword_signal
            + self.weight_depth * depth_signal
            + self.weight_corrections * correction_signal;

        raw.clamp(0.0, 1.0)
    }

    /// Lightweight single-text scoring for `auto_record_episode` where we only
    /// have the assistant reply, not the full conversation.
    pub fn score_single(text: &str) -> f32 {
        let lower = text.to_lowercase();
        let len_signal = (text.len() as f32 / 500.0).min(1.0);
        let kw = keyword_presence_str(&lower, DECISION_KEYWORDS);
        let corr = keyword_presence_str(&lower, CORRECTION_KEYWORDS);
        (0.4 * len_signal + 0.35 * kw + 0.25 * corr).clamp(0.0, 1.0)
    }
}

fn collect_text(messages: &[ChatMessage]) -> String {
    let mut buf = String::new();
    for m in messages {
        if let Some(t) = m.text_content() {
            buf.push_str(&t);
            buf.push('\n');
        }
    }
    buf.to_lowercase()
}

fn keyword_presence(text: &str, keywords: &[&str]) -> f32 {
    let hits = keywords.iter().filter(|kw| text.contains(**kw)).count();
    (hits as f32 / 3.0).min(1.0)
}

fn keyword_presence_str(text: &str, keywords: &[&str]) -> f32 {
    let hits = keywords.iter().filter(|kw| text.contains(**kw)).count();
    (hits as f32 / 3.0).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastclaw_core::types::ChatMessage;

    fn user_msg(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::User,
            content: Some(serde_json::Value::String(text.to_string())),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn assistant_msg(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::Assistant,
            content: Some(serde_json::Value::String(text.to_string())),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn system_msg(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::System,
            content: Some(serde_json::Value::String(text.to_string())),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    #[test]
    fn empty_messages_scores_zero() {
        let scorer = ImportanceScorer::default();
        assert_eq!(scorer.score(&[]), 0.0);
    }

    #[test]
    fn system_only_scores_zero() {
        let scorer = ImportanceScorer::default();
        let msgs = vec![system_msg("You are helpful")];
        assert_eq!(scorer.score(&msgs), 0.0);
    }

    #[test]
    fn short_conversation_scores_low() {
        let scorer = ImportanceScorer::default();
        let msgs = vec![user_msg("hello"), assistant_msg("hi")];
        let s = scorer.score(&msgs);
        assert!(s > 0.0 && s < 0.3, "expected low score, got {s}");
    }

    #[test]
    fn decision_keywords_boost_score() {
        let scorer = ImportanceScorer::default();
        let msgs = vec![
            user_msg("I decided to use Postgres"),
            assistant_msg("Good choice, I chose the same config"),
        ];
        let s = scorer.score(&msgs);
        let plain = scorer.score(&[user_msg("hello"), assistant_msg("hi")]);
        assert!(s > plain, "decision keywords should boost: {s} vs {plain}");
    }

    #[test]
    fn correction_keywords_boost_score() {
        let scorer = ImportanceScorer::default();
        let msgs = vec![
            user_msg("actually, that's wrong"),
            assistant_msg("sorry, let me fix that"),
        ];
        let s = scorer.score(&msgs);
        assert!(s > 0.1, "correction keywords should boost: {s}");
    }

    #[test]
    fn long_conversation_scores_higher() {
        let scorer = ImportanceScorer::default();
        let msgs: Vec<ChatMessage> = (0..20)
            .map(|i| {
                if i % 2 == 0 {
                    user_msg(&format!("question {i}"))
                } else {
                    assistant_msg(&format!("answer {i}"))
                }
            })
            .collect();
        let s = scorer.score(&msgs);
        assert!(s > 0.2, "long conversation should score higher: {s}");
    }

    #[test]
    fn score_single_returns_bounded() {
        let s = ImportanceScorer::score_single("short");
        assert!((0.0..=1.0).contains(&s));

        let s2 = ImportanceScorer::score_single(
            "We decided to use Redis. The user prefers caching with TTL. Actually the old approach was wrong.",
        );
        assert!(s2 > s, "keyword-rich text should score higher: {s2} vs {s}");
    }

    #[test]
    fn chinese_keywords_detected() {
        let scorer = ImportanceScorer::default();
        let msgs = vec![
            user_msg("记住我喜欢用 fish shell"),
            assistant_msg("好的，已记录你的偏好"),
        ];
        let s = scorer.score(&msgs);
        let plain = scorer.score(&[user_msg("你好"), assistant_msg("你好")]);
        assert!(s > plain, "Chinese keywords should boost: {s} vs {plain}");
    }

    #[test]
    fn score_clamped_to_one() {
        let scorer = ImportanceScorer {
            weight_length: 1.0,
            weight_tool_calls: 1.0,
            weight_keywords: 1.0,
            weight_depth: 1.0,
            weight_corrections: 1.0,
            min_threshold: 0.0,
        };
        let msgs: Vec<ChatMessage> = (0..30)
            .map(|i| user_msg(&format!("decided chose prefer wrong actually {i}")))
            .collect();
        let s = scorer.score(&msgs);
        assert_eq!(s, 1.0);
    }
}
