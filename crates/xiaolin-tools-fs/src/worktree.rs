use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use xiaolin_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolResult};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::RwLock;

// ── Shared State ─────────────────────────────────────────────────────

/// Tracks active worktree sessions (original_dir → worktree_path).
#[derive(Debug, Clone)]
pub struct WorktreeState {
    inner: Arc<RwLock<WorktreeInner>>,
}

#[derive(Debug, Default)]
struct WorktreeInner {
    /// original cwd before entering worktree
    original_dir: Option<PathBuf>,
    /// path to the created worktree
    worktree_path: Option<PathBuf>,
    /// branch name created for the worktree
    branch_name: Option<String>,
}

impl Default for WorktreeState {
    fn default() -> Self {
        Self::new()
    }
}

impl WorktreeState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(WorktreeInner::default())),
        }
    }

    pub async fn is_in_worktree(&self) -> bool {
        self.inner.read().await.worktree_path.is_some()
    }

    pub async fn worktree_path(&self) -> Option<PathBuf> {
        self.inner.read().await.worktree_path.clone()
    }

    pub async fn original_dir(&self) -> Option<PathBuf> {
        self.inner.read().await.original_dir.clone()
    }
}

// ── EnterWorktreeTool ────────────────────────────────────────────────

pub struct EnterWorktreeTool {
    state: WorktreeState,
}

impl EnterWorktreeTool {
    pub fn new(state: WorktreeState) -> Self {
        Self { state }
    }
}

#[derive(Debug, Deserialize)]
struct EnterArgs {
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    path: Option<String>,
}

#[async_trait]
impl Tool for EnterWorktreeTool {
    fn name(&self) -> &str {
        "enter_worktree"
    }

    fn description(&self) -> &str {
        "Create a git worktree with an isolated branch and switch the working directory \
         into it. Use for isolated experimentation (best-of-N, sub-agent work) without \
         affecting the main working tree."
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "branch".into(),
            json!({
                "type": "string",
                "description": "Branch name for the worktree (auto-generated if omitted)"
            }),
        );
        props.insert(
            "path".into(),
            json!({
                "type": "string",
                "description": "Directory path for the worktree (auto-generated in /tmp if omitted)"
            }),
        );
        ToolParameterSchema {
            schema_type: "object".into(),
            properties: props,
            required: vec![],
        }
    }

    async fn execute(&self, args: &str) -> ToolResult {
        if self.state.is_in_worktree().await {
            return ToolResult::err("Already inside a worktree. Exit the current one first.");
        }

        let parsed: EnterArgs = match serde_json::from_str(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::err(format!("invalid arguments: {e}")),
        };

        let original_dir = match resolve_effective_dir() {
            Ok(d) => d,
            Err(e) => return ToolResult::err(format!("cannot get cwd: {e}")),
        };

        if !is_git_repo(&original_dir).await {
            return ToolResult::err(
                "Current directory is not a git repository. Worktree requires git.",
            );
        }

        let branch = parsed
            .branch
            .unwrap_or_else(|| format!("worktree-{}", &uuid::Uuid::new_v4().to_string()[..8]));

        let worktree_path = parsed.path.map(PathBuf::from).unwrap_or_else(|| {
            std::env::temp_dir().join(format!(
                "xiaolin-wt-{}",
                &uuid::Uuid::new_v4().to_string()[..8]
            ))
        });

        let path_str = worktree_path.display().to_string();
        let output = match run_git_command(
            &original_dir,
            &["worktree", "add", &path_str, "-b", &branch],
        )
        .await
        {
            Ok(out) => out,
            Err(e) => return ToolResult::err(format!("git worktree add failed: {e}")),
        };

        {
            let mut inner = self.state.inner.write().await;
            inner.original_dir = Some(original_dir.clone());
            inner.worktree_path = Some(worktree_path.clone());
            inner.branch_name = Some(branch.clone());
        }

        ToolResult::ok(format!(
            "Entered worktree.\n\
             Branch: {branch}\n\
             Path: {path_str}\n\
             Original: {}\n\
             {output}",
            original_dir.display()
        ))
    }
}

// ── ExitWorktreeTool ─────────────────────────────────────────────────

pub struct ExitWorktreeTool {
    state: WorktreeState,
}

impl ExitWorktreeTool {
    pub fn new(state: WorktreeState) -> Self {
        Self { state }
    }
}

#[derive(Debug, Deserialize)]
struct ExitArgs {
    #[serde(default)]
    cleanup: Option<bool>,
}

#[async_trait]
impl Tool for ExitWorktreeTool {
    fn name(&self) -> &str {
        "exit_worktree"
    }

    fn description(&self) -> &str {
        "Exit the current worktree and return to the original working directory. \
         Optionally remove the worktree and its branch."
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert("cleanup".into(), json!({
            "type": "boolean",
            "description": "If true, remove the worktree directory and delete the branch (default: false)"
        }));
        ToolParameterSchema {
            schema_type: "object".into(),
            properties: props,
            required: vec![],
        }
    }

    async fn execute(&self, args: &str) -> ToolResult {
        let (original_dir, worktree_path, branch_name) = {
            let inner = self.state.inner.read().await;
            match (
                &inner.original_dir,
                &inner.worktree_path,
                &inner.branch_name,
            ) {
                (Some(orig), Some(wt), Some(br)) => (orig.clone(), wt.clone(), br.clone()),
                _ => return ToolResult::err("Not currently in a worktree."),
            }
        };

        let parsed: ExitArgs = match serde_json::from_str(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::err(format!("invalid arguments: {e}")),
        };

        let should_cleanup = parsed.cleanup.unwrap_or(false);
        let mut cleanup_msg = String::new();

        if should_cleanup {
            if let Err(e) = run_git_command(
                &original_dir,
                &[
                    "worktree",
                    "remove",
                    &worktree_path.display().to_string(),
                    "--force",
                ],
            )
            .await
            {
                cleanup_msg = format!("\nWarning: worktree removal failed: {e}");
            } else {
                if let Err(e) =
                    run_git_command(&original_dir, &["branch", "-D", &branch_name]).await
                {
                    cleanup_msg = format!("\nWorktree removed. Branch deletion failed: {e}");
                } else {
                    cleanup_msg = "\nWorktree and branch cleaned up.".to_string();
                }
            }
        }

        {
            let mut inner = self.state.inner.write().await;
            inner.original_dir = None;
            inner.worktree_path = None;
            inner.branch_name = None;
        }

        ToolResult::ok(format!(
            "Exited worktree.\nRestored to: {}{cleanup_msg}",
            original_dir.display()
        ))
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

fn resolve_effective_dir() -> std::io::Result<PathBuf> {
    std::env::current_dir()
}

async fn is_git_repo(dir: &Path) -> bool {
    tokio::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn run_git_command(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .await
        .map_err(|e| format!("failed to run git: {e}"))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(stderr)
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn init_git_repo(dir: &Path) {
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
    }

    #[tokio::test]
    async fn enter_creates_worktree() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());

        let prev_dir = std::env::current_dir().ok();
        std::env::set_current_dir(dir.path()).unwrap();

        let state = WorktreeState::new();
        let tool = EnterWorktreeTool::new(state.clone());

        let wt_path = dir.path().join("my-worktree");
        let args = json!({
            "branch": "test-branch",
            "path": wt_path.display().to_string()
        });

        let result = tool.execute(&args.to_string()).await;
        assert!(result.success, "enter should succeed: {}", result.output);
        assert!(result.output.contains("test-branch"));
        assert!(wt_path.exists(), "worktree dir should exist");
        assert!(state.is_in_worktree().await);

        if let Some(d) = prev_dir {
            let _ = std::env::set_current_dir(d);
        }
    }

    #[tokio::test]
    async fn enter_rejects_non_git() {
        let dir = tempfile::tempdir().unwrap();
        let prev_dir = std::env::current_dir().ok();
        std::env::set_current_dir(dir.path()).unwrap();

        let state = WorktreeState::new();
        let tool = EnterWorktreeTool::new(state);

        let result = tool.execute(r#"{}"#).await;
        assert!(!result.success);
        assert!(
            result.output.contains("not a git repository") || result.output.contains("git"),
            "unexpected error: {}",
            result.output
        );

        if let Some(d) = prev_dir {
            let _ = std::env::set_current_dir(d);
        }
    }

    #[tokio::test]
    async fn enter_rejects_double_entry() {
        let state = WorktreeState::new();
        {
            let mut inner = state.inner.write().await;
            inner.worktree_path = Some(PathBuf::from("/tmp/fake-wt"));
            inner.original_dir = Some(PathBuf::from("/tmp/orig"));
            inner.branch_name = Some("fake".into());
        }

        let tool = EnterWorktreeTool::new(state);
        let result = tool.execute(r#"{}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("Already inside"));
    }

    #[tokio::test]
    async fn exit_restores_state() {
        let state = WorktreeState::new();
        {
            let mut inner = state.inner.write().await;
            inner.original_dir = Some(PathBuf::from("/tmp/orig"));
            inner.worktree_path = Some(PathBuf::from("/tmp/wt"));
            inner.branch_name = Some("branch".into());
        }

        let tool = ExitWorktreeTool::new(state.clone());
        let result = tool.execute(r#"{"cleanup": false}"#).await;
        assert!(result.success, "exit should succeed: {}", result.output);
        assert!(result.output.contains("/tmp/orig"));
        assert!(!state.is_in_worktree().await);
    }

    #[tokio::test]
    async fn exit_when_not_in_worktree() {
        let state = WorktreeState::new();
        let tool = ExitWorktreeTool::new(state);
        let result = tool.execute(r#"{}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("Not currently"));
    }
}
