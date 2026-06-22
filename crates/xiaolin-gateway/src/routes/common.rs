use std::sync::Arc;

use xiaolin_core::agent_config::AgentConfig;
use xiaolin_core::tool::ToolDefinition;
use xiaolin_core::types::{ChatRequest, Usage};

use crate::memory_scope::memory_tool_agent_suffix;
use crate::state::AppState;

use super::error::AppError;

pub fn memory_scoped_tool_visible_for_agent(tool_name: &str, agent_id: &str) -> bool {
    let sfx = memory_tool_agent_suffix(agent_id);
    if let Some(rest) = tool_name.strip_prefix("memory_search__") {
        return rest == sfx;
    }
    if let Some(rest) = tool_name.strip_prefix("memory_store__") {
        return rest == sfx;
    }
    if tool_name == "memory_search" || tool_name == "memory_store" {
        return true;
    }
    true
}

/// Tool definitions exposed to the LLM for an agent, matching `AgentRuntime` allow/deny filtering.
pub fn filtered_tool_definitions(
    tool_registry: &xiaolin_core::tool::ToolRegistry,
    agent_config: &AgentConfig,
) -> Option<Vec<ToolDefinition>> {
    let all_tool_defs = tool_registry.definitions();
    let tool_defs: Vec<_> = all_tool_defs
        .iter()
        .filter(|td| {
            let name = &td.function.name;
            if !memory_scoped_tool_visible_for_agent(name, &agent_config.agent_id) {
                return false;
            }
            agent_config.behavior.is_tool_allowed(name)
        })
        .cloned()
        .collect();
    if tool_defs.is_empty() {
        None
    } else {
        Some(tool_defs)
    }
}

fn providers_compatible(agent_provider: &str, routed_provider: &str) -> bool {
    let a = agent_provider.to_ascii_lowercase();
    let b = routed_provider.to_ascii_lowercase();
    a == b
        || (matches!(a.as_str(), "google" | "gemini") && matches!(b.as_str(), "google" | "gemini"))
}

/// When the user explicitly selects a model (e.g. from the model dropdown),
/// look up the model in `config.models` and LLM plugin registry to find the
/// correct provider. Returns `Some(provider)` if the pinned model belongs to
/// a different provider than the agent's default.
fn resolve_pinned_model_provider(
    state: &AppState,
    agent_config: &AgentConfig,
    pinned_model: &str,
) -> Option<Arc<dyn xiaolin_agent::LlmProvider>> {
    let live_credentials = state.current_credentials_snapshot();

    // 1. Search live config models (same source as models.list API).
    let live = state.cfg.config_live.load();
    if let Some(models_obj) = live.get("models").and_then(|v| v.as_object()) {
        for (key, cfg) in models_obj {
            let cfg_model = cfg
                .get("model")
                .or_else(|| cfg.get("defaultModel"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if cfg_model.is_empty() {
                continue;
            }
            if cfg_model != pinned_model {
                continue;
            }
            let cfg_provider_type = cfg
                .get("provider")
                .or_else(|| cfg.get("providerType"))
                .and_then(|v| v.as_str())
                .unwrap_or(key.as_str());
            if providers_compatible(&agent_config.model.provider, cfg_provider_type) {
                return None;
            }
            let api_key = cfg.get("apiKey").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .or_else(|| live_credentials.get_api_key(key))
                .or_else(|| live_credentials.get_api_key(cfg_provider_type));
            let base_url = cfg.get("baseUrl").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .or_else(|| live_credentials.get_base_url(key))
                .or_else(|| live_credentials.get_base_url(cfg_provider_type));

            if let Some(plugin_id) = cfg_provider_type.strip_prefix("plugin:") {
                if let Ok(registry) = state.ext.llm_plugin_registry.try_read() {
                    match registry.create_provider(plugin_id) {
                        Ok(p) => return Some(Arc::from(p)),
                        Err(e) => {
                            tracing::warn!(error = %e, plugin_id, "failed to create plugin provider for pinned model");
                        }
                    }
                }
                return None;
            }
            match xiaolin_agent::create_provider_with_credentials(
                cfg_provider_type,
                base_url,
                api_key,
                Some(&live_credentials),
                None,
            ) {
                Ok(p) => return Some(Arc::from(p)),
                Err(e) => {
                    tracing::warn!(error = %e, provider = cfg_provider_type, "failed to create provider for pinned model");
                    return None;
                }
            }
        }
    }

    // 2. Search LLM plugin registry.
    if let Ok(registry) = state.ext.llm_plugin_registry.try_read() {
        for plugin in registry.list() {
            if !plugin.enabled {
                continue;
            }
            let has_model = plugin.models.iter().any(|m| m.id == pinned_model);
            if !has_model {
                continue;
            }
            let plugin_provider = format!("plugin:{}", plugin.id);
            if providers_compatible(&agent_config.model.provider, &plugin_provider) {
                return None;
            }
            match registry.create_provider(&plugin.id) {
                Ok(p) => return Some(Arc::from(p)),
                Err(e) => {
                    tracing::warn!(error = %e, plugin_id = %plugin.id, "failed to create plugin provider for pinned model");
                    return None;
                }
            }
        }
    }

    None
}

/// When model routing is enabled, pick model (and optionally a per-request LLM provider) before calling `AgentRuntime`.
/// Does nothing when router is off, the client pinned `model`, or routing fails.
///
/// When the client explicitly pins a `model`, we still check whether that model
/// belongs to a different provider than the agent's default and create an
/// `llm_override` if needed (e.g. user picks "deepseek-v4-flash" but the agent's
/// default provider is a plugin that would misroute the request).
pub fn apply_model_router_for_chat(
    state: &AppState,
    agent_config: &AgentConfig,
    request: &mut ChatRequest,
    tool_definition_count: usize,
) -> Option<Arc<dyn xiaolin_agent::LlmProvider>> {
    if let Some(ref pinned_model) = request.model {
        return resolve_pinned_model_provider(state, agent_config, pinned_model);
    }
    let router = state.obs.model_router.as_ref()?;
    let input_tokens = xiaolin_model_router::CostEstimator::estimate_chat_complexity_tokens(
        &request.messages,
        tool_definition_count,
    );
    let estimated =
        xiaolin_model_router::estimate_complexity_tier(xiaolin_model_router::TierEstimateInput {
            messages: &request.messages,
            tool_definition_count,
        });
    let tier_constraints = xiaolin_model_router::RouteTierConstraints {
        estimated,
        agent_min_tier: agent_config.min_tier,
        agent_max_tier: agent_config.max_tier,
    };
    let preferred = agent_config.model.model.as_str();
    let route = match router.route(Some(preferred), input_tokens, Some(tier_constraints)) {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(error = %e, "model router skipped");
            return None;
        }
    };
    let sel = route.selected;
    tracing::info!(
        model = %sel.model,
        provider = %sel.provider,
        reason = %sel.reason,
        "model router selection"
    );

    if providers_compatible(&agent_config.model.provider, &sel.provider) {
        request.model = Some(sel.model);
        return None;
    }

    match xiaolin_agent::create_provider_with_credentials(
        &sel.provider,
        None,
        None,
        Some(&state.cfg.config.credentials),
        None,
    ) {
        Ok(p) => {
            request.model = Some(sel.model);
            Some(Arc::from(p))
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                provider = %sel.provider,
                "model router: failed to create provider for routed model; using agent default provider"
            );
            None
        }
    }
}

/// Pre-flight budget check: atomically reserve estimated cost before the LLM call.
/// Returns `(estimated_cost, degraded)` on success — `degraded` is true when the budget lock
/// failed and the request was allowed through with zero reserved cost.
/// Returns an error if the budget would be exceeded.
pub fn try_reserve_budget(
    state: &AppState,
    model: &str,
    input_tokens: u32,
    tool_count: usize,
) -> Result<(f64, bool), AppError> {
    let estimated_output = (input_tokens / 3).max(100);
    let cost = state
        .obs
        .model_router
        .as_ref()
        .map(|r| r.usage_charge_for_tokens(model, input_tokens, estimated_output))
        .unwrap_or_else(|| {
            xiaolin_model_router::default_usage_charge(model, input_tokens, estimated_output)
        });
    if tool_count > 0 {
        let cost = cost * 1.5;
        match state.obs.budget_tracker.try_reserve(cost) {
            Ok(true) => return Ok((cost, false)),
            Ok(false) => {
                return Err(AppError::BadRequest(
                    "daily budget exceeded; request blocked".to_string(),
                ));
            }
            Err(e) => {
                tracing::warn!(error = %e, "budget reserve check failed, allowing request");
                return Ok((0.0, true));
            }
        }
    }
    match state.obs.budget_tracker.try_reserve(cost) {
        Ok(true) => {
            tracing::debug!(
                model = %model,
                estimated_cost = format!("{cost:.6}"),
                input_tokens = input_tokens,
                "budget: reserved estimated cost"
            );
            Ok((cost, false))
        }
        Ok(false) => {
            if let Ok(summary) = state.obs.budget_tracker.summary() {
                tracing::warn!(
                    model = %model,
                    estimated_cost = format!("{cost:.6}"),
                    total_spent = format!("{:.6}", summary.total_cost),
                    budget_limit = format!("{:?}", summary.budget_limit),
                    "budget: request blocked — daily budget exceeded"
                );
            } else {
                tracing::warn!(
                    model = %model,
                    estimated_cost = format!("{cost:.6}"),
                    "budget: request blocked — daily budget exceeded"
                );
            }
            Err(AppError::BadRequest(
                "daily budget exceeded; request blocked".to_string(),
            ))
        }
        Err(e) => {
            tracing::warn!(error = %e, "budget reserve check failed, allowing request");
            Ok((0.0, true))
        }
    }
}

pub fn record_chat_budget_actual(state: &AppState, model: &str, usage: Option<&Usage>) {
    let Some(u) = usage else {
        return;
    };
    let cost = state
        .obs
        .model_router
        .as_ref()
        .map(|r| r.usage_charge_for_tokens(model, u.prompt_tokens, u.completion_tokens))
        .unwrap_or_else(|| {
            xiaolin_model_router::default_usage_charge(model, u.prompt_tokens, u.completion_tokens)
        });
    if let Err(e) = state
        .obs
        .budget_tracker
        .record(&xiaolin_model_router::UsageRecord {
            model: model.to_string(),
            input_tokens: u.prompt_tokens,
            output_tokens: u.completion_tokens,
            cost,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })
    {
        tracing::warn!(error = %e, "budget tracker: failed to record usage");
    }
}

pub fn record_chat_budget_stream_estimate(
    state: &AppState,
    model: &str,
    input_tokens: u32,
    assistant_text_char_len: usize,
) {
    let out_toks = ((assistant_text_char_len as u32) / 2).max(1);
    let cost = state
        .obs
        .model_router
        .as_ref()
        .map(|r| r.usage_charge_for_tokens(model, input_tokens, out_toks))
        .unwrap_or_else(|| {
            xiaolin_model_router::default_usage_charge(model, input_tokens, out_toks)
        });
    if let Err(e) = state
        .obs
        .budget_tracker
        .record(&xiaolin_model_router::UsageRecord {
            model: model.to_string(),
            input_tokens,
            output_tokens: out_toks,
            cost,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })
    {
        tracing::warn!(error = %e, "budget tracker: failed to record stream estimate");
    }
}

pub fn map_router_resolve_err(e: anyhow::Error) -> AppError {
    let msg = e.to_string();
    let lower = msg.to_ascii_lowercase();
    if msg.contains("agent not found") {
        return AppError::NotFound(msg);
    }
    if lower.contains("unauthorized")
        || lower.contains("invalid api key")
        || lower.contains("authentication")
    {
        return AppError::Unauthorized(msg);
    }
    if lower.contains("rate limit") || lower.contains("too many requests") || lower.contains("429")
    {
        return AppError::RateLimited;
    }
    AppError::Internal(e)
}
