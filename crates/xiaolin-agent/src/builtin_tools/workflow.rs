use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::RwLock;
use xiaolin_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolResult};

// ── Data types ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub name: String,
    pub description: String,
    pub steps: Vec<WorkflowStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub name: String,
    pub prompt: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub validation: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStatus {
    Running,
    Completed,
    Cancelled,
}

impl std::fmt::Display for WorkflowStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub run_id: String,
    pub workflow: String,
    pub status: WorkflowStatus,
    pub current_step: usize,
    pub step_results: Vec<String>,
    pub created_at: String,
}

// ── Store ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WorkflowStore {
    workflow_dir: PathBuf,
    runs: Arc<RwLock<HashMap<String, WorkflowRun>>>,
}

impl WorkflowStore {
    pub fn new(workflow_dir: PathBuf) -> Self {
        Self {
            workflow_dir,
            runs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Load all workflow definitions from the workflow directory.
    pub async fn list_definitions(&self) -> Vec<WorkflowDefinition> {
        let mut defs = Vec::new();
        let dir = &self.workflow_dir;
        let entries = match tokio::fs::read_dir(dir).await {
            Ok(e) => e,
            Err(_) => return defs,
        };

        let mut entries = entries;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !matches!(ext, "json" | "json5" | "yaml" | "yml") {
                continue;
            }
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                let parsed: Option<WorkflowDefinition> = match ext {
                    "yaml" | "yml" => None, // yaml support placeholder
                    _ => serde_json::from_str(&content).ok(),
                };
                if let Some(def) = parsed {
                    defs.push(def);
                }
            }
        }
        defs
    }

    /// Find a workflow definition by name.
    pub async fn find_definition(&self, name: &str) -> Option<WorkflowDefinition> {
        self.list_definitions()
            .await
            .into_iter()
            .find(|d| d.name == name)
    }

    /// Start a new workflow run.
    pub async fn start(&self, workflow_name: &str) -> Result<WorkflowRun, String> {
        let def = self
            .find_definition(workflow_name)
            .await
            .ok_or_else(|| format!("workflow '{workflow_name}' not found"))?;

        if def.steps.is_empty() {
            return Err("workflow has no steps".into());
        }

        let run = WorkflowRun {
            run_id: uuid::Uuid::new_v4().to_string(),
            workflow: workflow_name.to_string(),
            status: WorkflowStatus::Running,
            current_step: 0,
            step_results: Vec::new(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        self.runs
            .write()
            .await
            .insert(run.run_id.clone(), run.clone());
        Ok(run)
    }

    /// Advance a running workflow to the next step.
    pub async fn advance(&self, run_id: &str, step_result: &str) -> Result<WorkflowRun, String> {
        let mut runs = self.runs.write().await;
        let run = runs
            .get_mut(run_id)
            .ok_or_else(|| format!("run '{run_id}' not found"))?;

        if run.status != WorkflowStatus::Running {
            return Err(format!("run is {}, cannot advance", run.status));
        }

        run.step_results.push(step_result.to_string());
        run.current_step += 1;

        let def = self.find_definition(&run.workflow).await;
        let total_steps = def.map(|d| d.steps.len()).unwrap_or(0);

        if run.current_step >= total_steps {
            run.status = WorkflowStatus::Completed;
        }

        Ok(run.clone())
    }

    /// Cancel a running workflow.
    pub async fn cancel(&self, run_id: &str) -> Result<WorkflowRun, String> {
        let mut runs = self.runs.write().await;
        let run = runs
            .get_mut(run_id)
            .ok_or_else(|| format!("run '{run_id}' not found"))?;

        if run.status != WorkflowStatus::Running {
            return Err(format!("run is already {}", run.status));
        }

        run.status = WorkflowStatus::Cancelled;
        Ok(run.clone())
    }

    /// Get the status of a specific run.
    pub async fn status(&self, run_id: &str) -> Option<WorkflowRun> {
        self.runs.read().await.get(run_id).cloned()
    }

    /// List all active runs.
    pub async fn list_runs(&self) -> Vec<WorkflowRun> {
        self.runs.read().await.values().cloned().collect()
    }
}

// ── Tool implementation ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct WorkflowArgs {
    action: String,
    #[serde(default)]
    workflow_name: Option<String>,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    step_result: Option<String>,
}

pub struct WorkflowTool {
    store: WorkflowStore,
}

impl WorkflowTool {
    pub fn new(store: WorkflowStore) -> Self {
        Self { store }
    }
}

const WORKFLOW_DESCRIPTION: &str = "\
Manage multi-step workflows. Workflows are defined as JSON files in the \
workflows directory. Each workflow has named steps with prompts and optional \
tool restrictions.\n\n\
Actions:\n\
- list: Show available workflow definitions and active runs\n\
- start: Begin a new workflow run (requires workflow_name)\n\
- status: Check status of a run (requires run_id)\n\
- advance: Move to the next step (requires run_id, step_result)\n\
- cancel: Cancel a running workflow (requires run_id)";

#[async_trait]
impl Tool for WorkflowTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }
    fn name(&self) -> &str {
        "workflow"
    }

    fn description(&self) -> &str {
        WORKFLOW_DESCRIPTION
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "action".into(),
            json!({
                "type": "string",
                "enum": ["list", "start", "status", "advance", "cancel"],
                "description": "One of: list, start, status, advance, cancel"
            }),
        );
        props.insert(
            "workflow_name".into(),
            json!({
                "type": "string",
                "description": "Name of the workflow to start"
            }),
        );
        props.insert(
            "run_id".into(),
            json!({
                "type": "string",
                "description": "ID of an active workflow run"
            }),
        );
        props.insert(
            "step_result".into(),
            json!({
                "type": "string",
                "description": "Result/output of the current step when advancing"
            }),
        );
        ToolParameterSchema {
            schema_type: "object".into(),
            properties: props,
            required: vec!["action".into()],
        }
    }

    async fn execute(&self, args: &str) -> ToolResult {
        let parsed: WorkflowArgs = match serde_json::from_str(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::err(format!("invalid arguments: {e}")),
        };

        match parsed.action.as_str() {
            "list" => self.handle_list().await,
            "start" => {
                let name = match parsed.workflow_name {
                    Some(n) => n,
                    None => return ToolResult::err("start requires workflow_name"),
                };
                self.handle_start(&name).await
            }
            "status" => {
                let id = match parsed.run_id {
                    Some(id) => id,
                    None => return ToolResult::err("status requires run_id"),
                };
                self.handle_status(&id).await
            }
            "advance" => {
                let id = match parsed.run_id {
                    Some(id) => id,
                    None => return ToolResult::err("advance requires run_id"),
                };
                let result = parsed.step_result.unwrap_or_default();
                self.handle_advance(&id, &result).await
            }
            "cancel" => {
                let id = match parsed.run_id {
                    Some(id) => id,
                    None => return ToolResult::err("cancel requires run_id"),
                };
                self.handle_cancel(&id).await
            }
            other => ToolResult::err(format!("unknown action: {other}")),
        }
    }
}

impl WorkflowTool {
    async fn handle_list(&self) -> ToolResult {
        let defs = self.store.list_definitions().await;
        let runs = self.store.list_runs().await;

        let mut out = String::new();
        out.push_str("## Available Workflows\n\n");
        if defs.is_empty() {
            out.push_str("No workflow definitions found.\n");
        } else {
            for def in &defs {
                out.push_str(&format!(
                    "- **{}**: {} ({} steps)\n",
                    def.name,
                    def.description,
                    def.steps.len()
                ));
            }
        }

        out.push_str("\n## Active Runs\n\n");
        let active: Vec<_> = runs
            .iter()
            .filter(|r| r.status == WorkflowStatus::Running)
            .collect();
        if active.is_empty() {
            out.push_str("No active runs.\n");
        } else {
            for run in active {
                out.push_str(&format!(
                    "- `{}` — workflow: {}, step: {}, status: {}\n",
                    run.run_id, run.workflow, run.current_step, run.status
                ));
            }
        }

        ToolResult::ok(out)
    }

    async fn handle_start(&self, name: &str) -> ToolResult {
        match self.store.start(name).await {
            Ok(run) => {
                let def = self.store.find_definition(name).await;
                let first_step = def
                    .and_then(|d| d.steps.first().cloned())
                    .map(|s| format!("\n\n**First step**: {}\n**Prompt**: {}", s.name, s.prompt))
                    .unwrap_or_default();

                ToolResult::ok(format!(
                    "Workflow '{}' started.\nRun ID: `{}`{first_step}",
                    name, run.run_id
                ))
            }
            Err(e) => ToolResult::err(e),
        }
    }

    async fn handle_status(&self, run_id: &str) -> ToolResult {
        match self.store.status(run_id).await {
            Some(run) => {
                let def = self.store.find_definition(&run.workflow).await;
                let step_info = def
                    .as_ref()
                    .and_then(|d| d.steps.get(run.current_step))
                    .map(|s| format!("\n**Current step**: {} — {}", s.name, s.prompt))
                    .unwrap_or_default();

                ToolResult::ok(format!(
                    "Run `{}`\nWorkflow: {}\nStatus: {}\nStep: {}/{}{step_info}\nResults so far: {}",
                    run.run_id,
                    run.workflow,
                    run.status,
                    run.current_step,
                    def.map(|d| d.steps.len()).unwrap_or(0),
                    run.step_results.len()
                ))
            }
            None => ToolResult::err(format!("run '{run_id}' not found")),
        }
    }

    async fn handle_advance(&self, run_id: &str, step_result: &str) -> ToolResult {
        match self.store.advance(run_id, step_result).await {
            Ok(run) => {
                if run.status == WorkflowStatus::Completed {
                    return ToolResult::ok(format!(
                        "Workflow '{}' completed! All {} steps done.",
                        run.workflow,
                        run.step_results.len()
                    ));
                }

                let def = self.store.find_definition(&run.workflow).await;
                let next_step = def
                    .and_then(|d| d.steps.get(run.current_step).cloned())
                    .map(|s| format!("\n**Next step**: {} — {}", s.name, s.prompt))
                    .unwrap_or_default();

                ToolResult::ok(format!("Advanced to step {}.{next_step}", run.current_step))
            }
            Err(e) => ToolResult::err(e),
        }
    }

    async fn handle_cancel(&self, run_id: &str) -> ToolResult {
        match self.store.cancel(run_id).await {
            Ok(run) => ToolResult::ok(format!(
                "Workflow '{}' run `{}` cancelled at step {}.",
                run.workflow, run.run_id, run.current_step
            )),
            Err(e) => ToolResult::err(e),
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    async fn write_workflow(dir: &std::path::Path, def: &WorkflowDefinition) {
        let path = dir.join(format!("{}.json", def.name));
        let json = serde_json::to_string_pretty(def).unwrap();
        tokio::fs::write(path, json).await.unwrap();
    }

    fn sample_workflow() -> WorkflowDefinition {
        WorkflowDefinition {
            name: "deploy".into(),
            description: "Deploy to production".into(),
            steps: vec![
                WorkflowStep {
                    name: "build".into(),
                    prompt: "Build the project".into(),
                    tools: vec!["shell".into()],
                    validation: None,
                },
                WorkflowStep {
                    name: "test".into(),
                    prompt: "Run tests".into(),
                    tools: vec!["shell".into()],
                    validation: Some("all tests pass".into()),
                },
                WorkflowStep {
                    name: "deploy".into(),
                    prompt: "Deploy to server".into(),
                    tools: vec![],
                    validation: None,
                },
            ],
        }
    }

    #[tokio::test]
    async fn list_definitions_from_dir() {
        let dir = test_dir();
        let store = WorkflowStore::new(dir.path().to_path_buf());
        write_workflow(dir.path(), &sample_workflow()).await;

        let defs = store.list_definitions().await;
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "deploy");
        assert_eq!(defs[0].steps.len(), 3);
    }

    #[tokio::test]
    async fn start_creates_run() {
        let dir = test_dir();
        let store = WorkflowStore::new(dir.path().to_path_buf());
        write_workflow(dir.path(), &sample_workflow()).await;

        let run = store.start("deploy").await.unwrap();
        assert_eq!(run.workflow, "deploy");
        assert_eq!(run.status, WorkflowStatus::Running);
        assert_eq!(run.current_step, 0);
    }

    #[tokio::test]
    async fn start_unknown_workflow_errors() {
        let dir = test_dir();
        let store = WorkflowStore::new(dir.path().to_path_buf());

        let err = store.start("nonexistent").await.unwrap_err();
        assert!(err.contains("not found"));
    }

    #[tokio::test]
    async fn advance_progresses_steps() {
        let dir = test_dir();
        let store = WorkflowStore::new(dir.path().to_path_buf());
        write_workflow(dir.path(), &sample_workflow()).await;

        let run = store.start("deploy").await.unwrap();
        let run = store.advance(&run.run_id, "build ok").await.unwrap();
        assert_eq!(run.current_step, 1);
        assert_eq!(run.status, WorkflowStatus::Running);
        assert_eq!(run.step_results, vec!["build ok"]);
    }

    #[tokio::test]
    async fn advance_completes_workflow() {
        let dir = test_dir();
        let store = WorkflowStore::new(dir.path().to_path_buf());
        write_workflow(dir.path(), &sample_workflow()).await;

        let run = store.start("deploy").await.unwrap();
        let run = store.advance(&run.run_id, "built").await.unwrap();
        let run = store.advance(&run.run_id, "tested").await.unwrap();
        let run = store.advance(&run.run_id, "deployed").await.unwrap();
        assert_eq!(run.status, WorkflowStatus::Completed);
        assert_eq!(run.step_results.len(), 3);
    }

    #[tokio::test]
    async fn cancel_stops_run() {
        let dir = test_dir();
        let store = WorkflowStore::new(dir.path().to_path_buf());
        write_workflow(dir.path(), &sample_workflow()).await;

        let run = store.start("deploy").await.unwrap();
        let run = store.cancel(&run.run_id).await.unwrap();
        assert_eq!(run.status, WorkflowStatus::Cancelled);

        let err = store.advance(&run.run_id, "nope").await.unwrap_err();
        assert!(err.contains("cancelled"));
    }

    #[tokio::test]
    async fn list_runs_shows_active() {
        let dir = test_dir();
        let store = WorkflowStore::new(dir.path().to_path_buf());
        write_workflow(dir.path(), &sample_workflow()).await;

        store.start("deploy").await.unwrap();
        store.start("deploy").await.unwrap();

        let runs = store.list_runs().await;
        assert_eq!(runs.len(), 2);
    }

    #[tokio::test]
    async fn tool_execute_list_action() {
        let dir = test_dir();
        let store = WorkflowStore::new(dir.path().to_path_buf());
        write_workflow(dir.path(), &sample_workflow()).await;

        let tool = WorkflowTool::new(store);
        let result = tool.execute(r#"{"action": "list"}"#).await;
        let text = &result.output;
        assert!(text.contains("deploy"), "should list deploy workflow");
        assert!(text.contains("3 steps"), "should show step count");
    }
}
