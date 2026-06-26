//! Goal management tools: `get_goal`, `create_goal`, `update_goal`.
//!
//! Allows the agent to set objectives with token budgets, track progress,
//! and mark completion. Goals are persisted to SQLite via `SessionStore`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use xiaolin_core::tool::{Tool, ToolGroup, ToolKind, ToolParameterSchema, ToolResult};
use xiaolin_protocol::event::GoalData;
use xiaolin_session::{GoalRow, SessionStore};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Goal {
    pub id: String,
    pub description: String,
    pub status: GoalStatus,
    pub token_budget: Option<u64>,
    pub tokens_used: u64,
    pub time_used_seconds: u64,
    pub pause_reason: Option<String>,
    pub continuation_rounds: u32,
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
    Paused,
    BudgetLimited,
}

impl GoalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Paused => "paused",
            Self::BudgetLimited => "budget_limited",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Self::Active),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "paused" => Some(Self::Paused),
            "budget_limited" => Some(Self::BudgetLimited),
            _ => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

impl Goal {
    pub(crate) fn from_row(row: GoalRow) -> Self {
        Self {
            id: row.id,
            description: row.description,
            status: GoalStatus::parse(&row.status).unwrap_or(GoalStatus::Active),
            token_budget: row.token_budget.map(|v| v as u64),
            tokens_used: row.tokens_used as u64,
            time_used_seconds: row.time_used_seconds as u64,
            pause_reason: row.pause_reason,
            continuation_rounds: row.continuation_rounds as u32,
            created_at: row.created_at as u64,
            updated_at: row.updated_at as u64,
        }
    }

    pub fn to_goal_data(&self) -> GoalData {
        GoalData {
            id: self.id.clone(),
            description: self.description.clone(),
            status: self.status.as_str().to_string(),
            token_budget: self.token_budget,
            tokens_used: self.tokens_used,
            time_used_seconds: self.time_used_seconds,
            pause_reason: self.pause_reason.clone(),
            continuation_rounds: self.continuation_rounds,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

const MAX_GOAL_CONTINUATION_ROUNDS: u32 = 50;
const MAX_GOAL_DESCRIPTION_CHARS: usize = 4000;

fn validate_goal_description(desc: &str) -> Result<String, String> {
    let trimmed = desc.trim().to_string();
    if trimmed.is_empty() {
        return Err("goal description must not be empty".into());
    }
    if trimmed.len() > MAX_GOAL_DESCRIPTION_CHARS {
        return Err(format!(
            "goal description exceeds maximum length ({} > {MAX_GOAL_DESCRIPTION_CHARS} chars)",
            trimmed.len()
        ));
    }
    Ok(trimmed)
}

const MAX_IDLE_CONTINUATION_ROUNDS: u32 = 3;
/// Max rounds where tools are called but no writes/executions happen.
/// Prevents read-only verification loops from running indefinitely.
const MAX_STAGNATION_ROUNDS: u32 = 5;

/// Result of recording continuation activity for a goal round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinuationActivityResult {
    /// Normal activity, goal can continue.
    Normal,
    /// No tool calls for too many rounds.
    IdleLimitReached,
    /// Tool calls present but no write/execution progress for too many rounds.
    StagnationLimitReached,
}

struct SessionGoalState {
    continuation_rounds: std::sync::atomic::AtomicU32,
    idle_rounds: std::sync::atomic::AtomicU32,
    stagnation_rounds: std::sync::atomic::AtomicU32,
    objective_updated: std::sync::atomic::AtomicBool,
    last_accounted_tokens: std::sync::atomic::AtomicU64,
    last_accounted_time_secs: std::sync::atomic::AtomicU64,
    budget_warning_sent: std::sync::atomic::AtomicBool,
}

impl Default for SessionGoalState {
    fn default() -> Self {
        Self {
            continuation_rounds: std::sync::atomic::AtomicU32::new(0),
            idle_rounds: std::sync::atomic::AtomicU32::new(0),
            stagnation_rounds: std::sync::atomic::AtomicU32::new(0),
            objective_updated: std::sync::atomic::AtomicBool::new(false),
            last_accounted_tokens: std::sync::atomic::AtomicU64::new(0),
            last_accounted_time_secs: std::sync::atomic::AtomicU64::new(0),
            budget_warning_sent: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

pub struct GoalStore {
    session_store: Arc<SessionStore>,
    session_id: tokio::sync::Mutex<Option<String>>,
    states: dashmap::DashMap<String, Arc<SessionGoalState>>,
}

impl GoalStore {
    pub fn session_store(&self) -> &SessionStore {
        &self.session_store
    }

    pub fn new(session_store: Arc<SessionStore>) -> Self {
        Self {
            session_store,
            session_id: tokio::sync::Mutex::new(None),
            states: dashmap::DashMap::new(),
        }
    }

    fn get_state(&self, session_id: &str) -> Arc<SessionGoalState> {
        self.states
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(SessionGoalState::default()))
            .value()
            .clone()
    }

    async fn current_state(&self) -> Option<Arc<SessionGoalState>> {
        let sid = self.sid().await?;
        Some(self.get_state(&sid))
    }

    /// Mark that the objective was updated; next continuation will inject updated prompt.
    pub async fn mark_objective_updated(&self) {
        if let Some(st) = self.current_state().await {
            st.objective_updated
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Check and clear the objective-updated flag.
    pub async fn take_objective_updated(&self) -> bool {
        match self.current_state().await {
            Some(st) => st
                .objective_updated
                .swap(false, std::sync::atomic::Ordering::Relaxed),
            None => false,
        }
    }

    pub async fn set_session_id(&self, session_id: String) {
        *self.session_id.lock().await = Some(session_id.clone());
        let st = self.get_state(&session_id);
        st.last_accounted_tokens
            .store(0, std::sync::atomic::Ordering::Relaxed);
        st.last_accounted_time_secs
            .store(0, std::sync::atomic::Ordering::Relaxed);
        st.budget_warning_sent
            .store(false, std::sync::atomic::Ordering::Relaxed);
        st.idle_rounds
            .store(0, std::sync::atomic::Ordering::Relaxed);
        if let Ok(Some(goal)) = self.session_store.get_actionable_goal(&session_id).await {
            st.continuation_rounds.store(
                goal.continuation_rounds as u32,
                std::sync::atomic::Ordering::Relaxed,
            );
        }
    }

    async fn sid(&self) -> Option<String> {
        self.session_id.lock().await.clone()
    }

    /// Get the active goal (status = 'active' only). Used for runtime accounting.
    pub async fn get_active(&self) -> Option<Goal> {
        let sid = self.sid().await?;
        self.session_store
            .get_active_goal(&sid)
            .await
            .ok()
            .flatten()
            .map(Goal::from_row)
    }

    /// Get the current non-terminal goal (active, paused, or budget_limited).
    /// Used for user actions (pause/resume/clear/edit) and stop hooks.
    pub async fn get_current(&self) -> Option<Goal> {
        let sid = self.sid().await?;
        self.session_store
            .get_actionable_goal(&sid)
            .await
            .ok()
            .flatten()
            .map(Goal::from_row)
    }

    /// Check if a goal row still exists (regardless of status).
    /// Returns true if the row is in the DB, false if deleted.
    pub async fn row_exists(&self, goal_id: &str) -> bool {
        self.session_store
            .get_goal(goal_id)
            .await
            .ok()
            .flatten()
            .is_some()
    }

    pub async fn create(
        &self,
        description: String,
        token_budget: Option<u64>,
    ) -> Result<Goal, String> {
        let description = validate_goal_description(&description)?;
        let sid = self.sid().await.ok_or("no active session")?;
        if let Some(budget) = token_budget {
            if budget == 0 {
                return Err("goal budgets must be positive when provided".into());
            }
        }
        if let Ok(Some(existing)) = self.session_store.get_actionable_goal(&sid).await {
            return Err(format!(
                "cannot create a new goal because this session already has a goal (id: {}, status: {}). \
                 Use `update_goal` to mark it complete or failed first.",
                existing.id, existing.status
            ));
        }
        let st = self.get_state(&sid);
        st.continuation_rounds
            .store(0, std::sync::atomic::Ordering::Relaxed);
        st.idle_rounds
            .store(0, std::sync::atomic::Ordering::Relaxed);
        st.last_accounted_tokens
            .store(0, std::sync::atomic::Ordering::Relaxed);
        st.last_accounted_time_secs
            .store(0, std::sync::atomic::Ordering::Relaxed);
        st.budget_warning_sent
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let goal_id = format!("goal_{}", uuid::Uuid::new_v4().as_simple());
        let row = GoalRow {
            id: goal_id,
            session_id: sid,
            description,
            status: "active".to_string(),
            token_budget: token_budget.map(|v| v as i64),
            tokens_used: 0,
            time_used_seconds: 0,
            pause_reason: None,
            continuation_rounds: 0,
            created_at: now as i64,
            updated_at: now as i64,
        };
        self.session_store
            .insert_goal(&row)
            .await
            .map_err(|e| format!("failed to insert goal: {e}"))?;
        Ok(Goal::from_row(row))
    }

    /// Update goal status with an optional reason (for pause/budget_limited).
    pub async fn update_status(
        &self,
        goal_id: &str,
        status: GoalStatus,
        reason: Option<&str>,
    ) -> Option<Goal> {
        self.session_store
            .update_goal_status(goal_id, status.as_str(), reason)
            .await
            .ok()?;
        self.session_store
            .get_goal(goal_id)
            .await
            .ok()
            .flatten()
            .map(Goal::from_row)
    }

    /// Update goal description (for goal editing).
    pub async fn update_description(
        &self,
        goal_id: &str,
        description: &str,
    ) -> Result<Goal, String> {
        let description = validate_goal_description(description)?;
        self.session_store
            .update_goal_description(goal_id, &description)
            .await
            .map_err(|e| format!("failed to update description: {e}"))?;
        self.mark_objective_updated().await;
        self.session_store
            .get_goal(goal_id)
            .await
            .ok()
            .flatten()
            .map(Goal::from_row)
            .ok_or_else(|| format!("goal '{goal_id}' not found after update"))
    }

    /// Add token budget to a goal (budget追加).
    pub async fn add_budget(&self, goal_id: &str, amount: u64) -> Option<Goal> {
        self.session_store
            .add_goal_budget(goal_id, amount as i64)
            .await
            .ok()?;
        self.session_store
            .get_goal(goal_id)
            .await
            .ok()
            .flatten()
            .map(Goal::from_row)
    }

    pub async fn add_tokens(&self, goal_id: &str, tokens: u64) -> Option<bool> {
        self.session_store
            .add_goal_tokens(goal_id, tokens as i64)
            .await
            .ok()
            .flatten()
            .map(|(_, over)| over)
    }

    pub async fn add_time(&self, goal_id: &str, seconds: u64) {
        let _ = self
            .session_store
            .add_goal_time(goal_id, seconds as i64)
            .await;
    }

    /// Incremental token accounting: only adds the delta since the last call.
    /// Returns (delta_added, over_budget) where over_budget is true if budget exceeded.
    pub async fn account_tokens(
        &self,
        goal_id: &str,
        cumulative_tokens: u64,
    ) -> Option<(u64, bool)> {
        let st = self.current_state().await?;
        let prev = st
            .last_accounted_tokens
            .swap(cumulative_tokens, std::sync::atomic::Ordering::Relaxed);
        let delta = cumulative_tokens.saturating_sub(prev);
        if delta == 0 {
            return Some((0, false));
        }
        self.add_tokens(goal_id, delta)
            .await
            .map(|over| (delta, over))
    }

    /// Incremental time accounting: only adds the delta since the last call.
    pub async fn account_time(&self, goal_id: &str, cumulative_secs: u64) {
        let st = match self.current_state().await {
            Some(s) => s,
            None => return,
        };
        let prev = st
            .last_accounted_time_secs
            .swap(cumulative_secs, std::sync::atomic::Ordering::Relaxed);
        let delta = cumulative_secs.saturating_sub(prev);
        if delta > 0 {
            self.add_time(goal_id, delta).await;
        }
    }

    /// Check if the goal's budget usage has crossed the 80% threshold.
    /// Returns `true` exactly once per goal (the first time usage exceeds 80%).
    pub async fn check_budget_warning(&self, goal: &Goal) -> bool {
        let budget = match goal.token_budget {
            Some(b) if b > 0 => b,
            _ => return false,
        };
        let st = match self.current_state().await {
            Some(s) => s,
            None => return false,
        };
        let threshold = budget * 80 / 100;
        if goal.tokens_used >= threshold
            && !st
                .budget_warning_sent
                .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            return true;
        }
        false
    }

    pub async fn delete(&self, goal_id: &str) -> bool {
        self.session_store
            .delete_goal(goal_id)
            .await
            .unwrap_or(false)
    }

    /// Increment the continuation round counter and return whether the max has been reached.
    /// Also persists the count to DB.
    pub async fn increment_rounds(&self, goal_id: &str) -> bool {
        let st = match self.current_state().await {
            Some(s) => s,
            None => return false,
        };
        let prev = st
            .continuation_rounds
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let new_val = prev + 1;
        let _ = self
            .session_store
            .update_continuation_rounds(goal_id, new_val as i64)
            .await;
        new_val >= MAX_GOAL_CONTINUATION_ROUNDS
    }

    /// Reset the continuation round counter (called when a new goal is created or resumed).
    pub async fn reset_rounds(&self, goal_id: &str) {
        if let Some(st) = self.current_state().await {
            st.continuation_rounds
                .store(0, std::sync::atomic::Ordering::Relaxed);
            st.idle_rounds
                .store(0, std::sync::atomic::Ordering::Relaxed);
        }
        let _ = self
            .session_store
            .update_continuation_rounds(goal_id, 0)
            .await;
    }

    pub async fn current_rounds(&self) -> u32 {
        match self.current_state().await {
            Some(st) => st
                .continuation_rounds
                .load(std::sync::atomic::Ordering::Relaxed),
            None => 0,
        }
    }

    /// Track whether a continuation round was idle or stagnating.
    /// - `had_tool_calls`: any tool was invoked this round
    /// - `had_progress`: a write/execution tool was invoked (file write, shell, subagent, etc.)
    ///
    /// Returns the activity result indicating normal, idle, or stagnation.
    pub async fn record_continuation_activity(
        &self,
        had_tool_calls: bool,
        had_progress: bool,
    ) -> ContinuationActivityResult {
        let st = match self.current_state().await {
            Some(s) => s,
            None => return ContinuationActivityResult::Normal,
        };
        if had_progress {
            // Real progress resets both counters
            st.idle_rounds
                .store(0, std::sync::atomic::Ordering::Relaxed);
            st.stagnation_rounds
                .store(0, std::sync::atomic::Ordering::Relaxed);
            ContinuationActivityResult::Normal
        } else if had_tool_calls {
            // Tools called but no writes — stagnation
            st.idle_rounds
                .store(0, std::sync::atomic::Ordering::Relaxed);
            let prev = st
                .stagnation_rounds
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if prev + 1 >= MAX_STAGNATION_ROUNDS {
                ContinuationActivityResult::StagnationLimitReached
            } else {
                ContinuationActivityResult::Normal
            }
        } else {
            // No tool calls at all — idle
            let prev = st
                .idle_rounds
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if prev + 1 >= MAX_IDLE_CONTINUATION_ROUNDS {
                ContinuationActivityResult::IdleLimitReached
            } else {
                ContinuationActivityResult::Normal
            }
        }
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
    fn parameters_schema(&self) -> ToolParameterSchema {
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: HashMap::new(),
            required: vec![],
        }
    }
    async fn execute(&self, _arguments: &str) -> ToolResult {
        match self.store.get_current().await {
            Some(goal) => {
                let remaining = goal
                    .token_budget
                    .map(|b| b.saturating_sub(goal.tokens_used));
                let resp = serde_json::json!({
                    "goal": goal,
                    "remaining_tokens": remaining,
                });
                ToolResult::ok(serde_json::to_string_pretty(&resp).unwrap_or_default())
            }
            None => ToolResult::ok(r#"{"goal": null}"#),
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
        "Create a goal only when explicitly requested by the user or system instructions; \
         do not infer goals from ordinary tasks. \
         Set token_budget only when an explicit token budget is requested. \
         Fails if a non-terminal goal already exists; use `update_goal` to mark it complete first."
    }
    fn group(&self) -> ToolGroup {
        ToolGroup::Task
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
                "description": "Optional token budget for this goal. Must be positive."
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
        let token_budget = args.get("token_budget").and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
        });
        if let Some(budget) = token_budget {
            if budget == 0 {
                return ToolResult::err("goal budgets must be positive when provided");
            }
        }
        match self.store.create(description, token_budget).await {
            Ok(goal) => ToolResult::ok(serde_json::to_string_pretty(&goal).unwrap_or_default()),
            Err(e) => ToolResult::err(e),
        }
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
        "Update the status of a goal. You may only mark a goal as completed or failed. \
         Do not mark a goal complete merely because its budget is nearly exhausted — \
         verify that the objective has actually been achieved."
    }
    fn group(&self) -> ToolGroup {
        ToolGroup::Task
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
                "enum": ["completed", "failed"],
                "description": "New status for the goal. Only 'completed' or 'failed' are allowed."
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
            other => {
                return ToolResult::err(format!(
                    "Invalid status: {other}. Only 'completed' or 'failed' are allowed."
                ))
            }
        };
        if !self.store.row_exists(&goal_id).await {
            return ToolResult::err(format!("Goal '{goal_id}' not found"));
        }
        match self.store.update_status(&goal_id, status, None).await {
            Some(goal) => ToolResult::ok(serde_json::to_string_pretty(&goal).unwrap_or_default()),
            None => ToolResult::err(format!(
                "Goal '{goal_id}' is already in a terminal state (completed/failed/cancelled) \
                 and cannot be updated."
            )),
        }
    }
}
