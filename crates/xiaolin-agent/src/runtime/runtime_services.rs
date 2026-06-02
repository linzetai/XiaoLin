//! Unified runtime services — bridges hook_executor, cost_tracker, magic_docs,
//! and permissions into the agent query loop.
//!
//! `RuntimeServices` is constructed once per `execute_stream_inner` invocation and
//! threaded through tool execution. It holds optional references to each subsystem
//! so that the caller can opt-in via configuration.

use std::path::Path;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use super::cost_tracker::{BudgetAlert, CallUsage, CostTracker, CostTrackerConfig};
use super::hook_config::HookConfig;
use super::hook_events::{HookEvent, HookResult};
use super::hook_executor::{HookEventFilter, HookExecutor, HookHandler, RegisteredHook};
use super::magic_docs::{DocIndex, MagicDocsConfig};
use super::permissions::{PermissionDecision, PermissionRuleEngine};

/// Aggregated runtime services available during the query loop.
pub(crate) struct RuntimeServices {
    pub hooks: Option<HookExecutor>,
    pub cost_tracker: Option<Mutex<CostTracker>>,
    pub magic_docs: Option<DocIndex>,
    pub permissions: Option<PermissionRuleEngine>,
    abort_token: CancellationToken,
}

impl RuntimeServices {
    /// Create a new `RuntimeServices` from configuration paths.
    ///
    /// Each subsystem is independently optional; a missing config file or
    /// disabled feature simply results in `None` for that field.
    pub fn from_config(
        workspace_dir: Option<&Path>,
        budget_limit_usd: Option<f64>,
        abort_token: CancellationToken,
    ) -> Self {
        let hooks = Self::load_hooks(workspace_dir);
        let cost_tracker = Self::build_cost_tracker(budget_limit_usd);
        let magic_docs = Self::load_magic_docs();
        let permissions = Self::load_permissions(workspace_dir);

        if let Some(ref h) = hooks {
            tracing::info!(hook_count = h.hook_count(), "runtime hooks loaded");
        }
        if magic_docs.is_some() {
            tracing::info!("magic_docs index loaded");
        }
        if permissions.is_some() {
            tracing::info!("permission rules loaded");
        }

        Self {
            hooks,
            cost_tracker,
            magic_docs,
            permissions,
            abort_token,
        }
    }

    // ── Hook integration ──────────────────────────────────────────────────

    /// Fire PreToolUse hooks. Returns the first blocking result, if any.
    pub async fn fire_pre_tool_hooks(
        &self,
        tool_name: &str,
        tool_use_id: &str,
        input: &serde_json::Value,
    ) -> Option<HookResult> {
        let executor = self.hooks.as_ref()?;
        let event = HookEvent::PreToolUse {
            tool_name: tool_name.into(),
            tool_use_id: tool_use_id.into(),
            input: input.clone(),
        };
        let results = executor
            .execute_pre_tool_hooks(&event, &self.abort_token)
            .await;
        results.into_iter().find(|r| r.is_blocked())
    }

    /// Fire PostToolUse hooks (non-blocking by default).
    pub async fn fire_post_tool_hooks(
        &self,
        tool_name: &str,
        tool_use_id: &str,
        input: &serde_json::Value,
        output: &serde_json::Value,
        duration: Duration,
    ) {
        let Some(executor) = self.hooks.as_ref() else {
            return;
        };
        let event = HookEvent::PostToolUse {
            tool_name: tool_name.into(),
            tool_use_id: tool_use_id.into(),
            input: input.clone(),
            output: output.clone(),
            duration,
        };
        let _ = executor
            .execute_post_tool_hooks(&event, &self.abort_token)
            .await;
    }

    /// Fire Stop hooks at the end of the query loop.
    pub async fn fire_stop_hooks(
        &self,
        messages: &[xiaolin_core::types::ChatMessage],
        assistant_messages: &[xiaolin_core::types::ChatMessage],
    ) {
        let Some(executor) = self.hooks.as_ref() else {
            return;
        };
        let event = HookEvent::Stop {
            messages: messages.to_vec(),
            assistant_messages: assistant_messages.to_vec(),
        };
        let _ = executor.execute_stop_hooks(&event, &self.abort_token).await;
    }

    // ── Cost tracking ─────────────────────────────────────────────────────

    /// Record an LLM call's token usage. Returns a budget alert if a
    /// threshold was crossed.
    pub async fn record_llm_usage(&self, usage: CallUsage) -> Option<BudgetAlert> {
        let tracker = self.cost_tracker.as_ref()?;
        let mut guard = tracker.lock().await;
        guard.record(&usage)
    }

    /// Current accumulated cost in USD.
    pub async fn accumulated_cost_usd(&self) -> f64 {
        match self.cost_tracker.as_ref() {
            Some(tracker) => tracker.lock().await.accumulated_cost_usd(),
            None => 0.0,
        }
    }

    // ── Magic docs ────────────────────────────────────────────────────────

    /// Query the magic docs index for relevant documentation snippets.
    /// Returns up to `max_chars` of relevant content, or an empty string.
    pub fn query_magic_docs(&self, keywords: &[&str], max_chars: usize) -> String {
        let Some(index) = self.magic_docs.as_ref() else {
            return String::new();
        };
        let query = keywords.join(" ");
        index
            .select_for_injection(&query, max_chars)
            .unwrap_or_default()
    }

    // ── Permissions ───────────────────────────────────────────────────────

    /// Check whether a tool call is permitted.
    /// Returns `None` if no permission engine is configured (default: allow).
    pub fn check_permission(&self, tool_name: &str) -> Option<PermissionDecision> {
        let engine = self.permissions.as_ref()?;
        Some(engine.evaluate(tool_name))
    }

    // ── Private builders ──────────────────────────────────────────────────

    fn load_hooks(workspace_dir: Option<&Path>) -> Option<HookExecutor> {
        let mut combined_config = HookConfig::default();

        // Load from workspace .xiaolin/hooks.json
        if let Some(ws) = workspace_dir {
            let xiaolin_dir = ws.join(".xiaolin");
            if let Ok(cfg) = HookConfig::load_from_dir(&xiaolin_dir) {
                combined_config.merge(cfg);
            }
        }

        // Load from user home ~/.xiaolin/hooks.json
        if let Some(home) = dirs::home_dir() {
            let home_dir = home.join(".xiaolin");
            if let Ok(cfg) = HookConfig::load_from_dir(&home_dir) {
                combined_config.merge(cfg);
            }
        }

        if combined_config.is_empty() {
            return None;
        }

        let mut executor = HookExecutor::new();

        for spec in &combined_config.pre_tool_use {
            executor.register(spec_to_registered_hook(spec, true));
        }
        for spec in &combined_config.post_tool_use {
            executor.register(spec_to_registered_hook(spec, false));
        }
        for spec in &combined_config.stop {
            executor.register(spec_to_registered_hook(spec, false));
        }

        Some(executor)
    }

    fn build_cost_tracker(budget_limit_usd: Option<f64>) -> Option<Mutex<CostTracker>> {
        let config = CostTrackerConfig {
            budget_limit_usd,
            ..Default::default()
        };
        Some(Mutex::new(CostTracker::new(config)))
    }

    fn load_magic_docs() -> Option<DocIndex> {
        let config = MagicDocsConfig::default();
        if !config.enabled || !config.docs_dir.exists() {
            return None;
        }
        let index = DocIndex::load_from_dir(&config.docs_dir);
        if index.entry_count() > 0 {
            Some(index)
        } else {
            None
        }
    }

    fn load_permissions(workspace_dir: Option<&Path>) -> Option<PermissionRuleEngine> {
        let mut engine = PermissionRuleEngine::new();
        let mut any_loaded = false;

        // Load from workspace .xiaolin/permissions.json
        if let Some(ws) = workspace_dir {
            let perm_path = ws.join(".xiaolin").join("permissions.json");
            if perm_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&perm_path) {
                    if let Ok(rules) =
                        serde_json::from_str::<Vec<super::permissions::PermissionRule>>(&content)
                    {
                        engine.add_rules(rules);
                        any_loaded = true;
                    }
                }
            }
        }

        // Load from user home ~/.xiaolin/permissions.json
        if let Some(home) = dirs::home_dir() {
            let perm_path = home.join(".xiaolin").join("permissions.json");
            if perm_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&perm_path) {
                    if let Ok(rules) =
                        serde_json::from_str::<Vec<super::permissions::PermissionRule>>(&content)
                    {
                        engine.add_rules(rules);
                        any_loaded = true;
                    }
                }
            }
        }

        if any_loaded {
            Some(engine)
        } else {
            None
        }
    }
}

/// Convert a `HookSpec` (config format) into a `RegisteredHook` (executor format).
fn spec_to_registered_hook(
    spec: &super::hook_config::HookSpec,
    is_pre_tool: bool,
) -> RegisteredHook {
    use super::hook_config::HookMatcher;

    let filter = match &spec.matcher {
        HookMatcher::AllTools => {
            if is_pre_tool {
                HookEventFilter::EventType("pre_tool_use")
            } else {
                HookEventFilter::All
            }
        }
        HookMatcher::ToolName { name } => HookEventFilter::ToolName(name.clone()),
        HookMatcher::ToolPattern { pattern } => HookEventFilter::ToolPattern(pattern.clone()),
    };

    let handler = HookHandler::Shell {
        command: spec.command.clone(),
        working_dir: spec.working_dir.as_ref().map(std::path::PathBuf::from),
    };

    RegisteredHook {
        filter,
        handler,
        timeout: spec.timeout(),
        blocking: spec.blocking,
    }
}

#[cfg(test)]
impl RuntimeServices {
    /// Empty services — no hooks, no cost tracking, no docs, no permissions.
    pub fn none(abort_token: CancellationToken) -> Self {
        Self {
            hooks: None,
            cost_tracker: None,
            magic_docs: None,
            permissions: None,
            abort_token,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_services_none_is_safe() {
        let svc = RuntimeServices::none(CancellationToken::new());
        assert!(svc.hooks.is_none());
        assert!(svc.cost_tracker.is_none());
        assert!(svc.magic_docs.is_none());
        assert!(svc.permissions.is_none());
    }

    #[tokio::test]
    async fn cost_tracker_always_available() {
        let svc = RuntimeServices {
            hooks: None,
            cost_tracker: Some(Mutex::new(CostTracker::default())),
            magic_docs: None,
            permissions: None,
            abort_token: CancellationToken::new(),
        };
        let alert = svc
            .record_llm_usage(CallUsage {
                model: "test".into(),
                prompt_tokens: 100,
                completion_tokens: 50,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
            })
            .await;
        assert!(alert.is_none());
        assert!(svc.accumulated_cost_usd().await > 0.0);
    }

    #[test]
    fn permission_check_returns_none_without_engine() {
        let svc = RuntimeServices::none(CancellationToken::new());
        assert!(svc.check_permission("anything").is_none());
    }

    #[test]
    fn magic_docs_returns_empty_without_index() {
        let svc = RuntimeServices::none(CancellationToken::new());
        assert!(svc.query_magic_docs(&["react"], 4000).is_empty());
    }
}
