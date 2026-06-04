## ADDED Requirements

### Requirement: Project store state shape
The frontend SHALL maintain a Zustand store `useProjectStore` with state: `projects: Record<string, Project>`, `activeProjectId: string | null`, where `Project` includes: `id`, `name`, `rootPath`, `color`, `pinned`, `archived`, `reachable`, `lastOpenedAt`, `sessionCount`.

#### Scenario: Initial state
- **WHEN** the app starts
- **THEN** `projects` SHALL be an empty record and `activeProjectId` SHALL be null
- **AND** the store SHALL immediately request `projects.list` from the backend

### Requirement: Sync projects from backend
The store SHALL synchronize project data from the backend via WebSocket.

#### Scenario: Initial sync
- **WHEN** the WebSocket connection is established
- **THEN** the store SHALL send `projects.list` and populate the `projects` record with the response

#### Scenario: Live sync on projects.changed event
- **WHEN** the store receives a `projects.changed` event
- **THEN** the store SHALL re-fetch the full project list via `projects.list`
- **AND** update the `projects` record

### Requirement: Active project tracking
The store SHALL track which project is currently active based on the active session's project association.

#### Scenario: Active project follows active session
- **WHEN** the active session has a `projectId`
- **THEN** `activeProjectId` SHALL be set to that session's `projectId`

#### Scenario: No active project for unassociated session
- **WHEN** the active session has no `projectId`
- **THEN** `activeProjectId` SHALL be null

### Requirement: Project CRUD actions
The store SHALL provide actions for creating, updating, and deleting projects via WebSocket calls.

#### Scenario: Create project
- **WHEN** `createProject(rootPath, name?)` is called
- **THEN** the store SHALL send `projects.create` to the backend
- **AND** update the local state when the response arrives

#### Scenario: Update project
- **WHEN** `updateProject(id, patch)` is called with fields like `name`, `color`, `pinned`
- **THEN** the store SHALL send `projects.update` to the backend
- **AND** optimistically update the local state

#### Scenario: Delete project
- **WHEN** `deleteProject(id)` is called
- **THEN** the store SHALL send `projects.delete` to the backend
- **AND** remove the project from the local state

### Requirement: ChatMeta projectId field
The `ChatMeta` type SHALL include a `projectId: string | null` field.

#### Scenario: Session with project
- **WHEN** a backend session includes `projectId`
- **THEN** the `ChatMeta` SHALL store the `projectId`

#### Scenario: Session without project
- **WHEN** a backend session has no `projectId` (or it is null)
- **THEN** the `ChatMeta.projectId` SHALL be null
