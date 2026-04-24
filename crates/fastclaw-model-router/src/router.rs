use serde::{Deserialize, Serialize};
use fastclaw_core::complexity::ComplexityTier;

use crate::budget::BudgetTracker;
use crate::estimator::{CostEstimator, ModelPricing, TokenEstimate};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingStrategy {
    /// Always use the configured model; no fallback.
    Fixed,
    /// Cheapest model that fits the context window.
    CostOptimized,
    /// Fallback chain: try primary, then alternates on failure.
    Fallback,
}

impl Default for RoutingStrategy {
    fn default() -> Self {
        Self::Fixed
    }
}

#[derive(Debug, Clone)]
pub struct ModelCandidate {
    pub model: String,
    pub provider: String,
    pub estimated_cost: f64,
    pub quality: f64,
    pub latency_ms: u32,
    pub tier: ComplexityTier,
    pub reason: String,
}

/// Optional tier window applied after workload estimation.
#[derive(Debug, Clone)]
pub struct RouteTierConstraints {
    pub estimated: ComplexityTier,
    pub agent_min_tier: Option<ComplexityTier>,
    pub agent_max_tier: Option<ComplexityTier>,
}

#[derive(Debug, Clone)]
pub struct RouteResult {
    pub selected: ModelCandidate,
    pub alternatives: Vec<ModelCandidate>,
}

pub struct ModelRouter {
    estimator: CostEstimator,
    budget: BudgetTracker,
    strategy: RoutingStrategy,
    /// Fallback ordering (model names).
    fallback_chain: Vec<String>,
}

fn tier_window(constraints: &RouteTierConstraints) -> (ComplexityTier, ComplexityTier) {
    let floor = constraints
        .agent_min_tier
        .unwrap_or(ComplexityTier::Tiny)
        .max(constraints.estimated);
    let cap = constraints
        .agent_max_tier
        .unwrap_or(ComplexityTier::Frontier);
    if floor > cap {
        (cap, cap)
    } else {
        (floor, cap)
    }
}

fn filter_candidates_by_tier(
    candidates: Vec<ModelCandidate>,
    tier: Option<&RouteTierConstraints>,
) -> Vec<ModelCandidate> {
    let Some(tc) = tier else {
        return candidates;
    };
    let (need, cap) = tier_window(tc);
    let filtered: Vec<_> = candidates
        .iter()
        .filter(|c| c.tier >= need && c.tier <= cap)
        .cloned()
        .collect();
    if filtered.is_empty() {
        tracing::warn!(
            ?need,
            ?cap,
            "model router: no models in tier window; ignoring tier filter"
        );
        candidates
    } else {
        filtered
    }
}

impl ModelRouter {
    pub fn new(strategy: RoutingStrategy, budget: BudgetTracker) -> Self {
        Self {
            estimator: CostEstimator::with_defaults(),
            budget,
            strategy,
            fallback_chain: Vec::new(),
        }
    }

    pub fn set_strategy(&mut self, strategy: RoutingStrategy) {
        self.strategy = strategy;
    }

    pub fn set_fallback_chain(&mut self, chain: Vec<String>) {
        self.fallback_chain = chain;
    }

    pub fn add_model(&mut self, pricing: ModelPricing) {
        self.estimator.add_pricing(pricing);
    }

    pub fn budget(&self) -> &BudgetTracker {
        &self.budget
    }

    /// Look up the `max_context` for a model in the pricing table.
    pub fn max_context_for_model(&self, model: &str) -> Option<u32> {
        self.estimator.get_pricing(model).map(|p| p.max_context)
    }

    /// Estimated USD cost for a completed request using this router's pricing table (including custom `add_model` entries).
    pub fn usage_charge_for_tokens(
        &self,
        model: &str,
        input_tokens: u32,
        output_tokens: u32,
    ) -> f64 {
        self.estimator
            .get_pricing(model)
            .map(|p| {
                TokenEstimate {
                    input_tokens,
                    estimated_output_tokens: output_tokens,
                }
                .estimated_cost(p)
            })
            .unwrap_or(0.0)
    }

    pub fn route(
        &self,
        preferred_model: Option<&str>,
        input_tokens: u32,
        tier_constraints: Option<RouteTierConstraints>,
    ) -> anyhow::Result<RouteResult> {
        let estimated_output = (input_tokens / 3).max(100);
        let all_models = self.estimator.all_models();

        if all_models.is_empty() {
            anyhow::bail!("no models configured in model router");
        }

        let mut candidates: Vec<ModelCandidate> = all_models
            .iter()
            .filter(|p| p.max_context >= input_tokens + estimated_output)
            .map(|p| {
                let est_cost = (input_tokens as f64 / 1000.0) * p.input_per_1k
                    + (estimated_output as f64 / 1000.0) * p.output_per_1k;
                ModelCandidate {
                    model: p.model.clone(),
                    provider: p.provider.clone(),
                    estimated_cost: est_cost,
                    quality: p.quality,
                    latency_ms: p.avg_latency_ms,
                    tier: p.tier,
                    reason: String::new(),
                }
            })
            .collect();

        if candidates.is_empty() {
            anyhow::bail!("no model with sufficient context window for {input_tokens} tokens");
        }

        candidates = filter_candidates_by_tier(candidates, tier_constraints.as_ref());

        match self.strategy {
            RoutingStrategy::Fixed => {
                let first_model = candidates
                    .first()
                    .map(|c| c.model.as_str())
                    .unwrap_or("");
                let model_name = preferred_model.unwrap_or(first_model);
                let selected = candidates
                    .iter()
                    .find(|c| c.model == model_name)
                    .cloned()
                    .or_else(|| candidates.first().cloned());
                let Some(mut sel) = selected else {
                    anyhow::bail!("no routable model candidates");
                };
                sel.reason = "fixed strategy".into();
                let alternatives: Vec<_> = candidates
                    .into_iter()
                    .filter(|c| c.model != sel.model)
                    .collect();
                Ok(RouteResult {
                    selected: sel,
                    alternatives,
                })
            }

            RoutingStrategy::CostOptimized => {
                candidates.sort_by(|a, b| {
                    a.estimated_cost
                        .partial_cmp(&b.estimated_cost)
                        .unwrap_or_else(|| {
                            a.estimated_cost
                                .is_nan()
                                .cmp(&b.estimated_cost.is_nan())
                        })
                });
                let sel = select_within_budget(
                    &self.budget,
                    &mut candidates,
                    "cheapest within budget",
                )?;
                let alternatives: Vec<_> = candidates
                    .into_iter()
                    .filter(|c| c.model != sel.model)
                    .collect();
                Ok(RouteResult {
                    selected: sel,
                    alternatives,
                })
            }

            RoutingStrategy::Fallback => {
                let chain = if self.fallback_chain.is_empty() {
                    candidates.iter().map(|c| c.model.clone()).collect()
                } else {
                    self.fallback_chain.clone()
                };

                let mut selected: Option<ModelCandidate> = None;
                for model_name in &chain {
                    if let Some(c) = candidates.iter().find(|c| c.model == *model_name) {
                        if self.budget.within_budget(c.estimated_cost)? {
                            let mut s = c.clone();
                            s.reason = "fallback chain (position in chain)".into();
                            selected = Some(s);
                            break;
                        }
                    }
                }

                let sel = if let Some(s) = selected {
                    s
                } else if candidates.is_empty() {
                    anyhow::bail!("no routable model candidates");
                } else {
                    tracing::warn!(
                        chain_len = chain.len(),
                        candidates = candidates.len(),
                        "fallback chain: no model in budget"
                    );
                    anyhow::bail!(
                        "budget exceeded: no model in the fallback chain fits the remaining budget"
                    )
                };

                let mut alts = Vec::new();
                for c in &candidates {
                    if c.model != sel.model {
                        alts.push(c.clone());
                    }
                }

                Ok(RouteResult {
                    selected: sel,
                    alternatives: alts,
                })
            }
        }
    }
}

fn select_within_budget(
    budget: &BudgetTracker,
    candidates: &mut [ModelCandidate],
    reason: &str,
) -> anyhow::Result<ModelCandidate> {
    for c in candidates.iter_mut() {
        if budget.within_budget(c.estimated_cost)? {
            c.reason = reason.into();
            return Ok(c.clone());
        }
    }
    tracing::warn!(
        candidates = candidates.len(),
        reason,
        "all model candidates exceed budget"
    );
    anyhow::bail!(
        "budget exceeded: no model candidate fits the remaining budget (strategy: {reason})"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::BudgetTracker;

    #[test]
    fn cost_optimized_picks_cheapest() {
        let budget = BudgetTracker::new(Some(100.0));
        let router = ModelRouter::new(RoutingStrategy::CostOptimized, budget);

        let result = router.route(None, 1000, None).unwrap();
        assert!(!result.selected.model.is_empty());
        if result.alternatives.len() > 0 {
            assert!(
                result.selected.estimated_cost <= result.alternatives[0].estimated_cost + 0.0001
            );
        }
    }

    #[test]
    fn fixed_uses_preferred() {
        let budget = BudgetTracker::new(None);
        let router = ModelRouter::new(RoutingStrategy::Fixed, budget);

        let result = router.route(Some("gpt-4o-mini"), 1000, None).unwrap();
        assert_eq!(result.selected.model, "gpt-4o-mini");
    }

    #[test]
    fn budget_constraint_respected() {
        let budget = BudgetTracker::new(Some(0.001));
        let router = ModelRouter::new(RoutingStrategy::CostOptimized, budget);

        let result = router.route(None, 1000, None);
        match result {
            Ok(r) => assert!(
                r.selected.estimated_cost <= 0.002,
                "model should fit within budget"
            ),
            Err(e) => assert!(
                e.to_string().contains("budget exceeded"),
                "should fail with budget exceeded, got: {e}"
            ),
        }
    }

    #[test]
    fn fallback_chain() {
        let budget = BudgetTracker::new(None);
        let mut router = ModelRouter::new(RoutingStrategy::Fallback, budget);
        router.set_fallback_chain(vec![
            "gpt-4o".into(),
            "gpt-4o-mini".into(),
            "deepseek-chat".into(),
        ]);

        let result = router.route(None, 1000, None).unwrap();
        assert_eq!(result.selected.model, "gpt-4o");
    }

    #[test]
    fn tier_max_caps_to_smaller_models() {
        use fastclaw_core::complexity::ComplexityTier;

        let budget = BudgetTracker::new(None);
        let router = ModelRouter::new(RoutingStrategy::CostOptimized, budget);
        let tier = RouteTierConstraints {
            estimated: ComplexityTier::Tiny,
            agent_min_tier: None,
            agent_max_tier: Some(ComplexityTier::Small),
        };
        let result = router.route(None, 1000, Some(tier)).unwrap();
        assert!(result.selected.tier <= ComplexityTier::Small);
    }
}
