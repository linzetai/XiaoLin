//! Goal management tools: `get_goal`, `create_goal`, `update_goal`.
//!
//! Allows the agent to set objectives with token budgets, track progress,
//! and mark completion. Integrates with context compaction by triggering
//! summarization when a goal exceeds its budget.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolGroup, ToolKind, ToolParameterSchema, ToolResult};
use tokio::sync::Mutex;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Goal {
    pub id: String,
    pub description: String,
    pub status: GoalStatus,
    pub token_budget: Option<u64>,
    pub tokens_used: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Active,
    Completed,
    Failed,
    Cancelled,
}

pub struct GoalStore {
    goals: Mutex<Vec<Goal>>,
}

impl GoalStore {
    pub fn new() -> Self {
        Self {
            goals: Mutex::new(Vec::new()),
        }
    }

    pub async fn get_active(&self) -> Option<Goal> {
        let goals = self.goals.lock().await;
        goals.iter().find(|g| g.status == GoalStatus::Active).cloned()
    }

    pub async fn create(&self, description: String, token_budget: Option<u64>) -> Goal {
        let mut goals = self.goals.lock().await;
        // Deactivate any current active goal
        for g in goals.iter_mut() {
            if g.status == GoalStatus::Active {
                g.status = GoalStatus::Cancelled;
            }
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let goal = Goal {
            id: format!("goal_{}", goals.len() + 1),
            description,
            status: GoalStatus::Active,
            token_budget,
            tokens_used: 0,
            created_at: now,
            updated_at: now,
        };
        goals.push(goal.clone());
        goal
    }

    pub async fn update_status(&self, goal_id: &str, status: GoalStatus) -> Option<Goal> {
        let mut goals = self.goals.lock().await;
        if let Some(g) = goals.iter_mut().find(|g| g.id == goal_id) {
            g.status = status;
            g.updated_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            Some(g.clone())
        } else {
            None
        }
    }

    pub async fn add_tokens(&self, goal_id: &str, tokens: u64) -> Option<bool> {
        let mut goals = self.goals.lock().await;
        if let Some(g) = goals.iter_mut().find(|g| g.id == goal_id) {
            g.tokens_used += tokens;
            let over_budget = g.token_budget.map(|b| g.tokens_used > b).unwrap_or(false);
            Some(over_budget)
        } else {
            None
        }
    }

    pub async fn all_goals(&self) -> Vec<Goal> {
        self.goals.lock().await.clone()
    }
}

impl Default for GoalStore {
    fn default() -> Self {
        Self::new()
    }
}

// ─── GetGoalTool ─────────────────────────────────────────────────────

pub struct GetGoalTool {
    store: Arc<GoalStore>,
}

impl GetGoalTool {
    pub fn new(store: Arc<GoalStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for GetGoalTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Think
    }
    fn supports_parallel(&self) -> bool {
        true
    }
    fn name(&self) -> &str {
        "get_goal"
    }
    fn description(&self) -> &str {
        "Get the current active goal, its token budget, and usage. \
         Returns null if no active goal is set."
    }
    fn group(&self) -> ToolGroup {
        ToolGroup::Task
    }
    fn is_deferred(&self) -> bool {
        true
    }
    fn search_hint(&self) -> &str {
        "goal objective budget token progress"
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: HashMap::new(),
            required: vec![],
        }
    }
    async fn execute(&self, _arguments: &str) -> ToolResult {
        match self.store.get_active().await {
            Some(goal) => ToolResult::ok(serde_json::to_string_pretty(&goal).unwrap_or_default()),
            None => ToolResult::ok(r#"{"active_goal": null}"#),
        }
    }
}

// ─── CreateGoalTool ──────────────────────────────────────────────────

pub struct CreateGoalTool {
    store: Arc<GoalStore>,
}

impl CreateGoalTool {
    pub fn new(store: Arc<GoalStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for CreateGoalTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }
    fn name(&self) -> &str {
        "create_goal"
    }
    fn description(&self) -> &str {
        "Set a new active goal with an optional token budget. \
         Replaces any existing active goal (marking it cancelled)."
    }
    fn group(&self) -> ToolGroup {
        ToolGroup::Task
    }
    fn is_deferred(&self) -> bool {
        true
    }
    fn search_hint(&self) -> &str {
        "goal objective budget create set plan"
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "description".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Description of the goal/objective."
            }),
        );
        props.insert(
            "token_budget".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Optional token budget for this goal. Triggers summarization when exceeded."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["description".to_string()],
        }
    }
    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("Invalid JSON: {e}")),
        };
        let description = match args.get("description").and_then(|v| v.as_str()) {
            Some(d) => d.to_string(),
            None => return ToolResult::err("Missing required parameter: description"),
        };
        let token_budget = args.get("token_budget").and_then(|v| v.as_u64());
        let goal = self.store.create(description, token_budget).await;
        ToolResult::ok(serde_json::to_string_pretty(&goal).unwrap_or_default())
    }
}

// ─── UpdateGoalTool ──────────────────────────────────────────────────

pub struct UpdateGoalTool {
    store: Arc<GoalStore>,
}

impl UpdateGoalTool {
    pub fn new(store: Arc<GoalStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for UpdateGoalTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }
    fn name(&self) -> &str {
        "update_goal"
    }
    fn description(&self) -> &str {
        "Update the status of a goal. Mark as completed, failed, or cancelled."
    }
    fn group(&self) -> ToolGroup {
        ToolGroup::Task
    }
    fn is_deferred(&self) -> bool {
        true
    }
    fn search_hint(&self) -> &str {
        "goal complete finish fail cancel update status"
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "goal_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The ID of the goal to update."
            }),
        );
        props.insert(
            "status".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["completed", "failed", "cancelled"],
                "description": "New status for the goal."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["goal_id".to_string(), "status".to_string()],
        }
    }
    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("Invalid JSON: {e}")),
        };
        let goal_id = match args.get("goal_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => return ToolResult::err("Missing required parameter: goal_id"),
        };
        let status_str = match args.get("status").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolResult::err("Missing required parameter: status"),
        };
        let status = match status_str {
            "completed" => GoalStatus::Completed,
            "failed" => GoalStatus::Failed,
            "cancelled" => GoalStatus::Cancelled,
            other => return ToolResult::err(format!("Invalid status: {other}. Use completed/failed/cancelled.")),
        };
        match self.store.update_status(&goal_id, status).await {
            Some(goal) => ToolResult::ok(serde_json::to_string_pretty(&goal).unwrap_or_default()),
            None => ToolResult::err(format!("Goal '{goal_id}' not found")),
        }
    }
}
