//! Track files created/modified/deleted by the agent during a session.
//!
//! Records are serializable for SQLite persistence and can be used to:
//! - List all file changes in a session
//! - Generate a change summary for session resume
//! - Compare against git diff for consistency checks

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// Type of file operation performed by the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileOp {
    Created,
    Modified,
    Deleted,
}

impl std::fmt::Display for FileOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Created => write!(f, "created"),
            Self::Modified => write!(f, "modified"),
            Self::Deleted => write!(f, "deleted"),
        }
    }
}

/// A single recorded file change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub path: PathBuf,
    pub op: FileOp,
    pub tool_name: String,
    pub timestamp_ms: u64,
}

/// Persistent artifact record for a file operation within a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileArtifact {
    pub session_id: String,
    pub path: PathBuf,
    pub operation: FileOp,
    pub timestamp_ms: u64,
    pub tool_call_id: String,
    pub bytes: u64,
}

impl FileArtifact {
    pub fn new(
        session_id: impl Into<String>,
        path: PathBuf,
        operation: FileOp,
        tool_call_id: impl Into<String>,
        bytes: u64,
    ) -> Self {
        let timestamp_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        Self {
            session_id: session_id.into(),
            path,
            operation,
            timestamp_ms,
            tool_call_id: tool_call_id.into(),
            bytes,
        }
    }

    pub fn operation_str(&self) -> &'static str {
        match self.operation {
            FileOp::Created => "created",
            FileOp::Modified => "modified",
            FileOp::Deleted => "deleted",
        }
    }
}

/// Tracks all file changes within a session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionFileTracker {
    changes: Vec<FileChange>,
    file_ops: HashMap<PathBuf, FileOp>,
}

impl SessionFileTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a file operation.
    pub fn record(&mut self, path: PathBuf, op: FileOp, tool_name: &str) {
        let timestamp_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        self.changes.push(FileChange {
            path: path.clone(),
            op,
            tool_name: tool_name.to_string(),
            timestamp_ms,
        });

        self.file_ops.insert(path, op);
    }

    /// List all unique files affected with their latest operation.
    pub fn list_session_files(&self) -> Vec<(&PathBuf, &FileOp)> {
        let mut files: Vec<_> = self.file_ops.iter().collect();
        files.sort_by(|a, b| a.0.cmp(b.0));
        files
    }

    /// Get all changes in chronological order.
    pub fn all_changes(&self) -> &[FileChange] {
        &self.changes
    }

    /// Generate a human-readable change summary.
    pub fn change_summary(&self) -> String {
        if self.file_ops.is_empty() {
            return "No file changes in this session.".to_string();
        }

        let mut lines = Vec::new();
        let mut created = 0;
        let mut modified = 0;
        let mut deleted = 0;

        let mut sorted: Vec<_> = self.file_ops.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(b.0));

        for (path, op) in &sorted {
            let display = path.display();
            match op {
                FileOp::Created => {
                    lines.push(format!("  + {display}"));
                    created += 1;
                }
                FileOp::Modified => {
                    lines.push(format!("  ~ {display}"));
                    modified += 1;
                }
                FileOp::Deleted => {
                    lines.push(format!("  - {display}"));
                    deleted += 1;
                }
            }
        }

        let header = format!(
            "Session file changes: {} created, {} modified, {} deleted\n",
            created, modified, deleted
        );
        format!("{}{}", header, lines.join("\n"))
    }

    /// Serialize to JSON for SQLite persistence.
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self.changes).unwrap_or_else(|_| "[]".to_string())
    }

    /// Restore from JSON (loaded from SQLite).
    pub fn from_json(json: &str) -> Self {
        let changes: Vec<FileChange> = serde_json::from_str(json).unwrap_or_default();

        let mut file_ops = HashMap::new();
        for change in &changes {
            file_ops.insert(change.path.clone(), change.op);
        }

        Self { changes, file_ops }
    }

    /// Number of unique files affected.
    pub fn file_count(&self) -> usize {
        self.file_ops.len()
    }

    /// Total number of recorded operations (may be > file_count if same file modified multiple times).
    pub fn operation_count(&self) -> usize {
        self.changes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_list_files() {
        let mut tracker = SessionFileTracker::new();
        tracker.record(PathBuf::from("src/main.rs"), FileOp::Modified, "edit_file");
        tracker.record(PathBuf::from("src/lib.rs"), FileOp::Created, "write_file");
        tracker.record(PathBuf::from("old.txt"), FileOp::Deleted, "shell");

        let files = tracker.list_session_files();
        assert_eq!(files.len(), 3);
        assert_eq!(tracker.file_count(), 3);
        assert_eq!(tracker.operation_count(), 3);
    }

    #[test]
    fn latest_op_wins_for_same_file() {
        let mut tracker = SessionFileTracker::new();
        tracker.record(PathBuf::from("file.rs"), FileOp::Created, "write_file");
        tracker.record(PathBuf::from("file.rs"), FileOp::Modified, "edit_file");

        assert_eq!(tracker.file_count(), 1);
        assert_eq!(tracker.operation_count(), 2);

        let files = tracker.list_session_files();
        assert_eq!(*files[0].1, FileOp::Modified);
    }

    #[test]
    fn change_summary_format() {
        let mut tracker = SessionFileTracker::new();
        tracker.record(PathBuf::from("a.rs"), FileOp::Created, "write_file");
        tracker.record(PathBuf::from("b.rs"), FileOp::Modified, "edit_file");
        tracker.record(PathBuf::from("c.rs"), FileOp::Deleted, "shell");

        let summary = tracker.change_summary();
        assert!(summary.contains("1 created"));
        assert!(summary.contains("1 modified"));
        assert!(summary.contains("1 deleted"));
        assert!(summary.contains("+ a.rs"));
        assert!(summary.contains("~ b.rs"));
        assert!(summary.contains("- c.rs"));
    }

    #[test]
    fn json_roundtrip() {
        let mut tracker = SessionFileTracker::new();
        tracker.record(PathBuf::from("src/main.rs"), FileOp::Modified, "edit_file");
        tracker.record(PathBuf::from("new.rs"), FileOp::Created, "write_file");

        let json = tracker.to_json();
        let restored = SessionFileTracker::from_json(&json);

        assert_eq!(restored.file_count(), 2);
        assert_eq!(restored.operation_count(), 2);

        let files = restored.list_session_files();
        assert!(files
            .iter()
            .any(|(p, _)| p.display().to_string().contains("main.rs")));
    }
}
