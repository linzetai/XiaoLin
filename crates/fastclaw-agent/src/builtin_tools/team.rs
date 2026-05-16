//! Team management tools for Coordinator mode.
//!
//! These tools are only available when ExecutionMode::Coordinator is active.
//! They allow the coordinator agent to create worker teams, assign tasks,
//! and collect results.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use fastclaw_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolResult};

use super::coordinator::{
    Coordinator, CoordinatorPlan, CoordinatorStrategy, WorkerTask,
};
use super::task::TaskManager;

fn coord_properties() -> HashMap<String, serde_json::Value> {
    let mut props = HashMap::new();
    props.insert(
        "goal".into(),
        serde_json::json!({
            "type": "string",
            "description": "The high-level goal to decompose into sub-tasks."
        }),
    );
    props.insert(
        "tasks".into(),
        serde_json::json!({
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "description": { "type": "string" },
                    "priority": { "type": "integer" }
                },
                "required": ["id", "description"]
            },
            "description": "Sub-tasks to assign to workers."
        }),
    );
    props.insert(
        "strategy".into(),
        serde_json::json!({
            "type": "string",
            "enum": ["all_required", "best_effort", "retry_on_failure"],
            "description": "How to handle worker failures."
        }),
    );
    props
}

/// Tool: create_team — decompose a goal into worker sub-tasks.
pub struct CreateTeamTool {
    task_manager: Arc<TaskManager>,
}

impl CreateTeamTool {
    pub fn new(task_manager: Arc<TaskManager>) -> Self {
        Self { task_manager }
    }
}

#[async_trait]
impl Tool for CreateTeamTool {
    fn name(&self) -> &str {
        "create_team"
    }

    fn description(&self) -> &str {
        "Create a team of workers to execute sub-tasks in parallel. Only available in Coordinator mode."
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }

    fn is_deferred(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        ToolParameterSchema {
            schema_type: "object".into(),
            properties: coord_properties(),
            required: vec!["goal".into(), "tasks".into()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("Invalid arguments: {e}")),
        };

        let goal = args["goal"].as_str().unwrap_or("").to_string();
        let strategy = match args["strategy"].as_str() {
            Some("all_required") => CoordinatorStrategy::AllRequired,
            Some("retry_on_failure") => CoordinatorStrategy::RetryOnFailure,
            _ => CoordinatorStrategy::BestEffort,
        };

        let tasks: Vec<WorkerTask> = match args["tasks"].as_array() {
            Some(arr) => arr
                .iter()
                .map(|t| WorkerTask {
                    id: t["id"].as_str().unwrap_or("").to_string(),
                    description: t["description"].as_str().unwrap_or("").to_string(),
                    assigned_agent: t["assigned_agent"].as_str().map(String::from),
                    priority: t["priority"].as_u64().unwrap_or(1) as u32,
                })
                .collect(),
            None => return ToolResult::err("'tasks' must be an array"),
        };

        if tasks.is_empty() {
            return ToolResult::err("At least one task is required");
        }

        let plan = CoordinatorPlan {
            goal: goal.clone(),
            tasks,
            strategy,
        };

        let coordinator = Coordinator::new(Arc::clone(&self.task_manager));
        let task_id_map = coordinator.dispatch_workers(&plan).await;
        let dispatched = task_id_map.len();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let results = coordinator.collect_results(&task_id_map, strategy).await;
        let summary = Coordinator::summarize_results(&plan, &results);

        ToolResult::ok(format!(
            "Team created for goal: {goal}\nDispatched {dispatched} workers.\n\n{summary}"
        ))
    }
}

/// Tool: send_message — send a message/instruction to a running worker.
pub struct SendMessageTool {
    task_manager: Arc<TaskManager>,
}

impl SendMessageTool {
    pub fn new(task_manager: Arc<TaskManager>) -> Self {
        Self { task_manager }
    }
}

#[async_trait]
impl Tool for SendMessageTool {
    fn name(&self) -> &str {
        "send_message"
    }

    fn description(&self) -> &str {
        "Send a message or additional instructions to a running worker task."
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }

    fn is_deferred(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "task_id".into(),
            serde_json::json!({
                "type": "string",
                "description": "The task ID to send the message to."
            }),
        );
        props.insert(
            "message".into(),
            serde_json::json!({
                "type": "string",
                "description": "The message content to send."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".into(),
            properties: props,
            required: vec!["task_id".into(), "message".into()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("Invalid arguments: {e}")),
        };

        let task_id = args["task_id"].as_str().unwrap_or("");
        let message = args["message"].as_str().unwrap_or("");

        if task_id.is_empty() || message.is_empty() {
            return ToolResult::err("Both task_id and message are required");
        }

        match self.task_manager.get(task_id) {
            Some(info) => ToolResult::ok(format!(
                "Message noted for task '{}' (status: {:?}). Message: {}",
                task_id, info.status, message
            )),
            None => ToolResult::err(format!("Task '{}' not found", task_id)),
        }
    }
}

/// Tool: list_team — list all current workers and their statuses.
pub struct ListTeamTool {
    task_manager: Arc<TaskManager>,
}

impl ListTeamTool {
    pub fn new(task_manager: Arc<TaskManager>) -> Self {
        Self { task_manager }
    }
}

#[async_trait]
impl Tool for ListTeamTool {
    fn name(&self) -> &str {
        "list_team"
    }

    fn description(&self) -> &str {
        "List all workers in the current team with their status and output."
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }

    fn is_deferred(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        ToolParameterSchema {
            schema_type: "object".into(),
            properties: HashMap::new(),
            required: Vec::new(),
        }
    }

    async fn execute(&self, _arguments: &str) -> ToolResult {
        let tasks = self.task_manager.list();
        if tasks.is_empty() {
            return ToolResult::ok("No active workers.");
        }

        let mut output = format!("Active workers: {}\n\n", tasks.len());
        for info in &tasks {
            output.push_str(&format!(
                "- {} ({:?}): {}\n",
                info.subject,
                info.status,
                info.output.as_deref().unwrap_or("running...")
            ));
        }

        ToolResult::ok(output)
    }
}
