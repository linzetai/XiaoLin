## ADDED Requirements

### Requirement: Git snapshot injection on first turn
When the working directory is a git repository, the system SHALL inject a git snapshot into the initial context containing: current branch, short status, and recent commits.

#### Scenario: Git repo with modifications
- **WHEN** the agent starts a turn in a git repository with uncommitted changes
- **THEN** the context SHALL include the current branch name, modified file list from `git status --short`, and the last 5 commit summaries

#### Scenario: Non-git directory
- **WHEN** the working directory is not a git repository
- **THEN** no git snapshot SHALL be injected (no error, silent skip)

### Requirement: Git status truncation
The git status output SHALL be truncated to 2000 characters maximum to prevent context bloat in large repositories.

#### Scenario: Large git status output
- **WHEN** `git status --short` output exceeds 2000 characters
- **THEN** the output SHALL be truncated with a "[truncated]" indicator

### Requirement: Git snapshot format
The git snapshot SHALL use a compact, structured format with clear delimiters.

#### Scenario: Standard git snapshot format
- **WHEN** git snapshot is injected
- **THEN** it SHALL follow the format: delimited block with Branch, Status (short), and Recent commits sections
