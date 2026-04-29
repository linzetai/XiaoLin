use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolResult};
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
}

impl std::fmt::Display for TodoStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::InProgress => write!(f, "in_progress"),
            Self::Completed => write!(f, "completed"),
        }
    }
}

/// In-session todo storage shared across tool invocations.
#[derive(Debug, Clone, Default)]
pub struct TodoStore {
    items: Arc<RwLock<Vec<TodoItem>>>,
}

impl TodoStore {
    pub fn new() -> Self {
        Self { items: Arc::new(RwLock::new(Vec::new())) }
    }

    pub async fn snapshot(&self) -> Vec<TodoItem> {
        self.items.read().await.clone()
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
    fn kind(&self) -> ToolKind { ToolKind::Think }
    fn name(&self) -> &str { "todo_write" }

    fn description(&self) -> &str {
        TODO_WRITE_DESCRIPTION
    }

    fn prompt(&self) -> String {
        format!("{TODO_WRITE_DESCRIPTION}\n\n\
## Examples of When to Use\n\n\
- User: \"Add dark mode toggle\" → Create todo list: add state management, implement styles, \
create toggle component, update components, run tests\n\
- User: \"Rename getCwd to getCurrentWorkingDirectory\" → Search codebase, find all instances, \
create todo per file\n\
- User: \"Implement registration, catalog, cart, checkout\" → Break each feature into specific tasks\n\n\
## Examples of When NOT to Use\n\n\
- User: \"What does git status do?\" → Informational, no coding task\n\
- User: \"Add a comment to this function\" → Single straightforward edit\n\
- User: \"Run npm install\" → Single command execution\n\n\
When in doubt, use this tool. Being proactive with task management demonstrates attentiveness \
and ensures you complete all requirements successfully.")
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert("todos".to_string(), serde_json::json!({
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "content": { "type": "string" },
                    "status": {
                        "type": "string",
                        "enum": ["pending", "in_progress", "completed"],
                        "default": "pending"
                    }
                },
                "required": ["id", "content", "status"]
            },
            "description": "Array of todo items."
        }));
        props.insert("merge".to_string(), serde_json::json!({
            "type": "boolean",
            "description": "If true, merge by id; if false (default), replace entire list."
        }));
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
                    Err(e) => return ToolResult::err(format!("Invalid modified_content JSON: {e}")),
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
    let completed = items.iter().filter(|t| t.status == TodoStatus::Completed).count();
    let in_progress = items.iter().filter(|t| t.status == TodoStatus::InProgress).count();
    let pending = items.iter().filter(|t| t.status == TodoStatus::Pending).count();

    let mut lines = Vec::with_capacity(total + 3);
    lines.push(format!(
        "Todos: {total} total | {completed} completed | {in_progress} in_progress | {pending} pending | 0 cancelled"
    ));
    lines.push(String::new());

    for item in items {
        let marker = match item.status {
            TodoStatus::Completed => "[x]",
            TodoStatus::InProgress => "[>]",
            TodoStatus::Pending => "[ ]",
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
</system-reminder>".to_string();
    }

    let todos_json = serde_json::to_string(items).unwrap_or_default();

    let has_in_progress = items.iter().any(|t| t.status == TodoStatus::InProgress);
    let has_pending = items.iter().any(|t| t.status == TodoStatus::Pending);
    let all_done = !has_in_progress && !has_pending;

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
        }).to_string();
        let result = tool.execute(&args).await;
        assert!(result.success);
        let display = result.display_output.as_deref().unwrap_or("");
        assert!(display.contains("2 total"));
        assert!(result.output.contains("<system-reminder>"));

        let args2 = serde_json::json!({
            "todos": [
                {"id": "c", "content": "Task C", "status": "completed"},
            ]
        }).to_string();
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
        }).to_string();
        tool.execute(&args).await;

        let merge_args = serde_json::json!({
            "todos": [
                {"id": "a", "content": "Task A (done)", "status": "completed"},
                {"id": "c", "content": "Task C", "status": "in_progress"},
            ],
            "merge": true
        }).to_string();
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
        }).to_string();
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
        }).to_string();
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
        }).to_string();
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

        tool.execute(&serde_json::json!({"todos": [{"id": "a", "content": "T", "status": "pending"}]}).to_string()).await;
        let result = tool.execute(&serde_json::json!({"todos": []}).to_string()).await;
        assert!(result.success);
        assert!(result.output.contains("cleared"));
    }
}
