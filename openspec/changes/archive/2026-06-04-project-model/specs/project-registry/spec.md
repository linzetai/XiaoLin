## ADDED Requirements

### Requirement: Projects table schema
The system SHALL maintain a `projects` table in SQLite with the following columns: `id TEXT PRIMARY KEY`, `name TEXT NOT NULL`, `root_path TEXT NOT NULL UNIQUE`, `color TEXT NOT NULL DEFAULT '#0066cc'`, `pinned INTEGER NOT NULL DEFAULT 0`, `archived INTEGER NOT NULL DEFAULT 0`, `created_at TEXT NOT NULL DEFAULT (datetime('now'))`, `last_opened_at TEXT NOT NULL DEFAULT (datetime('now'))`.

#### Scenario: Table creation on startup
- **WHEN** the SessionStore initializes and the `projects` table does not exist
- **THEN** the system SHALL create the table with the specified schema

#### Scenario: Unique root_path constraint
- **WHEN** a project with root_path `/home/user/my-app` already exists
- **AND** the system attempts to create another project with the same canonicalized root_path
- **THEN** the system SHALL return the existing project instead of creating a duplicate

### Requirement: Project ID generation
The system SHALL generate project IDs as the first 16 hex characters of SHA-256 of the canonicalized absolute root_path.

#### Scenario: Deterministic ID from path
- **WHEN** a project is created for path `/home/user/my-app`
- **THEN** the generated ID SHALL be `hex(sha256(canonicalize("/home/user/my-app")))[..16]`
- **AND** creating a project for the same path again SHALL produce the same ID

#### Scenario: Symlink resolution
- **WHEN** the root_path is a symbolic link `/home/user/link` pointing to `/home/user/actual`
- **THEN** the system SHALL canonicalize the path to `/home/user/actual` before hashing
- **AND** both paths SHALL produce the same project ID

### Requirement: Project CRUD operations
The system SHALL support create, read, update, and delete operations on the `projects` table.

#### Scenario: Create project
- **WHEN** `create_project(root_path, name)` is called with a new path
- **THEN** the system SHALL insert a new row with the generated ID, canonicalized root_path, and provided name
- **AND** return the created Project struct

#### Scenario: Create project with auto-detected name
- **WHEN** `create_project(root_path, None)` is called without an explicit name
- **THEN** the system SHALL use the last path component as the project name

#### Scenario: List projects
- **WHEN** `list_projects()` is called
- **THEN** the system SHALL return all non-archived projects ordered by `last_opened_at DESC`

#### Scenario: List projects with archived
- **WHEN** `list_projects_with_archived()` is called
- **THEN** the system SHALL return all projects including archived ones

#### Scenario: Update project
- **WHEN** `update_project(id, patch)` is called with valid fields (name, color, pinned)
- **THEN** the system SHALL update only the specified fields

#### Scenario: Delete project
- **WHEN** `delete_project(id)` is called
- **THEN** the system SHALL remove the project row
- **AND** set `project_id = NULL` on all sessions that referenced this project

### Requirement: Project reachability check
The system SHALL detect whether a project's root_path still exists on the filesystem.

#### Scenario: Reachable project
- **WHEN** listing projects and the project's root_path exists on disk
- **THEN** the project SHALL be marked as `reachable: true`

#### Scenario: Unreachable project
- **WHEN** listing projects and the project's root_path does not exist on disk
- **THEN** the project SHALL be marked as `reachable: false`
- **AND** the project SHALL still be returned in the list (not silently dropped)

### Requirement: Project auto-discovery from work_dir
The system SHALL find or create a project when given a work_dir path.

#### Scenario: Existing project found
- **WHEN** `find_or_create_project(work_dir)` is called
- **AND** a project with the canonicalized workspace root already exists
- **THEN** the system SHALL return the existing project and update its `last_opened_at`

#### Scenario: New project created
- **WHEN** `find_or_create_project(work_dir)` is called
- **AND** no project with the canonicalized workspace root exists
- **THEN** the system SHALL detect the workspace root using `detect_workspace_root(work_dir)`
- **AND** create a new project with the detected root as root_path
