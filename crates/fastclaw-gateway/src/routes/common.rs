use std::sync::Arc;

use fastclaw_core::agent_config::AgentConfig;
use fastclaw_core::tool::ToolDefinition;
use fastclaw_core::types::{ChatRequest, Usage};

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
    tool_registry: &fastclaw_core::tool::ToolRegistry,
    agent_config: &AgentConfig,
) -> Option<Vec<ToolDefinition>> {
    let all_tool_defs = tool_registry.definitions();
    let tool_defs: Vec<_> = all_tool_defs
        .into_iter()
        .filter(|td| {
            let name = &td.function.name;
            if !memory_scoped_tool_visible_for_agent(name, &agent_config.agent_id) {
                return false;
            }
            agent_config.behavior.is_tool_allowed(name)
        })
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

/// When model routing is enabled, pick model (and optionally a per-request LLM provider) before calling `AgentRuntime`.
/// Does nothing when router is off, the client pinned `model`, or routing fails.
pub fn apply_model_router_for_chat(
    state: &AppState,
    agent_config: &AgentConfig,
    request: &mut ChatRequest,
    tool_definition_count: usize,
) -> Option<Arc<dyn fastclaw_agent::LlmProvider>> {
    let router = state.model_router.as_ref()?;
    if request.model.is_some() {
        return None;
    }
    let input_tokens = fastclaw_model_router::CostEstimator::estimate_chat_complexity_tokens(
        &request.messages,
        tool_definition_count,
    );
    let estimated = fastclaw_model_router::estimate_complexity_tier(
        fastclaw_model_router::TierEstimateInput {
            messages: &request.messages,
            tool_definition_count,
        },
    );
    let tier_constraints = fastclaw_model_router::RouteTierConstraints {
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

    match fastclaw_agent::create_provider_with_credentials(
        &sel.provider,
        None,
        None,
        Some(&state.config.credentials),
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
        .model_router
        .as_ref()
        .map(|r| r.usage_charge_for_tokens(model, input_tokens, estimated_output))
        .unwrap_or_else(|| {
            fastclaw_model_router::default_usage_charge(model, input_tokens, estimated_output)
        });
    if tool_count > 0 {
        let cost = cost * 1.5;
        match state.budget_tracker.try_reserve(cost) {
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
    match state.budget_tracker.try_reserve(cost) {
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
            if let Ok(summary) = state.budget_tracker.summary() {
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
        .model_router
        .as_ref()
        .map(|r| r.usage_charge_for_tokens(model, u.prompt_tokens, u.completion_tokens))
        .unwrap_or_else(|| {
            fastclaw_model_router::default_usage_charge(model, u.prompt_tokens, u.completion_tokens)
        });
    if let Err(e) = state.budget_tracker.record(&fastclaw_model_router::UsageRecord {
        model: model.to_string(),
        input_tokens: u.prompt_tokens,
        output_tokens: u.completion_tokens,
        cost,
        timestamp: chrono::Utc::now().to_rfc3339(),
    }) {
        tracing::warn!(error = %e, "budget tracker: failed to record usage");
    }
}

pub fn record_chat_budget_stream_estimate(
    state: &AppState,
    model: &str,
    input_tokens: u32,
    assistant_text_char_len: usize,
) {
    let out_toks = ((assistant_text_char_len as u32).saturating_add(3)) / 4;
    let cost = state
        .model_router
        .as_ref()
        .map(|r| r.usage_charge_for_tokens(model, input_tokens, out_toks))
        .unwrap_or_else(|| {
            fastclaw_model_router::default_usage_charge(model, input_tokens, out_toks)
        });
    if let Err(e) = state.budget_tracker.record(&fastclaw_model_router::UsageRecord {
        model: model.to_string(),
        input_tokens,
        output_tokens: out_toks,
        cost,
        timestamp: chrono::Utc::now().to_rfc3339(),
    }) {
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
