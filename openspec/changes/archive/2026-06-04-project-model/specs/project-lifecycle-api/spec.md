## ADDED Requirements

### Requirement: projects.list WebSocket method
The system SHALL handle `projects.list` WebSocket requests and return all non-archived projects.

#### Scenario: List all projects
- **WHEN** a client sends `{ "type": "projects.list" }`
- **THEN** the system SHALL respond with `{ "projects": [...] }` containing all non-archived projects
- **AND** each project object SHALL include: `id`, `name`, `rootPath`, `color`, `pinned`, `reachable`, `lastOpenedAt`, `sessionCount`

#### Scenario: List with archived
- **WHEN** a client sends `{ "type": "projects.list", "params": { "includeArchived": true } }`
- **THEN** the response SHALL also include archived projects with `archived: true`

### Requirement: projects.create WebSocket method
The system SHALL handle `projects.create` WebSocket requests to register a new project.

#### Scenario: Create project with explicit name
- **WHEN** a client sends `{ "type": "projects.create", "params": { "rootPath": "/home/user/app", "name": "My App" } }`
- **THEN** the system SHALL create the project and respond with the full project object
- **AND** broadcast a `projects.changed` event

#### Scenario: Create project with auto name
- **WHEN** a client sends `{ "type": "projects.create", "params": { "rootPath": "/home/user/app" } }`
- **THEN** the system SHALL use `"app"` (last path component) as the name

#### Scenario: Create project for existing path
- **WHEN** a project for the given rootPath already exists
- **THEN** the system SHALL return the existing project instead of creating a duplicate
- **AND** NOT return an error

### Requirement: projects.update WebSocket method
The system SHALL handle `projects.update` WebSocket requests to modify project properties.

#### Scenario: Update project name
- **WHEN** a client sends `{ "type": "projects.update", "params": { "id": "abc123", "name": "New Name" } }`
- **THEN** the system SHALL update the project's name and broadcast `projects.changed`

#### Scenario: Toggle project pin
- **WHEN** a client sends `{ "type": "projects.update", "params": { "id": "abc123", "pinned": true } }`
- **THEN** the system SHALL set `pinned = 1` and broadcast `projects.changed`

#### Scenario: Archive project
- **WHEN** a client sends `{ "type": "projects.update", "params": { "id": "abc123", "archived": true } }`
- **THEN** the system SHALL set `archived = 1` and broadcast `projects.changed`

#### Scenario: Update project color
- **WHEN** a client sends `{ "type": "projects.update", "params": { "id": "abc123", "color": "#f97316" } }`
- **THEN** the system SHALL update the project's color and broadcast `projects.changed`

### Requirement: projects.delete WebSocket method
The system SHALL handle `projects.delete` WebSocket requests to remove a project from the registry.

#### Scenario: Delete project
- **WHEN** a client sends `{ "type": "projects.delete", "params": { "id": "abc123" } }`
- **THEN** the system SHALL delete the project record
- **AND** set `project_id = NULL` on all sessions that referenced this project
- **AND** broadcast `projects.changed`
- **AND** NOT delete the actual files on disk

#### Scenario: Delete non-existent project
- **WHEN** a client sends `projects.delete` with an ID that does not exist
- **THEN** the system SHALL respond with success (idempotent)

### Requirement: projects.detect WebSocket method
The system SHALL handle `projects.detect` WebSocket requests to detect project information from a given path.

#### Scenario: Detect project root
- **WHEN** a client sends `{ "type": "projects.detect", "params": { "path": "/home/user/app/src/main.rs" } }`
- **THEN** the system SHALL respond with `{ "rootPath": "/home/user/app", "name": "app", "hints": [...] }`
- **AND** `hints` SHALL contain project type indicators from `detect_project_hints`

#### Scenario: No project detected
- **WHEN** the given path has no detectable project root markers
- **THEN** the system SHALL respond with `{ "rootPath": "<given_path>", "name": "<basename>", "hints": [] }`

### Requirement: projects.changed event broadcast
The system SHALL broadcast a `projects.changed` event to all connected WebSocket clients whenever a project is created, updated, or deleted.

#### Scenario: Event format
- **WHEN** a project is modified
- **THEN** the system SHALL broadcast `{ "type": "event", "event": "projects.changed", "data": { "projectId": "<id>", "action": "created" | "updated" | "deleted" } }`
