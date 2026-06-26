use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use xiaolin_core::tool::{
    no_retry_recovery_hint, Tool, ToolErrorType, ToolKind, ToolParameterSchema, ToolResult,
};

fn task_invalid_args(e: impl std::fmt::Display) -> ToolResult {
    ToolResult::err_with_recovery(
        ToolErrorType::InvalidToolParams,
        format!("Invalid arguments: {e}"),
        "Fix the tool arguments JSON schema (required fields, types) before retrying.",
    )
}

fn task_not_found(task_id: &str) -> ToolResult {
    ToolResult::err_with_recovery(
        ToolErrorType::InvalidToolParams,
        format!("Task not found: {task_id}"),
        "Call task_list to see available task IDs, then retry with a valid task_id.",
    )
}

fn task_concurrency_denied(current: usize, max: usize) -> ToolResult {
    ToolResult::err_with_recovery(
        ToolErrorType::ExecutionDenied,
        format!("Cannot create task: concurrency limit reached ({current}/{max} running)."),
        no_retry_recovery_hint(
            "Wait for existing tasks to complete via task_list/task_get, or stop one with background_task_stop before spawning another.",
        ),
    )
}

fn task_create_failed(e: impl std::fmt::Display) -> ToolResult {
    let detail = e.to_string();
    tracing::warn!(error = %detail, "task_create failed");
    ToolResult::err_with_recovery(
        ToolErrorType::ExecutionFailed,
        "Failed to create task",
        "Check subject/description parameters and concurrency limits; call task_list to inspect running tasks.",
    )
}

fn task_operation_failed(action: &str, e: impl std::fmt::Display) -> ToolResult {
    let detail = e.to_string();
    tracing::warn!(error = %detail, action, "task operation failed");
    ToolResult::err_with_recovery(
        ToolErrorType::ExecutionFailed,
        format!("Failed to {action} task"),
        "Call task_get for details or task_list for available tasks.",
    )
}

fn task_no_fields_to_update() -> ToolResult {
    ToolResult::err_with_recovery(
        ToolErrorType::InvalidToolParams,
        "No fields to update.",
        "Provide at least one of: subject, description, status.",
    )
}

/// Status of a managed background task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Metadata for a managed background task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    pub task_id: String,
    pub subject: String,
    pub description: String,
    pub status: TaskStatus,
    pub created_at: u64,
    pub finished_at: Option<u64>,
    pub output: Option<String>,
    pub error: Option<String>,
}

struct TaskHandle {
    info: TaskInfo,
    join_handle: Option<JoinHandle<()>>,
}

/// Manages parallel background tasks with concurrency limits.
///
/// Tasks are stored in a `DashMap` keyed by `task_id`. The manager
/// enforces a maximum concurrency limit — `spawn` rejects new tasks
/// when the limit is reached. Completed/failed tasks auto-update
/// their status. `stop` aborts the tokio task and marks it cancelled.
pub struct TaskManager {
    tasks: Arc<DashMap<String, TaskHandle>>,
    max_concurrency: usize,
    running_count: Arc<AtomicUsize>,
}

/// Remove terminal tasks finished more than this many seconds ago.
const TASK_GC_MAX_AGE_SECS: u64 = 300;

/// Max retained completed/failed/cancelled tasks before pruning oldest.
const MAX_COMPLETED_TASKS: usize = 200;

/// Remove oldest terminal tasks when the completed set exceeds [`MAX_COMPLETED_TASKS`].
fn prune_oldest_completed_tasks_map(tasks: &DashMap<String, TaskHandle>) {
    let mut completed: Vec<(String, u64)> = tasks
        .iter()
        .filter_map(|entry| {
            let handle = entry.value();
            match handle.info.status {
                TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled => handle
                    .info
                    .finished_at
                    .map(|finished| (entry.key().clone(), finished)),
                _ => None,
            }
        })
        .collect();

    if completed.len() <= MAX_COMPLETED_TASKS {
        return;
    }

    completed.sort_by_key(|(_, finished_at)| *finished_at);
    let remove_count = completed.len() - MAX_COMPLETED_TASKS;
    for (id, _) in completed.into_iter().take(remove_count) {
        if let Some((_, mut handle)) = tasks.remove(&id) {
            if let Some(jh) = handle.join_handle.take() {
                jh.abort();
            }
        }
    }
    tracing::warn!(
        max = MAX_COMPLETED_TASKS,
        removed = remove_count,
        remaining = tasks.len(),
        "TaskManager pruned oldest completed tasks"
    );
}

impl TaskManager {
    pub fn new(max_concurrency: usize) -> Self {
        Self {
            tasks: Arc::new(DashMap::new()),
            max_concurrency,
            running_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    fn generate_task_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    /// Drop completed/failed/cancelled tasks older than [`TASK_GC_MAX_AGE_SECS`].
    fn gc_completed_tasks(&self) {
        let now = Self::now_ms();
        let max_age_ms = TASK_GC_MAX_AGE_SECS * 1000;
        self.tasks.retain(|_, handle| match handle.info.status {
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled => handle
                .info
                .finished_at
                .is_some_and(|finished| now.saturating_sub(finished) <= max_age_ms),
            _ => true,
        });
    }

    /// Spawn a new background task. Returns the task_id on success,
    /// or an error if the concurrency limit is reached.
    ///
    /// The `work` future runs on the tokio runtime. When it completes,
    /// the task status is automatically updated to `Completed` (on Ok)
    /// or `Failed` (on Err). The result string is stored in `output` or `error`.
    pub fn spawn<F>(
        &self,
        subject: String,
        description: String,
        work: F,
    ) -> Result<String, TaskManagerError>
    where
        F: Future<Output = Result<String, String>> + Send + 'static,
    {
        self.gc_completed_tasks();

        let current = self.running_count.load(Ordering::Acquire);
        if current >= self.max_concurrency {
            return Err(TaskManagerError::ConcurrencyLimitReached {
                max: self.max_concurrency,
                current,
            });
        }

        let task_id = Self::generate_task_id();
        let info = TaskInfo {
            task_id: task_id.clone(),
            subject,
            description,
            status: TaskStatus::Running,
            created_at: Self::now_ms(),
            finished_at: None,
            output: None,
            error: None,
        };

        let tasks = Arc::clone(&self.tasks);
        let running = Arc::clone(&self.running_count);
        let id = task_id.clone();

        running.fetch_add(1, Ordering::AcqRel);

        let handle = tokio::spawn(async move {
            let result = work.await;
            let now = TaskManager::now_ms();

            if let Some(mut entry) = tasks.get_mut(&id) {
                match result {
                    Ok(output) => {
                        entry.info.status = TaskStatus::Completed;
                        entry.info.output = Some(output);
                    }
                    Err(error) => {
                        entry.info.status = TaskStatus::Failed;
                        entry.info.error = Some(error);
                    }
                }
                entry.info.finished_at = Some(now);
                entry.join_handle = None;
            }

            prune_oldest_completed_tasks_map(&tasks);

            running.fetch_sub(1, Ordering::AcqRel);
        });

        self.tasks.insert(
            task_id.clone(),
            TaskHandle {
                info,
                join_handle: Some(handle),
            },
        );

        Ok(task_id)
    }

    /// Get a snapshot of a task's info.
    pub fn get(&self, task_id: &str) -> Option<TaskInfo> {
        self.tasks.get(task_id).map(|entry| entry.info.clone())
    }

    /// List all tasks.
    pub fn list(&self) -> Vec<TaskInfo> {
        self.tasks.iter().map(|entry| entry.info.clone()).collect()
    }

    /// Stop a running task by aborting its tokio JoinHandle.
    /// Returns `true` if the task was running and is now cancelled.
    pub fn stop(&self, task_id: &str) -> Result<bool, TaskManagerError> {
        // Mutate the task while holding the DashMap shard guard, then DROP the
        // guard before pruning. `prune_oldest_completed_tasks_map` iterates the
        // whole map (`tasks.iter()`), which locks every shard — calling it while
        // still holding `entry`'s shard write-lock self-deadlocks.
        let cancelled = {
            let mut entry = self
                .tasks
                .get_mut(task_id)
                .ok_or(TaskManagerError::NotFound(task_id.to_string()))?;

            match entry.info.status {
                TaskStatus::Running | TaskStatus::Pending => {
                    if let Some(handle) = entry.join_handle.take() {
                        handle.abort();
                        self.running_count.fetch_sub(1, Ordering::AcqRel);
                    }
                    entry.info.status = TaskStatus::Cancelled;
                    entry.info.finished_at = Some(Self::now_ms());
                    true
                }
                TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled => false,
            }
        };

        if cancelled {
            prune_oldest_completed_tasks_map(&self.tasks);
        }
        Ok(cancelled)
    }

    /// Update a task's metadata and/or status.
    ///
    /// Returns a list of field names that were actually changed.
    pub fn update(
        &self,
        task_id: &str,
        subject: Option<String>,
        description: Option<String>,
        status: Option<TaskStatus>,
    ) -> Result<Vec<&'static str>, TaskManagerError> {
        let mut entry = self
            .tasks
            .get_mut(task_id)
            .ok_or(TaskManagerError::NotFound(task_id.to_string()))?;

        let mut changed = Vec::new();

        if let Some(s) = subject {
            if s != entry.info.subject {
                entry.info.subject = s;
                changed.push("subject");
            }
        }
        if let Some(d) = description {
            if d != entry.info.description {
                entry.info.description = d;
                changed.push("description");
            }
        }
        if let Some(new_status) = status {
            if new_status != entry.info.status {
                let old_status = entry.info.status;
                entry.info.status = new_status;
                changed.push("status");

                let is_terminal = matches!(
                    new_status,
                    TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
                );
                if is_terminal && entry.info.finished_at.is_none() {
                    entry.info.finished_at = Some(Self::now_ms());
                }

                if old_status == TaskStatus::Running
                    && is_terminal
                    && entry.join_handle.take().is_some()
                {
                    self.running_count.fetch_sub(1, Ordering::AcqRel);
                }
            }
        }

        Ok(changed)
    }

    /// Number of currently running tasks.
    pub fn running_count(&self) -> usize {
        self.running_count.load(Ordering::Acquire)
    }

    /// Total number of tasks (all statuses).
    pub fn total_count(&self) -> usize {
        self.tasks.len()
    }
}

/// Errors from TaskManager operations.
#[derive(Debug, Clone)]
pub enum TaskManagerError {
    NotFound(String),
    ConcurrencyLimitReached { max: usize, current: usize },
}

impl std::fmt::Display for TaskManagerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "task not found: {id}"),
            Self::ConcurrencyLimitReached { max, current } => {
                write!(
                    f,
                    "concurrency limit reached: {current}/{max} tasks running"
                )
            }
        }
    }
}

impl std::error::Error for TaskManagerError {}

// ─── Task Work Factory ───────────────────────────────────────────────

/// Trait for creating the async work that a task executes.
///
/// Implementors receive the subject + description and return a future.
/// The default implementation is a no-op that immediately succeeds;
/// production code should provide an `AgentTaskWorkFactory` that calls
/// `AgentRuntime::execute` with a sub-agent config.
#[async_trait]
pub trait TaskWorkFactory: Send + Sync + 'static {
    /// Create and execute the work for a task. Called inside tokio::spawn.
    async fn run(&self, subject: String, description: String) -> Result<String, String>;
}

/// Default factory that immediately returns success (for unit tests).
pub struct NoopTaskWorkFactory;

#[async_trait]
impl TaskWorkFactory for NoopTaskWorkFactory {
    async fn run(&self, _subject: String, description: String) -> Result<String, String> {
        Ok(format!("Task completed: {description}"))
    }
}

// ─── TaskCreateTool ──────────────────────────────────────────────────

/// Tool that creates a new background task via the TaskManager.
///
/// The work factory determines what actually executes when a task is spawned.
/// In production, inject an `AgentTaskWorkFactory` that delegates to the
/// `AgentRuntime`. For testing, use `NoopTaskWorkFactory`.
pub struct TaskCreateTool {
    manager: Arc<TaskManager>,
    work_factory: Arc<dyn TaskWorkFactory>,
}

impl TaskCreateTool {
    pub fn new(manager: Arc<TaskManager>, work_factory: Arc<dyn TaskWorkFactory>) -> Self {
        Self {
            manager,
            work_factory,
        }
    }

    /// Convenience constructor with the default no-op factory.
    pub fn with_noop(manager: Arc<TaskManager>) -> Self {
        Self::new(manager, Arc::new(NoopTaskWorkFactory))
    }
}

#[derive(Deserialize)]
struct TaskCreateArgs {
    subject: String,
    #[serde(default)]
    description: Option<String>,
}

#[async_trait]
impl Tool for TaskCreateTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }

    fn name(&self) -> &str {
        "task_create"
    }

    fn description(&self) -> &str {
        "Create a new background task. The task runs asynchronously and its \
         progress can be monitored with task_list/task_get. Returns the unique task_id."
    }

    fn prompt(&self) -> String {
        "Launch a new agent to handle complex, multi-step tasks autonomously.\n\n\
The task_create tool launches specialized background agents that work independently \
in their own context. Each agent gets a fresh context window, its own tool access, \
and runs without seeing your conversation history.\n\n\
## Decision Tree: When to Delegate\n\n\
**USE task_create when:**\n\
- The task requires 5+ tool calls and is mostly independent\n\
- You need to explore multiple areas of the codebase in parallel\n\
- The task involves a large refactor across many files\n\
- You want to try multiple approaches simultaneously (best-of-N)\n\
- The task is research-heavy (reading docs, exploring unfamiliar code)\n\
- You're working on something else and want this done concurrently\n\n\
**Do NOT use task_create when:**\n\
- The task needs 1-3 tool calls — just do it directly\n\
- You need the result immediately to make a decision\n\
- The task requires back-and-forth with the user\n\
- You need to modify files that you're currently editing (conflict risk)\n\
- Reading a specific known file — use `read_file` instead\n\
- Searching for a symbol — use `search_in_files` instead\n\
- Simple git operations — do them directly\n\n\
## Isolation Model\n\n\
Each spawned agent operates in isolation:\n\
- **Separate context**: The agent does NOT see your conversation, only the prompt you provide\n\
- **Shared filesystem**: The agent can read/write the same files (coordinate carefully!)\n\
- **Independent tools**: The agent has its own tool budget and execution context\n\
- **No communication mid-task**: You cannot send follow-up instructions after launch\n\
- **Single result**: Agent returns one final message when complete\n\n\
### File Conflict Prevention\n\
- If you and the agent both modify the same file, conflicts will occur\n\
- Assign agents to DIFFERENT files or regions of the codebase\n\
- For large refactors: assign non-overlapping file sets to each agent\n\
- Use task_get to check completion BEFORE modifying shared files\n\n\
## Writing Effective Prompts\n\n\
Brief the agent like a smart colleague who just joined — they have no prior context.\n\n\
### Essential Elements\n\
1. **Goal**: What to accomplish and WHY it matters\n\
2. **Context**: What you've already learned or tried (avoid re-exploration)\n\
3. **Specifics**: File paths, line numbers, function names, exact changes needed\n\
4. **Constraints**: What NOT to do, style requirements, compatibility needs\n\
5. **Output format**: What you want back (code, summary, file list, etc.)\n\n\
### Prompt Quality Rules\n\
- **Never delegate understanding**: Don't say \"fix the bug\". Say \"the bug is in \
src/parser.rs:142 where it fails to handle empty input — add a guard clause\"\n\
- **Be specific about scope**: \"Only modify files in crates/xiaolin-agent/src/\" \
is better than \"fix the agent code\"\n\
- **Include success criteria**: \"The task is done when `cargo test -p X` passes\"\n\
- **Provide examples if the task is ambiguous**: Show input → expected output\n\n\
### Good vs Bad Prompts\n\
```\n\
BAD:  \"Fix the authentication bug\"\n\
GOOD: \"In src/auth/jwt.rs:89, the token validation skips expiry check when \
the 'iss' claim is missing. Add a guard that returns Err(TokenExpired) when \
exp < now. Run cargo test -p auth to verify.\"\n\n\
BAD:  \"Refactor the database code\"\n\
GOOD: \"Move all SQL queries from src/handlers/*.rs into src/queries/*.rs. \
Each handler file should import from the corresponding query module. \
Don't change any query logic, only file organization. Affected files: \
handlers/users.rs, handlers/posts.rs, handlers/comments.rs\"\n\
```\n\n\
## Parallel Launch Strategy\n\n\
Launch multiple agents concurrently for maximum throughput:\n\n\
### Independent Exploration\n\
```\n\
// Agent 1: Research the auth system\n\
// Agent 2: Research the database layer\n\
// Agent 3: Research the API routes\n\
// → All run in parallel, results collected together\n\
```\n\n\
### Best-of-N Pattern\n\
For tasks with multiple valid approaches:\n\
1. Launch N agents with DIFFERENT strategies in the same message\n\
2. Each agent works independently on the same problem\n\
3. When all complete, compare results and pick the best\n\
4. Use for: performance optimization, API design, error handling approaches\n\n\
### Fan-out / Fan-in\n\
For large codebase changes:\n\
1. Fan-out: Launch agents for each non-overlapping file set\n\
2. Wait: Monitor with task_list/task_get until all complete\n\
3. Fan-in: Review all results, resolve any conflicts, commit together\n\n\
## Result Handling\n\n\
### What You Get Back\n\
- The agent's final text message (summary of what was done)\n\
- This is NOT visible to the user — YOU must relay relevant parts\n\
- File changes made by the agent are already on disk\n\n\
### After Task Completion\n\
1. Read the result with `task_get`\n\
2. Verify the agent's work (run tests, review files)\n\
3. Summarize for the user what was accomplished\n\
4. If the agent failed or needs correction, handle it yourself or launch a new task\n\n\
### Trust but Verify\n\
- Agent outputs should generally be trusted for factual lookups\n\
- For code changes: run the test suite before reporting success\n\
- For research: spot-check a few findings if the result seems surprising\n\n\
## Task Lifecycle\n\n\
- `task_create` → launches the agent, returns task_id immediately\n\
- `task_list` → shows all active/completed tasks\n\
- `task_get` → retrieves result (blocks if still running, or returns current status)\n\
- `background_task_stop` → cancels a running task (agent's partial work remains on disk)\n\n\
## Anti-Patterns\n\n\
- Don't delegate trivial tasks (reading one file, running one command)\n\
- Don't delegate tasks that need user interaction\n\
- Don't launch an agent for something you'll need in the next 2 seconds\n\
- Don't delegate the full user query to an agent — understand it yourself first\n\
- Don't launch agents that will edit the same files simultaneously\n\
- Don't forget to summarize agent results back to the user"
            .to_string()
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "subject".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Short title describing what the task does."
            }),
        );
        props.insert(
            "description".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Detailed instructions for the task (optional)."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["subject".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: TaskCreateArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return task_invalid_args(e),
        };

        let desc = args.description.unwrap_or_default();
        let factory = Arc::clone(&self.work_factory);
        let subject_clone = args.subject.clone();
        let desc_clone = desc.clone();

        let result = self.manager.spawn(args.subject.clone(), desc, async move {
            factory.run(subject_clone, desc_clone).await
        });

        match result {
            Ok(task_id) => ToolResult::ok(
                serde_json::json!({
                    "task_id": task_id,
                    "status": "running",
                    "subject": args.subject,
                })
                .to_string(),
            ),
            Err(TaskManagerError::ConcurrencyLimitReached { max, current }) => {
                task_concurrency_denied(current, max)
            }
            Err(e) => task_create_failed(e),
        }
    }
}

// ─── TaskListTool ────────────────────────────────────────────────────

/// Tool that lists all managed background tasks with their status.
pub struct TaskListTool {
    manager: Arc<TaskManager>,
}

impl TaskListTool {
    pub fn new(manager: Arc<TaskManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for TaskListTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }

    fn name(&self) -> &str {
        "task_list"
    }

    fn description(&self) -> &str {
        "List all background tasks with their id, subject, and status."
    }

    fn search_hint(&self) -> &str {
        "list all tasks background"
    }

    fn is_deferred(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: HashMap::new(),
            required: vec![],
        }
    }

    async fn execute(&self, _arguments: &str) -> ToolResult {
        let mut tasks = self.manager.list();
        if tasks.is_empty() {
            return ToolResult::ok("No tasks found");
        }

        tasks.sort_by(|a, b| a.created_at.cmp(&b.created_at));

        let lines: Vec<String> = tasks
            .iter()
            .map(|t| {
                let status = serde_json::to_value(t.status)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| format!("{:?}", t.status));
                format!("#{} [{}] {}", t.task_id, status, t.subject)
            })
            .collect();

        ToolResult::ok(lines.join("\n"))
    }
}

// ─── TaskGetTool ─────────────────────────────────────────────────────

/// Tool that retrieves detailed information about a specific background task.
pub struct TaskGetTool {
    manager: Arc<TaskManager>,
}

impl TaskGetTool {
    pub fn new(manager: Arc<TaskManager>) -> Self {
        Self { manager }
    }
}

#[derive(Deserialize)]
struct TaskGetArgs {
    task_id: String,
}

#[async_trait]
impl Tool for TaskGetTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }

    fn name(&self) -> &str {
        "task_get"
    }

    fn description(&self) -> &str {
        "Get detailed information about a specific background task by its ID, \
         including status, description, output, and error details."
    }

    fn search_hint(&self) -> &str {
        "retrieve a task by ID"
    }

    fn is_deferred(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "task_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The ID of the task to retrieve."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["task_id".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: TaskGetArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return task_invalid_args(e),
        };

        let info = match self.manager.get(&args.task_id) {
            Some(info) => info,
            None => return task_not_found(&args.task_id),
        };

        let status = serde_json::to_value(info.status)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| format!("{:?}", info.status));

        let mut lines = vec![
            format!("Task #{}: {}", info.task_id, info.subject),
            format!("Status: {}", status),
        ];

        if !info.description.is_empty() {
            lines.push(format!("Description: {}", info.description));
        }

        if let Some(ref output) = info.output {
            lines.push(format!("Output: {}", output));
        }

        if let Some(ref error) = info.error {
            lines.push(format!("Error: {}", error));
        }

        if let Some(finished_at) = info.finished_at {
            let duration_ms = finished_at.saturating_sub(info.created_at);
            lines.push(format!("Duration: {}ms", duration_ms));
        }

        ToolResult::ok(lines.join("\n"))
    }
}

// ─── BackgroundTaskStopTool ──────────────────────────────────────────

/// Tool that stops/cancels a running background task.
pub struct BackgroundTaskStopTool {
    manager: Arc<TaskManager>,
}

impl BackgroundTaskStopTool {
    pub fn new(manager: Arc<TaskManager>) -> Self {
        Self { manager }
    }
}

#[derive(Deserialize)]
struct BackgroundTaskStopArgs {
    task_id: String,
}

#[async_trait]
impl Tool for BackgroundTaskStopTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }

    fn name(&self) -> &str {
        "background_task_stop"
    }

    fn description(&self) -> &str {
        "Stop/cancel a running background task by its ID."
    }

    fn search_hint(&self) -> &str {
        "stop cancel abort task"
    }

    fn is_deferred(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "task_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The ID of the task to stop."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["task_id".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: BackgroundTaskStopArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return task_invalid_args(e),
        };

        match self.manager.stop(&args.task_id) {
            Ok(true) => ToolResult::ok(format!("Task {} cancelled.", args.task_id)),
            Ok(false) => ToolResult::ok(format!(
                "Task {} already finished (not running).",
                args.task_id
            )),
            Err(TaskManagerError::NotFound(_)) => task_not_found(&args.task_id),
            Err(e) => task_operation_failed("stop", e),
        }
    }
}

// ─── TaskUpdateTool ──────────────────────────────────────────────────

/// Tool that updates a task's subject, description, or status.
pub struct TaskUpdateTool {
    manager: Arc<TaskManager>,
}

impl TaskUpdateTool {
    pub fn new(manager: Arc<TaskManager>) -> Self {
        Self { manager }
    }
}

#[derive(Deserialize)]
struct TaskUpdateArgs {
    task_id: String,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    status: Option<TaskStatus>,
}

#[async_trait]
impl Tool for TaskUpdateTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }

    fn name(&self) -> &str {
        "task_update"
    }

    fn description(&self) -> &str {
        "Update a background task's subject, description, or status. \
         At least one field must be provided."
    }

    fn search_hint(&self) -> &str {
        "update modify task status"
    }

    fn is_deferred(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "task_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The ID of the task to update."
            }),
        );
        props.insert(
            "subject".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "New subject/title for the task."
            }),
        );
        props.insert(
            "description".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "New description for the task."
            }),
        );
        props.insert(
            "status".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["pending", "running", "completed", "failed", "cancelled"],
                "description": "New status for the task."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["task_id".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: TaskUpdateArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return task_invalid_args(e),
        };

        if args.subject.is_none() && args.description.is_none() && args.status.is_none() {
            return task_no_fields_to_update();
        }

        match self
            .manager
            .update(&args.task_id, args.subject, args.description, args.status)
        {
            Ok(changed) => {
                if changed.is_empty() {
                    ToolResult::ok(format!(
                        "Task #{}: no changes (values already match).",
                        args.task_id
                    ))
                } else {
                    ToolResult::ok(format!(
                        "Updated task #{}: {}",
                        args.task_id,
                        changed.join(", ")
                    ))
                }
            }
            Err(TaskManagerError::NotFound(_)) => task_not_found(&args.task_id),
            Err(e) => task_operation_failed("update", e),
        }
    }
}

// ── Agent Summary ────────────────────────────────────────────────────

/// Configuration for agent output summarization.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SummaryConfig {
    pub model: String,
    pub max_summary_tokens: u32,
    pub target_compression_ratio: f32,
}

#[allow(dead_code)]
impl Default for SummaryConfig {
    fn default() -> Self {
        Self {
            model: "default".into(),
            max_summary_tokens: 512,
            target_compression_ratio: 0.3,
        }
    }
}

/// Structured summary of a SubAgent's execution result.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSummary {
    pub result: String,
    pub key_decisions: Vec<String>,
    pub files_modified: Vec<String>,
    pub raw_output_len: usize,
    pub summary_len: usize,
}

#[allow(dead_code)]
const SUMMARY_SYSTEM_PROMPT: &str = "\
You are a concise summarizer. Given the full output of a sub-agent task execution, \
produce a structured summary in the following JSON format:\n\
{\"result\": \"<one-sentence outcome>\", \"key_decisions\": [\"<decision1>\", ...], \"files_modified\": [\"<path1>\", ...]}\n\
Rules:\n\
- result: a single sentence describing what was accomplished or failed\n\
- key_decisions: 1-5 important choices the agent made (empty if none)\n\
- files_modified: list of file paths that were created/edited (empty if none)\n\
- Be factual and precise. Do NOT add information not present in the output.\n\
- Output ONLY the JSON object, no markdown fences, no explanation.";

/// Summarize a SubAgent's raw output using an LLM side-query.
///
/// On LLM failure, falls back to truncating the raw output.
#[allow(dead_code)]
pub async fn summarize_agent_output(
    provider: &Arc<dyn crate::llm::LlmProvider>,
    raw_output: &str,
    config: &SummaryConfig,
) -> AgentSummary {
    use crate::runtime::side_query::{side_query, SideQueryOptions, SideQuerySource};
    use xiaolin_core::types::ChatMessage;

    let raw_len = raw_output.len();

    if raw_len < 200 {
        return AgentSummary {
            result: raw_output.to_string(),
            key_decisions: Vec::new(),
            files_modified: Vec::new(),
            raw_output_len: raw_len,
            summary_len: raw_len,
        };
    }

    let user_msg = format!(
        "Summarize the following sub-agent output ({} chars):\n\n{}",
        raw_len,
        truncate_for_context(raw_output, 8000)
    );

    let opts = SideQueryOptions {
        model: config.model.clone(),
        system: Some(SUMMARY_SYSTEM_PROMPT.to_string()),
        messages: vec![ChatMessage {
            role: xiaolin_core::types::Role::User,
            content: Some(serde_json::Value::String(user_msg)),
            ..Default::default()
        }],
        max_tokens: Some(config.max_summary_tokens),
        temperature: 0.0,
        max_retries: 1,
        query_source: SideQuerySource::Foreground,
        optional: true,
        abort: None,
    };

    match side_query(provider, opts).await {
        Ok(Some(sq_result)) => parse_summary_response(&sq_result.content, raw_len),
        _ => fallback_summary(raw_output),
    }
}

#[allow(dead_code)]
fn parse_summary_response(response: &str, raw_output_len: usize) -> AgentSummary {
    #[derive(Deserialize)]
    struct SummaryJson {
        result: Option<String>,
        key_decisions: Option<Vec<String>>,
        files_modified: Option<Vec<String>>,
    }

    let trimmed = response
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    match serde_json::from_str::<SummaryJson>(trimmed) {
        Ok(parsed) => {
            let result = parsed.result.unwrap_or_else(|| "Task completed".into());
            let summary_len = result.len();
            AgentSummary {
                result,
                key_decisions: parsed.key_decisions.unwrap_or_default(),
                files_modified: parsed.files_modified.unwrap_or_default(),
                raw_output_len,
                summary_len,
            }
        }
        Err(_) => {
            let summary_len = response.len().min(500);
            AgentSummary {
                result: response.chars().take(500).collect(),
                key_decisions: Vec::new(),
                files_modified: Vec::new(),
                raw_output_len,
                summary_len,
            }
        }
    }
}

#[allow(dead_code)]
fn fallback_summary(raw_output: &str) -> AgentSummary {
    let lines: Vec<&str> = raw_output.lines().collect();
    let last_lines: String = lines
        .iter()
        .rev()
        .take(5)
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    let summary = format!("[Summary unavailable — last output]\n{}", last_lines);
    let summary_len = summary.len();
    AgentSummary {
        result: summary,
        key_decisions: Vec::new(),
        files_modified: Vec::new(),
        raw_output_len: raw_output.len(),
        summary_len,
    }
}

#[allow(dead_code)]
fn truncate_for_context(text: &str, max_chars: usize) -> &str {
    if text.len() <= max_chars {
        text
    } else {
        &text[..text.floor_char_boundary(max_chars)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn spawn_and_get_task() {
        let mgr = TaskManager::new(5);
        let id = mgr
            .spawn("test".into(), "a test task".into(), async {
                Ok("done".to_string())
            })
            .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        let info = mgr.get(&id).unwrap();
        assert_eq!(info.task_id, id);
        assert_eq!(info.subject, "test");
        assert_eq!(info.status, TaskStatus::Completed);
        assert_eq!(info.output.as_deref(), Some("done"));
    }

    #[tokio::test]
    async fn task_failure_updates_status() {
        let mgr = TaskManager::new(5);
        let id = mgr
            .spawn("fail".into(), "will fail".into(), async {
                Err("something went wrong".to_string())
            })
            .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        let info = mgr.get(&id).unwrap();
        assert_eq!(info.status, TaskStatus::Failed);
        assert_eq!(info.error.as_deref(), Some("something went wrong"));
        assert!(info.finished_at.is_some());
    }

    #[tokio::test]
    async fn concurrency_limit_rejects_excess() {
        let mgr = TaskManager::new(2);

        // Spawn 2 long-running tasks.
        mgr.spawn("t1".into(), "".into(), async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok("ok".to_string())
        })
        .unwrap();
        mgr.spawn("t2".into(), "".into(), async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok("ok".to_string())
        })
        .unwrap();

        // Third should be rejected.
        let result = mgr.spawn("t3".into(), "".into(), async { Ok("ok".to_string()) });
        assert!(result.is_err());
        match result.unwrap_err() {
            TaskManagerError::ConcurrencyLimitReached { max, current } => {
                assert_eq!(max, 2);
                assert_eq!(current, 2);
            }
            _ => panic!("expected ConcurrencyLimitReached"),
        }
    }

    #[tokio::test]
    async fn stop_cancels_running_task() {
        let mgr = TaskManager::new(5);
        let id = mgr
            .spawn("long".into(), "".into(), async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Ok("should not reach".to_string())
            })
            .unwrap();

        tokio::time::sleep(Duration::from_millis(10)).await;

        let stopped = mgr.stop(&id).unwrap();
        assert!(stopped);

        let info = mgr.get(&id).unwrap();
        assert_eq!(info.status, TaskStatus::Cancelled);
        assert!(info.finished_at.is_some());
    }

    #[tokio::test]
    async fn stop_completed_task_returns_false() {
        let mgr = TaskManager::new(5);
        let id = mgr
            .spawn("quick".into(), "".into(), async { Ok("done".to_string()) })
            .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        let stopped = mgr.stop(&id).unwrap();
        assert!(!stopped);
    }

    #[tokio::test]
    async fn stop_nonexistent_returns_error() {
        let mgr = TaskManager::new(5);
        let result = mgr.stop("nonexistent");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn list_returns_all_tasks() {
        let mgr = TaskManager::new(10);
        mgr.spawn("a".into(), "".into(), async { Ok("ok".to_string()) })
            .unwrap();
        mgr.spawn("b".into(), "".into(), async { Ok("ok".to_string()) })
            .unwrap();
        mgr.spawn("c".into(), "".into(), async { Ok("ok".to_string()) })
            .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        let list = mgr.list();
        assert_eq!(list.len(), 3);
    }

    #[tokio::test]
    async fn update_task_metadata() {
        let mgr = TaskManager::new(5);
        let id = mgr
            .spawn("original".into(), "desc".into(), async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                Ok("ok".to_string())
            })
            .unwrap();

        mgr.update(&id, Some("updated".into()), None, None).unwrap();

        let info = mgr.get(&id).unwrap();
        assert_eq!(info.subject, "updated");
        assert_eq!(info.description, "desc");
    }

    #[tokio::test]
    async fn running_count_tracks_active_tasks() {
        let mgr = TaskManager::new(10);
        assert_eq!(mgr.running_count(), 0);

        mgr.spawn("t1".into(), "".into(), async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok("ok".to_string())
        })
        .unwrap();

        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(mgr.running_count(), 1);
    }

    #[tokio::test]
    async fn completed_task_decrements_running_count() {
        let mgr = TaskManager::new(10);

        mgr.spawn("quick".into(), "".into(), async { Ok("done".to_string()) })
            .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(mgr.running_count(), 0);
    }

    #[tokio::test]
    async fn concurrency_slot_freed_after_completion() {
        let mgr = TaskManager::new(1);

        let id1 = mgr
            .spawn("t1".into(), "".into(), async { Ok("done".to_string()) })
            .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(mgr.get(&id1).unwrap().status, TaskStatus::Completed);

        // Now slot is freed, should accept a new task.
        let result = mgr.spawn("t2".into(), "".into(), async { Ok("ok".to_string()) });
        assert!(result.is_ok());
    }

    // ═══════════════════════════════════════════════════════════════
    // TaskCreateTool tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn task_create_tool_success() {
        let mgr = Arc::new(TaskManager::new(5));
        let tool = TaskCreateTool::with_noop(Arc::clone(&mgr));

        let result = tool
            .execute(r#"{"subject": "test task", "description": "do something"}"#)
            .await;
        assert!(result.success);

        let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert!(output.get("task_id").is_some());
        assert_eq!(output["status"], "running");
        assert_eq!(output["subject"], "test task");

        // Verify task was actually created in the manager.
        let task_id = output["task_id"].as_str().unwrap();
        let info = mgr.get(task_id).unwrap();
        assert_eq!(info.subject, "test task");
    }

    #[tokio::test]
    async fn task_create_tool_missing_subject() {
        let mgr = Arc::new(TaskManager::new(5));
        let tool = TaskCreateTool::with_noop(Arc::clone(&mgr));

        let result = tool.execute(r#"{"description": "no subject"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("Invalid arguments"));
    }

    #[tokio::test]
    async fn task_create_tool_concurrency_limit() {
        let mgr = Arc::new(TaskManager::new(1));

        // Fill the slot with a long-running task.
        mgr.spawn("blocker".into(), "".into(), async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok("ok".to_string())
        })
        .unwrap();

        let tool = TaskCreateTool::with_noop(Arc::clone(&mgr));
        let result = tool.execute(r#"{"subject": "will be rejected"}"#).await;
        assert!(!result.success);
        assert_eq!(result.error_type, Some(ToolErrorType::ExecutionDenied));
        assert!(result.output.contains("concurrency limit"));
        assert!(result.output.contains("What to do next:"));
        assert!(result.output.contains("Stop retrying"));
    }

    struct EchoWorkFactory;
    #[async_trait]
    impl TaskWorkFactory for EchoWorkFactory {
        async fn run(&self, subject: String, description: String) -> Result<String, String> {
            Ok(format!("Executed: {subject} - {description}"))
        }
    }

    #[tokio::test]
    async fn task_create_tool_with_custom_factory() {
        let mgr = Arc::new(TaskManager::new(5));
        let tool = TaskCreateTool::new(Arc::clone(&mgr), Arc::new(EchoWorkFactory));

        let result = tool
            .execute(r#"{"subject": "echo test", "description": "hello world"}"#)
            .await;
        assert!(result.success);

        let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        let task_id = output["task_id"].as_str().unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        let info = mgr.get(task_id).unwrap();
        assert_eq!(info.status, TaskStatus::Completed);
        assert_eq!(
            info.output.as_deref(),
            Some("Executed: echo test - hello world")
        );
    }

    #[tokio::test]
    async fn task_create_tool_returns_unique_ids() {
        let mgr = Arc::new(TaskManager::new(10));
        let tool = TaskCreateTool::with_noop(Arc::clone(&mgr));

        let r1 = tool.execute(r#"{"subject": "a"}"#).await;
        let r2 = tool.execute(r#"{"subject": "b"}"#).await;

        let o1: serde_json::Value = serde_json::from_str(&r1.output).unwrap();
        let o2: serde_json::Value = serde_json::from_str(&r2.output).unwrap();

        assert_ne!(o1["task_id"], o2["task_id"]);
    }

    // ═══════════════════════════════════════════════════════════════
    // TaskListTool tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn task_list_tool_empty() {
        let mgr = Arc::new(TaskManager::new(5));
        let tool = TaskListTool::new(Arc::clone(&mgr));

        let result = tool.execute("{}").await;
        assert!(result.success);
        assert_eq!(result.output, "No tasks found");
    }

    #[tokio::test]
    async fn task_list_tool_shows_all_tasks() {
        let mgr = Arc::new(TaskManager::new(10));
        mgr.spawn("alpha".into(), "desc a".into(), async { Ok("ok".into()) })
            .unwrap();
        mgr.spawn("beta".into(), "desc b".into(), async { Err("fail".into()) })
            .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        let tool = TaskListTool::new(Arc::clone(&mgr));
        let result = tool.execute("{}").await;
        assert!(result.success);
        assert!(result.output.contains("alpha"));
        assert!(result.output.contains("beta"));
        assert!(result.output.contains("[completed]"));
        assert!(result.output.contains("[failed]"));
    }

    #[tokio::test]
    async fn task_list_tool_is_deferred() {
        let mgr = Arc::new(TaskManager::new(5));
        let tool = TaskListTool::new(mgr);
        assert!(tool.is_deferred());
        assert_eq!(tool.kind(), ToolKind::Read);
    }

    // ═══════════════════════════════════════════════════════════════
    // TaskGetTool tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn task_get_tool_completed_task() {
        let mgr = Arc::new(TaskManager::new(5));
        let id = mgr
            .spawn("build project".into(), "run cargo build".into(), async {
                Ok("Build succeeded".into())
            })
            .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        let tool = TaskGetTool::new(Arc::clone(&mgr));
        let result = tool.execute(&format!(r#"{{"task_id": "{}"}}"#, id)).await;
        assert!(result.success);
        assert!(result.output.contains("build project"));
        assert!(result.output.contains("completed"));
        assert!(result.output.contains("Build succeeded"));
        assert!(result.output.contains("run cargo build"));
        assert!(result.output.contains("Duration:"));
    }

    #[tokio::test]
    async fn task_get_tool_failed_task() {
        let mgr = Arc::new(TaskManager::new(5));
        let id = mgr
            .spawn("failing task".into(), "will fail".into(), async {
                Err("compilation error".into())
            })
            .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        let tool = TaskGetTool::new(Arc::clone(&mgr));
        let result = tool.execute(&format!(r#"{{"task_id": "{}"}}"#, id)).await;
        assert!(result.success);
        assert!(result.output.contains("failed"));
        assert!(result.output.contains("compilation error"));
    }

    #[tokio::test]
    async fn task_get_tool_not_found() {
        let mgr = Arc::new(TaskManager::new(5));
        let tool = TaskGetTool::new(Arc::clone(&mgr));

        let result = tool.execute(r#"{"task_id": "nonexistent-id"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("Task not found"));
        assert!(result.output.contains("task_list"));
    }

    #[tokio::test]
    async fn task_get_tool_invalid_args() {
        let mgr = Arc::new(TaskManager::new(5));
        let tool = TaskGetTool::new(Arc::clone(&mgr));

        let result = tool.execute(r#"{"wrong_field": "x"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("Invalid arguments"));
    }

    #[tokio::test]
    async fn task_get_tool_is_deferred() {
        let mgr = Arc::new(TaskManager::new(5));
        let tool = TaskGetTool::new(mgr);
        assert!(tool.is_deferred());
        assert_eq!(tool.kind(), ToolKind::Read);
    }

    // ═══════════════════════════════════════════════════════════════
    // BackgroundTaskStopTool tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn background_task_stop_tool_cancels_running() {
        let mgr = Arc::new(TaskManager::new(5));
        let id = mgr
            .spawn("long task".into(), "".into(), async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Ok("never".into())
            })
            .unwrap();

        tokio::time::sleep(Duration::from_millis(10)).await;

        let tool = BackgroundTaskStopTool::new(Arc::clone(&mgr));
        let result = tool.execute(&format!(r#"{{"task_id": "{}"}}"#, id)).await;
        assert!(result.success);
        assert!(result.output.contains("cancelled"));

        let info = mgr.get(&id).unwrap();
        assert_eq!(info.status, TaskStatus::Cancelled);
    }

    #[tokio::test]
    async fn background_task_stop_tool_already_finished() {
        let mgr = Arc::new(TaskManager::new(5));
        let id = mgr
            .spawn("quick".into(), "".into(), async { Ok("done".into()) })
            .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        let tool = BackgroundTaskStopTool::new(Arc::clone(&mgr));
        let result = tool.execute(&format!(r#"{{"task_id": "{}"}}"#, id)).await;
        assert!(result.success);
        assert!(result.output.contains("already finished"));
    }

    #[tokio::test]
    async fn background_task_stop_tool_not_found() {
        let mgr = Arc::new(TaskManager::new(5));
        let tool = BackgroundTaskStopTool::new(Arc::clone(&mgr));

        let result = tool.execute(r#"{"task_id": "nonexistent"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("Task not found"));
    }

    #[tokio::test]
    async fn background_task_stop_tool_is_deferred() {
        let mgr = Arc::new(TaskManager::new(5));
        let tool = BackgroundTaskStopTool::new(mgr);
        assert!(tool.is_deferred());
        assert_eq!(tool.kind(), ToolKind::Execute);
        assert_eq!(tool.name(), "background_task_stop");
    }

    // ═══════════════════════════════════════════════════════════════
    // TaskUpdateTool tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn task_update_tool_subject_and_description() {
        let mgr = Arc::new(TaskManager::new(5));
        let id = mgr
            .spawn("original".into(), "old desc".into(), async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Ok("ok".into())
            })
            .unwrap();

        let tool = TaskUpdateTool::new(Arc::clone(&mgr));
        let result = tool
            .execute(&format!(
                r#"{{"task_id": "{id}", "subject": "renamed", "description": "new desc"}}"#
            ))
            .await;
        assert!(result.success);
        assert!(result.output.contains("subject"));
        assert!(result.output.contains("description"));

        let info = mgr.get(&id).unwrap();
        assert_eq!(info.subject, "renamed");
        assert_eq!(info.description, "new desc");
    }

    #[tokio::test]
    async fn task_update_tool_status_to_completed() {
        let mgr = Arc::new(TaskManager::new(5));
        let id = mgr
            .spawn("task1".into(), "".into(), async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Ok("ok".into())
            })
            .unwrap();

        tokio::time::sleep(Duration::from_millis(10)).await;

        let tool = TaskUpdateTool::new(Arc::clone(&mgr));
        let result = tool
            .execute(&format!(r#"{{"task_id": "{id}", "status": "completed"}}"#))
            .await;
        assert!(result.success);
        assert!(result.output.contains("status"));

        let info = mgr.get(&id).unwrap();
        assert_eq!(info.status, TaskStatus::Completed);
        assert!(info.finished_at.is_some());
    }

    #[tokio::test]
    async fn task_update_tool_no_change_same_values() {
        let mgr = Arc::new(TaskManager::new(5));
        let id = mgr
            .spawn("same".into(), "same desc".into(), async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Ok("ok".into())
            })
            .unwrap();

        let tool = TaskUpdateTool::new(Arc::clone(&mgr));
        let result = tool
            .execute(&format!(
                r#"{{"task_id": "{id}", "subject": "same", "description": "same desc"}}"#
            ))
            .await;
        assert!(result.success);
        assert!(result.output.contains("no changes"));
    }

    #[tokio::test]
    async fn task_update_tool_no_fields_provided() {
        let mgr = Arc::new(TaskManager::new(5));
        let id = mgr
            .spawn("t".into(), "".into(), async { Ok("ok".into()) })
            .unwrap();

        let tool = TaskUpdateTool::new(Arc::clone(&mgr));
        let result = tool.execute(&format!(r#"{{"task_id": "{id}"}}"#)).await;
        assert!(!result.success);
        assert!(result.output.contains("No fields to update"));
    }

    #[tokio::test]
    async fn task_update_tool_not_found() {
        let mgr = Arc::new(TaskManager::new(5));
        let tool = TaskUpdateTool::new(Arc::clone(&mgr));

        let result = tool.execute(r#"{"task_id": "nope", "subject": "x"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("Task not found"));
    }

    #[tokio::test]
    async fn task_update_tool_is_deferred() {
        let mgr = Arc::new(TaskManager::new(5));
        let tool = TaskUpdateTool::new(mgr);
        assert!(tool.is_deferred());
        assert_eq!(tool.kind(), ToolKind::Execute);
    }

    #[tokio::test]
    async fn task_update_status_decrements_running_count() {
        let mgr = Arc::new(TaskManager::new(5));
        let id = mgr
            .spawn("running".into(), "".into(), async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Ok("ok".into())
            })
            .unwrap();

        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(mgr.running_count(), 1);

        mgr.update(&id, None, None, Some(TaskStatus::Completed))
            .unwrap();
        assert_eq!(mgr.running_count(), 0);
    }

    // ═══════════════════════════════════════════════════════════════
    // AgentSummary tests
    // ═══════════════════════════════════════════════════════════════

    #[test]
    fn parse_summary_response_valid_json() {
        let response = r#"{"result": "Deployed backend to prod", "key_decisions": ["Used blue-green deploy"], "files_modified": ["deploy.sh", "config.yaml"]}"#;
        let summary = parse_summary_response(response, 5000);
        assert_eq!(summary.result, "Deployed backend to prod");
        assert_eq!(summary.key_decisions.len(), 1);
        assert_eq!(summary.files_modified.len(), 2);
        assert_eq!(summary.raw_output_len, 5000);
        assert!(summary.summary_len < 5000);
    }

    #[test]
    fn parse_summary_response_invalid_json_uses_raw() {
        let response = "This is not JSON at all";
        let summary = parse_summary_response(response, 10000);
        assert_eq!(summary.result, "This is not JSON at all");
        assert!(summary.key_decisions.is_empty());
        assert!(summary.files_modified.is_empty());
    }

    #[test]
    fn fallback_summary_uses_last_lines() {
        let raw = "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8";
        let summary = fallback_summary(raw);
        assert!(summary.result.contains("[Summary unavailable"));
        assert!(summary.result.contains("line8"));
        assert!(summary.result.contains("line4"));
        assert_eq!(summary.raw_output_len, raw.len());
    }
}
