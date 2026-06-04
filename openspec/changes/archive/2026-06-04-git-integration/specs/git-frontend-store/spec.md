## ADDED Requirements

### Requirement: Git store state shape
The frontend SHALL maintain a Zustand store `useGitStore` with state: `isGitRepo: boolean`, `branch: string`, `branches: Branch[]`, `staged: FileChange[]`, `unstaged: FileChange[]`, `stats: DiffStats`, `selectedFile: string | null`, `selectedDiff: DiffHunk[] | null`, `isLoading: boolean`.

#### Scenario: Initial state
- **WHEN** the app starts
- **THEN** `isGitRepo` SHALL be `false`, all arrays SHALL be empty, `selectedFile` SHALL be null

### Requirement: Sync git status from backend
The store SHALL synchronize git status data from the backend.

#### Scenario: Initial load
- **WHEN** the active project changes to a project with a projectId
- **THEN** the store SHALL request `git.status` and populate the state

#### Scenario: Live update on git.status_changed event
- **WHEN** the store receives a `git.status_changed` WebSocket event for the active project
- **THEN** the store SHALL update `branch`, `staged`, `unstaged`, and `stats` from the event's snapshot data

#### Scenario: Project has no git
- **WHEN** `git.status` returns `{ "isGitRepo": false }`
- **THEN** the store SHALL set `isGitRepo = false` and clear all git-related state

### Requirement: File diff loading
The store SHALL load diff data for a selected file.

#### Scenario: Select file for diff
- **WHEN** `selectFile(path, staged)` is called
- **THEN** the store SHALL set `selectedFile` and request `git.diff` from the backend
- **AND** set `selectedDiff` to the returned hunks

#### Scenario: Deselect file
- **WHEN** `selectFile(null)` is called
- **THEN** `selectedFile` and `selectedDiff` SHALL be cleared

### Requirement: Git action dispatchers
The store SHALL provide actions for stage, unstage, commit, and revert operations.

#### Scenario: Stage action
- **WHEN** `stageFiles(files)` is called
- **THEN** the store SHALL send `git.stage` to the backend

#### Scenario: Unstage action
- **WHEN** `unstageFiles(files)` is called
- **THEN** the store SHALL send `git.unstage` to the backend

#### Scenario: Commit action
- **WHEN** `commit(message)` is called
- **THEN** the store SHALL send `git.commit` to the backend
- **AND** return the result (success with SHA or error)

#### Scenario: Revert action
- **WHEN** `revertFiles(files)` is called
- **THEN** the store SHALL send `git.revert` to the backend

### Requirement: Polling fallback
The store SHALL implement a 30-second polling fallback for git status.

#### Scenario: Poll when no recent updates
- **WHEN** 30 seconds have elapsed since the last git status update
- **AND** the active project is a git repo
- **THEN** the store SHALL request `git.status` from the backend

#### Scenario: Skip poll after recent event
- **WHEN** a `git.status_changed` event was received within the last 30 seconds
- **THEN** the next scheduled poll SHALL be skipped

### Requirement: Active project tracking
The git store SHALL automatically follow the active project.

#### Scenario: Project switch
- **WHEN** the active project changes (via project-store or session switch)
- **THEN** the git store SHALL clear current state and load git status for the new project

#### Scenario: No active project
- **WHEN** there is no active project (activeProjectId is null)
- **THEN** the git store SHALL clear all state and set `isGitRepo = false`
