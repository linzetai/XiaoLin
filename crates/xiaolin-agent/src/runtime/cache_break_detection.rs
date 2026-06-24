//! Prompt cache break detection for LLM API calls.
//!
//! Detects when prompt caching unexpectedly stops working (cache_read_tokens
//! drops from >0 to 0) and identifies the cause. This helps maintain high
//! cache hit rates which significantly reduce latency and cost.
//!
//! The detector works in two phases:
//! 1. **Pre-call snapshot**: Hash system prompt, tools, and model before the call.
//! 2. **Post-call analysis**: Compare cache_read_tokens against previous call;
//!    if a break is detected, diff the snapshots to identify the cause.

use std::collections::hash_map::DefaultHasher;
// DefaultHasher: in-memory only (same-process comparison). Use blake3 if persistence is needed.
use std::hash::{Hash, Hasher};
use xiaolin_core::types::ChatMessage;

/// Extended usage data that includes cache-specific token counts.
/// Not all providers return these; fields are optional.
#[derive(Debug, Clone, Default)]
pub struct CacheAwareUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_creation_tokens: u32,
}

/// Snapshot of inputs that affect prompt caching, taken before an LLM call.
#[derive(Debug, Clone)]
pub struct PreCallSnapshot {
    pub system_hash: u64,
    pub tools_hash: u64,
    pub model: String,
    pub tool_count: usize,
    pub has_cache_control: bool,
    pub cache_edits_active: bool,
    /// Per-message content hashes of the conversation prefix (in order).
    /// Empty when the caller did not supply messages (e.g. unit tests).
    /// Used to attribute cache breaks to history mutation/compaction: a clean
    /// append keeps the previous hashes as a prefix of the current ones, while
    /// compaction/editing diverges the prefix. (§11.4)
    pub message_prefix_hashes: Vec<u64>,
}

/// Why the prompt cache broke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BreakCause {
    /// System prompt content changed.
    SystemPromptChanged,
    /// Tools were added, removed, or modified.
    ToolsChanged {
        prev_count: usize,
        curr_count: usize,
    },
    /// Model was switched between calls.
    ModelSwitched { from: String, to: String },
    /// cache_control annotations changed.
    CacheControlChanged,
    /// cache_edits API removed cached content (expected, not a real break).
    CacheEditsEviction,
    /// Conversation history prefix diverged (compaction / micro-compact / snip /
    /// editing of already-sent messages). Expected when compaction ran; a real
    /// concern only if it happens without an intentional compaction (§11.3/§11.4).
    HistoryChanged {
        prev_len: usize,
        curr_len: usize,
    },
    /// Multiple causes detected simultaneously.
    Multiple(Vec<BreakCause>),
    /// Could not determine the specific cause.
    Unknown,
}

/// Report generated when a cache break is detected.
#[derive(Debug, Clone)]
pub struct CacheBreakReport {
    pub cause: BreakCause,
    pub prev_cache_read_tokens: u32,
    pub curr_cache_read_tokens: u32,
    pub is_expected: bool,
}

impl CacheBreakReport {
    pub fn summary(&self) -> String {
        let cause_desc = match &self.cause {
            BreakCause::SystemPromptChanged => "system prompt changed".to_string(),
            BreakCause::ToolsChanged {
                prev_count,
                curr_count,
            } => {
                format!("tools changed ({} → {})", prev_count, curr_count)
            }
            BreakCause::ModelSwitched { from, to } => {
                format!("model switched ({} → {})", from, to)
            }
            BreakCause::CacheControlChanged => "cache_control annotations changed".to_string(),
            BreakCause::CacheEditsEviction => "cache_edits eviction (expected)".to_string(),
            BreakCause::HistoryChanged { prev_len, curr_len } => {
                format!("history compacted/edited ({prev_len} → {curr_len} msgs, expected)")
            }
            BreakCause::Multiple(causes) => {
                let descs: Vec<String> = causes
                    .iter()
                    .map(|c| {
                        CacheBreakReport {
                            cause: c.clone(),
                            prev_cache_read_tokens: 0,
                            curr_cache_read_tokens: 0,
                            is_expected: false,
                        }
                        .summary()
                    })
                    .collect();
                format!("multiple: {}", descs.join(", "))
            }
            BreakCause::Unknown => "unknown cause".to_string(),
        };

        format!(
            "cache break: {} (cache_read: {} → {})",
            cause_desc, self.prev_cache_read_tokens, self.curr_cache_read_tokens
        )
    }
}

/// Stateful detector that tracks snapshots across consecutive LLM calls.
#[derive(Debug)]
pub struct CacheBreakDetector {
    prev_snapshot: Option<PreCallSnapshot>,
    prev_cache_read_tokens: u32,
    total_breaks: u32,
    total_calls: u32,
}

impl CacheBreakDetector {
    pub fn new() -> Self {
        Self {
            prev_snapshot: None,
            prev_cache_read_tokens: 0,
            total_breaks: 0,
            total_calls: 0,
        }
    }

    /// Take a snapshot of inputs before an LLM call.
    ///
    /// `system_prompt`: the full system prompt text
    /// `tools_json`: serialized tool definitions (order-stable)
    /// `model`: model identifier string
    /// `has_cache_control`: whether cache_control annotations are present
    /// `cache_edits_active`: whether cache_edits API is being used this call
    pub fn pre_call_snapshot(
        &self,
        system_prompt: &str,
        tools_json: &str,
        model: &str,
        has_cache_control: bool,
        cache_edits_active: bool,
    ) -> PreCallSnapshot {
        let tool_count = count_tools(tools_json);
        PreCallSnapshot {
            system_hash: hash_str(system_prompt),
            tools_hash: hash_str(tools_json),
            model: model.to_string(),
            tool_count,
            has_cache_control,
            cache_edits_active,
            message_prefix_hashes: Vec::new(),
        }
    }

    /// Analyze the result of an LLM call for cache breaks.
    ///
    /// Call this after every LLM response with the current snapshot and usage.
    /// Returns `Some(report)` if a cache break was detected, `None` otherwise.
    pub fn post_call_analyze(
        &mut self,
        current_snapshot: &PreCallSnapshot,
        usage: &CacheAwareUsage,
    ) -> Option<CacheBreakReport> {
        self.total_calls += 1;
        let prev_cache = self.prev_cache_read_tokens;
        let curr_cache = usage.cache_read_tokens;

        // Update state for next call
        self.prev_cache_read_tokens = curr_cache;
        let prev_snap = self.prev_snapshot.replace(current_snapshot.clone());

        // Cache break = previously had cache reads, now zero
        if prev_cache == 0 || curr_cache > 0 {
            return None;
        }

        // We have a break: prev_cache > 0, curr_cache == 0
        let Some(prev) = prev_snap else {
            return Some(CacheBreakReport {
                cause: BreakCause::Unknown,
                prev_cache_read_tokens: prev_cache,
                curr_cache_read_tokens: curr_cache,
                is_expected: false,
            });
        };

        let cause = self.diagnose_cause(&prev, current_snapshot);
        // Compaction-induced history changes and cache_edits evictions are
        // expected (they intentionally rewrite the prefix). A `Multiple` that
        // *also* contains a system/tools/model change is NOT expected — those
        // are real regressions even if compaction happened in the same turn.
        let is_expected = matches!(
            cause,
            BreakCause::CacheEditsEviction | BreakCause::HistoryChanged { .. }
        );

        if !is_expected {
            self.total_breaks += 1;
            tracing::warn!(
                cause = ?cause,
                prev_cache_read = prev_cache,
                "prompt cache break detected"
            );
        } else {
            tracing::debug!(
                cause = ?cause,
                "expected cache miss (cache_edits eviction)"
            );
        }

        Some(CacheBreakReport {
            cause,
            prev_cache_read_tokens: prev_cache,
            curr_cache_read_tokens: curr_cache,
            is_expected,
        })
    }

    /// Diagnose the root cause of a cache break by diffing snapshots.
    fn diagnose_cause(&self, prev: &PreCallSnapshot, curr: &PreCallSnapshot) -> BreakCause {
        // If cache_edits were active, this is an expected eviction
        if curr.cache_edits_active {
            return BreakCause::CacheEditsEviction;
        }

        let mut causes = Vec::new();

        // History prefix divergence: the previous message hashes are no longer a
        // prefix of the current ones → history was compacted/edited (not a clean
        // append). Only check when both snapshots carry message hashes. (§11.4)
        if !prev.message_prefix_hashes.is_empty()
            && !curr.message_prefix_hashes.is_empty()
            && !is_prefix(&prev.message_prefix_hashes, &curr.message_prefix_hashes)
        {
            causes.push(BreakCause::HistoryChanged {
                prev_len: prev.message_prefix_hashes.len(),
                curr_len: curr.message_prefix_hashes.len(),
            });
        }

        if prev.system_hash != curr.system_hash {
            causes.push(BreakCause::SystemPromptChanged);
        }

        if prev.tools_hash != curr.tools_hash {
            causes.push(BreakCause::ToolsChanged {
                prev_count: prev.tool_count,
                curr_count: curr.tool_count,
            });
        }

        if prev.model != curr.model {
            causes.push(BreakCause::ModelSwitched {
                from: prev.model.clone(),
                to: curr.model.clone(),
            });
        }

        if prev.has_cache_control != curr.has_cache_control {
            causes.push(BreakCause::CacheControlChanged);
        }

        match causes.len() {
            0 => BreakCause::Unknown,
            1 => causes.into_iter().next().unwrap(),
            _ => BreakCause::Multiple(causes),
        }
    }

    pub fn total_breaks(&self) -> u32 {
        self.total_breaks
    }

    pub fn total_calls(&self) -> u32 {
        self.total_calls
    }

    /// Cache hit rate as a percentage (0.0–1.0).
    /// Returns None if no calls have been made.
    pub fn cache_hit_rate(&self) -> Option<f64> {
        if self.total_calls == 0 {
            return None;
        }
        let hits = self.total_calls.saturating_sub(self.total_breaks);
        Some(hits as f64 / self.total_calls as f64)
    }
}

impl Default for CacheBreakDetector {
    fn default() -> Self {
        Self::new()
    }
}

/// Hash a string using DefaultHasher for fast in-memory comparison (not for persistence).
fn hash_str(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// Returns true if `prefix` is an ordered prefix of `full` (a clean append).
fn is_prefix(prefix: &[u64], full: &[u64]) -> bool {
    prefix.len() <= full.len() && full.starts_with(prefix)
}

/// Compute per-message content hashes for cache-break attribution (§11.4).
///
/// In-memory only (DefaultHasher); compared within the same process across
/// consecutive LLM calls to distinguish a clean append (prefix preserved) from
/// a compaction/edit (prefix diverged). Cheap relative to the LLM call itself.
pub fn hash_message_prefix(messages: &[ChatMessage]) -> Vec<u64> {
    messages
        .iter()
        .map(|m| {
            // Serialize for a stable, content-sensitive hash. Fall back to the
            // role discriminator if serialization somehow fails.
            match serde_json::to_string(m) {
                Ok(s) => hash_str(&s),
                Err(_) => hash_str(&format!("{:?}", m.role)),
            }
        })
        .collect()
}

/// Count the number of tool definitions in a JSON tools string.
/// Simple heuristic: count top-level objects in an array.
fn count_tools(tools_json: &str) -> usize {
    let trimmed = tools_json.trim();
    if trimmed.is_empty() || trimmed == "[]" || trimmed == "null" {
        return 0;
    }
    // Count occurrences of `"name"` as a proxy for tool count
    trimmed.matches("\"name\"").count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_snapshot(system: &str, tools: &str, model: &str) -> PreCallSnapshot {
        let detector = CacheBreakDetector::new();
        detector.pre_call_snapshot(system, tools, model, false, false)
    }

    fn usage_with_cache(cache_read: u32) -> CacheAwareUsage {
        CacheAwareUsage {
            prompt_tokens: 1000,
            completion_tokens: 200,
            cache_read_tokens: cache_read,
            cache_creation_tokens: 0,
        }
    }

    #[test]
    fn no_break_when_first_call() {
        let mut detector = CacheBreakDetector::new();
        let snap = make_snapshot("system", "[]", "claude-3");
        let usage = usage_with_cache(0);
        let report = detector.post_call_analyze(&snap, &usage);
        assert!(report.is_none());
    }

    #[test]
    fn no_break_when_cache_maintained() {
        let mut detector = CacheBreakDetector::new();
        let snap = make_snapshot("system", "[]", "claude-3");

        // First call establishes baseline
        detector.post_call_analyze(&snap, &usage_with_cache(5000));
        // Second call still has cache
        let report = detector.post_call_analyze(&snap, &usage_with_cache(4800));
        assert!(report.is_none());
    }

    #[test]
    fn detects_break_when_cache_drops_to_zero() {
        let mut detector = CacheBreakDetector::new();
        let snap = make_snapshot("system", "[]", "claude-3");

        detector.post_call_analyze(&snap, &usage_with_cache(5000));

        let snap2 = make_snapshot("modified system", "[]", "claude-3");
        let report = detector.post_call_analyze(&snap2, &usage_with_cache(0));
        assert!(report.is_some());
        let r = report.unwrap();
        assert_eq!(r.prev_cache_read_tokens, 5000);
        assert_eq!(r.curr_cache_read_tokens, 0);
        assert!(!r.is_expected);
    }

    #[test]
    fn diagnoses_system_prompt_change() {
        let mut detector = CacheBreakDetector::new();
        let snap1 = make_snapshot("original system", "[{\"name\":\"tool1\"}]", "claude-3");
        detector.post_call_analyze(&snap1, &usage_with_cache(5000));

        let snap2 = make_snapshot("changed system", "[{\"name\":\"tool1\"}]", "claude-3");
        let report = detector
            .post_call_analyze(&snap2, &usage_with_cache(0))
            .unwrap();
        assert_eq!(report.cause, BreakCause::SystemPromptChanged);
    }

    #[test]
    fn diagnoses_tools_change() {
        let mut detector = CacheBreakDetector::new();
        let snap1 = make_snapshot("system", "[{\"name\":\"tool1\"}]", "claude-3");
        detector.post_call_analyze(&snap1, &usage_with_cache(5000));

        let snap2 = make_snapshot(
            "system",
            "[{\"name\":\"tool1\"},{\"name\":\"tool2\"}]",
            "claude-3",
        );
        let report = detector
            .post_call_analyze(&snap2, &usage_with_cache(0))
            .unwrap();
        match report.cause {
            BreakCause::ToolsChanged {
                prev_count,
                curr_count,
            } => {
                assert_eq!(prev_count, 1);
                assert_eq!(curr_count, 2);
            }
            _ => panic!("expected ToolsChanged, got {:?}", report.cause),
        }
    }

    #[test]
    fn diagnoses_model_switch() {
        let mut detector = CacheBreakDetector::new();
        let snap1 = make_snapshot("system", "[]", "claude-3-opus");
        detector.post_call_analyze(&snap1, &usage_with_cache(5000));

        let snap2 = make_snapshot("system", "[]", "claude-3-sonnet");
        let report = detector
            .post_call_analyze(&snap2, &usage_with_cache(0))
            .unwrap();
        match report.cause {
            BreakCause::ModelSwitched { ref from, ref to } => {
                assert_eq!(from, "claude-3-opus");
                assert_eq!(to, "claude-3-sonnet");
            }
            _ => panic!("expected ModelSwitched, got {:?}", report.cause),
        }
    }

    fn snapshot_with_history(
        system: &str,
        tools: &str,
        model: &str,
        msg_hashes: Vec<u64>,
    ) -> PreCallSnapshot {
        let mut s = make_snapshot(system, tools, model);
        s.message_prefix_hashes = msg_hashes;
        s
    }

    #[test]
    fn clean_append_is_not_history_change() {
        // prev hashes are a prefix of curr (an assistant+user turn was appended).
        let mut detector = CacheBreakDetector::new();
        let snap1 = snapshot_with_history("sys", "[]", "m", vec![1, 2, 3]);
        detector.post_call_analyze(&snap1, &usage_with_cache(5000));

        let snap2 = snapshot_with_history("sys", "[]", "m", vec![1, 2, 3, 4, 5]);
        let report = detector
            .post_call_analyze(&snap2, &usage_with_cache(0))
            .unwrap();
        // No history/system/tools/model change → Unknown (genuine provider miss).
        assert_eq!(report.cause, BreakCause::Unknown);
    }

    #[test]
    fn compaction_diverges_prefix_and_is_expected() {
        // Compaction rewrote the prefix: hashes diverge (not an append).
        let mut detector = CacheBreakDetector::new();
        let snap1 = snapshot_with_history("sys", "[]", "m", vec![1, 2, 3, 4, 5]);
        detector.post_call_analyze(&snap1, &usage_with_cache(5000));

        let snap2 = snapshot_with_history("sys", "[]", "m", vec![1, 99, 6]);
        let report = detector
            .post_call_analyze(&snap2, &usage_with_cache(0))
            .unwrap();
        match report.cause {
            BreakCause::HistoryChanged { prev_len, curr_len } => {
                assert_eq!(prev_len, 5);
                assert_eq!(curr_len, 3);
            }
            other => panic!("expected HistoryChanged, got {other:?}"),
        }
        // Compaction is expected → not counted as a break.
        assert!(report.is_expected);
        assert_eq!(detector.total_breaks(), 0);
    }

    #[test]
    fn history_change_plus_system_change_is_unexpected() {
        // Prefix diverged AND system prompt changed → real regression, warns.
        let mut detector = CacheBreakDetector::new();
        let snap1 = snapshot_with_history("sys", "[]", "m", vec![1, 2, 3]);
        detector.post_call_analyze(&snap1, &usage_with_cache(5000));

        let snap2 = snapshot_with_history("new sys", "[]", "m", vec![9, 8]);
        let report = detector
            .post_call_analyze(&snap2, &usage_with_cache(0))
            .unwrap();
        match report.cause {
            BreakCause::Multiple(ref causes) => {
                assert!(causes
                    .iter()
                    .any(|c| matches!(c, BreakCause::HistoryChanged { .. })));
                assert!(causes
                    .iter()
                    .any(|c| matches!(c, BreakCause::SystemPromptChanged)));
            }
            other => panic!("expected Multiple, got {other:?}"),
        }
        assert!(!report.is_expected);
        assert_eq!(detector.total_breaks(), 1);
    }

    #[test]
    fn cache_edits_eviction_is_expected() {
        let mut detector = CacheBreakDetector::new();
        let snap1 = make_snapshot("system", "[]", "claude-3");
        detector.post_call_analyze(&snap1, &usage_with_cache(5000));

        // Same system/tools/model but cache_edits is active
        let snap2 = detector.pre_call_snapshot("system", "[]", "claude-3", false, true);
        let report = detector
            .post_call_analyze(&snap2, &usage_with_cache(0))
            .unwrap();
        assert_eq!(report.cause, BreakCause::CacheEditsEviction);
        assert!(report.is_expected);
        // Expected evictions don't count as breaks
        assert_eq!(detector.total_breaks(), 0);
    }

    #[test]
    fn multiple_causes_detected() {
        let mut detector = CacheBreakDetector::new();
        let snap1 = make_snapshot("system", "[{\"name\":\"t1\"}]", "claude-3");
        detector.post_call_analyze(&snap1, &usage_with_cache(5000));

        let snap2 = make_snapshot(
            "new system",
            "[{\"name\":\"t1\"},{\"name\":\"t2\"}]",
            "claude-4",
        );
        let report = detector
            .post_call_analyze(&snap2, &usage_with_cache(0))
            .unwrap();
        match report.cause {
            BreakCause::Multiple(ref causes) => {
                assert!(causes.len() >= 2);
                assert!(causes.contains(&BreakCause::SystemPromptChanged));
                assert!(causes
                    .iter()
                    .any(|c| matches!(c, BreakCause::ModelSwitched { .. })));
            }
            _ => panic!("expected Multiple, got {:?}", report.cause),
        }
    }

    #[test]
    fn cache_hit_rate_tracking() {
        let mut detector = CacheBreakDetector::new();
        let snap = make_snapshot("system", "[]", "claude-3");

        // 3 calls with cache, 1 break
        detector.post_call_analyze(&snap, &usage_with_cache(5000));
        detector.post_call_analyze(&snap, &usage_with_cache(4800));
        detector.post_call_analyze(&snap, &usage_with_cache(4600));

        let snap_changed = make_snapshot("new system", "[]", "claude-3");
        detector.post_call_analyze(&snap_changed, &usage_with_cache(0));

        assert_eq!(detector.total_calls(), 4);
        assert_eq!(detector.total_breaks(), 1);
        let rate = detector.cache_hit_rate().unwrap();
        assert!((rate - 0.75).abs() < 0.01);
    }

    #[test]
    fn report_summary_format() {
        let report = CacheBreakReport {
            cause: BreakCause::SystemPromptChanged,
            prev_cache_read_tokens: 5000,
            curr_cache_read_tokens: 0,
            is_expected: false,
        };
        let summary = report.summary();
        assert!(summary.contains("system prompt changed"));
        assert!(summary.contains("5000"));
        assert!(summary.contains("0"));
    }

    #[test]
    fn no_break_when_prev_cache_was_zero() {
        let mut detector = CacheBreakDetector::new();
        let snap1 = make_snapshot("system", "[]", "claude-3");
        detector.post_call_analyze(&snap1, &usage_with_cache(0));

        let snap2 = make_snapshot("new system", "[]", "claude-3");
        let report = detector.post_call_analyze(&snap2, &usage_with_cache(0));
        // No break: prev was already 0
        assert!(report.is_none());
    }
}
