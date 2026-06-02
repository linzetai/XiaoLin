use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

fn map_poison<T>(r: std::sync::LockResult<T>) -> anyhow::Result<T> {
    match r {
        Ok(guard) => Ok(guard),
        Err(poisoned) => {
            tracing::warn!("BudgetTracker lock was poisoned, recovering with last known state");
            Ok(poisoned.into_inner())
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost: f64,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetSummary {
    pub total_cost: f64,
    pub budget_limit: Option<f64>,
    pub remaining: Option<f64>,
    pub request_count: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub by_model: HashMap<String, ModelUsage>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelUsage {
    pub cost: f64,
    pub request_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

pub struct BudgetTracker {
    inner: Arc<RwLock<BudgetInner>>,
}

struct BudgetInner {
    limit: Option<f64>,
    total_cost: f64,
    request_count: u64,
    total_input_tokens: u64,
    total_output_tokens: u64,
    by_model: HashMap<String, ModelUsage>,
}

impl BudgetTracker {
    pub fn new(limit: Option<f64>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(BudgetInner {
                limit,
                total_cost: 0.0,
                request_count: 0,
                total_input_tokens: 0,
                total_output_tokens: 0,
                by_model: HashMap::new(),
            })),
        }
    }

    pub fn record(&self, record: &UsageRecord) -> anyhow::Result<()> {
        let mut inner = map_poison(self.inner.write())?;
        inner.total_cost += record.cost;
        inner.request_count += 1;
        inner.total_input_tokens += record.input_tokens as u64;
        inner.total_output_tokens += record.output_tokens as u64;

        let model_usage = inner.by_model.entry(record.model.clone()).or_default();
        model_usage.cost += record.cost;
        model_usage.request_count += 1;
        model_usage.input_tokens += record.input_tokens as u64;
        model_usage.output_tokens += record.output_tokens as u64;
        Ok(())
    }

    pub fn within_budget(&self, estimated_cost: f64) -> anyhow::Result<bool> {
        let inner = map_poison(self.inner.read())?;
        Ok(match inner.limit {
            Some(limit) => inner.total_cost + estimated_cost <= limit,
            None => true,
        })
    }

    /// Atomically check budget and reserve `estimated_cost`.
    ///
    /// Returns `true` if the reservation succeeded (cost fits within budget).
    /// The reserved amount is added to `total_cost` immediately, preventing
    /// concurrent requests from double-spending. Call [`release_reservation`]
    /// to return unused budget if the request is cancelled.
    pub fn try_reserve(&self, estimated_cost: f64) -> anyhow::Result<bool> {
        let mut inner = map_poison(self.inner.write())?;
        match inner.limit {
            Some(limit) if inner.total_cost + estimated_cost > limit => Ok(false),
            _ => {
                inner.total_cost += estimated_cost;
                Ok(true)
            }
        }
    }

    /// Release a previously reserved cost (e.g. request was cancelled).
    pub fn release_reservation(&self, estimated_cost: f64) -> anyhow::Result<()> {
        let mut inner = map_poison(self.inner.write())?;
        inner.total_cost = (inner.total_cost - estimated_cost).max(0.0);
        Ok(())
    }

    pub fn summary(&self) -> anyhow::Result<BudgetSummary> {
        let inner = map_poison(self.inner.read())?;
        Ok(BudgetSummary {
            total_cost: inner.total_cost,
            budget_limit: inner.limit,
            remaining: inner.limit.map(|l| (l - inner.total_cost).max(0.0)),
            request_count: inner.request_count,
            total_input_tokens: inner.total_input_tokens,
            total_output_tokens: inner.total_output_tokens,
            by_model: inner.by_model.clone(),
        })
    }

    pub fn set_limit(&self, limit: Option<f64>) -> anyhow::Result<()> {
        let mut inner = map_poison(self.inner.write())?;
        inner.limit = limit;
        Ok(())
    }

    pub fn reset(&self) -> anyhow::Result<()> {
        let mut inner = map_poison(self.inner.write())?;
        inner.total_cost = 0.0;
        inner.request_count = 0;
        inner.total_input_tokens = 0;
        inner.total_output_tokens = 0;
        inner.by_model.clear();
        Ok(())
    }
}

impl Clone for BudgetTracker {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_tracking() {
        let tracker = BudgetTracker::new(Some(1.0));

        tracker
            .record(&UsageRecord {
                model: "gpt-4o".into(),
                input_tokens: 1000,
                output_tokens: 500,
                cost: 0.3,
                timestamp: "2026-01-01T00:00:00Z".into(),
            })
            .unwrap();

        assert!(tracker.within_budget(0.5).unwrap());
        assert!(!tracker.within_budget(0.8).unwrap());

        let summary = tracker.summary().unwrap();
        assert_eq!(summary.request_count, 1);
        assert!((summary.total_cost - 0.3).abs() < 0.001);
        assert!(summary.by_model.contains_key("gpt-4o"));
    }

    #[test]
    fn no_limit_always_within() {
        let tracker = BudgetTracker::new(None);
        assert!(tracker.within_budget(1_000_000.0).unwrap());
    }

    #[test]
    fn reset_clears_state() {
        let tracker = BudgetTracker::new(Some(10.0));
        tracker
            .record(&UsageRecord {
                model: "test".into(),
                input_tokens: 100,
                output_tokens: 100,
                cost: 5.0,
                timestamp: "now".into(),
            })
            .unwrap();
        tracker.reset().unwrap();
        let s = tracker.summary().unwrap();
        assert_eq!(s.request_count, 0);
        assert!((s.total_cost).abs() < 0.001);
    }
}
