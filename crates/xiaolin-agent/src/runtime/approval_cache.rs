use std::collections::HashMap;

use xiaolin_protocol::ApprovalDecision;

/// Session-scoped approval cache.
///
/// Stores `ApprovedForSession` decisions keyed by a canonical string
/// derived from tool-specific approval keys. The orchestrator checks this
/// cache before prompting the user — if all keys match a prior approval,
/// the tool call is silently allowed.
///
/// Also supports tool-level approval: when a tool-level key like
/// `"tool_session:shell_exec"` is stored, ALL subsequent calls to that
/// tool type are auto-approved for the session.
#[derive(Debug, Default)]
pub struct ApprovalCache {
    decisions: HashMap<String, ApprovalDecision>,
}

impl ApprovalCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if all provided keys have a cached `ApprovedForSession` decision,
    /// OR if a tool-level key covers this tool type.
    pub fn check(&self, keys: &[String]) -> Option<ApprovalDecision> {
        if keys.is_empty() {
            return None;
        }
        // Check tool-level approval first (e.g., "tool_session:shell_exec")
        for k in keys {
            if let Some(tool_type) = k.split(':').next() {
                let tool_key = format!("tool_session:{tool_type}");
                if matches!(
                    self.decisions.get(&tool_key),
                    Some(ApprovalDecision::ApprovedForSession)
                ) {
                    return Some(ApprovalDecision::ApprovedForSession);
                }
            }
        }
        // Check specific key approval
        let all_approved = keys.iter().all(|k| {
            matches!(
                self.decisions.get(k),
                Some(ApprovalDecision::ApprovedForSession)
            )
        });
        if all_approved {
            Some(ApprovalDecision::ApprovedForSession)
        } else {
            None
        }
    }

    /// Store a decision for the given keys. Only `ApprovedForSession` is cached;
    /// other decisions are not stored (they're one-shot).
    pub fn store(&mut self, keys: &[String], decision: ApprovalDecision) {
        if decision == ApprovalDecision::ApprovedForSession {
            for key in keys {
                self.decisions.insert(key.clone(), decision.clone());
                // Also store tool-level key so future calls to the same tool type
                // are auto-approved (e.g., "shell:/path:cmd" → "tool_session:shell")
                if let Some(tool_type) = key.split(':').next() {
                    let tool_key = format!("tool_session:{tool_type}");
                    self.decisions.insert(tool_key, decision.clone());
                }
            }
        }
    }

    /// Clear all cached decisions (e.g. on session end).
    pub fn clear(&mut self) {
        self.decisions.clear();
    }

    /// Number of cached approval entries.
    pub fn len(&self) -> usize {
        self.decisions.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.decisions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_keys_returns_none() {
        let cache = ApprovalCache::new();
        assert_eq!(cache.check(&[]), None);
    }

    #[test]
    fn store_and_check_approved_for_session() {
        let mut cache = ApprovalCache::new();
        let keys = vec!["shell:ls:/tmp".to_string()];
        cache.store(&keys, ApprovalDecision::ApprovedForSession);
        assert_eq!(cache.check(&keys), Some(ApprovalDecision::ApprovedForSession));
    }

    #[test]
    fn non_session_approval_not_cached() {
        let mut cache = ApprovalCache::new();
        let keys = vec!["shell:rm:/tmp".to_string()];
        cache.store(&keys, ApprovalDecision::Approved);
        assert_eq!(cache.check(&keys), None);
    }

    #[test]
    fn tool_level_approval_covers_same_tool_type() {
        let mut cache = ApprovalCache::new();
        cache.store(&["cmd:a".to_string()], ApprovalDecision::ApprovedForSession);
        // After approving "cmd:a", all "cmd:*" keys are covered by tool-level key
        let keys = vec!["cmd:b".to_string()];
        assert_eq!(cache.check(&keys), Some(ApprovalDecision::ApprovedForSession));
    }

    #[test]
    fn different_tool_types_not_covered() {
        let mut cache = ApprovalCache::new();
        cache.store(&["shell:ls".to_string()], ApprovalDecision::ApprovedForSession);
        // "file_write:..." has a different tool type and should NOT be covered
        let keys = vec!["file_write:/tmp/foo".to_string()];
        assert_eq!(cache.check(&keys), None);
    }

    #[test]
    fn clear_removes_all() {
        let mut cache = ApprovalCache::new();
        cache.store(
            &["shell:ls".to_string()],
            ApprovalDecision::ApprovedForSession,
        );
        // Stores both "shell:ls" and "tool_session:shell"
        assert_eq!(cache.len(), 2);
        cache.clear();
        assert!(cache.is_empty());
    }
}
