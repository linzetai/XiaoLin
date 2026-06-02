use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use xiaolin_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolResult};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub content: String,
    pub status: TodoStatus,
    #[serde(default)]
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

impl std::fmt::Display for TodoStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::InProgress => write!(f, "in_progress"),
            Self::Completed => write!(f, "completed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// In-session todo storage shared across tool invocations.
///
/// Optionally backed by a JSON file for persistence across process restarts.
#[derive(Debug, Clone, Default)]
pub struct TodoStore {
    items: Arc<RwLock<Vec<TodoItem>>>,
    session_path: Option<Arc<PathBuf>>,
}

impl TodoStore {
    pub fn new() -> Self {
        Self {
            items: Arc::new(RwLock::new(Vec::new())),
            session_path: None,
        }
    }

    /// Create a store that auto-persists to the given JSON file.
    pub fn with_session_path(path: PathBuf) -> Self {
        Self {
            items: Arc::new(RwLock::new(Vec::new())),
            session_path: Some(Arc::new(path)),
        }
    }

    /// Restore a store from a session file, falling back to empty if the file
    /// doesn't exist or is malformed.
    pub fn from_session(path: PathBuf) -> Self {
        let items = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<TodoItem>>(&s).ok())
            .unwrap_or_default();
        Self {
            items: Arc::new(RwLock::new(items)),
            session_path: Some(Arc::new(path)),
        }
    }

    pub async fn snapshot(&self) -> Vec<TodoItem> {
        self.items.read().await.clone()
    }

    /// Non-async check for whether there are any in-progress items.
    /// Returns `false` if the lock is contended (safe default: don't raise threshold).
    pub fn has_in_progress_items(&self) -> bool {
        self.items
            .try_read()
            .map(|items| items.iter().any(|t| t.status == TodoStatus::InProgress))
            .unwrap_or(false)
    }

    /// Non-async summary of pending/in_progress todos for prompt injection.
    /// Returns `None` if there are no actionable items or the lock is contended.
    pub fn pending_summary(&self) -> Option<String> {
        let items = self.items.try_read().ok()?;
        let actionable: Vec<_> = items
            .iter()
            .filter(|t| t.status == TodoStatus::Pending || t.status == TodoStatus::InProgress)
            .collect();
        if actionable.is_empty() {
            return None;
        }
        let mut lines = Vec::with_capacity(actionable.len());
        for t in &actionable {
            let icon = if t.status == TodoStatus::InProgress {
                "🔄"
            } else {
                "⬜"
            };
            lines.push(format!("- {icon} [{}] {}", t.id, t.content));
        }
        Some(lines.join("\n"))
    }

    pub async fn replace_all(&self, mut items: Vec<TodoItem>) {
        let now = chrono::Utc::now().to_rfc3339();
        for item in &mut items {
            if item.created_at.is_empty() {
                item.created_at = now.clone();
            }
            if item.status == TodoStatus::Completed && item.completed_at.is_none() {
                item.completed_at = Some(now.clone());
            }
        }
        *self.items.write().await = items;
        self.persist().await;
    }

    pub async fn merge(&self, updates: Vec<TodoItem>) {
        let mut current = self.items.write().await;
        let now = chrono::Utc::now().to_rfc3339();

        for mut update in updates {
            if update.created_at.is_empty() {
                update.created_at = now.clone();
            }
            if update.status == TodoStatus::Completed && update.completed_at.is_none() {
                update.completed_at = Some(now.clone());
            }

            if let Some(existing) = current.iter_mut().find(|t| t.id == update.id) {
                existing.content = update.content;
                existing.status = update.status;
                existing.completed_at = update.completed_at;
                if existing.created_at.is_empty() {
                    existing.created_at = update.created_at;
                }
            } else {
                current.push(update);
            }
        }
        drop(current);
        self.persist().await;
    }

    async fn persist(&self) {
        let Some(path) = &self.session_path else {
            return;
        };
        let items = self.items.read().await;
        let json = match serde_json::to_string_pretty(&*items) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!(error = %e, "failed to serialize todo list for persistence");
                return;
            }
        };
        drop(items);

        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(path.as_ref(), json) {
            tracing::warn!(path = %path.display(), error = %e, "failed to persist todo list");
        }
    }
}

#[derive(Debug, Deserialize)]
struct TodoWriteArgs {
    todos: Vec<NewTodoItem>,
    #[serde(default)]
    merge: Option<bool>,
    #[serde(default)]
    modified_by_user: Option<bool>,
    #[serde(default)]
    modified_content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NewTodoItem {
    id: String,
    content: String,
    #[serde(default)]
    status: TodoStatus,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    completed_at: Option<String>,
}

impl From<NewTodoItem> for TodoItem {
    fn from(item: NewTodoItem) -> Self {
        TodoItem {
            id: item.id,
            content: item.content,
            status: item.status,
            created_at: if item.created_at.is_empty() {
                chrono::Utc::now().to_rfc3339()
            } else {
                item.created_at
            },
            completed_at: item.completed_at,
        }
    }
}

pub struct TodoWriteTool {
    store: TodoStore,
}

impl TodoWriteTool {
    pub fn new(store: TodoStore) -> Self {
        Self { store }
    }
}

const TODO_WRITE_DESCRIPTION: &str = "\
Create and manage a structured task list for the current session. \
Helps track progress and organize complex tasks.\n\n\
## When to Use\n\
- Complex multi-step tasks (3+ steps)\n\
- User provides multiple tasks\n\
- After receiving new instructions: capture as todos\n\
- When starting a task: mark in_progress (ONE at a time)\n\
- After completing: mark completed immediately, then move to next\n\n\
## When NOT to Use\n\
- Single straightforward task or trivial 1-2 step work\n\
- Purely conversational requests\n\n\
## Task Management\n\
- Mark in_progress BEFORE starting work. Only ONE at a time.\n\
- Mark completed IMMEDIATELY after finishing — don't batch.\n\
- If blocked by errors, keep in_progress and add a new task for resolution.\n\
- Prefer creating the first todo as in_progress and start working on it in the same turn.\n\n\
## Workflow\n\
1. Create plan with todos. Mark first task in_progress and begin working immediately.\n\
2. As you work, update statuses. Add new todos if scope expands.\n\
3. After all tasks done, verify against the original request.";

#[async_trait]
impl Tool for TodoWriteTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Think
    }
    fn name(&self) -> &str {
        "todo_write"
    }

    fn description(&self) -> &str {
        TODO_WRITE_DESCRIPTION
    }

    fn prompt(&self) -> String {
        format!(
            "{TODO_WRITE_DESCRIPTION}\n\n\
## Decision Guide: Create or Skip?\n\n\
**CREATE a todo list when:**\n\
- Task requires 3+ distinct steps with dependencies\n\
- User provides multiple tasks (numbered or comma-separated)\n\
- Task touches multiple files or systems (refactoring, migrations)\n\
- Requirements are complex enough that you might forget a step\n\
- The task involves research → implementation → verification phases\n\
- After receiving new instructions — capture requirements immediately\n\n\
**SKIP the todo list when:**\n\
- Single, straightforward task (add a comment, run a command)\n\
- Trivial 1-2 step work with obvious execution path\n\
- Purely conversational/informational requests\n\
- Quick lookups or simple questions\n\n\
## Task Decomposition Principles\n\n\
### Granularity\n\
- Each todo should be completable in 1-5 tool calls\n\
- If a todo needs 10+ tool calls, break it into sub-tasks\n\
- If a todo is just one tool call, merge it with related items\n\n\
### Ordering\n\
- Put dependency-sensitive tasks in the right order\n\
- Group related tasks (all file reads, then all edits, then tests)\n\
- Put verification/testing as the last item\n\n\
### Naming\n\
- Use action verbs: \"Implement X\", \"Add Y to Z\", \"Fix W\"\n\
- Be specific: \"Add validation to login form\" not \"Handle forms\"\n\
- Include the target: file name, function name, or component\n\n\
## State Management Rules\n\n\
### The One-Active Rule\n\
Only ONE todo should be `in_progress` at any time. This signals focus:\n\
- Mark `in_progress` BEFORE starting work on it\n\
- Mark `completed` IMMEDIATELY when done — don't batch completions\n\
- Then mark the next todo `in_progress`\n\n\
### Status Transitions\n\
- `pending` → `in_progress`: About to start working on it\n\
- `in_progress` → `completed`: Work is done and verified\n\
- `pending` → `cancelled`: No longer needed (scope change, duplicate)\n\
- NEVER go backward (`completed` → `in_progress`)\n\n\
### Handling Failures\n\
- If a task hits an error: keep it `in_progress` while debugging\n\
- If you need to pivot: add a NEW todo for the fix, cancel the broken one\n\
- If scope expands: add new todos, don't modify existing completed ones\n\n\
## The merge Parameter\n\n\
- `merge: false` (default for first call): REPLACES the entire todo list\n\
- `merge: true`: Updates specific items by `id`, leaves others unchanged\n\n\
### When to use `merge: true`:\n\
- Marking a single item as completed\n\
- Adding new tasks to an existing list\n\
- Updating a task's content without resetting others\n\n\
### When to use `merge: false`:\n\
- Creating the initial todo list\n\
- Completely restructuring the plan\n\n\
## Parallel Execution Pattern\n\n\
Prefer creating the first todo as `in_progress` AND starting work on it \
in the same tool call batch:\n\n\
In one response:\n\
1. Call todo_write with todos marked in_progress\n\
2. Call the first tool needed for that task\n\
Both execute in the same turn, saving a round-trip.\n\n\
## Examples\n\n\
### When to Create\n\
- \"Add dark mode toggle\" → [state management, styles, component, integration, test]\n\
- \"Rename getCwd everywhere\" → [search codebase, update file1, file2, ..., verify]\n\
- \"Implement auth + cart + checkout\" → [separate todo per feature, each broken down]\n\n\
### When to Skip\n\
- \"What does git status do?\" → Just answer (informational)\n\
- \"Add a comment to calculateTotal\" → Just edit (single step)\n\
- \"Run npm install\" → Just run it (trivial)\n\n\
## Anti-Patterns\n\n\
- Don't create todos just to show you're organized — only when genuinely helpful\n\
- Don't add a \"test the change\" todo unless user asks (prevents over-focus on testing)\n\
- Don't create single-item todo lists (pointless overhead)\n\
- Don't forget to mark items completed as you go (stale list = confusing)\n\
- Don't modify completed items — add new ones instead\n\
- Don't end your turn with incomplete todos without explanation"
        )
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "todos".to_string(),
            serde_json::json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "content": { "type": "string" },
                        "status": {
                            "type": "string",
                            "enum": ["pending", "in_progress", "completed", "cancelled"],
                            "default": "pending"
                        }
                    },
                    "required": ["id", "content", "status"]
                },
                "description": "Array of todo items."
            }),
        );
        props.insert(
            "merge".to_string(),
            serde_json::json!({
                "type": "boolean",
                "description": "If true, merge by id; if false (default), replace entire list."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["todos".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: TodoWriteArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("todo_write invalid JSON: {e}")),
        };

        if args.modified_by_user.unwrap_or(false) {
            if let Some(ref raw) = args.modified_content {
                let user_todos: Vec<TodoItem> = match serde_json::from_str(raw) {
                    Ok(v) => v,
                    Err(e) => {
                        return ToolResult::err(format!("Invalid modified_content JSON: {e}"))
                    }
                };
                self.store.replace_all(user_todos).await;
                let snapshot = self.store.snapshot().await;
                let display = format_todo_display(&snapshot);
                let llm_output = format_todo_llm_output(&snapshot);
                return ToolResult::ok_split(llm_output, display);
            }
        }

        {
            let mut seen = HashSet::with_capacity(args.todos.len());
            for item in &args.todos {
                if item.id.trim().is_empty() {
                    return ToolResult::err("Each todo must have a non-empty \"id\".");
                }
                if !seen.insert(&item.id) {
                    return ToolResult::err(format!(
                        "Duplicate todo id \"{}\". Each id must be unique.",
                        item.id
                    ));
                }
            }
        }

        let todos: Vec<TodoItem> = args.todos.into_iter().map(|item| item.into()).collect();

        if args.merge.unwrap_or(false) {
            self.store.merge(todos).await;
        } else {
            self.store.replace_all(todos).await;
        }

        let snapshot = self.store.snapshot().await;
        let display = format_todo_display(&snapshot);
        let llm_output = format_todo_llm_output(&snapshot);
        ToolResult::ok_split(llm_output, display)
    }
}

/// UI-facing display: formatted text for the TodoCard component to parse.
fn format_todo_display(items: &[TodoItem]) -> String {
    let total = items.len();
    let completed = items
        .iter()
        .filter(|t| t.status == TodoStatus::Completed)
        .count();
    let in_progress = items
        .iter()
        .filter(|t| t.status == TodoStatus::InProgress)
        .count();
    let pending = items
        .iter()
        .filter(|t| t.status == TodoStatus::Pending)
        .count();
    let cancelled = items
        .iter()
        .filter(|t| t.status == TodoStatus::Cancelled)
        .count();

    let mut lines = Vec::with_capacity(total + 3);
    lines.push(format!(
        "Todos: {total} total | {completed} completed | {in_progress} in_progress | {pending} pending | {cancelled} cancelled"
    ));
    lines.push(String::new());

    for item in items {
        let marker = match item.status {
            TodoStatus::Completed => "[x]",
            TodoStatus::InProgress => "[>]",
            TodoStatus::Pending => "[ ]",
            TodoStatus::Cancelled => "[-]",
        };
        lines.push(format!("{marker} {} — {}", item.id, item.content));
    }

    lines.join("\n")
}

/// LLM-facing output: concise with <system-reminder> for invisible guidance.
fn format_todo_llm_output(items: &[TodoItem]) -> String {
    if items.is_empty() {
        return "Todo list has been cleared.\n\n\
<system-reminder>\n\
Your todo list is now empty. DO NOT mention this explicitly to the user. \
You have no pending tasks in your todo list.\n\
</system-reminder>"
            .to_string();
    }

    let todos_json = serde_json::to_string(items).unwrap_or_default();

    let has_in_progress = items.iter().any(|t| t.status == TodoStatus::InProgress);
    let all_done = items
        .iter()
        .all(|t| matches!(t.status, TodoStatus::Completed | TodoStatus::Cancelled));

    let guidance = if all_done {
        "All tasks completed. Verify the overall result against the original request, then present a summary to the user."
    } else if has_in_progress {
        "Continue working on the in_progress task. Mark it completed when done, then move to the next pending task."
    } else {
        "Pick the next pending task, mark it in_progress, and start working on it."
    };

    format!(
        "Todos have been modified successfully. Proceed with the current tasks if applicable.\n\n\
<system-reminder>\n\
Your todo list has changed. DO NOT mention this explicitly to the user. \
Here are the latest contents of your todo list:\n\n\
{todos_json}\n\n\
{guidance}\n\
</system-reminder>"
    )
}

// ---------------------------------------------------------------------------
// TodoReadTool — read current todo list snapshot
// ---------------------------------------------------------------------------

pub struct TodoReadTool {
    store: TodoStore,
}

impl TodoReadTool {
    pub fn new(store: TodoStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for TodoReadTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn name(&self) -> &str {
        "todo_read"
    }

    fn description(&self) -> &str {
        "Read the current todo list. Returns all todo items with their status."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: HashMap::new(),
            required: vec![],
        }
    }

    async fn execute(&self, _arguments: &str) -> ToolResult {
        let items = self.store.snapshot().await;
        if items.is_empty() {
            return ToolResult::ok("No todos in the current session.".to_string());
        }
        let display = format_todo_display(&items);
        let json = serde_json::to_string(&items).unwrap_or_default();
        ToolResult::ok_split(json, display)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn todo_write_replaces_list() {
        let store = TodoStore::new();
        let tool = TodoWriteTool::new(store.clone());

        let args = serde_json::json!({
            "todos": [
                {"id": "a", "content": "Task A", "status": "pending"},
                {"id": "b", "content": "Task B", "status": "in_progress"},
            ]
        })
        .to_string();
        let result = tool.execute(&args).await;
        assert!(result.success);
        let display = result.display_output.as_deref().unwrap_or("");
        assert!(display.contains("2 total"));
        assert!(result.output.contains("<system-reminder>"));

        let args2 = serde_json::json!({
            "todos": [
                {"id": "c", "content": "Task C", "status": "completed"},
            ]
        })
        .to_string();
        let result2 = tool.execute(&args2).await;
        assert!(result2.success);
        let display2 = result2.display_output.as_deref().unwrap_or("");
        assert!(display2.contains("1 total"));
    }

    #[tokio::test]
    async fn todo_write_merge_updates() {
        let store = TodoStore::new();
        let tool = TodoWriteTool::new(store.clone());

        let args = serde_json::json!({
            "todos": [
                {"id": "a", "content": "Task A", "status": "pending"},
                {"id": "b", "content": "Task B", "status": "pending"},
            ]
        })
        .to_string();
        tool.execute(&args).await;

        let merge_args = serde_json::json!({
            "todos": [
                {"id": "a", "content": "Task A (done)", "status": "completed"},
                {"id": "c", "content": "Task C", "status": "in_progress"},
            ],
            "merge": true
        })
        .to_string();
        let result = tool.execute(&merge_args).await;
        assert!(result.success);
        let display = result.display_output.as_deref().unwrap_or("");
        assert!(display.contains("3 total"));
        assert!(display.contains("1 completed"));
    }

    #[tokio::test]
    async fn todo_write_rejects_duplicate_ids() {
        let store = TodoStore::new();
        let tool = TodoWriteTool::new(store);

        let args = serde_json::json!({
            "todos": [
                {"id": "a", "content": "Task A", "status": "pending"},
                {"id": "a", "content": "Task A dup", "status": "pending"},
            ]
        })
        .to_string();
        let result = tool.execute(&args).await;
        assert!(!result.success);
        assert!(result.output.contains("Duplicate todo id"));
    }

    #[tokio::test]
    async fn todo_write_rejects_empty_id() {
        let store = TodoStore::new();
        let tool = TodoWriteTool::new(store);

        let args = serde_json::json!({
            "todos": [
                {"id": "", "content": "Task", "status": "pending"},
            ]
        })
        .to_string();
        let result = tool.execute(&args).await;
        assert!(!result.success);
        assert!(result.output.contains("non-empty"));
    }

    #[tokio::test]
    async fn todo_llm_output_uses_system_reminder() {
        let store = TodoStore::new();
        let tool = TodoWriteTool::new(store);

        let args = serde_json::json!({
            "todos": [
                {"id": "a", "content": "Task A", "status": "pending"},
            ]
        })
        .to_string();
        let result = tool.execute(&args).await;
        assert!(result.success);
        assert!(result.output.contains("<system-reminder>"));
        assert!(result.output.contains("DO NOT mention this explicitly"));
        assert!(!result.output.contains("WAIT"));
        assert!(!result.output.contains("STOP"));
    }

    #[tokio::test]
    async fn todo_empty_clears() {
        let store = TodoStore::new();
        let tool = TodoWriteTool::new(store.clone());

        tool.execute(
            &serde_json::json!({"todos": [{"id": "a", "content": "T", "status": "pending"}]})
                .to_string(),
        )
        .await;
        let result = tool
            .execute(&serde_json::json!({"todos": []}).to_string())
            .await;
        assert!(result.success);
        assert!(result.output.contains("cleared"));
    }
}
