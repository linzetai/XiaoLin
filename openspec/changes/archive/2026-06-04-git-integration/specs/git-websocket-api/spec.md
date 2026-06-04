## ADDED Requirements

### Requirement: git.status WebSocket method
The system SHALL handle `git.status` requests and return structured git status data.

#### Scenario: Status for git project
- **WHEN** a client sends `{ "type": "git.status", "params": { "projectId": "<id>" } }`
- **THEN** the system SHALL resolve the project's root_path and return `{ "isGitRepo": true, "branch": "main", "staged": [...], "unstaged": [...], "stats": { "filesChanged": N, "insertions": N, "deletions": N } }`

#### Scenario: Status for non-git project
- **WHEN** a client sends `git.status` for a project that is not a git repo
- **THEN** the system SHALL return `{ "isGitRepo": false }`

#### Scenario: Status for unknown project
- **WHEN** a client sends `git.status` with an invalid projectId
- **THEN** the system SHALL return an error: "project not found"

### Requirement: git.diff WebSocket method
The system SHALL handle `git.diff` requests and return per-file diff hunks.

#### Scenario: Diff for a changed file
- **WHEN** a client sends `{ "type": "git.diff", "params": { "projectId": "<id>", "path": "src/main.rs", "staged": false } }`
- **THEN** the system SHALL return `{ "path": "src/main.rs", "binary": false, "hunks": [{ "oldStart": N, "oldCount": N, "newStart": N, "newCount": N, "lines": [...] }] }`

#### Scenario: Diff for unchanged file
- **WHEN** the specified file has no changes
- **THEN** the system SHALL return `{ "path": "...", "binary": false, "hunks": [] }`

### Requirement: git.branches WebSocket method
The system SHALL handle `git.branches` requests and return branch information.

#### Scenario: List branches
- **WHEN** a client sends `{ "type": "git.branches", "params": { "projectId": "<id>" } }`
- **THEN** the system SHALL return `{ "current": "main", "branches": [{ "name": "main", "sha": "abc123", "current": true }, ...] }`

### Requirement: git.log WebSocket method
The system SHALL handle `git.log` requests and return recent commit history.

#### Scenario: Log with default limit
- **WHEN** a client sends `{ "type": "git.log", "params": { "projectId": "<id>" } }`
- **THEN** the system SHALL return the most recent 20 commits

#### Scenario: Log with custom limit
- **WHEN** a client sends `{ "type": "git.log", "params": { "projectId": "<id>", "limit": 50 } }`
- **THEN** the system SHALL return at most 50 commits

### Requirement: git.stage WebSocket method
The system SHALL handle `git.stage` requests to add files to the staging area.

#### Scenario: Stage files
- **WHEN** a client sends `{ "type": "git.stage", "params": { "projectId": "<id>", "files": ["src/main.rs"] } }`
- **THEN** the system SHALL execute git add for the specified files
- **AND** broadcast `git.status_changed` with the updated status

#### Scenario: Stage all
- **WHEN** a client sends `{ "type": "git.stage", "params": { "projectId": "<id>", "files": ["."] } }`
- **THEN** the system SHALL stage all changes

### Requirement: git.unstage WebSocket method
The system SHALL handle `git.unstage` requests to remove files from the staging area.

#### Scenario: Unstage files
- **WHEN** a client sends `{ "type": "git.unstage", "params": { "projectId": "<id>", "files": ["src/main.rs"] } }`
- **THEN** the system SHALL execute git restore --staged for the specified files
- **AND** broadcast `git.status_changed`

### Requirement: git.commit WebSocket method
The system SHALL handle `git.commit` requests to create a commit.

#### Scenario: Commit with message
- **WHEN** a client sends `{ "type": "git.commit", "params": { "projectId": "<id>", "message": "fix: resolve null check" } }`
- **THEN** the system SHALL execute `git commit -m <message>`
- **AND** return `{ "sha": "<hash>", "message": "<message>" }`
- **AND** broadcast `git.status_changed`

#### Scenario: Commit with empty staging area
- **WHEN** no files are staged
- **THEN** the system SHALL return an error: "nothing to commit"

### Requirement: git.revert WebSocket method
The system SHALL handle `git.revert` requests to discard unstaged changes.

#### Scenario: Revert specific files
- **WHEN** a client sends `{ "type": "git.revert", "params": { "projectId": "<id>", "files": ["src/main.rs"] } }`
- **THEN** the system SHALL revert the specified files to their last committed state
- **AND** broadcast `git.status_changed`

#### Scenario: Revert all
- **WHEN** a client sends `{ "type": "git.revert", "params": { "projectId": "<id>", "files": ["."] } }`
- **THEN** the system SHALL revert all unstaged changes
