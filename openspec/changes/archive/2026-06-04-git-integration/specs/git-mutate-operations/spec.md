## ADDED Requirements

### Requirement: Stage files
The system SHALL support staging files for commit.

#### Scenario: Stage specific files
- **WHEN** `git_stage(dir, files)` is called with a list of file paths
- **THEN** the system SHALL execute `git add` for each specified file
- **AND** trigger a git status refresh after completion

#### Scenario: Stage all changes
- **WHEN** `git_stage(dir, ["."])` is called
- **THEN** the system SHALL execute `git add .`
- **AND** trigger a git status refresh after completion

### Requirement: Unstage files
The system SHALL support unstaging files from the index.

#### Scenario: Unstage specific files
- **WHEN** `git_unstage(dir, files)` is called with a list of file paths
- **THEN** the system SHALL execute `git restore --staged` for each specified file
- **AND** trigger a git status refresh after completion

#### Scenario: Unstage all
- **WHEN** `git_unstage(dir, ["."])` is called
- **THEN** the system SHALL execute `git restore --staged .`
- **AND** trigger a git status refresh after completion

### Requirement: Commit staged changes
The system SHALL support creating a commit with staged changes.

#### Scenario: Commit with message
- **WHEN** `git_commit(dir, message)` is called with a non-empty message
- **THEN** the system SHALL execute `git commit -m <message>`
- **AND** return the new commit SHA and summary
- **AND** trigger a git status refresh after completion

#### Scenario: Nothing to commit
- **WHEN** `git_commit(dir, message)` is called but there are no staged changes
- **THEN** the system SHALL return an error indicating "nothing to commit"

### Requirement: Revert file changes
The system SHALL support reverting unstaged changes in working directory files.

#### Scenario: Revert specific files
- **WHEN** `git_revert_files(dir, files)` is called with a list of file paths
- **THEN** the system SHALL execute `git checkout -- <files>` for tracked files
- **AND** for untracked files, the system SHALL delete them from the working directory
- **AND** trigger a git status refresh after completion

#### Scenario: Revert all unstaged changes
- **WHEN** `git_revert_files(dir, ["."])` is called
- **THEN** the system SHALL execute `git checkout -- .` and `git clean -fd` for untracked files
- **AND** trigger a git status refresh after completion

### Requirement: Write operation serialization
All git write operations for a given project directory SHALL be serialized (one at a time).

#### Scenario: Concurrent write prevention
- **WHEN** two git write operations are requested simultaneously for the same project
- **THEN** the second operation SHALL wait until the first completes before executing

#### Scenario: Read operations unblocked
- **WHEN** a git write operation is in progress
- **THEN** git read operations (status, diff, log) SHALL NOT be blocked

### Requirement: Git lock detection
The system SHALL detect git index lock files and handle them gracefully.

#### Scenario: Lock file present
- **WHEN** a git write operation is attempted and `.git/index.lock` exists
- **THEN** the system SHALL wait up to 5 seconds for the lock to clear
- **AND** if the lock persists, return an error: "Git operation in progress, please try again"
