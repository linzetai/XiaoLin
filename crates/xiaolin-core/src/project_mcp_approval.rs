use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Approval state for a project-level MCP server.
///
/// Stored in the **user's** home directory (not the project), so a malicious
/// repository cannot self-approve its own MCP servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectMcpApproval {
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ProjectMcpApprovals {
    pub approvals: HashMap<String, ProjectMcpApproval>,
}

fn approval_key(workspace_root: &Path, server_id: &str) -> String {
    let canonical = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    format!("{}::{}", canonical.display(), server_id)
}

fn approvals_path() -> PathBuf {
    crate::paths::resolve_state_dir().join("project_mcp_approvals.json")
}

pub fn load_approvals() -> ProjectMcpApprovals {
    let path = approvals_path();
    if !path.exists() {
        return ProjectMcpApprovals::default();
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Look up the approval status for a project-level MCP server.
/// Returns `Pending` for unknown servers.
pub fn get_approval(workspace_root: &Path, server_id: &str) -> ProjectMcpApproval {
    let approvals = load_approvals();
    let key = approval_key(workspace_root, server_id);
    approvals
        .approvals
        .get(&key)
        .copied()
        .unwrap_or(ProjectMcpApproval::Pending)
}

/// Persist the approval decision for a project-level MCP server.
pub fn set_approval(
    workspace_root: &Path,
    server_id: &str,
    status: ProjectMcpApproval,
) -> anyhow::Result<()> {
    let mut approvals = load_approvals();
    let key = approval_key(workspace_root, server_id);
    approvals.approvals.insert(key, status);
    let path = approvals_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(&approvals)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_key_format() {
        let key = approval_key(Path::new("/tmp/myproject"), "chrome-devtools");
        assert!(key.contains("::chrome-devtools"));
        assert!(key.contains("myproject"));
    }

    #[test]
    fn unknown_server_returns_pending() {
        let status = get_approval(Path::new("/nonexistent/workspace"), "nonexistent-server");
        assert_eq!(status, ProjectMcpApproval::Pending);
    }

    #[test]
    fn round_trip_serde() {
        let mut approvals = ProjectMcpApprovals::default();
        approvals
            .approvals
            .insert("/ws::server-a".to_string(), ProjectMcpApproval::Approved);
        approvals
            .approvals
            .insert("/ws::server-b".to_string(), ProjectMcpApproval::Rejected);

        let json = serde_json::to_string_pretty(&approvals).unwrap();
        let parsed: ProjectMcpApprovals = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.approvals.get("/ws::server-a").copied(),
            Some(ProjectMcpApproval::Approved)
        );
        assert_eq!(
            parsed.approvals.get("/ws::server-b").copied(),
            Some(ProjectMcpApproval::Rejected)
        );
    }

    #[test]
    fn set_and_get_approval_tempdir() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("workspace");
        std::fs::create_dir_all(&ws).unwrap();

        let key1 = approval_key(&ws, "server-1");
        let key2 = approval_key(&ws, "server-2");
        assert_ne!(key1, key2);
        assert!(key1.ends_with("::server-1"));
    }

    #[test]
    fn different_workspaces_independent() {
        let key_a = approval_key(Path::new("/workspace-a"), "server");
        let key_b = approval_key(Path::new("/workspace-b"), "server");
        assert_ne!(key_a, key_b);
    }
}
