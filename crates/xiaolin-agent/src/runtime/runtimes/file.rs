use std::path::Path;

use async_trait::async_trait;
use xiaolin_core::tool_runtime::{
    Approvable, ExecApprovalRequirement, SandboxAttempt, SandboxPreference, Sandboxable,
    ToolExecContext, ToolRunOutput, ToolRuntime, ToolRuntimeError,
};
use xiaolin_protocol::approval::PendingAction;

/// Truncate a string to at most `max_chars` characters at a valid UTF-8 boundary.
fn truncate_preview(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let byte_idx = s
        .char_indices()
        .nth(max_chars)
        .map(|(idx, _)| idx)
        .unwrap_or(s.len());
    s[..byte_idx].to_string()
}

/// Build a unified-diff-style preview from edit_file arguments.
fn build_diff_preview(args: &serde_json::Value) -> Option<String> {
    if let Some(diff) = args.get("diff").and_then(|v| v.as_str()) {
        return Some(truncate_preview(diff, 2000));
    }
    let old = args.get("old_string").and_then(|v| v.as_str())?;
    let new = args.get("new_string").and_then(|v| v.as_str())?;
    let mut result = String::new();
    for line in old.lines() {
        result.push('-');
        result.push_str(line);
        result.push('\n');
    }
    for line in new.lines() {
        result.push('+');
        result.push_str(line);
        result.push('\n');
    }
    Some(truncate_preview(&result, 2000))
}

/// Runtime for `write_file` / `create_file` tool calls.
pub struct FileWriteRuntime;

impl FileWriteRuntime {
    fn is_outside_workspace(path: &Path, cwd: &Path) -> bool {
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path)
        };
        !resolved.starts_with(cwd)
    }

    fn is_system_path(path: &Path) -> bool {
        let s = path.to_string_lossy();
        s.starts_with("/etc")
            || s.starts_with("/usr")
            || s.starts_with("/bin")
            || s.starts_with("/sbin")
            || s.starts_with("/boot")
            || s.starts_with("/sys")
            || s.starts_with("/proc")
    }
}

impl FileWriteRuntime {
    fn extract_path(args: &serde_json::Value) -> Option<&str> {
        args.get("file_path")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("path").and_then(|v| v.as_str()))
    }
}

impl Approvable for FileWriteRuntime {
    fn approval_keys(&self, args: &serde_json::Value) -> Vec<String> {
        let path = Self::extract_path(args).unwrap_or("unknown");
        vec![format!("file_write:{path}")]
    }

    fn exec_requirement(&self, args: &serde_json::Value, cwd: &Path) -> ExecApprovalRequirement {
        let path_str = Self::extract_path(args).unwrap_or("unknown");
        let path = Path::new(path_str);

        if Self::is_system_path(path) {
            return ExecApprovalRequirement::Forbidden {
                reason: format!("writing to system path is forbidden: {path_str}"),
            };
        }
        if Self::is_outside_workspace(path, cwd) {
            return ExecApprovalRequirement::NeedsApproval {
                reason: format!("file write outside workspace: {path_str}"),
            };
        }
        ExecApprovalRequirement::Skip
    }

    fn to_pending_action(&self, args: &serde_json::Value, _cwd: &Path) -> PendingAction {
        let path = Self::extract_path(args).unwrap_or("unknown").to_string();
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .map(|s| truncate_preview(s, 2000));
        PendingAction::FileWrite { path, content }
    }
}

impl Sandboxable for FileWriteRuntime {
    fn sandbox_preference(&self) -> SandboxPreference {
        SandboxPreference::Skip
    }
}

#[async_trait]
impl ToolRuntime for FileWriteRuntime {
    async fn run(
        &self,
        args: &serde_json::Value,
        _sandbox: &SandboxAttempt,
        ctx: &ToolExecContext,
    ) -> Result<ToolRunOutput, ToolRuntimeError> {
        let path_str = Self::extract_path(args).ok_or_else(|| ToolRuntimeError::Internal {
            message: "missing 'file_path' (or 'path') argument".into(),
        })?;
        let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");

        let path = if Path::new(path_str).is_absolute() {
            std::path::PathBuf::from(path_str)
        } else {
            ctx.cwd.join(path_str)
        };

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ToolRuntimeError::Internal {
                    message: format!("failed to create parent dirs: {e}"),
                })?;
        }

        tokio::fs::write(&path, content)
            .await
            .map_err(|e| ToolRuntimeError::Internal {
                message: format!("failed to write file: {e}"),
            })?;

        if let Some(pc) = crate::builtin_tools::current_plan_context() {
            let plan_path = pc.store.plan_path(&pc.session_id);
            let is_plan = path == plan_path
                || path
                    .canonicalize()
                    .ok()
                    .zip(plan_path.canonicalize().ok())
                    .is_some_and(|(a, b)| a == b);
            if is_plan {
                return Ok(ToolRunOutput::plain(format!(
                    "Plan updated — {} (view in Plan panel)",
                    path.display()
                )));
            }
        }

        Ok(ToolRunOutput::plain(format!(
            "wrote {} bytes to {}",
            content.len(),
            path.display()
        )))
    }

    fn name(&self) -> &str {
        "write_file"
    }
}

/// Runtime for `edit_file` / `multi_edit` tool calls.
pub struct FileEditRuntime;

impl FileEditRuntime {
    fn extract_path(args: &serde_json::Value) -> Option<&str> {
        args.get("file_path")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("path").and_then(|v| v.as_str()))
    }
}

impl Approvable for FileEditRuntime {
    fn approval_keys(&self, args: &serde_json::Value) -> Vec<String> {
        let path = Self::extract_path(args).unwrap_or("unknown");
        vec![format!("file_edit:{path}")]
    }

    fn exec_requirement(&self, args: &serde_json::Value, cwd: &Path) -> ExecApprovalRequirement {
        let path_str = Self::extract_path(args).unwrap_or("unknown");
        let path = Path::new(path_str);

        if FileWriteRuntime::is_system_path(path) {
            return ExecApprovalRequirement::Forbidden {
                reason: format!("editing system file is forbidden: {path_str}"),
            };
        }
        if FileWriteRuntime::is_outside_workspace(path, cwd) {
            return ExecApprovalRequirement::NeedsApproval {
                reason: format!("file edit outside workspace: {path_str}"),
            };
        }
        ExecApprovalRequirement::Skip
    }

    fn to_pending_action(&self, args: &serde_json::Value, _cwd: &Path) -> PendingAction {
        let path = Self::extract_path(args).unwrap_or("unknown").to_string();
        let diff = build_diff_preview(args);
        PendingAction::ApplyPatch {
            paths: vec![path],
            diff,
        }
    }
}

impl Sandboxable for FileEditRuntime {
    fn sandbox_preference(&self) -> SandboxPreference {
        SandboxPreference::Skip
    }
}

#[async_trait]
impl ToolRuntime for FileEditRuntime {
    async fn run(
        &self,
        args: &serde_json::Value,
        _sandbox: &SandboxAttempt,
        ctx: &ToolExecContext,
    ) -> Result<ToolRunOutput, ToolRuntimeError> {
        let path_str = Self::extract_path(args).ok_or_else(|| ToolRuntimeError::Internal {
            message: "missing 'file_path' (or 'path') argument".into(),
        })?;
        let old_string = args
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolRuntimeError::Internal {
                message: "missing 'old_string' argument".into(),
            })?;
        let new_string = args
            .get("new_string")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let path = if Path::new(path_str).is_absolute() {
            std::path::PathBuf::from(path_str)
        } else {
            ctx.cwd.join(path_str)
        };

        let content =
            tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| ToolRuntimeError::Internal {
                    message: format!("failed to read file: {e}"),
                })?;

        if !content.contains(old_string) {
            return Err(ToolRuntimeError::Internal {
                message: format!("old_string not found in {}", path.display()),
            });
        }

        let new_content = content.replacen(old_string, new_string, 1);
        tokio::fs::write(&path, &new_content)
            .await
            .map_err(|e| ToolRuntimeError::Internal {
                message: format!("failed to write file: {e}"),
            })?;

        Ok(ToolRunOutput::plain(format!("edited {}", path.display())))
    }

    fn name(&self) -> &str {
        "edit_file"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_in_workspace_skips_approval() {
        let rt = FileWriteRuntime;
        let args = serde_json::json!({"path": "src/main.rs"});
        let req = rt.exec_requirement(&args, Path::new("/project"));
        assert!(matches!(req, ExecApprovalRequirement::Skip));
    }

    #[test]
    fn write_outside_workspace_needs_approval() {
        let rt = FileWriteRuntime;
        let args = serde_json::json!({"path": "/tmp/outside.txt"});
        let req = rt.exec_requirement(&args, Path::new("/project"));
        assert!(matches!(req, ExecApprovalRequirement::NeedsApproval { .. }));
    }

    #[test]
    fn write_to_etc_is_forbidden() {
        let rt = FileWriteRuntime;
        let args = serde_json::json!({"path": "/etc/passwd"});
        let req = rt.exec_requirement(&args, Path::new("/project"));
        assert!(matches!(req, ExecApprovalRequirement::Forbidden { .. }));
    }

    #[test]
    fn edit_in_workspace_skips_approval() {
        let rt = FileEditRuntime;
        let args = serde_json::json!({"path": "lib.rs", "old_string": "foo", "new_string": "bar"});
        let req = rt.exec_requirement(&args, Path::new("/project"));
        assert!(matches!(req, ExecApprovalRequirement::Skip));
    }

    #[test]
    fn edit_system_file_forbidden() {
        let rt = FileEditRuntime;
        let args = serde_json::json!({"path": "/usr/lib/x.so"});
        let req = rt.exec_requirement(&args, Path::new("/project"));
        assert!(matches!(req, ExecApprovalRequirement::Forbidden { .. }));
    }
}
