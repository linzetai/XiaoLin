## ADDED Requirements

### Requirement: Git repository detection
The system SHALL detect whether a given directory is inside a Git repository.

#### Scenario: Valid git repo
- **WHEN** `is_git_repo(dir)` is called on a directory containing `.git/`
- **THEN** the result SHALL be `true`

#### Scenario: Non-git directory
- **WHEN** `is_git_repo(dir)` is called on a directory without `.git/`
- **THEN** the result SHALL be `false`

#### Scenario: Git worktree directory
- **WHEN** `is_git_repo(dir)` is called on a git worktree (`.git` is a file, not directory)
- **THEN** the result SHALL be `true`

### Requirement: Current branch query
The system SHALL return the current branch name for a git repository.

#### Scenario: On a named branch
- **WHEN** `current_branch(dir)` is called and HEAD points to `refs/heads/main`
- **THEN** the result SHALL be `"main"`

#### Scenario: Detached HEAD
- **WHEN** `current_branch(dir)` is called and HEAD is detached
- **THEN** the result SHALL be the abbreviated commit SHA (e.g., `"a1b2c3d"`)

### Requirement: Branch list query
The system SHALL return all local and remote branches.

#### Scenario: List branches
- **WHEN** `branch_list(dir)` is called
- **THEN** the result SHALL include each branch's name, whether it is the current branch, and the latest commit SHA

### Requirement: Git status query
The system SHALL return structured file change information, separated into staged and unstaged groups.

#### Scenario: Status with changes
- **WHEN** `git_status(dir)` is called and there are modified files
- **THEN** the result SHALL include a `staged` array and an `unstaged` array
- **AND** each entry SHALL contain: `path` (string), `status` (added/modified/deleted/renamed/copied), `old_path` (optional, for renames)

#### Scenario: Status with untracked files
- **WHEN** there are untracked files in the working directory
- **THEN** they SHALL appear in the `unstaged` array with `status: "untracked"`

#### Scenario: Clean working directory
- **WHEN** `git_status(dir)` is called on a clean repository
- **THEN** both `staged` and `unstaged` arrays SHALL be empty

#### Scenario: Porcelain v2 format
- **WHEN** executing git status internally
- **THEN** the system SHALL use `git status --porcelain=v2 --branch` for stable, machine-readable output

### Requirement: Diff stat query
The system SHALL return aggregate diff statistics for the working directory.

#### Scenario: Diff stats
- **WHEN** `diff_stat(dir)` is called
- **THEN** the result SHALL include `files_changed` (count), `insertions` (count), `deletions` (count)

### Requirement: Per-file diff query
The system SHALL return diff hunks for a specific file.

#### Scenario: Unstaged diff
- **WHEN** `file_diff(dir, path, staged: false)` is called
- **THEN** the result SHALL return unified diff hunks for the unstaged changes of that file
- **AND** each hunk SHALL include: `old_start`, `old_count`, `new_start`, `new_count`, `lines` (array of context/add/delete lines)

#### Scenario: Staged diff
- **WHEN** `file_diff(dir, path, staged: true)` is called
- **THEN** the result SHALL return unified diff hunks for the staged (index vs HEAD) changes of that file

#### Scenario: Binary file
- **WHEN** the file is binary
- **THEN** the result SHALL indicate `binary: true` and not include line-level diff

### Requirement: Git log query
The system SHALL return recent commit history.

#### Scenario: Log with limit
- **WHEN** `git_log(dir, limit)` is called
- **THEN** the result SHALL return at most `limit` commits
- **AND** each commit SHALL include: `sha` (abbreviated), `message` (first line), `author`, `date`, `files_changed` (count)

### Requirement: Git dir resolution for worktrees
The system SHALL resolve the actual `.git` directory path, handling both regular repos and worktrees.

#### Scenario: Regular repo
- **WHEN** `resolve_git_dir(dir)` is called on a regular repo
- **THEN** the result SHALL be `dir/.git`

#### Scenario: Worktree
- **WHEN** `resolve_git_dir(dir)` is called inside a git worktree
- **THEN** the system SHALL use `git rev-parse --git-dir` to find the actual git directory
