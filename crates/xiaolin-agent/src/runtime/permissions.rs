use serde::{Deserialize, Serialize};

/// Rule effect: whether a matching rule allows or denies access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleEffect {
    Allow,
    Deny,
}

/// Rule scope: determines the priority/lifetime of a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleScope {
    /// Session-level rule (higher priority, expires with session).
    Session,
    /// Global rule (lower priority, persists across sessions).
    Global,
}

/// Matcher for determining which tools/resources a rule applies to.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuleMatcher {
    /// Exact tool name match.
    Exact { tool: String },
    /// Prefix match (e.g. "git:" matches "git:push", "git:pull").
    Prefix { prefix: String },
    /// Wildcard/glob pattern (e.g. "file_*").
    Wildcard { pattern: String },
}

impl RuleMatcher {
    pub fn matches(&self, tool_name: &str) -> bool {
        match self {
            Self::Exact { tool } => tool == tool_name,
            Self::Prefix { prefix } => tool_name.starts_with(prefix),
            Self::Wildcard { pattern } => wildcard_match(pattern, tool_name),
        }
    }
}

/// A single permission rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRule {
    pub matcher: RuleMatcher,
    pub effect: RuleEffect,
    pub scope: RuleScope,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Result of evaluating permissions for a tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    /// Access is allowed.
    Allowed,
    /// Access is denied with an optional reason.
    Denied(Option<String>),
    /// No matching rule found — falls through to default behavior.
    NoMatch,
}

/// Engine that evaluates a chain of permission rules.
#[derive(Debug, Clone, Default)]
pub struct PermissionRuleEngine {
    session_rules: Vec<PermissionRule>,
    global_rules: Vec<PermissionRule>,
}

impl PermissionRuleEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a rule. Automatically placed into session or global bucket.
    pub fn add_rule(&mut self, rule: PermissionRule) {
        match rule.scope {
            RuleScope::Session => self.session_rules.push(rule),
            RuleScope::Global => self.global_rules.push(rule),
        }
    }

    /// Add multiple rules.
    pub fn add_rules(&mut self, rules: Vec<PermissionRule>) {
        for rule in rules {
            self.add_rule(rule);
        }
    }

    /// Evaluate whether a tool call is permitted.
    /// Priority order:
    /// 1. Session deny rules (highest priority)
    /// 2. Session allow rules
    /// 3. Global deny rules
    /// 4. Global allow rules
    /// 5. NoMatch (fallback)
    ///
    /// Within the same scope, deny takes priority over allow.
    pub fn evaluate(&self, tool_name: &str) -> PermissionDecision {
        // Check session rules first (higher priority)
        if let Some(decision) = self.evaluate_scope(&self.session_rules, tool_name) {
            return decision;
        }

        // Then global rules
        if let Some(decision) = self.evaluate_scope(&self.global_rules, tool_name) {
            return decision;
        }

        PermissionDecision::NoMatch
    }

    fn evaluate_scope(
        &self,
        rules: &[PermissionRule],
        tool_name: &str,
    ) -> Option<PermissionDecision> {
        let mut has_allow = false;
        let mut allow_reason: Option<String> = None;

        for rule in rules {
            if !rule.matcher.matches(tool_name) {
                continue;
            }
            match rule.effect {
                RuleEffect::Deny => {
                    return Some(PermissionDecision::Denied(rule.reason.clone()));
                }
                RuleEffect::Allow => {
                    has_allow = true;
                    if allow_reason.is_none() {
                        allow_reason = rule.reason.clone();
                    }
                }
            }
        }

        if has_allow {
            Some(PermissionDecision::Allowed)
        } else {
            None
        }
    }

    /// Clear all session-scoped rules (e.g. when session ends).
    #[allow(dead_code)] // TODO(integrate): expose in CLI
    pub fn clear_session_rules(&mut self) {
        self.session_rules.clear();
    }

    /// Number of rules in the engine.
    #[allow(dead_code)] // TODO(integrate): expose in CLI
    pub fn rule_count(&self) -> usize {
        self.session_rules.len() + self.global_rules.len()
    }

    /// Get a detailed explanation of why a tool is allowed/denied,
    /// including which rule matched and its matcher.
    #[allow(dead_code)] // TODO(integrate): expose in CLI
    pub fn permission_explain(&self, tool_name: &str) -> String {
        if let Some((idx, rule, scope)) = self
            .find_matching_rule(&self.session_rules, tool_name, "session")
            .or_else(|| self.find_matching_rule(&self.global_rules, tool_name, "global"))
        {
            let matcher_desc = match &rule.matcher {
                RuleMatcher::Exact { tool } => format!("exact({})", tool),
                RuleMatcher::Prefix { prefix } => format!("prefix({}*)", prefix),
                RuleMatcher::Wildcard { pattern } => format!("wildcard({})", pattern),
            };
            let effect = match rule.effect {
                RuleEffect::Allow => "allowed",
                RuleEffect::Deny => "denied",
            };
            let reason_part = rule
                .reason
                .as_deref()
                .map(|r| format!(", reason: {r}"))
                .unwrap_or_default();
            format!(
                "'{tool_name}' is {effect} by {scope} rule #{idx} [{matcher_desc}]{reason_part}"
            )
        } else {
            format!("'{tool_name}' has no matching rule (default behavior applies)")
        }
    }

    fn find_matching_rule<'a>(
        &self,
        rules: &'a [PermissionRule],
        tool_name: &str,
        scope: &'static str,
    ) -> Option<(usize, &'a PermissionRule, &'static str)> {
        for (i, rule) in rules.iter().enumerate() {
            if rule.matcher.matches(tool_name) {
                return Some((i, rule, scope));
            }
        }
        None
    }
}

/// Simple wildcard matching (supports `*` and `?`).
fn wildcard_match(pattern: &str, text: &str) -> bool {
    super::hook_executor::glob_match(pattern, text)
}

// ── Convenience constructors (tests) ─────────────────────────────────

#[cfg(test)]
impl PermissionRule {
    pub fn allow_exact(tool: &str) -> Self {
        Self {
            matcher: RuleMatcher::Exact { tool: tool.into() },
            effect: RuleEffect::Allow,
            scope: RuleScope::Global,
            reason: None,
        }
    }

    pub fn deny_exact(tool: &str) -> Self {
        Self {
            matcher: RuleMatcher::Exact { tool: tool.into() },
            effect: RuleEffect::Deny,
            scope: RuleScope::Global,
            reason: None,
        }
    }

    pub fn allow_prefix(prefix: &str) -> Self {
        Self {
            matcher: RuleMatcher::Prefix {
                prefix: prefix.into(),
            },
            effect: RuleEffect::Allow,
            scope: RuleScope::Global,
            reason: None,
        }
    }

    pub fn deny_prefix(prefix: &str) -> Self {
        Self {
            matcher: RuleMatcher::Prefix {
                prefix: prefix.into(),
            },
            effect: RuleEffect::Deny,
            scope: RuleScope::Global,
            reason: None,
        }
    }

    pub fn with_scope(mut self, scope: RuleScope) -> Self {
        self.scope = scope;
        self
    }

    pub fn with_reason(mut self, reason: &str) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

// ── Shadowed Rule Detection ──────────────────────────────────────────

/// A warning about a rule that is shadowed (will never fire) by an earlier rule.
#[derive(Debug, Clone)]
#[allow(dead_code)] // TODO(integrate): call from load_permissions
pub struct ShadowWarning {
    pub shadowed_index: usize,
    pub shadowing_index: usize,
    pub message: String,
}

impl PermissionRuleEngine {
    /// Detect rules that are shadowed by earlier rules in the same scope.
    /// A deny rule is shadowed if an earlier deny rule with a broader matcher
    /// already covers the same tools. Similarly for allow rules.
    #[allow(dead_code)] // TODO(integrate): call from load_permissions
    pub fn detect_shadowed_rules(&self) -> Vec<ShadowWarning> {
        let mut warnings = Vec::new();
        Self::detect_in_scope(&self.global_rules, 0, &mut warnings);
        let offset = self.global_rules.len();
        Self::detect_in_scope(&self.session_rules, offset, &mut warnings);
        warnings
    }

    #[allow(dead_code)] // TODO(integrate): call from load_permissions
    fn detect_in_scope(
        rules: &[PermissionRule],
        index_offset: usize,
        warnings: &mut Vec<ShadowWarning>,
    ) {
        for i in 0..rules.len() {
            for j in (i + 1)..rules.len() {
                if rules[i].effect != rules[j].effect {
                    continue;
                }
                if Self::matcher_covers(&rules[i].matcher, &rules[j].matcher) {
                    warnings.push(ShadowWarning {
                        shadowing_index: index_offset + i,
                        shadowed_index: index_offset + j,
                        message: format!(
                            "Rule #{} shadows rule #{}: broader matcher with same effect",
                            index_offset + i,
                            index_offset + j
                        ),
                    });
                }
            }
        }
    }

    /// Check if matcher `a` covers all cases that matcher `b` would match.
    #[allow(dead_code)] // TODO(integrate): call from load_permissions
    fn matcher_covers(a: &RuleMatcher, b: &RuleMatcher) -> bool {
        match (a, b) {
            (RuleMatcher::Wildcard { pattern }, _) if pattern == "*" => true,
            (RuleMatcher::Prefix { prefix: a_pre }, RuleMatcher::Prefix { prefix: b_pre }) => {
                b_pre.starts_with(a_pre.as_str())
            }
            (RuleMatcher::Prefix { prefix }, RuleMatcher::Exact { tool }) => {
                tool.starts_with(prefix.as_str())
            }
            (RuleMatcher::Exact { tool: a_tool }, RuleMatcher::Exact { tool: b_tool }) => {
                a_tool == b_tool
            }
            _ => false,
        }
    }
}

// ── Denial Tracking ──────────────────────────────────────────────────

/// Records a user denial for a specific tool+pattern combination.
#[derive(Debug, Clone)]
#[allow(dead_code)] // TODO(integrate): use in confirm flow
pub struct DenialRecord {
    pub tool_name: String,
    pub input_pattern: String,
    pub timestamp: std::time::Instant,
}

/// Tracks denied permissions within a session to avoid re-asking.
#[derive(Debug, Clone, Default)]
pub struct DenialTracker {
    denials: Vec<DenialRecord>,
}

impl DenialTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a user denial.
    pub fn record_denial(&mut self, tool_name: &str, input_pattern: &str) {
        self.denials.push(DenialRecord {
            tool_name: tool_name.to_string(),
            input_pattern: input_pattern.to_string(),
            timestamp: std::time::Instant::now(),
        });
    }

    /// Check if a similar request was previously denied.
    pub fn is_denied(&self, tool_name: &str, input_pattern: &str) -> bool {
        self.denials
            .iter()
            .any(|d| d.tool_name == tool_name && d.input_pattern == input_pattern)
    }

    /// Check if any request for this tool was denied (broader match).
    pub fn is_tool_denied(&self, tool_name: &str) -> bool {
        self.denials.iter().any(|d| d.tool_name == tool_name)
    }

    /// Clear all denial records (e.g. on session end).
    pub fn clear(&mut self) {
        self.denials.clear();
    }

    /// Number of recorded denials.
    pub fn count(&self) -> usize {
        self.denials.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_works() {
        let matcher = RuleMatcher::Exact {
            tool: "shell_exec".into(),
        };
        assert!(matcher.matches("shell_exec"));
        assert!(!matcher.matches("shell_exec_v2"));
        assert!(!matcher.matches("file_read"));
    }

    #[test]
    fn prefix_match_works() {
        let matcher = RuleMatcher::Prefix {
            prefix: "git:".into(),
        };
        assert!(matcher.matches("git:push"));
        assert!(matcher.matches("git:pull"));
        assert!(matcher.matches("git:"));
        assert!(!matcher.matches("github_search"));
    }

    #[test]
    fn wildcard_match_works() {
        let matcher = RuleMatcher::Wildcard {
            pattern: "file_*".into(),
        };
        assert!(matcher.matches("file_read"));
        assert!(matcher.matches("file_write"));
        assert!(!matcher.matches("shell_exec"));
    }

    #[test]
    fn deny_takes_priority_over_allow_in_same_scope() {
        let mut engine = PermissionRuleEngine::new();
        engine.add_rule(PermissionRule::allow_exact("shell_exec"));
        engine.add_rule(PermissionRule::deny_exact("shell_exec").with_reason("dangerous"));

        assert_eq!(
            engine.evaluate("shell_exec"),
            PermissionDecision::Denied(Some("dangerous".into()))
        );
    }

    #[test]
    fn session_rules_override_global() {
        let mut engine = PermissionRuleEngine::new();
        engine.add_rule(PermissionRule::allow_exact("shell_exec"));
        engine.add_rule(
            PermissionRule::deny_exact("shell_exec")
                .with_scope(RuleScope::Session)
                .with_reason("session blocked"),
        );

        assert_eq!(
            engine.evaluate("shell_exec"),
            PermissionDecision::Denied(Some("session blocked".into()))
        );
    }

    #[test]
    fn session_allow_overrides_global_deny() {
        let mut engine = PermissionRuleEngine::new();
        engine.add_rule(PermissionRule::deny_exact("shell_exec"));
        engine.add_rule(PermissionRule::allow_exact("shell_exec").with_scope(RuleScope::Session));

        assert_eq!(engine.evaluate("shell_exec"), PermissionDecision::Allowed);
    }

    #[test]
    fn no_match_returns_nomatch() {
        let engine = PermissionRuleEngine::new();
        assert_eq!(engine.evaluate("any_tool"), PermissionDecision::NoMatch);
    }

    #[test]
    fn prefix_deny_blocks_subcommands() {
        let mut engine = PermissionRuleEngine::new();
        engine.add_rule(PermissionRule::deny_prefix("rm:").with_reason("destructive"));

        assert_eq!(
            engine.evaluate("rm:-rf"),
            PermissionDecision::Denied(Some("destructive".into()))
        );
        assert_eq!(
            engine.evaluate("rm:file.txt"),
            PermissionDecision::Denied(Some("destructive".into()))
        );
        assert_eq!(engine.evaluate("ls"), PermissionDecision::NoMatch);
    }

    #[test]
    fn wildcard_allow_permits_matching() {
        let mut engine = PermissionRuleEngine::new();
        engine.add_rule(PermissionRule {
            matcher: RuleMatcher::Wildcard {
                pattern: "file_*".into(),
            },
            effect: RuleEffect::Allow,
            scope: RuleScope::Global,
            reason: None,
        });

        assert_eq!(engine.evaluate("file_read"), PermissionDecision::Allowed);
        assert_eq!(engine.evaluate("file_write"), PermissionDecision::Allowed);
        assert_eq!(engine.evaluate("shell_exec"), PermissionDecision::NoMatch);
    }

    #[test]
    fn clear_session_rules_removes_session_only() {
        let mut engine = PermissionRuleEngine::new();
        engine.add_rule(PermissionRule::allow_exact("tool_a").with_scope(RuleScope::Session));
        engine.add_rule(PermissionRule::allow_exact("tool_b"));

        assert_eq!(engine.rule_count(), 2);
        engine.clear_session_rules();
        assert_eq!(engine.rule_count(), 1);
        assert_eq!(engine.evaluate("tool_a"), PermissionDecision::NoMatch);
        assert_eq!(engine.evaluate("tool_b"), PermissionDecision::Allowed);
    }

    #[test]
    fn permission_explain_messages() {
        let mut engine = PermissionRuleEngine::new();
        engine.add_rule(PermissionRule::allow_exact("safe_tool"));
        engine.add_rule(PermissionRule::deny_exact("bad_tool").with_reason("unsafe"));

        let safe_msg = engine.permission_explain("safe_tool");
        assert!(safe_msg.contains("allowed"), "got: {safe_msg}");
        assert!(safe_msg.contains("exact(safe_tool)"), "got: {safe_msg}");

        let bad_msg = engine.permission_explain("bad_tool");
        assert!(bad_msg.contains("denied"), "got: {bad_msg}");
        assert!(bad_msg.contains("unsafe"), "got: {bad_msg}");

        assert!(engine
            .permission_explain("unknown")
            .contains("no matching rule"));
    }

    #[test]
    fn multiple_rules_first_deny_wins() {
        let mut engine = PermissionRuleEngine::new();
        engine.add_rule(PermissionRule {
            matcher: RuleMatcher::Wildcard {
                pattern: "*".into(),
            },
            effect: RuleEffect::Allow,
            scope: RuleScope::Global,
            reason: Some("default allow".into()),
        });
        engine.add_rule(PermissionRule::deny_exact("dangerous").with_reason("blocked"));

        assert_eq!(
            engine.evaluate("dangerous"),
            PermissionDecision::Denied(Some("blocked".into()))
        );
        assert_eq!(engine.evaluate("safe"), PermissionDecision::Allowed);
    }

    #[test]
    fn serialization_roundtrip() {
        let rule = PermissionRule::deny_prefix("git:force-")
            .with_scope(RuleScope::Session)
            .with_reason("no force push");
        let json = serde_json::to_string(&rule).unwrap();
        let deserialized: PermissionRule = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.effect, RuleEffect::Deny);
        assert_eq!(deserialized.scope, RuleScope::Session);
        assert!(deserialized.matcher.matches("git:force-push"));
        assert!(!deserialized.matcher.matches("git:pull"));
    }

    // ── Denial Tracking Tests ─────────────────────────────────────────

    #[test]
    fn denial_tracker_records_and_checks() {
        let mut tracker = DenialTracker::new();
        assert!(!tracker.is_denied("shell_exec", "rm -rf /"));

        tracker.record_denial("shell_exec", "rm -rf /");
        assert!(tracker.is_denied("shell_exec", "rm -rf /"));
        assert!(!tracker.is_denied("shell_exec", "ls"));
        assert_eq!(tracker.count(), 1);
    }

    #[test]
    fn denial_tracker_session_scoped() {
        let mut tracker = DenialTracker::new();
        tracker.record_denial("shell_exec", "dangerous");
        tracker.record_denial("file_write", "/etc/passwd");
        assert_eq!(tracker.count(), 2);

        tracker.clear();
        assert_eq!(tracker.count(), 0);
        assert!(!tracker.is_denied("shell_exec", "dangerous"));
    }

    #[test]
    fn denial_tracker_tool_level_check() {
        let mut tracker = DenialTracker::new();
        tracker.record_denial("shell_exec", "rm -rf /");

        assert!(tracker.is_tool_denied("shell_exec"));
        assert!(!tracker.is_tool_denied("file_read"));
    }

    // ── Shadow Detection Tests ──

    #[test]
    fn shadow_detect_wildcard_covers_exact() {
        let mut engine = PermissionRuleEngine::new();
        engine.add_rule(PermissionRule {
            matcher: RuleMatcher::Wildcard {
                pattern: "*".into(),
            },
            effect: RuleEffect::Allow,
            scope: RuleScope::Global,
            reason: None,
        });
        engine.add_rule(PermissionRule::allow_exact("read_file"));
        let warnings = engine.detect_shadowed_rules();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].shadowed_index, 1);
        assert_eq!(warnings[0].shadowing_index, 0);
    }

    #[test]
    fn shadow_detect_prefix_covers_exact() {
        let mut engine = PermissionRuleEngine::new();
        engine.add_rule(PermissionRule {
            matcher: RuleMatcher::Prefix {
                prefix: "file_".into(),
            },
            effect: RuleEffect::Deny,
            scope: RuleScope::Global,
            reason: None,
        });
        engine.add_rule(PermissionRule::deny_exact("file_write"));
        let warnings = engine.detect_shadowed_rules();
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn shadow_detect_no_false_positive_different_effects() {
        let mut engine = PermissionRuleEngine::new();
        engine.add_rule(PermissionRule {
            matcher: RuleMatcher::Wildcard {
                pattern: "*".into(),
            },
            effect: RuleEffect::Allow,
            scope: RuleScope::Global,
            reason: None,
        });
        engine.add_rule(PermissionRule::deny_exact("bad_tool"));
        let warnings = engine.detect_shadowed_rules();
        assert!(warnings.is_empty());
    }

    #[test]
    fn shadow_detect_no_warning_for_unrelated_rules() {
        let mut engine = PermissionRuleEngine::new();
        engine.add_rule(PermissionRule::allow_exact("tool_a"));
        engine.add_rule(PermissionRule::allow_exact("tool_b"));
        let warnings = engine.detect_shadowed_rules();
        assert!(warnings.is_empty());
    }

    #[test]
    fn explain_includes_scope_and_matcher() {
        let mut engine = PermissionRuleEngine::new();
        engine.add_rule(PermissionRule {
            matcher: RuleMatcher::Prefix {
                prefix: "file_".into(),
            },
            effect: RuleEffect::Allow,
            scope: RuleScope::Global,
            reason: Some("file ops allowed".into()),
        });
        let msg = engine.permission_explain("file_read");
        assert!(msg.contains("global"), "got: {msg}");
        assert!(msg.contains("prefix(file_*)"), "got: {msg}");
        assert!(msg.contains("file ops allowed"), "got: {msg}");
    }
}
