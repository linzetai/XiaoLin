pub mod amend;
mod config;
pub mod executable_name;
mod matcher;

pub use config::{
    normalize_network_rule_host, Defaults, NetworkRule, NetworkRuleProtocol, PatternElement,
    PolicyConfig, PolicyTest, PrefixRule,
};

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Decision from policy evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    /// Command is explicitly allowed.
    Allow { rule_id: Option<String> },
    /// Command is explicitly forbidden.
    Forbidden {
        rule_id: Option<String>,
        justification: String,
    },
    /// Command needs human or Guardian confirmation.
    Prompt {
        rule_id: Option<String>,
        reason: String,
    },
}

impl PolicyDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow { .. })
    }

    pub fn is_forbidden(&self) -> bool {
        matches!(self, Self::Forbidden { .. })
    }

    /// Priority ordering: Forbidden(3) > Prompt(2) > Allow(1).
    fn priority(&self) -> u8 {
        match self {
            Self::Allow { .. } => 1,
            Self::Prompt { .. } => 2,
            Self::Forbidden { .. } => 3,
        }
    }
}

/// A rule that matched during evaluation.
#[derive(Debug, Clone)]
pub struct MatchedRule {
    /// The rule identifier (if any).
    pub rule_id: Option<String>,
    /// The prefix tokens that matched.
    pub matched_prefix: Vec<String>,
    /// Justification from the rule.
    pub justification: Option<String>,
    /// Whether this match came from a heuristic fallback rather than an
    /// explicit policy rule.
    pub is_heuristic: bool,
}

/// Full evaluation result including decision and all matched rules.
#[derive(Debug, Clone)]
pub struct Evaluation {
    /// The final decision (highest priority among matched rules).
    pub decision: PolicyDecision,
    /// All rules that matched, in evaluation order.
    pub matched_rules: Vec<MatchedRule>,
}

impl Evaluation {
    pub fn is_allowed(&self) -> bool {
        self.decision.is_allowed()
    }

    pub fn is_forbidden(&self) -> bool {
        self.decision.is_forbidden()
    }
}

/// Options that control how command evaluation resolves executables.
#[derive(Debug, Clone, Default)]
pub struct MatchOptions {
    /// When `true`, if the first token is an absolute path (e.g.
    /// `/usr/bin/git`), the engine extracts the basename (`git`) and
    /// retries matching against rules keyed by that basename.
    pub resolve_host_executables: bool,
}

/// The policy engine: evaluates commands against layered rules.
pub struct PolicyEngine {
    /// Whether at least one policy layer was successfully loaded.
    loaded: bool,
    /// Rules from highest to lowest priority: session, project, system.
    layers: Vec<PolicyLayer>,
    /// Known host executables by basename, for `resolve_host_executables` matching.
    host_executables_by_name: HashMap<String, Vec<PathBuf>>,
    /// Default decision when no rule matches (from lowest-priority layer's `[defaults]`).
    defaults_fallback: String,
}

#[derive(Clone)]
struct PolicyLayer {
    name: String,
    /// Rules indexed by their first token for O(1) lookup.
    rules_by_first_token: HashMap<String, Vec<PrefixRule>>,
    /// Rules with empty patterns or alternative first tokens that need
    /// special handling.
    catch_all_rules: Vec<PrefixRule>,
    network_rules: Vec<NetworkRule>,
}

impl PolicyLayer {
    fn new(name: String, prefix_rules: Vec<PrefixRule>, network_rules: Vec<NetworkRule>) -> Self {
        let mut rules_by_first_token: HashMap<String, Vec<PrefixRule>> = HashMap::new();
        let mut catch_all_rules: Vec<PrefixRule> = Vec::new();

        for rule in prefix_rules {
            match rule.pattern.first() {
                Some(config::PatternElement::Exact(token)) => {
                    rules_by_first_token
                        .entry(token.clone())
                        .or_default()
                        .push(rule);
                }
                Some(config::PatternElement::Alternatives(alts)) => {
                    for alt in alts {
                        rules_by_first_token
                            .entry(alt.clone())
                            .or_default()
                            .push(rule.clone());
                    }
                }
                None => {
                    catch_all_rules.push(rule);
                }
            }
        }

        Self {
            name,
            rules_by_first_token,
            catch_all_rules,
            network_rules,
        }
    }

    fn matching_rules<'a>(&'a self, first_token: Option<&str>) -> Vec<&'a PrefixRule> {
        let mut result: Vec<&PrefixRule> = Vec::new();

        if let Some(token) = first_token {
            if let Some(rules) = self.rules_by_first_token.get(token) {
                result.extend(rules.iter());
            }
        }
        result.extend(self.catch_all_rules.iter());

        result
    }

    fn rule_count(&self) -> usize {
        let indexed: usize = self.rules_by_first_token.values().map(|v| v.len()).sum();
        indexed + self.catch_all_rules.len()
    }
}

impl PolicyEngine {
    /// Create a new engine with no rules.
    pub fn new() -> Self {
        Self {
            loaded: false,
            layers: Vec::new(),
            host_executables_by_name: HashMap::new(),
            defaults_fallback: "forbidden".to_string(),
        }
    }

    /// Whether any policy layer has been successfully loaded.
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Load policy from a TOML file, adding it as a named layer.
    pub fn load_file(&mut self, path: &Path, layer_name: &str) -> anyhow::Result<()> {
        let content = std::fs::read_to_string(path)?;
        let config: PolicyConfig = toml::from_str(&content)?;
        self.add_layer(layer_name, config);
        Ok(())
    }

    /// Load policy from a TOML string.
    pub fn load_str(&mut self, toml_str: &str, layer_name: &str) -> anyhow::Result<()> {
        let config: PolicyConfig = toml::from_str(toml_str)?;
        self.add_layer(layer_name, config);
        Ok(())
    }

    /// Add a configuration as a new layer (appended at lowest priority).
    pub fn add_layer(&mut self, name: &str, config: PolicyConfig) {
        self.loaded = true;
        let mut prefix_rules = config.rules;

        if let Some(defaults) = &config.defaults {
            for cmd in &defaults.allow_readonly {
                prefix_rules.push(PrefixRule {
                    id: Some(format!("auto_readonly_{cmd}")),
                    pattern: vec![config::PatternElement::Exact(cmd.clone())],
                    decision: "allow".to_string(),
                    justification: None,
                });
            }
            self.defaults_fallback = defaults.fallback.clone();
        }

        self.layers.push(PolicyLayer::new(
            name.to_string(),
            prefix_rules,
            config.network,
        ));
    }

    /// Add a single rule at session level (prepended as highest priority).
    pub fn add_session_rule(&mut self, rule: PrefixRule) {
        if self.layers.is_empty() || self.layers[0].name != "session" {
            self.layers.insert(
                0,
                PolicyLayer::new("session".to_string(), vec![rule], Vec::new()),
            );
        } else {
            let layer = &mut self.layers[0];
            match rule.pattern.first() {
                Some(config::PatternElement::Exact(token)) => {
                    layer
                        .rules_by_first_token
                        .entry(token.clone())
                        .or_default()
                        .push(rule);
                }
                Some(config::PatternElement::Alternatives(alts)) => {
                    for alt in alts {
                        layer
                            .rules_by_first_token
                            .entry(alt.clone())
                            .or_default()
                            .push(rule.clone());
                    }
                }
                None => {
                    layer.catch_all_rules.push(rule);
                }
            }
        }
    }

    /// Evaluate a command and return a detailed `Evaluation` with all matched rules.
    pub fn evaluate(&self, command_tokens: &[&str]) -> Evaluation {
        if !self.loaded {
            return Evaluation {
                decision: PolicyDecision::Forbidden {
                    rule_id: None,
                    justification: "exec-policy not loaded".to_string(),
                },
                matched_rules: vec![],
            };
        }

        let first_token = command_tokens.first().copied();
        let mut best: Option<PolicyDecision> = None;
        let mut matched_rules: Vec<MatchedRule> = Vec::new();

        for layer in &self.layers {
            for rule in layer.matching_rules(first_token) {
                if matcher::matches_prefix(&rule.pattern, command_tokens) {
                    let decision = parse_decision(rule);

                    matched_rules.push(MatchedRule {
                        rule_id: rule.id.clone(),
                        matched_prefix: rule
                            .pattern
                            .iter()
                            .map(|p| match p {
                                config::PatternElement::Exact(s) => s.clone(),
                                config::PatternElement::Alternatives(alts) => alts.join("|"),
                            })
                            .collect(),
                        justification: rule.justification.clone(),
                        is_heuristic: false,
                    });

                    best = Some(match best {
                        None => decision,
                        Some(prev) => {
                            if decision.priority() > prev.priority() {
                                decision
                            } else {
                                prev
                            }
                        }
                    });

                    if best.as_ref().is_some_and(|d| d.is_forbidden()) {
                        return Evaluation {
                            decision: best.unwrap(),
                            matched_rules,
                        };
                    }
                }
            }
        }

        let decision = best.unwrap_or_else(|| parse_fallback_decision(&self.defaults_fallback));

        Evaluation {
            decision,
            matched_rules,
        }
    }

    /// Evaluate a command with a heuristic fallback.
    ///
    /// When no explicit policy rules match, `fallback` is called with the
    /// command tokens to produce a decision. The resulting `MatchedRule` is
    /// tagged with `is_heuristic: true`.
    pub fn evaluate_with_heuristics(
        &self,
        command_tokens: &[&str],
        fallback: impl Fn(&[&str]) -> PolicyDecision,
    ) -> Evaluation {
        if !self.loaded {
            return Evaluation {
                decision: PolicyDecision::Forbidden {
                    rule_id: None,
                    justification: "exec-policy not loaded".to_string(),
                },
                matched_rules: vec![],
            };
        }

        let eval = self.evaluate(command_tokens);
        if !eval.matched_rules.is_empty() {
            return eval;
        }

        let decision = fallback(command_tokens);
        let matched_rules = vec![MatchedRule {
            rule_id: None,
            matched_prefix: command_tokens.iter().map(|t| t.to_string()).collect(),
            justification: Some("heuristic fallback".to_string()),
            is_heuristic: true,
        }];

        Evaluation {
            decision,
            matched_rules,
        }
    }

    /// Evaluate a command with additional match options.
    ///
    /// When `options.resolve_host_executables` is true and the first token is
    /// an absolute path (e.g. `/usr/bin/git`), the engine extracts the basename
    /// and retries matching against rules keyed by that name.
    pub fn evaluate_with_options(
        &self,
        command_tokens: &[&str],
        options: &MatchOptions,
    ) -> Evaluation {
        let eval = self.evaluate(command_tokens);
        if !eval.matched_rules.is_empty() || !options.resolve_host_executables {
            return eval;
        }

        let first = match command_tokens.first() {
            Some(t) => *t,
            None => return eval,
        };

        if !first.starts_with('/') {
            return eval;
        }

        let Some(basename) = executable_name::executable_path_lookup_key(Path::new(first)) else {
            return eval;
        };

        let mut resolved_tokens: Vec<&str> = Vec::with_capacity(command_tokens.len());
        resolved_tokens.push(&basename);
        resolved_tokens.extend_from_slice(&command_tokens[1..]);

        // Intentionally leak the owned basename into the stack-local vec via
        // a raw str reference. This is safe because `resolved_tokens` does not
        // outlive `basename`.
        let eval2 = self.evaluate(&resolved_tokens);
        if eval2.matched_rules.is_empty() {
            eval
        } else {
            eval2
        }
    }

    /// Evaluate multiple commands and aggregate the results.
    ///
    /// All matched rules are collected and the highest-priority decision
    /// wins. This mirrors Codex's `check_multiple`.
    pub fn evaluate_multiple<'a>(
        &self,
        commands: impl IntoIterator<Item = &'a [&'a str]>,
    ) -> Evaluation {
        if !self.loaded {
            return Evaluation {
                decision: PolicyDecision::Forbidden {
                    rule_id: None,
                    justification: "exec-policy not loaded".to_string(),
                },
                matched_rules: vec![],
            };
        }

        let mut all_rules: Vec<MatchedRule> = Vec::new();
        let mut best: Option<PolicyDecision> = None;

        for cmd in commands {
            let eval = self.evaluate(cmd);
            for rule in &eval.matched_rules {
                all_rules.push(rule.clone());
            }
            let priority = eval.decision.priority();
            if best.as_ref().is_none_or(|b| priority > b.priority()) {
                best = Some(eval.decision);
            }
        }

        Evaluation {
            decision: best.unwrap_or_else(|| parse_fallback_decision(&self.defaults_fallback)),
            matched_rules: all_rules,
        }
    }

    /// Evaluate a network connection attempt.
    ///
    /// The `host` parameter is leniently normalized (scheme/path stripped for
    /// caller convenience). Rule hosts use strict normalization. The `protocol`
    /// parameter is matched against the typed `NetworkRuleProtocol`.
    pub fn evaluate_network(&self, host: &str, protocol: &str) -> PolicyDecision {
        if !self.loaded {
            return PolicyDecision::Forbidden {
                rule_id: None,
                justification: "exec-policy not loaded".to_string(),
            };
        }

        let normalized_host = lenient_normalize_host(host);

        for layer in &self.layers {
            for rule in &layer.network_rules {
                let is_catch_all = rule.host.trim() == "*";
                let rule_host_matches = if is_catch_all {
                    true
                } else {
                    let rule_host = config::normalize_network_rule_host(&rule.host)
                        .unwrap_or_else(|_| rule.host.to_lowercase());
                    rule_host == normalized_host
                };

                if rule_host_matches {
                    let proto_match = match &rule.protocol {
                        Some(p) => p.matches(protocol),
                        None => true,
                    };
                    if proto_match {
                        return match rule.decision.as_str() {
                            "allow" => PolicyDecision::Allow {
                                rule_id: rule.id.clone(),
                            },
                            "forbidden" | "deny" => PolicyDecision::Forbidden {
                                rule_id: rule.id.clone(),
                                justification: "network access denied by policy".to_string(),
                            },
                            _ => PolicyDecision::Prompt {
                                rule_id: rule.id.clone(),
                                reason: "network access requires confirmation".to_string(),
                            },
                        };
                    }
                }
            }
        }

        PolicyDecision::Forbidden {
            rule_id: None,
            justification: "no matching network rule (deny by default)".to_string(),
        }
    }

    /// Run inline tests. Supports both `expect` (positive) and `not_match`
    /// (negative) validation.
    pub fn validate_tests(&self, tests: &[PolicyTest]) -> Vec<String> {
        let mut errors = Vec::new();
        for test in tests {
            let tokens: Vec<&str> = test.command.split_whitespace().collect();
            let evaluation = self.evaluate(&tokens);
            let actual = match &evaluation.decision {
                PolicyDecision::Allow { .. } => "allow",
                PolicyDecision::Forbidden { .. } => "forbidden",
                PolicyDecision::Prompt { .. } => "prompt",
            };

            if actual != test.expect {
                errors.push(format!(
                    "Test failed: command='{}' expected='{}' got='{}'",
                    test.command, test.expect, actual
                ));
            }

            if test.not_match.unwrap_or(false) && !evaluation.matched_rules.is_empty() {
                errors.push(format!(
                    "Test failed: command='{}' expected no matching rules but got {}",
                    test.command,
                    evaluation.matched_rules.len()
                ));
            }
        }
        errors
    }

    /// Number of prefix rules across all layers.
    pub fn rule_count(&self) -> usize {
        self.layers.iter().map(|l| l.rule_count()).sum()
    }

    /// Set known host executables for a given basename.
    pub fn set_host_executable_paths(&mut self, name: String, paths: Vec<PathBuf>) {
        self.host_executables_by_name.insert(name, paths);
    }

    /// Access the host executables map.
    pub fn host_executables(&self) -> &HashMap<String, Vec<PathBuf>> {
        &self.host_executables_by_name
    }

    /// Collect all allowed command prefixes from all layers.
    ///
    /// Returns a deduplicated list of the first pattern element for every
    /// "allow" rule across all layers. Useful for building allowlists
    /// in external sandbox configurations.
    pub fn get_allowed_prefixes(&self) -> Vec<String> {
        let mut prefix_set: HashSet<String> = HashSet::new();
        for layer in &self.layers {
            for (token, rules) in &layer.rules_by_first_token {
                if rules.iter().any(|r| r.decision == "allow") {
                    prefix_set.insert(token.clone());
                }
            }
            for rule in &layer.catch_all_rules {
                if rule.decision == "allow" {
                    for element in &rule.pattern {
                        match element {
                            config::PatternElement::Exact(s) => {
                                prefix_set.insert(s.clone());
                            }
                            config::PatternElement::Alternatives(alts) => {
                                for alt in alts {
                                    prefix_set.insert(alt.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
        let mut prefixes: Vec<String> = prefix_set.into_iter().collect();
        prefixes.sort();
        prefixes
    }

    /// Merge an overlay engine into this one, returning a new combined engine.
    ///
    /// - Prefix rules from overlay layers are appended (lower priority).
    /// - Network rules from overlay layers are appended.
    /// - Host executables from overlay override by name.
    #[must_use]
    pub fn merge_overlay(&self, overlay: &PolicyEngine) -> PolicyEngine {
        let mut combined_layers = self.layers.clone();
        for overlay_layer in &overlay.layers {
            let existing = combined_layers
                .iter_mut()
                .find(|l| l.name == overlay_layer.name);
            if let Some(target) = existing {
                for (token, rules) in &overlay_layer.rules_by_first_token {
                    target
                        .rules_by_first_token
                        .entry(token.clone())
                        .or_default()
                        .extend(rules.iter().cloned());
                }
                target
                    .catch_all_rules
                    .extend(overlay_layer.catch_all_rules.iter().cloned());
                target
                    .network_rules
                    .extend(overlay_layer.network_rules.iter().cloned());
            } else {
                combined_layers.push(overlay_layer.clone());
            }
        }

        let mut host_exes = self.host_executables_by_name.clone();
        host_exes.extend(
            overlay
                .host_executables_by_name
                .iter()
                .map(|(k, v)| (k.clone(), v.clone())),
        );

        PolicyEngine {
            loaded: self.loaded || overlay.loaded,
            layers: combined_layers,
            host_executables_by_name: host_exes,
            defaults_fallback: overlay.defaults_fallback.clone(),
        }
    }

    /// Compile network rules across all layers into (allowed, denied) domain lists.
    ///
    /// - `allow` rules move a host from denied to allowed.
    /// - `forbidden` rules move a host from allowed to denied.
    /// - `prompt` rules are ignored.
    pub fn compiled_network_domains(&self) -> (Vec<String>, Vec<String>) {
        let mut allowed: Vec<String> = Vec::new();
        let mut denied: Vec<String> = Vec::new();

        for layer in &self.layers {
            for rule in &layer.network_rules {
                let host = config::normalize_network_rule_host(&rule.host)
                    .unwrap_or_else(|_| rule.host.to_lowercase());
                match rule.decision.as_str() {
                    "allow" => {
                        denied.retain(|e| e != &host);
                        if !allowed.contains(&host) {
                            allowed.push(host);
                        }
                    }
                    "forbidden" => {
                        allowed.retain(|e| e != &host);
                        if !denied.contains(&host) {
                            denied.push(host);
                        }
                    }
                    _ => {}
                }
            }
        }

        (allowed, denied)
    }
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Leniently normalize a host string from callers (strip scheme, path, port).
///
/// Unlike `normalize_network_rule_host` (strict mode for rule definitions),
/// this accepts URLs and strips problematic parts instead of rejecting them.
fn lenient_normalize_host(raw: &str) -> String {
    let mut host = raw.trim().to_string();

    for prefix in &["https://", "http://", "ssh://", "socks5://"] {
        if let Some(stripped) = host.strip_prefix(prefix) {
            host = stripped.to_string();
            break;
        }
    }

    if let Some(pos) = host.find('/') {
        host.truncate(pos);
    }
    if let Some(pos) = host.find('?') {
        host.truncate(pos);
    }
    if let Some(pos) = host.find('#') {
        host.truncate(pos);
    }

    if host.starts_with('[') {
        if let Some(bracket_end) = host.find(']') {
            host = host[1..bracket_end].to_string();
        }
    } else if let Some(pos) = host.rfind(':') {
        let after = &host[pos + 1..];
        if !after.is_empty() && after.chars().all(|c| c.is_ascii_digit()) {
            host.truncate(pos);
        }
    }

    while host.ends_with('.') {
        host.pop();
    }

    host.to_lowercase()
}

fn parse_fallback_decision(fallback: &str) -> PolicyDecision {
    match fallback {
        "allow" => PolicyDecision::Allow { rule_id: None },
        "forbidden" | "deny" => PolicyDecision::Forbidden {
            rule_id: None,
            justification: "forbidden by default policy".to_string(),
        },
        _ => PolicyDecision::Prompt {
            rule_id: None,
            reason: format!("no matching rule (default: {fallback})"),
        },
    }
}

fn parse_decision(rule: &PrefixRule) -> PolicyDecision {
    match rule.decision.as_str() {
        "allow" => PolicyDecision::Allow {
            rule_id: rule.id.clone(),
        },
        "forbidden" => PolicyDecision::Forbidden {
            rule_id: rule.id.clone(),
            justification: rule
                .justification
                .clone()
                .unwrap_or_else(|| "forbidden by policy".to_string()),
        },
        _ => PolicyDecision::Prompt {
            rule_id: rule.id.clone(),
            reason: rule
                .justification
                .clone()
                .unwrap_or_else(|| "requires confirmation".to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn basic_config() -> &'static str {
        r#"
[defaults]
allow_readonly = ["ls", "cat", "head", "tail", "wc"]

[[rules]]
id = "allow-git-status"
pattern = ["git", "status"]
decision = "allow"

[[rules]]
id = "allow-git-diff"
pattern = ["git", "diff"]
decision = "allow"

[[rules]]
id = "forbid-force-push"
pattern = ["git", "push", "--force"]
decision = "forbidden"
justification = "Force push is dangerous"

[[rules]]
id = "prompt-git-push"
pattern = ["git", "push"]
decision = "prompt"
justification = "Push needs confirmation"

[[rules]]
id = "forbid-rm-rf-root"
pattern = ["rm", "-rf", "/"]
decision = "forbidden"
justification = "Would destroy the system"

[[network]]
id = "allow-github"
host = "api.github.com"
protocol = "https"
decision = "allow"

[[network]]
id = "deny-all-other"
host = "*"
decision = "forbidden"

[[tests]]
command = "ls -la"
expect = "allow"

[[tests]]
command = "git status"
expect = "allow"

[[tests]]
command = "git push --force origin main"
expect = "forbidden"

[[tests]]
command = "git push origin main"
expect = "prompt"
"#
    }

    #[test]
    fn load_and_evaluate_basic() {
        let mut engine = PolicyEngine::new();
        engine.load_str(basic_config(), "project").unwrap();

        assert!(engine.evaluate(&["ls", "-la"]).is_allowed());
        assert!(engine.evaluate(&["cat", "file.txt"]).is_allowed());
        assert!(engine.evaluate(&["git", "status"]).is_allowed());
        assert!(engine.evaluate(&["git", "diff"]).is_allowed());
    }

    #[test]
    fn forbidden_rules() {
        let mut engine = PolicyEngine::new();
        engine.load_str(basic_config(), "project").unwrap();

        let eval = engine.evaluate(&["git", "push", "--force", "origin", "main"]);
        assert!(eval.is_forbidden());
        assert!(!eval.matched_rules.is_empty());
        match &eval.decision {
            PolicyDecision::Forbidden { justification, .. } => {
                assert!(justification.contains("dangerous"));
            }
            _ => panic!("expected Forbidden"),
        }
    }

    #[test]
    fn prompt_rules() {
        let mut engine = PolicyEngine::new();
        engine.load_str(basic_config(), "project").unwrap();

        let eval = engine.evaluate(&["git", "push", "origin", "main"]);
        assert!(!eval.is_allowed());
        assert!(!eval.is_forbidden());
    }

    #[test]
    fn no_matching_rule_defaults_to_prompt() {
        let mut engine = PolicyEngine::new();
        engine.load_str(basic_config(), "project").unwrap();

        let eval = engine.evaluate(&["unknown-command"]);
        assert!(!eval.is_allowed());
        assert!(!eval.is_forbidden());
        assert!(eval.matched_rules.is_empty());
    }

    #[test]
    fn forbidden_overrides_allow_across_layers() {
        let mut engine = PolicyEngine::new();
        engine
            .load_str(
                r#"
[[rules]]
pattern = ["rm", "-rf", "/"]
decision = "forbidden"
justification = "Never allowed"
"#,
                "system",
            )
            .unwrap();
        engine.add_session_rule(PrefixRule {
            id: Some("session-allow-rm".into()),
            pattern: vec![config::PatternElement::Exact("rm".into())],
            decision: "allow".into(),
            justification: None,
        });

        let eval = engine.evaluate(&["rm", "-rf", "/"]);
        assert!(eval.is_forbidden());
        assert!(eval.matched_rules.len() >= 2);
    }

    #[test]
    fn session_layer_has_highest_priority_for_same_decision_type() {
        let mut engine = PolicyEngine::new();
        engine
            .load_str(
                r#"
[[rules]]
pattern = ["curl"]
decision = "prompt"
"#,
                "project",
            )
            .unwrap();

        assert!(!engine
            .evaluate(&["curl", "http://example.com"])
            .is_allowed());

        engine.add_session_rule(PrefixRule {
            id: Some("session-allow-curl".into()),
            pattern: vec![config::PatternElement::Exact("curl".into())],
            decision: "allow".into(),
            justification: None,
        });

        let eval = engine.evaluate(&["curl", "http://example.com"]);
        assert!(!eval.is_allowed());
    }

    #[test]
    fn inline_tests_pass() {
        let mut engine = PolicyEngine::new();
        let config: PolicyConfig = toml::from_str(basic_config()).unwrap();
        let tests = config.tests.clone();
        engine.add_layer("project", config);

        let errors = engine.validate_tests(&tests);
        assert!(errors.is_empty(), "Test failures: {errors:?}");
    }

    #[test]
    fn inline_test_failure_reported() {
        let mut engine = PolicyEngine::new();
        engine
            .load_str(
                r#"
[[rules]]
pattern = ["echo"]
decision = "allow"
"#,
                "project",
            )
            .unwrap();

        let tests = vec![PolicyTest {
            command: "echo hello".into(),
            expect: "forbidden".into(),
            not_match: None,
        }];
        let errors = engine.validate_tests(&tests);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("expected='forbidden'"));
    }

    #[test]
    fn network_rule_evaluation() {
        let mut engine = PolicyEngine::new();
        engine.load_str(basic_config(), "project").unwrap();

        let d = engine.evaluate_network("api.github.com", "https");
        assert!(d.is_allowed());

        let d = engine.evaluate_network("evil.com", "https");
        assert!(d.is_forbidden());
    }

    #[test]
    fn defaults_expand_to_allow_rules() {
        let mut engine = PolicyEngine::new();
        engine
            .load_str(
                r#"
[defaults]
allow_readonly = ["ls", "cat", "pwd"]
"#,
                "project",
            )
            .unwrap();

        assert_eq!(engine.rule_count(), 3);
        assert!(engine.evaluate(&["ls"]).is_allowed());
        assert!(engine.evaluate(&["cat", "file"]).is_allowed());
        assert!(engine.evaluate(&["pwd"]).is_allowed());
    }

    #[test]
    fn alternative_pattern_matching() {
        let mut engine = PolicyEngine::new();
        engine
            .load_str(
                r#"
[[rules]]
id = "git-merge-rebase"
pattern = ["git", ["merge", "rebase"]]
decision = "prompt"
"#,
                "project",
            )
            .unwrap();

        let d = engine.evaluate(&["git", "merge", "main"]);
        assert!(!d.is_allowed());
        assert!(!d.is_forbidden());

        let d = engine.evaluate(&["git", "rebase", "main"]);
        assert!(!d.is_allowed());
        assert!(!d.is_forbidden());

        let d = engine.evaluate(&["git", "pull"]);
        assert!(!d.is_allowed());
    }

    #[test]
    fn evaluation_includes_matched_rules() {
        let mut engine = PolicyEngine::new();
        engine.load_str(basic_config(), "project").unwrap();

        let eval = engine.evaluate(&["git", "status"]);
        assert!(eval.is_allowed());
        assert!(!eval.matched_rules.is_empty());
        assert_eq!(
            eval.matched_rules[0].rule_id,
            Some("allow-git-status".to_string())
        );
    }

    #[test]
    fn unloaded_engine_denies_all() {
        let engine = PolicyEngine::new();
        assert!(!engine.is_loaded());
        let eval = engine.evaluate(&["ls", "-la"]);
        assert!(eval.is_forbidden());
        assert!(engine
            .evaluate_network("example.com", "https")
            .is_forbidden());
    }

    #[test]
    fn not_match_validation() {
        let mut engine = PolicyEngine::new();
        engine
            .load_str(
                r#"
[defaults]
fallback = "prompt"

[[rules]]
pattern = ["echo"]
decision = "allow"
"#,
                "project",
            )
            .unwrap();

        // "ls" has no matching rules -> not_match should pass
        let tests = vec![PolicyTest {
            command: "ls -la".into(),
            expect: "prompt".into(),
            not_match: Some(true),
        }];
        let errors = engine.validate_tests(&tests);
        assert!(errors.is_empty());

        // "echo hello" has matching rules -> not_match should fail
        let tests = vec![PolicyTest {
            command: "echo hello".into(),
            expect: "allow".into(),
            not_match: Some(true),
        }];
        let errors = engine.validate_tests(&tests);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("no matching rules"));
    }

    #[test]
    fn hashmap_indexing_performance() {
        let mut engine = PolicyEngine::new();
        let mut config_str = String::new();
        for i in 0..100 {
            config_str.push_str(&format!(
                "[[rules]]\nid = \"rule-{i}\"\npattern = [\"cmd{i}\"]\ndecision = \"allow\"\n\n"
            ));
        }
        engine.load_str(&config_str, "project").unwrap();
        assert_eq!(engine.rule_count(), 100);

        // Lookup should be O(1) via HashMap, not O(n) linear scan
        let eval = engine.evaluate(&["cmd50"]);
        assert!(eval.is_allowed());
        assert_eq!(eval.matched_rules[0].rule_id, Some("rule-50".to_string()));

        let eval = engine.evaluate(&["cmd99"]);
        assert!(eval.is_allowed());
    }

    #[test]
    fn evaluate_with_heuristics_uses_fallback_when_no_rules() {
        let mut engine = PolicyEngine::new();
        engine
            .load_str(
                r#"
[[rules]]
pattern = ["echo"]
decision = "allow"
"#,
                "project",
            )
            .unwrap();

        let eval = engine.evaluate_with_heuristics(&["unknown-cmd", "arg"], |_tokens| {
            PolicyDecision::Forbidden {
                rule_id: None,
                justification: "heuristic deny".to_string(),
            }
        });
        assert!(eval.is_forbidden());
        assert_eq!(eval.matched_rules.len(), 1);
        assert!(eval.matched_rules[0].is_heuristic);
    }

    #[test]
    fn evaluate_with_heuristics_skips_fallback_when_rules_match() {
        let mut engine = PolicyEngine::new();
        engine
            .load_str(
                r#"
[[rules]]
pattern = ["echo"]
decision = "allow"
"#,
                "project",
            )
            .unwrap();

        let eval =
            engine.evaluate_with_heuristics(&["echo", "hello"], |_| PolicyDecision::Forbidden {
                rule_id: None,
                justification: "should not be called".to_string(),
            });
        assert!(eval.is_allowed());
        assert!(!eval.matched_rules[0].is_heuristic);
    }

    #[test]
    fn matched_rule_is_heuristic_default_false() {
        let mut engine = PolicyEngine::new();
        engine
            .load_str(
                r#"
[[rules]]
pattern = ["echo"]
decision = "allow"
"#,
                "project",
            )
            .unwrap();

        let eval = engine.evaluate(&["echo"]);
        assert!(!eval.matched_rules[0].is_heuristic);
    }

    #[test]
    fn network_protocol_typed_matching() {
        let mut engine = PolicyEngine::new();
        engine
            .load_str(
                r#"
[[network]]
id = "allow-github-https"
host = "api.github.com"
protocol = "https"
decision = "allow"

[[network]]
id = "allow-ssh-git"
host = "github.com"
protocol = "ssh"
decision = "allow"

[[network]]
id = "deny-all"
host = "*"
decision = "forbidden"
"#,
                "project",
            )
            .unwrap();

        assert!(engine
            .evaluate_network("api.github.com", "https")
            .is_allowed());
        assert!(engine
            .evaluate_network("api.github.com", "HTTPS")
            .is_allowed());
        assert!(engine.evaluate_network("github.com", "ssh").is_allowed());
        assert!(engine
            .evaluate_network("api.github.com", "http")
            .is_forbidden());
        assert!(engine.evaluate_network("evil.com", "https").is_forbidden());
    }

    #[test]
    fn network_host_normalization_strict() {
        assert_eq!(
            normalize_network_rule_host("  api.github.com  ").unwrap(),
            "api.github.com"
        );
        assert_eq!(
            normalize_network_rule_host("api.github.com:443").unwrap(),
            "api.github.com"
        );
        assert_eq!(
            normalize_network_rule_host("API.GITHUB.COM.").unwrap(),
            "api.github.com"
        );
        // Strict mode rejects scheme, path, wildcard
        assert!(normalize_network_rule_host("https://api.github.com").is_err());
        assert!(normalize_network_rule_host("http://example.com/path").is_err());
        assert!(normalize_network_rule_host("*").is_err());
        assert!(normalize_network_rule_host("").is_err());
        assert!(normalize_network_rule_host("   ").is_err());
        // IPv6
        assert_eq!(normalize_network_rule_host("[::1]:8080").unwrap(), "::1");
    }

    #[test]
    fn network_eval_normalizes_hosts() {
        let mut engine = PolicyEngine::new();
        engine
            .load_str(
                r#"
[[network]]
host = "api.github.com"
protocol = "https"
decision = "allow"

[[network]]
host = "*"
decision = "forbidden"
"#,
                "project",
            )
            .unwrap();

        // Match even when caller provides URL-like host or uppercase
        assert!(engine
            .evaluate_network("https://api.github.com", "https")
            .is_allowed());
        assert!(engine
            .evaluate_network("API.GITHUB.COM", "https")
            .is_allowed());
        assert!(engine
            .evaluate_network("api.github.com:443", "https")
            .is_allowed());
    }

    #[test]
    fn network_protocol_any_matches_all() {
        let mut engine = PolicyEngine::new();
        engine
            .load_str(
                r#"
[[network]]
host = "internal.corp"
protocol = "any"
decision = "allow"
"#,
                "project",
            )
            .unwrap();

        assert!(engine
            .evaluate_network("internal.corp", "https")
            .is_allowed());
        assert!(engine
            .evaluate_network("internal.corp", "http")
            .is_allowed());
        assert!(engine.evaluate_network("internal.corp", "ssh").is_allowed());
    }

    #[test]
    fn evaluate_with_options_resolves_absolute_path() {
        let mut engine = PolicyEngine::new();
        engine
            .load_str(
                r#"
[[rules]]
pattern = ["git", "status"]
decision = "allow"
"#,
                "project",
            )
            .unwrap();

        let opts = MatchOptions {
            resolve_host_executables: true,
        };
        let eval = engine.evaluate_with_options(&["/usr/bin/git", "status"], &opts);
        assert!(eval.is_allowed());
    }

    #[test]
    fn evaluate_with_options_no_resolve_misses_absolute_path() {
        let mut engine = PolicyEngine::new();
        engine
            .load_str(
                r#"
[[rules]]
pattern = ["git", "status"]
decision = "allow"
"#,
                "project",
            )
            .unwrap();

        let opts = MatchOptions::default();
        let eval = engine.evaluate_with_options(&["/usr/bin/git", "status"], &opts);
        assert!(eval.matched_rules.is_empty());
    }

    #[test]
    fn evaluate_multiple_aggregates_decisions() {
        let mut engine = PolicyEngine::new();
        engine
            .load_str(
                r#"
[[rules]]
pattern = ["echo"]
decision = "allow"

[[rules]]
pattern = ["rm", "-rf", "/"]
decision = "forbidden"
justification = "dangerous"
"#,
                "project",
            )
            .unwrap();

        let cmds: Vec<&[&str]> = vec![&["echo", "hi"], &["rm", "-rf", "/"]];
        let eval = engine.evaluate_multiple(cmds);
        assert!(eval.is_forbidden());
        assert!(eval.matched_rules.len() >= 2);
    }

    #[test]
    fn evaluate_multiple_all_allowed() {
        let mut engine = PolicyEngine::new();
        engine
            .load_str(
                r#"
[[rules]]
pattern = ["echo"]
decision = "allow"

[[rules]]
pattern = ["ls"]
decision = "allow"
"#,
                "project",
            )
            .unwrap();

        let cmds: Vec<&[&str]> = vec![&["echo", "hi"], &["ls", "-la"]];
        let eval = engine.evaluate_multiple(cmds);
        assert!(eval.is_allowed());
    }

    #[test]
    fn evaluate_multiple_empty_input() {
        let engine = PolicyEngine::new();
        let cmds: Vec<&[&str]> = vec![];
        let eval = engine.evaluate_multiple(cmds);
        assert!(eval.is_forbidden());
    }

    #[test]
    fn evaluate_with_options_non_absolute_path_unaffected() {
        let mut engine = PolicyEngine::new();
        engine
            .load_str(
                r#"
[[rules]]
pattern = ["git", "status"]
decision = "allow"
"#,
                "project",
            )
            .unwrap();

        let opts = MatchOptions {
            resolve_host_executables: true,
        };
        let eval = engine.evaluate_with_options(&["git", "status"], &opts);
        assert!(eval.is_allowed());
    }

    #[test]
    fn merge_overlay_combines_rules_and_network() {
        let base_toml = r#"
[[rules]]
id = "allow_ls"
pattern = ["ls"]
decision = "allow"

[[network]]
host = "github.com"
protocol = "https"
decision = "allow"
"#;
        let overlay_toml = r#"
[[rules]]
id = "allow_cat"
pattern = ["cat"]
decision = "allow"

[[network]]
host = "evil.com"
decision = "forbidden"
"#;
        let mut base = PolicyEngine::new();
        base.load_str(base_toml, "project").unwrap();

        let mut overlay = PolicyEngine::new();
        overlay.load_str(overlay_toml, "project").unwrap();

        let merged = base.merge_overlay(&overlay);
        assert!(merged.evaluate(&["ls"]).is_allowed());
        assert!(merged.evaluate(&["cat"]).is_allowed());

        let (allowed, denied) = merged.compiled_network_domains();
        assert!(allowed.contains(&"github.com".to_string()));
        assert!(denied.contains(&"evil.com".to_string()));
    }

    #[test]
    fn merge_overlay_host_executables_override() {
        let mut base = PolicyEngine::new();
        base.set_host_executable_paths("node".into(), vec![PathBuf::from("/usr/bin/node")]);

        let mut overlay = PolicyEngine::new();
        overlay
            .set_host_executable_paths("node".into(), vec![PathBuf::from("/usr/local/bin/node")]);

        let merged = base.merge_overlay(&overlay);
        let paths = merged.host_executables().get("node").unwrap();
        assert_eq!(paths, &[PathBuf::from("/usr/local/bin/node")]);
    }

    #[test]
    fn compiled_network_domains_allow_overrides_deny() {
        let toml_str = r#"
[[network]]
host = "example.com"
decision = "forbidden"

[[network]]
host = "example.com"
protocol = "https"
decision = "allow"
"#;
        let mut engine = PolicyEngine::new();
        engine.load_str(toml_str, "project").unwrap();

        let (allowed, denied) = engine.compiled_network_domains();
        assert!(allowed.contains(&"example.com".to_string()));
        assert!(!denied.contains(&"example.com".to_string()));
    }

    #[test]
    fn get_allowed_prefixes_basic() {
        let mut engine = PolicyEngine::new();
        engine.load_str(basic_config(), "project").unwrap();

        let prefixes = engine.get_allowed_prefixes();
        assert!(prefixes.contains(&"git".to_string()));
        assert!(prefixes.contains(&"ls".to_string()));
        assert!(prefixes.contains(&"cat".to_string()));
        // "rm" has forbidden decision only, not allow
        assert!(!prefixes.contains(&"rm".to_string()));
    }

    #[test]
    fn get_allowed_prefixes_empty_engine() {
        let engine = PolicyEngine::new();
        assert!(engine.get_allowed_prefixes().is_empty());
    }

    #[test]
    fn get_allowed_prefixes_deduplicates_across_layers() {
        let mut engine = PolicyEngine::new();
        engine
            .load_str(
                r#"
[[rules]]
pattern = ["echo"]
decision = "allow"
"#,
                "system",
            )
            .unwrap();
        engine
            .load_str(
                r#"
[[rules]]
pattern = ["echo"]
decision = "allow"
"#,
                "project",
            )
            .unwrap();

        let prefixes = engine.get_allowed_prefixes();
        assert_eq!(prefixes.iter().filter(|p| *p == "echo").count(), 1);
    }

    #[test]
    fn compiled_network_domains_ignores_prompt() {
        let toml_str = r#"
[[network]]
host = "neutral.com"
decision = "prompt"
"#;
        let mut engine = PolicyEngine::new();
        engine.load_str(toml_str, "project").unwrap();

        let (allowed, denied) = engine.compiled_network_domains();
        assert!(allowed.is_empty());
        assert!(denied.is_empty());
    }
}
