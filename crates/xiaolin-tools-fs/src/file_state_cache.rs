use std::path::{Path, PathBuf};
use std::time::SystemTime;

use dashmap::DashMap;

#[derive(Debug, Clone)]
struct FileState {
    content_hash: u64,
    modified_at: SystemTime,
    content_preview: String,
    /// The offset/limit range that was read. Used for dedup detection.
    read_offset: Option<i64>,
    read_limit: Option<usize>,
}

/// Stale-detection result when comparing file state before a write/edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaleCheckResult {
    /// File has not been modified since we last read it — safe to proceed.
    Fresh,
    /// File was modified externally since we last read it.
    Stale,
    /// We have no cached state for this file (never read it in this session).
    NeverRead,
}

/// Cache that tracks file content hashes and modification times.
///
/// Provides three key capabilities inspired by Claude Code's `readFileState`:
/// 1. **Dedup detection**: Skip re-reading files that haven't changed on disk.
/// 2. **Stale detection**: Reject edits/writes when the file was modified externally
///    after we last read it (prevents data loss from concurrent edits).
/// 3. **Post-write update**: Record new state after we write, so subsequent
///    stale checks compare against our write (not the earlier read).
#[derive(Debug, Clone)]
pub struct FileStateCache {
    cache: DashMap<PathBuf, FileState>,
}

impl Default for FileStateCache {
    fn default() -> Self {
        Self::new()
    }
}

impl FileStateCache {
    pub fn new() -> Self {
        Self {
            cache: DashMap::new(),
        }
    }

    /// Check whether the file at `path` is unchanged since last update.
    /// Returns `true` if the file's modification time matches our cached value.
    /// Returns `false` if the file has been modified, is missing from cache,
    /// or if we cannot stat the file.
    pub fn is_unchanged(&self, path: &Path) -> bool {
        let entry = match self.cache.get(path) {
            Some(e) => e,
            None => return false,
        };

        let current_mtime = match std::fs::metadata(path).and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(_) => return false,
        };

        entry.modified_at == current_mtime
    }

    /// Check if the same read range was used and the file is unchanged.
    /// Used by ReadFileTool to skip re-sending unchanged content.
    pub fn is_unchanged_for_range(
        &self,
        path: &Path,
        offset: Option<i64>,
        limit: Option<usize>,
    ) -> bool {
        let entry = match self.cache.get(path) {
            Some(e) => e,
            None => return false,
        };

        if entry.read_offset != offset || entry.read_limit != limit {
            return false;
        }

        let current_mtime = match std::fs::metadata(path).and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(_) => return false,
        };

        entry.modified_at == current_mtime
    }

    /// Perform a stale check before write/edit: has the file been modified
    /// since we last read (or wrote) it?
    ///
    /// Returns:
    /// - `Fresh` — safe to proceed with the write/edit
    /// - `Stale` — file changed externally; must re-read before editing
    /// - `NeverRead` — we have no record; the tool should decide whether to
    ///   require a read-first or allow the operation
    pub fn check_stale(&self, path: &Path) -> StaleCheckResult {
        let entry = match self.cache.get(path) {
            Some(e) => e,
            None => return StaleCheckResult::NeverRead,
        };

        let current_mtime = match std::fs::metadata(path).and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(_) => return StaleCheckResult::Fresh,
        };

        if current_mtime > entry.modified_at {
            // mtime changed — fall back to content hash comparison.
            // Editors like vim/emacs rewrite the file on save even without
            // content changes, causing mtime bumps. Comparing the hash
            // avoids false-positive stale rejections.
            if let Ok(content) = std::fs::read_to_string(path) {
                if compute_hash(&content) == entry.content_hash {
                    return StaleCheckResult::Fresh;
                }
            }
            StaleCheckResult::Stale
        } else {
            StaleCheckResult::Fresh
        }
    }

    /// Record the current state of a file's content.
    /// Stores the content hash, modification time, and a preview (first 200 lines).
    pub fn update(&self, path: &Path, content: &str) {
        self.update_with_range(path, content, None, None);
    }

    /// Record the current state with offset/limit metadata (from a read operation).
    pub fn update_with_range(
        &self,
        path: &Path,
        content: &str,
        offset: Option<i64>,
        limit: Option<usize>,
    ) {
        let mtime = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let hash = compute_hash(content);
        let preview = content.lines().take(200).collect::<Vec<_>>().join("\n");

        self.cache.insert(
            path.to_path_buf(),
            FileState {
                content_hash: hash,
                modified_at: mtime,
                content_preview: preview,
                read_offset: offset,
                read_limit: limit,
            },
        );
    }

    /// Update cached modification time after a successful write/edit.
    /// Keeps the content hash intact so subsequent edits to the same
    /// content are detected as stale if something else modifies the file.
    pub fn refresh_mtime(&self, path: &Path) {
        if let Some(mut entry) = self.cache.get_mut(path) {
            if let Ok(mtime) = std::fs::metadata(path).and_then(|m| m.modified()) {
                entry.modified_at = mtime;
                entry.read_offset = None;
                entry.read_limit = None;
            }
        }
    }

    /// Remove a single path from the cache (e.g. after a write/edit operation).
    pub fn invalidate(&self, path: &Path) {
        self.cache.remove(path);
    }

    /// Clear the entire cache.
    pub fn invalidate_all(&self) {
        self.cache.clear();
    }

    /// Get the cached content preview for a path, if available and unchanged.
    pub fn get_preview(&self, path: &Path) -> Option<String> {
        if self.is_unchanged(path) {
            self.cache.get(path).map(|e| e.content_preview.clone())
        } else {
            None
        }
    }

    /// Get the content hash for a path, if cached.
    pub fn content_hash(&self, path: &Path) -> Option<u64> {
        self.cache.get(path).map(|e| e.content_hash)
    }

    /// Check if we have ever read this file in the current session.
    pub fn has_read(&self, path: &Path) -> bool {
        self.cache.contains_key(path)
    }

    /// Number of entries currently cached.
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

fn compute_hash(content: &str) -> u64 {
    let hash = blake3::hash(content.as_bytes());
    u64::from_le_bytes(hash.as_bytes()[..8].try_into().expect("blake3 hash is 32 bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn is_unchanged_returns_true_for_unmodified_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello world").unwrap();

        let cache = FileStateCache::new();
        cache.update(&file_path, "hello world");

        assert!(cache.is_unchanged(&file_path));
    }

    #[test]
    fn is_unchanged_returns_false_after_modification() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello").unwrap();

        let cache = FileStateCache::new();
        cache.update(&file_path, "hello");
        assert!(cache.is_unchanged(&file_path));

        std::thread::sleep(std::time::Duration::from_millis(50));
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&file_path)
            .unwrap();
        f.write_all(b"modified content").unwrap();
        f.flush().unwrap();
        drop(f);

        assert!(!cache.is_unchanged(&file_path));
    }

    #[test]
    fn invalidate_removes_single_path() {
        let dir = tempfile::tempdir().unwrap();
        let path_a = dir.path().join("a.txt");
        let path_b = dir.path().join("b.txt");
        std::fs::write(&path_a, "aaa").unwrap();
        std::fs::write(&path_b, "bbb").unwrap();

        let cache = FileStateCache::new();
        cache.update(&path_a, "aaa");
        cache.update(&path_b, "bbb");
        assert_eq!(cache.len(), 2);

        cache.invalidate(&path_a);
        assert_eq!(cache.len(), 1);
        assert!(!cache.is_unchanged(&path_a));
        assert!(cache.is_unchanged(&path_b));
    }

    #[test]
    fn invalidate_all_clears_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path_a = dir.path().join("a.txt");
        let path_b = dir.path().join("b.txt");
        std::fs::write(&path_a, "aaa").unwrap();
        std::fs::write(&path_b, "bbb").unwrap();

        let cache = FileStateCache::new();
        cache.update(&path_a, "aaa");
        cache.update(&path_b, "bbb");
        assert_eq!(cache.len(), 2);

        cache.invalidate_all();
        assert!(cache.is_empty());
        assert!(!cache.is_unchanged(&path_a));
        assert!(!cache.is_unchanged(&path_b));
    }

    #[test]
    fn check_stale_mtime_changed_content_same_returns_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("stale_test.txt");
        std::fs::write(&file_path, "same content").unwrap();

        let cache = FileStateCache::new();
        cache.update(&file_path, "same content");
        assert_eq!(cache.check_stale(&file_path), StaleCheckResult::Fresh);

        // Bump mtime by re-writing the same content after a delay
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(&file_path, "same content").unwrap();

        // mtime changed but content hash matches → should be Fresh
        assert_eq!(
            cache.check_stale(&file_path),
            StaleCheckResult::Fresh,
            "should be Fresh when mtime changed but content hash is the same"
        );
    }

    #[test]
    fn check_stale_mtime_changed_content_different_returns_stale() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("stale_test2.txt");
        std::fs::write(&file_path, "original content").unwrap();

        let cache = FileStateCache::new();
        cache.update(&file_path, "original content");

        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(&file_path, "modified content").unwrap();

        assert_eq!(
            cache.check_stale(&file_path),
            StaleCheckResult::Stale,
            "should be Stale when both mtime and content changed"
        );
    }

    #[test]
    fn get_preview_returns_first_200_lines() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("long.txt");

        let content: String = (0..300)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&file_path, &content).unwrap();

        let cache = FileStateCache::new();
        cache.update(&file_path, &content);

        let preview = cache.get_preview(&file_path).unwrap();
        let preview_lines: Vec<_> = preview.lines().collect();
        assert_eq!(preview_lines.len(), 200);
        assert_eq!(preview_lines[0], "line 0");
        assert_eq!(preview_lines[199], "line 199");
    }
}
