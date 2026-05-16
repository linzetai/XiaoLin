//! Runtime observer — collects execution observations and feeds them
//! into the evolution pipeline (skill extraction, trajectory recording).

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use fastclaw_evolution::{
    Trajectory, TrajectoryOutcome, TrajectoryStep, TrajectoryStore,
};

/// A single observation recorded during agent execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Observation {
    pub event_type: ObservationType,
    pub tool_name: Option<String>,
    pub success: bool,
    pub duration_ms: u64,
    pub metadata: serde_json::Value,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ObservationType {
    ToolCall,
    LlmCall,
    Compact,
    HookFired,
    ErrorRecovery,
    SkillInjected,
}

/// Accumulated observations for a single execution session.
#[derive(Debug)]
pub(crate) struct ObservationStore {
    observations: Vec<Observation>,
    trajectory_steps: Vec<TrajectoryStep>,
    session_id: String,
    agent_id: String,
    start_time: std::time::Instant,
}

impl ObservationStore {
    pub fn new(session_id: &str, agent_id: &str) -> Self {
        Self {
            observations: Vec::new(),
            trajectory_steps: Vec::new(),
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            start_time: std::time::Instant::now(),
        }
    }

    pub fn record(&mut self, obs: Observation) {
        self.observations.push(obs);
    }

    pub fn record_tool_call(
        &mut self,
        tool_name: &str,
        success: bool,
        duration: Duration,
        summary: &str,
    ) {
        let now_ms = self.start_time.elapsed().as_millis() as u64;
        self.observations.push(Observation {
            event_type: ObservationType::ToolCall,
            tool_name: Some(tool_name.to_string()),
            success,
            duration_ms: duration.as_millis() as u64,
            metadata: serde_json::json!({ "summary": summary }),
            timestamp_ms: now_ms,
        });

        self.trajectory_steps.push(TrajectoryStep {
            role: "assistant".into(),
            action_type: "tool_call".into(),
            tool_name: Some(tool_name.to_string()),
            summary: truncate(summary, 200),
            success: Some(success),
        });
    }

    pub fn record_llm_call(
        &mut self,
        model: &str,
        prompt_tokens: u32,
        completion_tokens: u32,
        duration: Duration,
    ) {
        let now_ms = self.start_time.elapsed().as_millis() as u64;
        self.observations.push(Observation {
            event_type: ObservationType::LlmCall,
            tool_name: None,
            success: true,
            duration_ms: duration.as_millis() as u64,
            metadata: serde_json::json!({
                "model": model,
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
            }),
            timestamp_ms: now_ms,
        });
    }

    pub fn record_compact(
        &mut self,
        tokens_before: usize,
        tokens_after: usize,
        method: &str,
    ) {
        let now_ms = self.start_time.elapsed().as_millis() as u64;
        self.observations.push(Observation {
            event_type: ObservationType::Compact,
            tool_name: None,
            success: true,
            duration_ms: 0,
            metadata: serde_json::json!({
                "tokens_before": tokens_before,
                "tokens_after": tokens_after,
                "method": method,
                "tokens_freed": tokens_before.saturating_sub(tokens_after),
            }),
            timestamp_ms: now_ms,
        });
    }

    /// Build a summary for downstream analysis (skill gap detection, etc.).
    pub fn summary(&self) -> ObservationSummary {
        let total_tool_calls = self
            .observations
            .iter()
            .filter(|o| o.event_type == ObservationType::ToolCall)
            .count();
        let failed_tool_calls = self
            .observations
            .iter()
            .filter(|o| o.event_type == ObservationType::ToolCall && !o.success)
            .count();
        let total_llm_calls = self
            .observations
            .iter()
            .filter(|o| o.event_type == ObservationType::LlmCall)
            .count();

        let tool_duration_ms: u64 = self
            .observations
            .iter()
            .filter(|o| o.event_type == ObservationType::ToolCall)
            .map(|o| o.duration_ms)
            .sum();

        let unique_tools: std::collections::HashSet<&str> = self
            .observations
            .iter()
            .filter_map(|o| o.tool_name.as_deref())
            .collect();

        ObservationSummary {
            total_tool_calls,
            failed_tool_calls,
            total_llm_calls,
            tool_duration_ms,
            unique_tools_used: unique_tools.len(),
            elapsed_ms: self.start_time.elapsed().as_millis() as u64,
        }
    }

    /// Drain observations into a Trajectory for recording.
    pub fn into_trajectory(self, outcome: TrajectoryOutcome) -> Trajectory {
        Trajectory {
            id: uuid::Uuid::new_v4().to_string(),
            agent_id: self.agent_id,
            session_id: self.session_id,
            task_type: fastclaw_evolution::infer_task_type(&self.trajectory_steps),
            steps: self.trajectory_steps,
            outcome,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ObservationSummary {
    pub total_tool_calls: usize,
    pub failed_tool_calls: usize,
    pub total_llm_calls: usize,
    pub tool_duration_ms: u64,
    pub unique_tools_used: usize,
    pub elapsed_ms: u64,
}

/// Thread-safe observer handle shared across the runtime.
#[derive(Clone)]
pub(crate) struct RuntimeObserver {
    store: Arc<Mutex<ObservationStore>>,
    trajectory_store: Option<Arc<TrajectoryStore>>,
}

impl RuntimeObserver {
    pub fn new(
        session_id: &str,
        agent_id: &str,
        trajectory_store: Option<Arc<TrajectoryStore>>,
    ) -> Self {
        Self {
            store: Arc::new(Mutex::new(ObservationStore::new(session_id, agent_id))),
            trajectory_store,
        }
    }

    pub async fn record_tool_call(
        &self,
        tool_name: &str,
        success: bool,
        duration: Duration,
        summary: &str,
    ) {
        self.store
            .lock()
            .await
            .record_tool_call(tool_name, success, duration, summary);
    }

    pub async fn record_llm_call(
        &self,
        model: &str,
        prompt_tokens: u32,
        completion_tokens: u32,
        duration: Duration,
    ) {
        self.store
            .lock()
            .await
            .record_llm_call(model, prompt_tokens, completion_tokens, duration);
    }

    pub async fn record_compact(
        &self,
        tokens_before: usize,
        tokens_after: usize,
        method: &str,
    ) {
        self.store
            .lock()
            .await
            .record_compact(tokens_before, tokens_after, method);
    }

    pub async fn summary(&self) -> ObservationSummary {
        self.store.lock().await.summary()
    }

    /// Finalize observations: build trajectory and persist it.
    pub async fn finalize(self, outcome: TrajectoryOutcome) {
        let store = match Arc::try_unwrap(self.store) {
            Ok(mutex) => mutex.into_inner(),
            Err(arc) => {
                let guard = arc.lock().await;
                ObservationStore {
                    observations: guard.observations.clone(),
                    trajectory_steps: guard.trajectory_steps.clone(),
                    session_id: guard.session_id.clone(),
                    agent_id: guard.agent_id.clone(),
                    start_time: guard.start_time,
                }
            }
        };

        let summary = store.summary();
        tracing::info!(
            tool_calls = summary.total_tool_calls,
            failed = summary.failed_tool_calls,
            llm_calls = summary.total_llm_calls,
            elapsed_ms = summary.elapsed_ms,
            "runtime observer finalized"
        );

        if store.trajectory_steps.is_empty() {
            return;
        }

        let trajectory = store.into_trajectory(outcome);

        if let Some(ts) = &self.trajectory_store {
            if let Err(e) = ts.record_trajectory(&trajectory).await {
                tracing::warn!(error = %e, "failed to persist trajectory");
            }
        }
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}
