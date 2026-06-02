use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use xiaolin_core::tool::{Tool, ToolParameterSchema, ToolResult};
use xiaolin_cron::{CronJob, CronJobStore, JobAction, JobStatus, NotifyChannel};

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
         Actions: \"list\" — list all cron jobs; \
         \"create\" — create a new job; \
         \"update\" — update by id; \
         \"delete\" — delete by id; \
         \"presets\" — list ready-made CI/CD automation templates.\n\n\
         Schedule uses 6-field cron syntax: 'sec min hour day_of_month month day_of_week'.\n\
         Examples: '0 */5 * * * *' = every 5 min, '0 0 9 * * 1-5' = 9am weekdays, '0 0 2 * * *' = 2am daily.\n\n\
         For agent_chat action, 'message' is the prompt sent to the agent (with full coding tools).\n\
         For webhook action, 'url', optional 'method', and optional 'body' are supported.\n\
         Use 'notify_channels' to push results to IM channels (Feishu, Slack, etc.).\n\n\
         CI/CD Presets (use action=\"presets\" to list, or action=\"create\" with preset=\"<name>\"):\n\
         - lint_fix: Auto-fix lint/clippy warnings nightly\n\
         - test_check: Run test suite and report results\n\
         - build_check: Compile/build and report errors\n\
         - deps_audit: Check for outdated/vulnerable dependencies"
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "action".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["list", "create", "update", "delete", "presets"],
                "description": "The action to perform. Use 'presets' to list CI/CD templates."
            }),
        );
        props.insert(
            "preset".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["lint_fix", "test_check", "build_check", "deps_audit"],
                "description": "Use with action=\"create\" to create a job from a CI/CD preset template. The preset fills in name, schedule, and message automatically."
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
        props.insert(
            "notify_channels".to_string(),
            serde_json::json!({
                "type": "array",
                "description": "Channels to notify when the job completes or fails. Each item: {channel_id, target_id, target_type?}. channel_id is the channel type (e.g. 'feishu', 'slack'). target_id is the chat/group ID to send to. target_type defaults to 'p2p'.",
                "items": {
                    "type": "object",
                    "properties": {
                        "channel_id": {"type": "string"},
                        "target_id": {"type": "string"},
                        "target_type": {"type": "string", "enum": ["p2p", "group"]}
                    },
                    "required": ["channel_id", "target_id"]
                }
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
            "presets" => self.handle_presets().await,
            other => ToolResult::err(format!(
                "unknown action '{other}'. Must be one of: list, create, update, delete, presets"
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

    async fn handle_presets(&self) -> ToolResult {
        let presets = serde_json::json!({
            "presets": [
                {
                    "id": "lint_fix",
                    "name": "自动修复 Lint 错误",
                    "description": "每天凌晨运行 linter，自动修复可修复的警告，提交修复并报告结果。",
                    "default_schedule": "0 0 2 * * *",
                    "message_template": "Run the project linter/clippy in the workspace. For each auto-fixable warning, apply the fix. After all fixes, run the full build to verify. Report: (1) how many warnings found, (2) how many fixed, (3) whether the build passes. If you made changes, commit them with message 'chore: auto-fix lint warnings'."
                },
                {
                    "id": "test_check",
                    "name": "定时测试检查",
                    "description": "定时运行测试套件并报告通过率和失败的测试。",
                    "default_schedule": "0 0 8 * * 1-5",
                    "message_template": "Run the full test suite in the workspace. Report: (1) total tests, (2) passed, (3) failed with names and error summaries, (4) test coverage if available. Keep the report concise."
                },
                {
                    "id": "build_check",
                    "name": "定时编译检查",
                    "description": "定时编译项目，发现编译错误时尝试修复。",
                    "default_schedule": "0 30 7 * * *",
                    "message_template": "Build/compile the project in the workspace. If there are compile errors, analyze each error and attempt to fix it. After fixing, rebuild to verify. Report: (1) initial error count, (2) fixes applied, (3) final build status (pass/fail). If you made changes, commit with message 'fix: resolve compile errors'."
                },
                {
                    "id": "deps_audit",
                    "name": "依赖安全审计",
                    "description": "每周检查依赖更新和已知安全漏洞。",
                    "default_schedule": "0 0 9 * * 1",
                    "message_template": "Audit the project dependencies for known vulnerabilities and outdated packages. Report: (1) total dependencies, (2) outdated packages with current vs latest version, (3) any known security advisories. Do NOT auto-update — just report findings."
                }
            ],
            "usage": "Use action='create' with preset='<id>' to create a job from a template. You can override name, schedule, or message."
        });
        ToolResult::ok(serde_json::to_string_pretty(&presets).unwrap_or_default())
    }

    fn resolve_preset(preset_id: &str) -> Option<(&'static str, &'static str, &'static str)> {
        match preset_id {
            "lint_fix" => Some((
                "自动修复 Lint 错误",
                "0 0 2 * * *",
                "Run the project linter/clippy in the workspace. For each auto-fixable warning, apply the fix. After all fixes, run the full build to verify. Report: (1) how many warnings found, (2) how many fixed, (3) whether the build passes. If you made changes, commit them with message 'chore: auto-fix lint warnings'.",
            )),
            "test_check" => Some((
                "定时测试检查",
                "0 0 8 * * 1-5",
                "Run the full test suite in the workspace. Report: (1) total tests, (2) passed, (3) failed with names and error summaries, (4) test coverage if available. Keep the report concise.",
            )),
            "build_check" => Some((
                "定时编译检查",
                "0 30 7 * * *",
                "Build/compile the project in the workspace. If there are compile errors, analyze each error and attempt to fix it. After fixing, rebuild to verify. Report: (1) initial error count, (2) fixes applied, (3) final build status (pass/fail). If you made changes, commit with message 'fix: resolve compile errors'.",
            )),
            "deps_audit" => Some((
                "依赖安全审计",
                "0 0 9 * * 1",
                "Audit the project dependencies for known vulnerabilities and outdated packages. Report: (1) total dependencies, (2) outdated packages with current vs latest version, (3) any known security advisories. Do NOT auto-update — just report findings.",
            )),
            _ => None,
        }
    }

    async fn handle_create(&self, args: &serde_json::Value) -> ToolResult {
        let agent_id = args
            .get("agent_id")
            .and_then(|v| v.as_str())
            .unwrap_or("main")
            .to_string();

        // Apply preset defaults if specified, then let explicit fields override.
        let preset = args.get("preset").and_then(|v| v.as_str());
        let (default_name, default_schedule, default_message) = if let Some(pid) = preset {
            match Self::resolve_preset(pid) {
                Some((n, s, m)) => (Some(n), Some(s), Some(m)),
                None => {
                    return ToolResult::err(format!(
                        "unknown preset '{pid}'. Use action='presets' to list available templates."
                    ));
                }
            }
        } else {
            (None, None, None)
        };

        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .or(default_name)
            .map(String::from);
        let name = match name {
            Some(n) => n,
            None => return ToolResult::err("'name' is required for create".to_string()),
        };

        let schedule = args
            .get("schedule")
            .and_then(|v| v.as_str())
            .or(default_schedule)
            .map(String::from);
        let schedule = match schedule {
            Some(s) => s,
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
                    .or(default_message)
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
                if let Err(e) = xiaolin_security::ssrf::ssrf_check_url(&url) {
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

        let notify_channels: Vec<NotifyChannel> = args
            .get("notify_channels")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

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
            notify_channels,
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
        if let Some(nc) = args.get("notify_channels") {
            if let Ok(channels) = serde_json::from_value::<Vec<NotifyChannel>>(nc.clone()) {
                job.notify_channels = channels;
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
