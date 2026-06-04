## ADDED Requirements

### Requirement: Session project_id column
The `sessions` table SHALL have a `project_id TEXT` nullable column that references `projects(id)`.

#### Scenario: Column migration
- **WHEN** the SessionStore initializes and the `project_id` column does not exist on the `sessions` table
- **THEN** the system SHALL execute `ALTER TABLE sessions ADD COLUMN project_id TEXT`

#### Scenario: Foreign key behavior on project deletion
- **WHEN** a project is deleted
- **THEN** all sessions with that project_id SHALL have their `project_id` set to NULL
- **AND** the sessions themselves SHALL NOT be deleted

### Requirement: Auto-bind session to project on creation
When a session is created with a work_dir, the system SHALL automatically find or create the corresponding project and set the session's project_id.

#### Scenario: New session with work_dir
- **WHEN** a session is created with `work_dir = "/home/user/my-app"`
- **THEN** the system SHALL call `find_or_create_project("/home/user/my-app")`
- **AND** set the session's `project_id` to the returned project ID

#### Scenario: New session without work_dir
- **WHEN** a session is created without a work_dir
- **THEN** the session's `project_id` SHALL be NULL

### Requirement: Update project_id when work_dir changes
When a session's work_dir is changed via `sessions.set_work_dir`, the system SHALL update the session's project_id accordingly.

#### Scenario: Set work_dir on existing session
- **WHEN** `sessions.set_work_dir` is called with a new work_dir
- **THEN** the system SHALL find or create the project for the new work_dir
- **AND** update the session's `project_id` to match

#### Scenario: Clear work_dir on existing session
- **WHEN** `sessions.set_work_dir` is called with `work_dir = null`
- **THEN** the session's `project_id` SHALL be set to NULL

### Requirement: Startup migration of existing sessions
On gateway startup, the system SHALL migrate existing sessions that have `work_dir` but no `project_id`.

#### Scenario: Migrate sessions with valid work_dir
- **WHEN** the gateway starts and finds sessions with `work_dir IS NOT NULL AND project_id IS NULL`
- **THEN** the system SHALL process each such session:
- **AND** call `find_or_create_project(work_dir)` for each unique work_dir
- **AND** update the session's `project_id` to the returned project ID

#### Scenario: Migrate sessions with unreachable work_dir
- **WHEN** a session's work_dir path does not exist on disk during migration
- **THEN** the system SHALL still create a Project record (marked as unreachable)
- **AND** set the session's project_id to that project
