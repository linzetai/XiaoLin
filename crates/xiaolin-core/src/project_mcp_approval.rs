use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::agent_config::ProjectMcpApproval;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ProjectMcpApprovals {
    pub approvals: HashMap<String, ProjectMcpApproval>,
}

fn approval_key(workspace_root: &Path, server_id: &str) -> String {
    format!("{}::{}", workspace_root.display(), server_id)
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

pub fn get_approval(workspace_root: &Path, server_id: &str) -> ProjectMcpApproval {
    let approvals = load_approvals();
    let key = approval_key(workspace_root, server_id);
    approvals
        .approvals
        .get(&key)
        .copied()
        .unwrap_or(ProjectMcpApproval::Pending)
}

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
    use std::path::PathBuf;

    #[test]
    fn unknown_server_returns_pending() {
        let approvals = ProjectMcpApprovals::default();
        let key = approval_key(&PathBuf::from("/tmp/test"), "unknown");
        assert_eq!(
            approvals
                .approvals
                .get(&key)
                .copied()
                .unwrap_or(ProjectMcpApproval::Pending),
            ProjectMcpApproval::Pending
        );
    }

    #[test]
    fn different_workspaces_are_independent() {
        let mut approvals = ProjectMcpApprovals::default();
        let ws1 = PathBuf::from("/workspace/project1");
        let ws2 = PathBuf::from("/workspace/project2");
        let key1 = approval_key(&ws1, "server");
        let key2 = approval_key(&ws2, "server");
        approvals
            .approvals
            .insert(key1.clone(), ProjectMcpApproval::Approved);

        assert_eq!(
            approvals.approvals.get(&key1).copied(),
            Some(ProjectMcpApproval::Approved)
        );
        assert_eq!(approvals.approvals.get(&key2).copied(), None);
    }

    #[test]
    fn approval_key_format() {
        let key = approval_key(&PathBuf::from("/home/user/project"), "chrome-mcp");
        assert_eq!(key, "/home/user/project::chrome-mcp");
    }
}
