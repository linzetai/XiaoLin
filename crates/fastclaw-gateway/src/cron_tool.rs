use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolResult};
use fastclaw_cron::{CronJob, CronJobStore, JobAction, JobStatus};

/// Built-in tool allowing agents to manage their own cron jobs at runtime.
pub struct ManageCronTool {
    store: Arc<CronJobStore>,
    wake: Arc<tokio::sync::Notify>,
}

impl ManageCronTool {
    pub fn new(store: Arc<CronJobStore>, wake: Arc<tokio::sync::Notify>) -> Self {
        Self { store, wake }
    }
}

#[async_trait]
impl Tool for ManageCronTool {
    fn name(&self) -> &str {
        "manage_cron"
    }

    fn description(&self) -> &str {
        "Manage scheduled cron jobs. \
         Actions: \"list\" — list all cron jobs for this agent; \
         \"create\" — create a new cron job; \
         \"update\" — update an existing cron job by id; \
         \"delete\" — delete a cron job by id. \
         Cron jobs can trigger an agent chat message on a schedule, or call a webhook URL. \
         The schedule uses 6-field cron syntax: 'sec min hour day_of_month month day_of_week'. \
         Examples: '0 */5 * * * *' = every 5 minutes, '0 0 9 * * 1-5' = 9am weekdays, '0 30 8 1 * *' = 8:30am on the 1st. \
         For agent_chat action, 'message' is the prompt sent to the agent. \
         For webhook action, 'url', optional 'method' (POST/GET/PUT/DELETE), and optional 'body' (JSON) are supported."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "action".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["list", "create", "update", "delete"],
                "description": "The action to perform."
            }),
        );
        props.insert(
            "agent_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The agent ID that owns the cron jobs. Defaults to 'main' if omitted."
            }),
        );
        props.insert(
            "job_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Job ID (required for update/delete)."
            }),
        );
        props.insert(
            "name".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Human-readable job name (required for create, optional for update)."
            }),
        );
        props.insert(
            "schedule".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "6-field cron expression: 'sec min hour day_of_month month day_of_week'. Required for create, optional for update."
            }),
        );
        props.insert(
            "enabled".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "Whether the job is enabled. Defaults to true."
            }),
        );
        props.insert(
            "action_type".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["agent_chat", "webhook"],
                "description": "Type of action to perform when the cron fires. Required for create."
            }),
        );
        props.insert(
            "message".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The prompt message to send (for agent_chat action)."
            }),
        );
        props.insert(
            "session_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional session ID for agent_chat. If omitted, a dedicated session per cron job is used."
            }),
        );
        props.insert(
            "url".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Webhook URL (for webhook action)."
            }),
        );
        props.insert(
            "method".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "HTTP method for webhook (POST, GET, PUT, DELETE). Default: POST."
            }),
        );
        props.insert(
            "body".to_string(),
            serde_json::json!({
                "type": "object",
                "description": "Optional JSON body for webhook."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["action".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::err(format!("invalid JSON arguments: {e}"));
            }
        };

        let action = match args.get("action").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => return ToolResult::err("missing required field 'action'".to_string()),
        };

        match action {
            "list" => self.handle_list(&args).await,
            "create" => self.handle_create(&args).await,
            "update" => self.handle_update(&args).await,
            "delete" => self.handle_delete(&args).await,
            other => ToolResult::err(format!(
                "unknown action '{other}'. Must be one of: list, create, update, delete"
            )),
        }
    }
}

impl ManageCronTool {
    async fn handle_list(&self, args: &serde_json::Value) -> ToolResult {
        let agent_id = args
            .get("agent_id")
            .and_then(|v| v.as_str())
            .unwrap_or("main");
        match self.store.list_by_agent(agent_id).await {
            Ok(jobs) => {
                let result = serde_json::json!({ "jobs": jobs, "count": jobs.len() });
                ToolResult::ok(serde_json::to_string_pretty(&result).unwrap_or_default())
            }
            Err(e) => ToolResult::err(format!("failed to list cron jobs: {e}")),
        }
    }

    async fn handle_create(&self, args: &serde_json::Value) -> ToolResult {
        let agent_id = args
            .get("agent_id")
            .and_then(|v| v.as_str())
            .unwrap_or("main")
            .to_string();
        let name = match args.get("name").and_then(|v| v.as_str()) {
            Some(n) => n.to_string(),
            None => return ToolResult::err("'name' is required for create".to_string()),
        };
        let schedule = match args.get("schedule").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => return ToolResult::err("'schedule' is required for create".to_string()),
        };

        if schedule.parse::<cron::Schedule>().is_err() {
            return ToolResult::err(format!("invalid cron expression: '{schedule}'"));
        }

        let action_type = args
            .get("action_type")
            .and_then(|v| v.as_str())
            .unwrap_or("agent_chat");

        let job_action = match action_type {
            "agent_chat" => {
                let message = args
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("scheduled task triggered")
                    .to_string();
                let session_id = args
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                JobAction::AgentChat {
                    agent_id,
                    message,
                    session_id,
                }
            }
            "webhook" => {
                let url = match args.get("url").and_then(|v| v.as_str()) {
                    Some(u) => u.to_string(),
                    None => {
                        return ToolResult::err("'url' is required for webhook action".to_string());
                    }
                };
                if let Err(e) = fastclaw_security::ssrf::ssrf_check_url(&url) {
                    return ToolResult::err(format!("webhook URL rejected: {e}"));
                }
                let method = args
                    .get("method")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let body = args.get("body").cloned();
                JobAction::Webhook { url, method, body }
            }
            other => {
                return ToolResult::err(format!(
                    "unknown action_type '{other}'. Must be 'agent_chat' or 'webhook'"
                ));
            }
        };

        let enabled = args
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let job = CronJob {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            schedule,
            action: job_action,
            enabled,
            last_run: None,
            next_run: None,
            status: JobStatus::Idle,
            created_at: chrono::Utc::now().to_rfc3339(),
            run_count: 0,
            error_count: 0,
            last_error: None,
        };

        match self.store.upsert(&job).await {
            Ok(()) => {
                self.wake.notify_one();
                let result = serde_json::json!({ "id": job.id, "ok": true });
                ToolResult::ok(serde_json::to_string(&result).unwrap_or_default())
            }
            Err(e) => ToolResult::err(format!("failed to create cron job: {e}")),
        }
    }

    async fn handle_update(&self, args: &serde_json::Value) -> ToolResult {
        let job_id = match args.get("job_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => return ToolResult::err("'job_id' is required for update".to_string()),
        };

        let mut job = match self.store.get(job_id).await {
            Ok(Some(j)) => j,
            Ok(None) => return ToolResult::err(format!("cron job not found: {job_id}")),
            Err(e) => return ToolResult::err(format!("failed to get job: {e}")),
        };

        if let Some(name) = args.get("name").and_then(|v| v.as_str()) {
            job.name = name.to_string();
        }
        if let Some(schedule) = args.get("schedule").and_then(|v| v.as_str()) {
            if schedule.parse::<cron::Schedule>().is_err() {
                return ToolResult::err(format!("invalid cron expression: '{schedule}'"));
            }
            job.schedule = schedule.to_string();
            job.next_run = None;
        }
        if let Some(enabled) = args.get("enabled").and_then(|v| v.as_bool()) {
            job.enabled = enabled;
        }
        if let Some(new_message) = args.get("message").and_then(|v| v.as_str()) {
            if let JobAction::AgentChat {
                ref mut message, ..
            } = job.action
            {
                *message = new_message.to_string();
            }
        }

        match self.store.upsert(&job).await {
            Ok(()) => {
                self.wake.notify_one();
                let result = serde_json::json!({ "id": job.id, "ok": true });
                ToolResult::ok(serde_json::to_string(&result).unwrap_or_default())
            }
            Err(e) => ToolResult::err(format!("failed to update cron job: {e}")),
        }
    }

    async fn handle_delete(&self, args: &serde_json::Value) -> ToolResult {
        let job_id = match args.get("job_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => return ToolResult::err("'job_id' is required for delete".to_string()),
        };

        match self.store.delete(job_id).await {
            Ok(deleted) => {
                let result = serde_json::json!({ "deleted": deleted });
                ToolResult::ok(serde_json::to_string(&result).unwrap_or_default())
            }
            Err(e) => ToolResult::err(format!("failed to delete cron job: {e}")),
        }
    }
}
