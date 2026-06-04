## ADDED Requirements

### Requirement: Git directory monitoring
The system SHALL use the `notify` crate to monitor changes in `.git/HEAD`, `.git/index`, and `.git/refs/heads/` for active projects.

#### Scenario: Branch change detected
- **WHEN** `.git/HEAD` is modified (user switches branch)
- **THEN** the system SHALL trigger a git status refresh within 300ms

#### Scenario: Index change detected
- **WHEN** `.git/index` is modified (user runs git add/reset/commit)
- **THEN** the system SHALL trigger a git status refresh within 300ms

#### Scenario: New commit detected
- **WHEN** a file in `.git/refs/heads/` is modified
- **THEN** the system SHALL trigger a git status refresh within 300ms

### Requirement: Debounce rapid changes
The system SHALL debounce rapid file change events to avoid redundant git status queries.

#### Scenario: Multiple events in quick succession
- **WHEN** `.git/index` and `.git/refs/heads/main` change within 100ms (e.g., during git commit)
- **THEN** the system SHALL execute only ONE git status query after a 200ms quiet period

### Requirement: Per-project watcher lifecycle
Each active project SHALL have its own independent GitWatcher instance.

#### Scenario: Start watcher for active project
- **WHEN** a project becomes active (session opened with projectId pointing to a git repo)
- **THEN** the system SHALL create a GitWatcher for that project's root_path
- **AND** begin monitoring `.git/` changes

#### Scenario: Stop watcher for inactive project
- **WHEN** no sessions reference a project (all sessions with that projectId are closed)
- **THEN** the system SHALL stop and drop the GitWatcher for that project
- **AND** release file descriptor resources

#### Scenario: Non-git project
- **WHEN** a project's root_path is not a git repository
- **THEN** the system SHALL NOT create a GitWatcher for that project

### Requirement: Worktree git directory resolution
The GitWatcher SHALL resolve the actual `.git` directory for worktree scenarios.

#### Scenario: Regular repo watching
- **WHEN** setting up a watcher for a regular git repo at `/home/user/app`
- **THEN** the system SHALL watch `/home/user/app/.git/HEAD`, `.git/index`, `.git/refs/heads/`

#### Scenario: Worktree repo watching
- **WHEN** setting up a watcher for a git worktree where `.git` is a file
- **THEN** the system SHALL use `git rev-parse --git-dir` to find the actual git directory
- **AND** watch the resolved git directory's `HEAD`, `index`, and `refs/heads/`

### Requirement: Status change broadcast
When git status changes are detected, the system SHALL broadcast a `git.status_changed` event via WebSocket.

#### Scenario: Broadcast format
- **WHEN** a git status change is detected for a project
- **THEN** the system SHALL broadcast `{ "type": "event", "event": "git.status_changed", "data": { "projectId": "<id>", "snapshot": { "branch", "staged", "unstaged", "stats" } } }`

### Requirement: Agent tool trigger
File write tools SHALL trigger a git status refresh for the affected project after successful execution.

#### Scenario: File edit triggers refresh
- **WHEN** a file write tool (EditFile, WriteFile, ApplyPatch) completes successfully
- **AND** the file is inside a project root that is a git repository
- **THEN** the system SHALL trigger a git status refresh for that project (debounced)

### Requirement: Frontend polling fallback
The frontend SHALL poll git status as a fallback mechanism.

#### Scenario: Polling interval
- **WHEN** 30 seconds have passed since the last git status update (from any source)
- **THEN** the frontend SHALL request `git.status` from the backend

#### Scenario: Skip redundant poll
- **WHEN** a `git.status_changed` event was received within the last 30 seconds
- **THEN** the frontend SHALL skip the scheduled polling request
