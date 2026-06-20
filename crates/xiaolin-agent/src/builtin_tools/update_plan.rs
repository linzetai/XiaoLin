use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use serde::Deserialize;
use tokio::sync::RwLock;
use xiaolin_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolResult};
use xiaolin_protocol::{AgentEvent, PlanStep, PlanStepStatus, TurnId};

use super::ask_question::ASK_QUESTION_STREAM_KEY;

type EventTxMap = Arc<DashMap<String, tokio::sync::mpsc::Sender<AgentEvent>>>;

/// Shared plan step state (global, keyed by stream_key internally via EventTxMap).
#[derive(Debug, Clone, Default)]
pub struct PlanStepStore {
    steps: Arc<RwLock<Vec<PlanStep>>>,
    explanation: Arc<RwLock<Option<String>>>,
}

impl PlanStepStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn update(&self, explanation: Option<String>, steps: Vec<PlanStep>) {
        *self.explanation.write().await = explanation;
        *self.steps.write().await = steps;
    }

    pub async fn snapshot(&self) -> (Option<String>, Vec<PlanStep>) {
        let explanation = self.explanation.read().await.clone();
        let steps = self.steps.read().await.clone();
        (explanation, steps)
    }
}

#[derive(Deserialize)]
struct UpdatePlanInput {
    #[serde(default)]
    explanation: Option<String>,
    steps: Vec<InputStep>,
}

#[derive(Deserialize)]
struct InputStep {
    step: String,
    status: String,
}

impl InputStep {
    fn to_plan_step(&self) -> PlanStep {
        let status = match self.status.as_str() {
            "in_progress" => PlanStepStatus::InProgress,
            "completed" => PlanStepStatus::Completed,
            _ => PlanStepStatus::Pending,
        };
        PlanStep {
            step: self.step.clone(),
            status,
        }
    }
}

pub struct UpdatePlanTool {
    store: PlanStepStore,
    stream_event_txs: EventTxMap,
}

impl UpdatePlanTool {
    pub fn new(stream_event_txs: EventTxMap, store: PlanStepStore) -> Self {
        Self {
            store,
            stream_event_txs,
        }
    }
}

#[async_trait]
impl Tool for UpdatePlanTool {
    fn name(&self) -> &str {
        "update_plan"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Think
    }

    fn description(&self) -> &str {
        "Update the step-by-step plan displayed to the user. Call at the start of multi-step tasks and whenever a step's status changes."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "explanation".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional brief note about what changed in the plan (1 sentence)."
            }),
        );
        properties.insert(
            "steps".to_string(),
            serde_json::json!({
                "type": "array",
                "description": "The full plan as ordered steps. Each step is a short action (5-15 words). Exactly one step should be in_progress.",
                "items": {
                    "type": "object",
                    "properties": {
                        "step": {
                            "type": "string",
                            "description": "Short action description"
                        },
                        "status": {
                            "type": "string",
                            "enum": ["pending", "in_progress", "completed"],
                            "description": "Current status of this step"
                        }
                    },
                    "required": ["step", "status"]
                }
            }),
        );

        ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required: vec!["steps".into()],
        }
    }

    fn supports_parallel(&self) -> bool {
        true
    }

    async fn execute(&self, args: &str) -> ToolResult {
        let input: UpdatePlanInput = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("invalid input: {e}")),
        };

        if input.steps.is_empty() {
            return ToolResult::err("steps array cannot be empty");
        }

        if input.steps.len() > 15 {
            return ToolResult::err("too many steps (max 15). Keep plans concise.");
        }

        let plan_steps: Vec<PlanStep> = input.steps.iter().map(|s| s.to_plan_step()).collect();

        self.store
            .update(input.explanation.clone(), plan_steps.clone())
            .await;

        let stream_key = match ASK_QUESTION_STREAM_KEY.try_with(|k| k.clone()) {
            Ok(k) => k,
            Err(_) => {
                return ToolResult::err(
                    "update_plan not available outside chat stream context",
                );
            }
        };

        if let Some(tx) = self.stream_event_txs.get(&stream_key).map(|r| r.value().clone()) {
            let _ = tx
                .send(AgentEvent::PlanUpdate {
                    turn_id: TurnId::new("builtin"),
                    session_id: stream_key.clone(),
                    explanation: input.explanation.clone(),
                    steps: plan_steps.clone(),
                })
                .await;
        }

        let in_progress_count = plan_steps
            .iter()
            .filter(|s| s.status == PlanStepStatus::InProgress)
            .count();
        let completed_count = plan_steps
            .iter()
            .filter(|s| s.status == PlanStepStatus::Completed)
            .count();
        let total = plan_steps.len();

        ToolResult::ok(format!(
            "Plan updated: {completed_count}/{total} completed, {in_progress_count} in progress."
        ))
    }
}
