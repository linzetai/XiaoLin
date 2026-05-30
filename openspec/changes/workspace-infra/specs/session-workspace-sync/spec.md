## MODIFIED Requirements

### Requirement: WS sessions.set_work_dir implementation
The backend SHALL implement the `sessions.set_work_dir` WebSocket method to persist the working directory for a session.

#### Scenario: Set work_dir via WebSocket
- **WHEN** a client sends `sessions.set_work_dir` with `{ session_id, work_dir }`
- **THEN** the backend calls `SessionStore::update_work_dir` and broadcasts `sessions.changed`

### Requirement: WS sessions.list returns complete fields
The WebSocket `sessions.list` and `sessions.get` responses SHALL include `workDir` and `source` fields.

#### Scenario: List sessions with workDir
- **WHEN** a client sends `sessions.list`
- **THEN** each session in the response SHALL include `work_dir` (nullable) and `source` fields

#### Scenario: Get session with workDir
- **WHEN** a client sends `sessions.get`
- **THEN** the response SHALL include `work_dir` and `source`

### Requirement: Smart title generation broadcasts change
- **WHEN** `generate_smart_title` successfully updates a session title
- **THEN** the system SHALL emit a `sessions.changed` event with the session ID

### Requirement: Frontend workDir sync on session list
- **WHEN** the frontend syncs sessions from the backend (via WS `sessions.list`)
- **THEN** each session's `workDir` SHALL be restored from the backend response
- **AND** the `workDir` SHALL NOT be lost on page refresh or reconnect
