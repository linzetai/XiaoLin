use std::path::Path;

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

    /// Load rules from a JSON file in the given directory.
    /// Looks for `settings.json5` or `settings.json`.
    pub fn load_from_dir(dir: &Path) -> anyhow::Result<Self> {
        let json5_path = dir.join("settings.json5");
        let json_path = dir.join("settings.json");

        let content = if json5_path.exists() {
            std::fs::read_to_string(&json5_path)?
        } else if json_path.exists() {
            std::fs::read_to_string(&json_path)?
        } else {
            return Ok(Self::new());
        };

        let config: PermissionConfig = serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("failed to parse permission config: {e}"))?;

        let mut engine = Self::new();
        engine.add_rules(config.permissions);

        let shadows = engine.detect_shadowed_rules();
        for w in &shadows {
            tracing::warn!(
                shadowed = w.shadowed_index,
                by = w.shadowing_index,
                "{}",
                w.message
            );
        }

        Ok(engine)
    }

    /// Clear all session-scoped rules (e.g. when session ends).
    pub fn clear_session_rules(&mut self) {
        self.session_rules.clear();
    }

    /// Number of rules in the engine.
    pub fn rule_count(&self) -> usize {
        self.session_rules.len() + self.global_rules.len()
    }

    /// Get a detailed explanation of why a tool is allowed/denied,
    /// including which rule matched and its matcher.
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

/// Configuration file structure for permissions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PermissionConfig {
    #[serde(default)]
    permissions: Vec<PermissionRule>,
}

/// Simple wildcard matching (supports `*` and `?`).
fn wildcard_match(pattern: &str, text: &str) -> bool {
    super::hook_executor::glob_match(pattern, text)
}

// ── Convenience constructors ─────────────────────────────────────────

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

// ── Shell Rule Matching ──────────────────────────────────────────────

/// Parsed shell rule in the format "command:subcommand" or "command:-flag".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellRule {
    pub command: String,
    pub subcommand: Option<String>,
    pub flag: Option<String>,
}

impl ShellRule {
    /// Parse a shell rule string like "git:push", "rm:-rf", or "docker:compose:up".
    pub fn parse(rule: &str) -> Option<Self> {
        let parts: Vec<&str> = rule.splitn(3, ':').collect();
        if parts.is_empty() || parts[0].is_empty() {
            return None;
        }

        let command = parts[0].to_string();
        let (subcommand, flag) = if parts.len() >= 2 {
            let second = parts[1];
            if second.starts_with('-') {
                (None, Some(second.to_string()))
            } else {
                let sub = if parts.len() == 3 {
                    format!("{}:{}", second, parts[2])
                } else {
                    second.to_string()
                };
                (Some(sub), None)
            }
        } else {
            (None, None)
        };

        Some(Self {
            command,
            subcommand,
            flag,
        })
    }

    /// Check whether a shell command line matches this rule.
    /// Performs structural matching on parsed command components.
    pub fn matches_command(&self, command_line: &str) -> bool {
        let tokens = shell_tokenize(command_line);
        if tokens.is_empty() {
            return false;
        }

        // Match the base command (may be a path like /usr/bin/git → "git")
        let base_cmd = extract_command_name(&tokens[0]);
        if base_cmd != self.command {
            return false;
        }

        // If we have a subcommand requirement, check it's present
        if let Some(sub) = &self.subcommand {
            let non_flag_args: Vec<&str> = tokens[1..]
                .iter()
                .filter(|t| !t.starts_with('-'))
                .map(|s| s.as_str())
                .collect();

            if sub.contains(':') {
                // Multi-level subcommand like "compose:up"
                let sub_parts: Vec<&str> = sub.split(':').collect();
                if non_flag_args.len() < sub_parts.len() {
                    return false;
                }
                for (i, sp) in sub_parts.iter().enumerate() {
                    if non_flag_args.get(i) != Some(sp) {
                        return false;
                    }
                }
            } else if !non_flag_args.contains(&sub.as_str()) {
                return false;
            }
        }

        // If we have a flag requirement, check it's present
        if let Some(flag) = &self.flag {
            let has_flag = tokens[1..]
                .iter()
                .any(|t| t == flag || (flag.len() > 2 && t.contains(&flag[1..])));
            if !has_flag {
                return false;
            }
        }

        true
    }
}

// ── Auto Mode Classifier ─────────────────────────────────────────────

/// Safety classification result from LLM side-query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SafetyDecision {
    Safe,
    Unsafe,
    NeedsConfirmation,
}

/// Result of auto-mode classification.
#[derive(Debug, Clone)]
pub struct AutoClassification {
    pub decision: SafetyDecision,
    pub reason: String,
}

impl AutoClassification {
    /// Parse an LLM response into a classification.
    /// Expected format: "SAFE: <reason>" or "UNSAFE: <reason>".
    pub fn parse_response(response: &str) -> Self {
        let trimmed = response.trim();
        if trimmed.is_empty() {
            return Self {
                decision: SafetyDecision::NeedsConfirmation,
                reason: "empty response from classifier".into(),
            };
        }

        let upper = trimmed.to_uppercase();
        if upper.starts_with("SAFE") {
            let reason = trimmed
                .get(4..)
                .map(|s| s.trim_start_matches(':').trim())
                .unwrap_or("")
                .to_string();
            Self {
                decision: SafetyDecision::Safe,
                reason: if reason.is_empty() {
                    "classified as safe".into()
                } else {
                    reason
                },
            }
        } else if upper.starts_with("UNSAFE") {
            let reason = trimmed
                .get(6..)
                .map(|s| s.trim_start_matches(':').trim())
                .unwrap_or("")
                .to_string();
            Self {
                decision: SafetyDecision::Unsafe,
                reason: if reason.is_empty() {
                    "classified as unsafe".into()
                } else {
                    reason
                },
            }
        } else {
            Self {
                decision: SafetyDecision::NeedsConfirmation,
                reason: format!(
                    "unrecognized classification: {}",
                    &trimmed[..trimmed.len().min(100)]
                ),
            }
        }
    }
}

/// Auto-mode classifier that uses LLM side-query to determine if a
/// command is safe to execute without user confirmation.
pub struct AutoModeClassifier {
    _placeholder: (),
}

impl AutoModeClassifier {
    pub fn new() -> Self {
        Self { _placeholder: () }
    }

    /// Build the LLM prompt for classifying a command's safety.
    pub fn build_classification_prompt(command: &str, context: &str) -> String {
        format!(
            "You are a security classifier. Determine if the following command is safe to \
             execute automatically without user confirmation.\n\n\
             Command: {command}\n\
             Context: {context}\n\n\
             Rules:\n\
             - Commands that only READ data are SAFE\n\
             - Commands that WRITE/DELETE/MODIFY important data are UNSAFE\n\
             - Commands that affect system configuration are UNSAFE\n\
             - Network requests to trusted APIs are generally SAFE\n\
             - Commands with irreversible effects (rm -rf, git push --force) are UNSAFE\n\n\
             Respond with exactly one line in the format:\n\
             SAFE: <brief reason>\n\
             or\n\
             UNSAFE: <brief reason>"
        )
    }

    /// Classify a command using the provided LLM provider.
    /// Returns NeedsConfirmation on any failure (safe fallback).
    pub async fn classify(
        &self,
        command: &str,
        context: &str,
        provider: &std::sync::Arc<dyn crate::llm::LlmProvider>,
        model: &str,
    ) -> AutoClassification {
        use crate::llm::CompletionParams;
        use fastclaw_core::types::{ChatMessage, Role};

        let prompt = Self::build_classification_prompt(command, context);
        let messages = vec![ChatMessage {
            role: Role::User,
            content: Some(serde_json::Value::String(prompt)),
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
            compact_metadata: None,
        }];

        let params = CompletionParams {
            model,
            messages: &messages,
            max_tokens: Some(100),
            temperature: 0.0,
            tools: None,
        };

        match provider.chat_completion(&params).await {
            Ok(response) => {
                let text = response
                    .choices
                    .first()
                    .and_then(|c| c.message.content.as_ref())
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                AutoClassification::parse_response(text)
            }
            Err(_) => AutoClassification {
                decision: SafetyDecision::NeedsConfirmation,
                reason: "classifier LLM call failed; falling back to confirmation".into(),
            },
        }
    }
}

impl Default for AutoModeClassifier {
    fn default() -> Self {
        Self::new()
    }
}

// ── Shadowed Rule Detection ──────────────────────────────────────────

/// A warning about a rule that is shadowed (will never fire) by an earlier rule.
#[derive(Debug, Clone)]
pub struct ShadowWarning {
    pub shadowed_index: usize,
    pub shadowing_index: usize,
    pub message: String,
}

impl PermissionRuleEngine {
    /// Detect rules that are shadowed by earlier rules in the same scope.
    /// A deny rule is shadowed if an earlier deny rule with a broader matcher
    /// already covers the same tools. Similarly for allow rules.
    pub fn detect_shadowed_rules(&self) -> Vec<ShadowWarning> {
        let mut warnings = Vec::new();
        Self::detect_in_scope(&self.global_rules, 0, &mut warnings);
        let offset = self.global_rules.len();
        Self::detect_in_scope(&self.session_rules, offset, &mut warnings);
        warnings
    }

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

/// Basic shell tokenization (splits on whitespace, respects quotes).
fn shell_tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    for ch in input.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if !in_single_quote => escaped = true,
            '\'' if !in_double_quote => in_single_quote = !in_single_quote,
            '"' if !in_single_quote => in_double_quote = !in_double_quote,
            ' ' | '\t' if !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Extract the command name from a potentially full path.
fn extract_command_name(token: &str) -> &str {
    token.rsplit('/').next().unwrap_or(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

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
    fn load_from_dir_parses_json() {
        let dir = tempfile::tempdir().unwrap();
        let config = PermissionConfig {
            permissions: vec![
                PermissionRule::allow_exact("file_read"),
                PermissionRule::deny_prefix("rm:").with_reason("no deletion"),
            ],
        };
        let json = serde_json::to_string_pretty(&config).unwrap();
        let mut f = std::fs::File::create(dir.path().join("settings.json")).unwrap();
        f.write_all(json.as_bytes()).unwrap();

        let engine = PermissionRuleEngine::load_from_dir(dir.path()).unwrap();
        assert_eq!(engine.rule_count(), 2);
        assert_eq!(engine.evaluate("file_read"), PermissionDecision::Allowed);
        assert_eq!(
            engine.evaluate("rm:-rf"),
            PermissionDecision::Denied(Some("no deletion".into()))
        );
    }

    #[test]
    fn load_from_dir_returns_empty_when_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PermissionRuleEngine::load_from_dir(dir.path()).unwrap();
        assert_eq!(engine.rule_count(), 0);
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

    // ── Auto Mode Classifier Tests ────────────────────────────────────

    #[test]
    fn safety_classification_parse_safe() {
        let result = AutoClassification::parse_response("SAFE: this is a read-only command");
        assert_eq!(result.decision, SafetyDecision::Safe);
        assert!(result.reason.contains("read-only"));
    }

    #[test]
    fn safety_classification_parse_unsafe() {
        let result = AutoClassification::parse_response("UNSAFE: deletes important files");
        assert_eq!(result.decision, SafetyDecision::Unsafe);
        assert!(result.reason.contains("deletes"));
    }

    #[test]
    fn safety_classification_parse_unknown_defaults_confirm() {
        let result = AutoClassification::parse_response("not sure about this");
        assert_eq!(result.decision, SafetyDecision::NeedsConfirmation);
    }

    #[test]
    fn safety_classification_parse_empty() {
        let result = AutoClassification::parse_response("");
        assert_eq!(result.decision, SafetyDecision::NeedsConfirmation);
    }

    #[test]
    fn auto_classifier_prompt_contains_command() {
        let prompt =
            AutoModeClassifier::build_classification_prompt("rm -rf /", "user wants cleanup");
        assert!(prompt.contains("rm -rf /"));
        assert!(prompt.contains("user wants cleanup"));
    }

    // ── Shell Rule Matching Tests ────────────────────────────────────

    #[test]
    fn shell_rule_parse_basic() {
        let rule = ShellRule::parse("git:push").unwrap();
        assert_eq!(rule.command, "git");
        assert_eq!(rule.subcommand, Some("push".into()));
        assert_eq!(rule.flag, None);
    }

    #[test]
    fn shell_rule_parse_flag() {
        let rule = ShellRule::parse("rm:-rf").unwrap();
        assert_eq!(rule.command, "rm");
        assert_eq!(rule.subcommand, None);
        assert_eq!(rule.flag, Some("-rf".into()));
    }

    #[test]
    fn shell_rule_parse_command_only() {
        let rule = ShellRule::parse("curl").unwrap();
        assert_eq!(rule.command, "curl");
        assert_eq!(rule.subcommand, None);
        assert_eq!(rule.flag, None);
    }

    #[test]
    fn shell_rule_git_push_matches() {
        let rule = ShellRule::parse("git:push").unwrap();
        assert!(rule.matches_command("git push origin main"));
        assert!(rule.matches_command("git push"));
        assert!(!rule.matches_command("git pull"));
        assert!(!rule.matches_command("git commit -m 'msg'"));
    }

    #[test]
    fn shell_rule_rm_rf_matches() {
        let rule = ShellRule::parse("rm:-rf").unwrap();
        assert!(rule.matches_command("rm -rf /tmp/foo"));
        assert!(rule.matches_command("rm -rf ."));
        assert!(!rule.matches_command("rm file.txt"));
        assert!(!rule.matches_command("rm -r dir/"));
    }

    #[test]
    fn shell_rule_path_command_matches() {
        let rule = ShellRule::parse("git:push").unwrap();
        assert!(rule.matches_command("/usr/bin/git push"));
    }

    #[test]
    fn shell_rule_quoted_args() {
        let rule = ShellRule::parse("git:commit").unwrap();
        assert!(rule.matches_command("git commit -m 'hello world'"));
        assert!(rule.matches_command("git commit --amend"));
        assert!(!rule.matches_command("git push"));
    }

    #[test]
    fn shell_tokenize_handles_quotes() {
        let tokens = shell_tokenize("echo 'hello world' \"foo bar\"");
        assert_eq!(tokens, vec!["echo", "hello world", "foo bar"]);
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
