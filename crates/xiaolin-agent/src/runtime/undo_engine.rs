//! Undo/rollback engine for reversible tool operations.
//!
//! Before executing file-modifying tools, the engine captures the original
//! file content. When consecutive failures accumulate (autofix/recovery unable
//! to resolve), the engine can roll back to the last known-good state and
//! inject guidance telling the LLM to try a different approach.
//!
//! This prevents weak models from "making things worse" through repeated
//! failed edits — a common failure mode in code and document scenarios.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Configuration for the undo engine.
#[derive(Debug, Clone)]
pub struct UndoEngineConfig {
    pub enabled: bool,
    /// Number of consecutive failures before triggering auto-rollback.
    pub rollback_after_failures: u32,
    /// Maximum number of file snapshots to keep in memory.
    pub max_snapshots: usize,
}

impl Default for UndoEngineConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            rollback_after_failures: 5,
            max_snapshots: 50,
        }
    }
}

/// A snapshot of a file's content before modification.
#[derive(Debug, Clone)]
struct FileSnapshot {
    content: String,
    /// Which edit iteration created this snapshot (used in rollback guidance).
    #[allow(dead_code)]
    edit_index: u32,
}

/// Tracks the edit history and rollback state for a session.
#[derive(Debug)]
pub struct UndoEngine {
    config: UndoEngineConfig,
    /// Map from file path to its original content (before first edit in this batch).
    snapshots: HashMap<PathBuf, FileSnapshot>,
    /// Counter for total edits in the current "batch" (between checkpoints).
    edit_count: u32,
    /// Counter for consecutive failures since last successful checkpoint.
    consecutive_failures: u32,
    /// Paths that have been rolled back (to avoid re-snapshotting stale data).
    rolled_back: Vec<PathBuf>,
    /// History of failed attempts (for injection into the LLM prompt).
    failure_summaries: Vec<String>,
}

/// Result of a rollback operation.
#[derive(Debug, Clone)]
pub struct RollbackResult {
    /// Files that were successfully rolled back.
    pub restored_files: Vec<PathBuf>,
    /// Formatted guidance message to inject into the LLM context.
    pub guidance: String,
}

impl UndoEngine {
    pub fn new(config: UndoEngineConfig) -> Self {
        Self {
            config,
            snapshots: HashMap::new(),
            edit_count: 0,
            consecutive_failures: 0,
            rolled_back: Vec::new(),
            failure_summaries: Vec::new(),
        }
    }

    /// Whether the undo engine is active.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Record a file's content before it's about to be modified.
    ///
    /// Only captures the first snapshot per file in a batch — subsequent edits
    /// to the same file don't overwrite the original baseline.
    pub fn capture_before_edit(&mut self, path: &Path, current_content: &str) {
        if !self.config.enabled {
            return;
        }

        if self.snapshots.len() >= self.config.max_snapshots {
            return;
        }

        let canonical = path.to_path_buf();
        if !self.snapshots.contains_key(&canonical) && !self.rolled_back.contains(&canonical) {
            self.snapshots.insert(
                canonical,
                FileSnapshot {
                    content: current_content.to_string(),
                    edit_index: self.edit_count,
                },
            );
        }
        self.edit_count += 1;
    }

    /// Record a successful tool execution — resets the failure counter.
    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
    }

    /// Record a failed tool execution.
    ///
    /// Returns `true` if the failure threshold has been reached and rollback
    /// should be triggered.
    pub fn record_failure(&mut self, summary: &str) -> bool {
        if !self.config.enabled {
            return false;
        }

        self.consecutive_failures += 1;
        if !summary.is_empty() {
            self.failure_summaries.push(truncate_summary(summary, 200));
            if self.failure_summaries.len() > 10 {
                self.failure_summaries.remove(0);
            }
        }
        self.consecutive_failures >= self.config.rollback_after_failures
    }

    /// Check if rollback threshold has been reached.
    pub fn should_rollback(&self) -> bool {
        self.config.enabled
            && self.consecutive_failures >= self.config.rollback_after_failures
            && !self.snapshots.is_empty()
    }

    /// Execute the rollback: returns the files to restore and guidance text.
    ///
    /// The caller is responsible for actually writing the restored content
    /// back to disk.
    pub fn execute_rollback(&mut self) -> Option<RollbackResult> {
        if self.snapshots.is_empty() {
            return None;
        }

        let mut restored_files = Vec::new();
        let snapshots: Vec<(PathBuf, String)> = self
            .snapshots
            .drain()
            .map(|(path, snap)| {
                restored_files.push(path.clone());
                (path, snap.content)
            })
            .collect();

        self.rolled_back.extend(restored_files.iter().cloned());

        let guidance = self.format_rollback_guidance(&restored_files);

        self.consecutive_failures = 0;
        self.edit_count = 0;

        // Store the snapshot contents for the caller to write back
        // We re-insert them tagged so caller can extract
        for (path, content) in snapshots {
            self.snapshots.insert(
                path,
                FileSnapshot {
                    content,
                    edit_index: 0,
                },
            );
        }

        Some(RollbackResult {
            restored_files,
            guidance,
        })
    }

    /// Get the original content to restore for a given file.
    pub fn get_restore_content(&self, path: &Path) -> Option<&str> {
        self.snapshots.get(path).map(|s| s.content.as_str())
    }

    /// Clear all snapshots and state after a successful checkpoint.
    pub fn checkpoint(&mut self) {
        self.snapshots.clear();
        self.rolled_back.clear();
        self.edit_count = 0;
        self.consecutive_failures = 0;
        self.failure_summaries.clear();
    }

    /// Number of files currently tracked for potential rollback.
    pub fn tracked_file_count(&self) -> usize {
        self.snapshots.len()
    }

    fn format_rollback_guidance(&self, restored_files: &[PathBuf]) -> String {
        let mut guidance = String::with_capacity(512);
        guidance.push_str("─── ROLLBACK ───────────────────────────────────────\n");
        guidance.push_str(&format!(
            "Your last {} attempts all failed. Files have been rolled back to their original state.\n\n",
            self.consecutive_failures
        ));

        guidance.push_str("Restored files:\n");
        for f in restored_files {
            guidance.push_str(&format!("  • {}\n", f.display()));
        }

        if !self.failure_summaries.is_empty() {
            guidance.push_str("\nPrevious failure summary:\n");
            for (i, summary) in self.failure_summaries.iter().enumerate() {
                guidance.push_str(&format!("  {}. {}\n", i + 1, summary));
            }
        }

        guidance.push_str(
            "\nYou MUST try a completely different approach. Do NOT repeat the same strategy.\n",
        );
        guidance.push_str("────────────────────────────────────────────────────\n");
        guidance
    }
}

fn truncate_summary(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars - 3).collect();
        format!("{}...", truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_before_edit_stores_first_snapshot_only() {
        let mut engine = UndoEngine::new(UndoEngineConfig::default());
        let path = PathBuf::from("/tmp/test.rs");

        engine.capture_before_edit(&path, "original content");
        engine.capture_before_edit(&path, "modified content");

        assert_eq!(engine.get_restore_content(&path), Some("original content"));
    }

    #[test]
    fn record_failure_triggers_at_threshold() {
        let mut engine = UndoEngine::new(UndoEngineConfig {
            rollback_after_failures: 3,
            ..Default::default()
        });

        assert!(!engine.record_failure("error 1"));
        assert!(!engine.record_failure("error 2"));
        assert!(engine.record_failure("error 3"));
    }

    #[test]
    fn record_success_resets_counter() {
        let mut engine = UndoEngine::new(UndoEngineConfig {
            rollback_after_failures: 3,
            ..Default::default()
        });

        engine.record_failure("error 1");
        engine.record_failure("error 2");
        engine.record_success();
        assert!(!engine.record_failure("error 3"));
        assert!(!engine.record_failure("error 4"));
        assert!(engine.record_failure("error 5"));
    }

    #[test]
    fn should_rollback_requires_snapshots() {
        let mut engine = UndoEngine::new(UndoEngineConfig {
            rollback_after_failures: 2,
            ..Default::default()
        });

        engine.record_failure("err");
        engine.record_failure("err");
        assert!(!engine.should_rollback());

        engine.capture_before_edit(Path::new("/tmp/f.rs"), "content");
        // Need to re-trigger threshold
        engine.consecutive_failures = 0;
        engine.record_failure("err");
        engine.record_failure("err");
        assert!(engine.should_rollback());
    }

    #[test]
    fn execute_rollback_returns_files_and_guidance() {
        let mut engine = UndoEngine::new(UndoEngineConfig {
            rollback_after_failures: 2,
            ..Default::default()
        });

        engine.capture_before_edit(Path::new("/tmp/a.rs"), "original a");
        engine.capture_before_edit(Path::new("/tmp/b.rs"), "original b");
        engine.record_failure("compile error in a.rs");
        engine.record_failure("still broken");

        let result = engine.execute_rollback().unwrap();
        assert_eq!(result.restored_files.len(), 2);
        assert!(result.guidance.contains("ROLLBACK"));
        assert!(result.guidance.contains("different approach"));
        assert!(result.guidance.contains("compile error"));
    }

    #[test]
    fn checkpoint_clears_all_state() {
        let mut engine = UndoEngine::new(UndoEngineConfig::default());
        engine.capture_before_edit(Path::new("/tmp/f.rs"), "content");
        engine.record_failure("err");
        engine.checkpoint();

        assert_eq!(engine.tracked_file_count(), 0);
        assert!(!engine.should_rollback());
    }

    #[test]
    fn disabled_engine_does_nothing() {
        let mut engine = UndoEngine::new(UndoEngineConfig {
            enabled: false,
            ..Default::default()
        });

        engine.capture_before_edit(Path::new("/tmp/f.rs"), "content");
        assert_eq!(engine.tracked_file_count(), 0);
        assert!(!engine.record_failure("err"));
    }

    #[test]
    fn max_snapshots_respected() {
        let mut engine = UndoEngine::new(UndoEngineConfig {
            max_snapshots: 2,
            ..Default::default()
        });

        engine.capture_before_edit(Path::new("/tmp/a"), "a");
        engine.capture_before_edit(Path::new("/tmp/b"), "b");
        engine.capture_before_edit(Path::new("/tmp/c"), "c");

        assert_eq!(engine.tracked_file_count(), 2);
        assert!(engine.get_restore_content(Path::new("/tmp/c")).is_none());
    }

    #[test]
    fn rollback_guidance_includes_failure_summaries() {
        let mut engine = UndoEngine::new(UndoEngineConfig {
            rollback_after_failures: 2,
            ..Default::default()
        });

        engine.capture_before_edit(Path::new("/tmp/main.rs"), "fn main() {}");
        engine.record_failure("type mismatch: expected u32, found String");
        engine.record_failure("cannot find value 'x' in this scope");

        let result = engine.execute_rollback().unwrap();
        assert!(result.guidance.contains("type mismatch"));
        assert!(result.guidance.contains("cannot find"));
    }
}
