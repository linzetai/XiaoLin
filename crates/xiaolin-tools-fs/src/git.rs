use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

// ── Public Helpers (extracted from worktree.rs) ─────────────────────────

pub async fn run_git(dir: &Path, args: &[&str]) -> Result<String, String> {
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

pub async fn is_git_repo(dir: &Path) -> bool {
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

// ── Data Structures ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub is_git_repo: bool,
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub staged: Vec<FileChange>,
    pub unstaged: Vec<FileChange>,
    pub untracked: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    pub status: FileStatus,
    pub old_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Unmerged,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DiffStat {
    pub files_changed: u32,
    pub insertions: u32,
    pub deletions: u32,
    pub files: Vec<FileDiffStat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiffStat {
    pub path: String,
    pub insertions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffHunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffLineKind {
    Context,
    Add,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Branch {
    pub name: String,
    pub is_current: bool,
    pub is_remote: bool,
    pub upstream: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitSummary {
    pub sha: String,
    pub short_sha: String,
    pub author: String,
    pub date: String,
    pub message: String,
    pub files_changed: u32,
    pub insertions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitResult {
    pub sha: String,
    pub message: String,
}

// ── Query Functions ─────────────────────────────────────────────────────

pub async fn resolve_git_dir(dir: &Path) -> Result<PathBuf, String> {
    let result = run_git(dir, &["rev-parse", "--git-dir"]).await?;
    let git_dir = Path::new(&result);
    if git_dir.is_absolute() {
        Ok(git_dir.to_path_buf())
    } else {
        Ok(dir.join(git_dir))
    }
}

pub async fn current_branch(dir: &Path) -> Result<String, String> {
    run_git(dir, &["rev-parse", "--abbrev-ref", "HEAD"]).await
}

pub async fn branch_list(dir: &Path) -> Result<Vec<Branch>, String> {
    let format = "%(if)%(HEAD)%(then)*%(else) %(end)%(refname:short)\t%(upstream:short)\t%(if)%(symref)%(then)%(else)remote%(end)";
    let output = run_git(dir, &["branch", "-a", "--format", format]).await?;

    let mut branches = Vec::new();
    for line in output.lines() {
        let is_current = line.starts_with('*');
        let line = line.trim_start_matches(['*', ' ']);
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.is_empty() {
            continue;
        }
        let name = parts[0].to_string();
        let upstream = parts.get(1).and_then(|s| {
            if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        });
        let is_remote = parts.get(2).map(|s| *s == "remote").unwrap_or(false)
            || name.starts_with("remotes/");
        branches.push(Branch {
            name,
            is_current,
            is_remote,
            upstream,
        });
    }
    Ok(branches)
}

pub async fn git_status(dir: &Path) -> Result<GitStatus, String> {
    if !is_git_repo(dir).await {
        return Ok(GitStatus {
            is_git_repo: false,
            branch: String::new(),
            upstream: None,
            ahead: 0,
            behind: 0,
            staged: vec![],
            unstaged: vec![],
            untracked: vec![],
        });
    }

    let output = run_git(dir, &["status", "--porcelain=v2", "--branch"]).await?;

    let mut branch = String::new();
    let mut upstream = None;
    let mut ahead = 0u32;
    let mut behind = 0u32;
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    let mut untracked = Vec::new();

    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("# branch.head ") {
            branch = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("# branch.upstream ") {
            upstream = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
            for part in rest.split_whitespace() {
                if let Some(n) = part.strip_prefix('+') {
                    ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = part.strip_prefix('-') {
                    behind = n.parse().unwrap_or(0);
                }
            }
        } else if line.starts_with("1 ") || line.starts_with("2 ") {
            parse_porcelain_v2_entry(line, &mut staged, &mut unstaged);
        } else if let Some(rest) = line.strip_prefix("? ") {
            untracked.push(rest.to_string());
        }
    }

    Ok(GitStatus {
        is_git_repo: true,
        branch,
        upstream,
        ahead,
        behind,
        staged,
        unstaged,
        untracked,
    })
}

fn parse_porcelain_v2_entry(
    line: &str,
    staged: &mut Vec<FileChange>,
    unstaged: &mut Vec<FileChange>,
) {
    let parts: Vec<&str> = line.splitn(9, ' ').collect();
    if parts.len() < 8 {
        return;
    }

    let xy = parts[1];
    let x = xy.as_bytes().first().copied().unwrap_or(b'.');
    let y = xy.as_bytes().get(1).copied().unwrap_or(b'.');

    let is_rename = line.starts_with("2 ");

    let (path, old_path) = if is_rename {
        let tab_parts: Vec<&str> = line.rsplitn(3, '\t').collect();
        if tab_parts.len() >= 2 {
            (
                tab_parts[0].to_string(),
                Some(tab_parts[1].to_string()),
            )
        } else {
            (parts.last().unwrap_or(&"").to_string(), None)
        }
    } else {
        (parts.last().unwrap_or(&"").to_string(), None)
    };

    if x != b'.' {
        staged.push(FileChange {
            path: path.clone(),
            status: char_to_status(x),
            old_path: old_path.clone(),
        });
    }

    if y != b'.' {
        unstaged.push(FileChange {
            path,
            status: char_to_status(y),
            old_path,
        });
    }
}

fn char_to_status(c: u8) -> FileStatus {
    match c {
        b'A' => FileStatus::Added,
        b'M' => FileStatus::Modified,
        b'D' => FileStatus::Deleted,
        b'R' => FileStatus::Renamed,
        b'C' => FileStatus::Copied,
        b'T' => FileStatus::TypeChanged,
        b'U' => FileStatus::Unmerged,
        _ => FileStatus::Modified,
    }
}

pub async fn diff_stat(dir: &Path) -> Result<DiffStat, String> {
    let output = run_git(dir, &["diff", "--numstat"]).await?;

    let mut files = Vec::new();
    let mut total_ins = 0u32;
    let mut total_del = 0u32;

    for line in output.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 3 {
            let ins: u32 = parts[0].parse().unwrap_or(0);
            let del: u32 = parts[1].parse().unwrap_or(0);
            let path = parts[2].to_string();
            total_ins += ins;
            total_del += del;
            files.push(FileDiffStat {
                path,
                insertions: ins,
                deletions: del,
            });
        }
    }

    Ok(DiffStat {
        files_changed: files.len() as u32,
        insertions: total_ins,
        deletions: total_del,
        files,
    })
}

pub async fn file_diff(
    dir: &Path,
    path: &str,
    staged: bool,
) -> Result<Vec<DiffHunk>, String> {
    let args = if staged {
        vec!["diff", "--cached", "--", path]
    } else {
        vec!["diff", "--", path]
    };
    let output = run_git(dir, &args).await?;
    Ok(parse_unified_diff(&output))
}

fn parse_unified_diff(diff_output: &str) -> Vec<DiffHunk> {
    let mut hunks = Vec::new();
    let mut current_hunk: Option<DiffHunk> = None;

    for line in diff_output.lines() {
        if line.starts_with("@@") {
            if let Some(h) = current_hunk.take() {
                hunks.push(h);
            }
            let (old_start, old_lines, new_start, new_lines) = parse_hunk_header(line);
            current_hunk = Some(DiffHunk {
                old_start,
                old_lines,
                new_start,
                new_lines,
                header: line.to_string(),
                lines: Vec::new(),
            });
        } else if let Some(ref mut hunk) = current_hunk {
            let (kind, content) = if let Some(rest) = line.strip_prefix('+') {
                (DiffLineKind::Add, rest.to_string())
            } else if let Some(rest) = line.strip_prefix('-') {
                (DiffLineKind::Delete, rest.to_string())
            } else if let Some(rest) = line.strip_prefix(' ') {
                (DiffLineKind::Context, rest.to_string())
            } else {
                (DiffLineKind::Context, line.to_string())
            };
            hunk.lines.push(DiffLine { kind, content });
        }
    }

    if let Some(h) = current_hunk {
        hunks.push(h);
    }
    hunks
}

fn parse_hunk_header(line: &str) -> (u32, u32, u32, u32) {
    // @@ -old_start,old_lines +new_start,new_lines @@
    let line = line.trim_start_matches("@@ ");
    let parts: Vec<&str> = line.splitn(3, ' ').collect();
    let (old_start, old_lines) = parse_range(parts.first().unwrap_or(&""));
    let (new_start, new_lines) = parse_range(parts.get(1).unwrap_or(&""));
    (old_start, old_lines, new_start, new_lines)
}

fn parse_range(s: &str) -> (u32, u32) {
    let s = s.trim_start_matches(['-', '+']);
    if let Some((start, lines)) = s.split_once(',') {
        (
            start.parse().unwrap_or(0),
            lines.parse().unwrap_or(0),
        )
    } else {
        (s.parse().unwrap_or(0), 1)
    }
}

pub async fn git_log(dir: &Path, limit: u32) -> Result<Vec<CommitSummary>, String> {
    let limit_str = format!("-{}", limit);
    let format = "%H%n%h%n%an%n%aI%n%s";
    let output = run_git(
        dir,
        &["log", &limit_str, &format!("--format={format}"), "--shortstat"],
    )
    .await?;

    let mut commits = Vec::new();
    let mut lines_iter = output.lines().peekable();

    while lines_iter.peek().is_some() {
        let sha = lines_iter.next().unwrap_or_default().to_string();
        if sha.is_empty() {
            continue;
        }
        let short_sha = lines_iter.next().unwrap_or_default().to_string();
        let author = lines_iter.next().unwrap_or_default().to_string();
        let date = lines_iter.next().unwrap_or_default().to_string();
        let message = lines_iter.next().unwrap_or_default().to_string();

        // shortstat line (may be empty for merge commits)
        let mut files_changed = 0u32;
        let mut insertions = 0u32;
        let mut deletions = 0u32;

        if let Some(stat_line) = lines_iter.peek() {
            if stat_line.contains("changed") || stat_line.contains("insertion") || stat_line.contains("deletion") {
                let stat = lines_iter.next().unwrap_or_default();
                for part in stat.split(',') {
                    let part = part.trim();
                    if part.contains("file") {
                        files_changed = part.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(0);
                    } else if part.contains("insertion") {
                        insertions = part.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(0);
                    } else if part.contains("deletion") {
                        deletions = part.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(0);
                    }
                }
            }
        }

        // skip empty separator line
        if lines_iter.peek().map(|l| l.is_empty()).unwrap_or(false) {
            lines_iter.next();
        }

        commits.push(CommitSummary {
            sha,
            short_sha,
            author,
            date,
            message,
            files_changed,
            insertions,
            deletions,
        });
    }

    Ok(commits)
}

// ── Write Operations ────────────────────────────────────────────────────

pub async fn git_stage(dir: &Path, files: &[String]) -> Result<(), String> {
    if files.is_empty() {
        return run_git(dir, &["add", "-A"]).await.map(|_| ());
    }
    let mut args = vec!["add", "--"];
    let refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
    args.extend(refs);
    run_git(dir, &args).await.map(|_| ())
}

pub async fn git_unstage(dir: &Path, files: &[String]) -> Result<(), String> {
    if files.is_empty() {
        return run_git(dir, &["restore", "--staged", "."]).await.map(|_| ());
    }
    let mut args = vec!["restore", "--staged", "--"];
    let refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
    args.extend(refs);
    run_git(dir, &args).await.map(|_| ())
}

pub async fn git_commit(dir: &Path, message: &str) -> Result<CommitResult, String> {
    run_git(dir, &["commit", "-m", message]).await?;
    let sha = run_git(dir, &["rev-parse", "HEAD"]).await?;
    Ok(CommitResult {
        sha,
        message: message.to_string(),
    })
}

pub async fn git_revert_files(dir: &Path, files: &[String]) -> Result<(), String> {
    if files.is_empty() {
        return Err("no files specified for revert".into());
    }

    let mut tracked = Vec::new();
    let mut untracked_to_delete = Vec::new();

    let status = git_status(dir).await?;
    let all_untracked: std::collections::HashSet<&str> =
        status.untracked.iter().map(|s| s.as_str()).collect();

    for f in files {
        if all_untracked.contains(f.as_str()) {
            untracked_to_delete.push(f.clone());
        } else {
            tracked.push(f.clone());
        }
    }

    if !tracked.is_empty() {
        let mut args = vec!["checkout", "--"];
        let refs: Vec<&str> = tracked.iter().map(|s| s.as_str()).collect();
        args.extend(refs);
        run_git(dir, &args).await?;
    }

    for f in &untracked_to_delete {
        let path = dir.join(f);
        let _ = tokio::fs::remove_file(&path).await;
    }

    Ok(())
}

// ── Write Mutex ─────────────────────────────────────────────────────────

pub struct GitWriteLock {
    locks: std::sync::Mutex<HashMap<PathBuf, std::sync::Arc<Mutex<()>>>>,
}

impl GitWriteLock {
    pub fn new() -> Self {
        Self {
            locks: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, dir: &Path) -> std::sync::Arc<Mutex<()>> {
        let mut map = self.locks.lock().unwrap();
        map.entry(dir.to_path_buf())
            .or_insert_with(|| std::sync::Arc::new(Mutex::new(())))
            .clone()
    }
}

impl Default for GitWriteLock {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn wait_for_git_lock(dir: &Path, timeout_ms: u64) -> Result<(), String> {
    let git_dir = resolve_git_dir(dir).await?;
    let lock_file = git_dir.join("index.lock");

    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(timeout_ms);

    while lock_file.exists() {
        if tokio::time::Instant::now() > deadline {
            return Err("git index.lock still held after timeout".into());
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_porcelain_v2_modified() {
        let line = "1 .M N... 100644 100644 100644 abc123 def456 src/main.rs";
        let mut staged = Vec::new();
        let mut unstaged = Vec::new();
        parse_porcelain_v2_entry(line, &mut staged, &mut unstaged);
        assert!(staged.is_empty());
        assert_eq!(unstaged.len(), 1);
        assert_eq!(unstaged[0].path, "src/main.rs");
        assert_eq!(unstaged[0].status, FileStatus::Modified);
    }

    #[test]
    fn test_parse_porcelain_v2_staged_added() {
        let line = "1 A. N... 000000 100644 100644 0000000 abc1234 new_file.rs";
        let mut staged = Vec::new();
        let mut unstaged = Vec::new();
        parse_porcelain_v2_entry(line, &mut staged, &mut unstaged);
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].status, FileStatus::Added);
        assert!(unstaged.is_empty());
    }

    #[test]
    fn test_parse_unified_diff() {
        let diff = r#"@@ -1,3 +1,4 @@
 line1
+new line
 line2
 line3
@@ -10,2 +11,3 @@
 context
+added
 end"#;
        let hunks = parse_unified_diff(diff);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].old_start, 1);
        assert_eq!(hunks[0].old_lines, 3);
        assert_eq!(hunks[0].new_start, 1);
        assert_eq!(hunks[0].new_lines, 4);
        assert_eq!(hunks[0].lines.len(), 4);
        assert_eq!(hunks[0].lines[1].kind, DiffLineKind::Add);
    }

    #[test]
    fn test_parse_hunk_header() {
        let (os, ol, ns, nl) = parse_hunk_header("@@ -10,5 +12,7 @@ fn main()");
        assert_eq!((os, ol, ns, nl), (10, 5, 12, 7));
    }

    #[test]
    fn test_parse_range_single() {
        assert_eq!(parse_range("-5"), (5, 1));
        assert_eq!(parse_range("+3,10"), (3, 10));
    }
}
