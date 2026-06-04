## MODIFIED Requirements

### Requirement: WS sessions.set_work_dir implementation
The backend SHALL implement the `sessions.set_work_dir` WebSocket method to persist the working directory for a session, and additionally find or create the associated project.

#### Scenario: Set work_dir via WebSocket
- **WHEN** a client sends `sessions.set_work_dir` with `{ session_id, work_dir }`
- **THEN** the backend SHALL call `SessionStore::update_work_dir`
- **AND** call `find_or_create_project(work_dir)` to obtain the project ID
- **AND** update the session's `project_id` to match
- **AND** broadcast `sessions.changed`

#### Scenario: Clear work_dir via WebSocket
- **WHEN** a client sends `sessions.set_work_dir` with `{ session_id, work_dir: null }`
- **THEN** the backend SHALL set `work_dir = NULL` and `project_id = NULL`
- **AND** broadcast `sessions.changed`

### Requirement: WS sessions.list returns complete fields
The WebSocket `sessions.list` and `sessions.get` responses SHALL include `workDir`, `source`, and `projectId` fields.

#### Scenario: List sessions with projectId
- **WHEN** a client sends `sessions.list`
- **THEN** each session in the response SHALL include `work_dir` (nullable), `source`, and `project_id` (nullable) fields

#### Scenario: Get session with projectId
- **WHEN** a client sends `sessions.get`
- **THEN** the response SHALL include `work_dir`, `source`, and `project_id`

### Requirement: WS sessions.new associates project
When creating a new session, the backend SHALL auto-detect and associate a project.

#### Scenario: New session with workspace-detected work_dir
- **WHEN** a client sends `sessions.new` and the session is created with a `work_dir`
- **THEN** the backend SHALL call `find_or_create_project(work_dir)`
- **AND** set the session's `project_id` in the response

### Requirement: Smart title generation broadcasts change
- **WHEN** `generate_smart_title` successfully updates a session title
- **THEN** the system SHALL emit a `sessions.changed` event with the session ID

### Requirement: Frontend workDir sync on session list
- **WHEN** the frontend syncs sessions from the backend (via WS `sessions.list`)
- **THEN** each session's `workDir` and `projectId` SHALL be restored from the backend response
- **AND** the `workDir` and `projectId` SHALL NOT be lost on page refresh or reconnect
