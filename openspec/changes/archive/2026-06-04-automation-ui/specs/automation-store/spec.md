## ADDED Requirements

### Requirement: useAutomationStore Zustand store
The frontend SHALL provide `useAutomationStore` to manage automation UI state.

#### Scenario: Store initial state
- **WHEN** the store is first accessed
- **THEN** it SHALL have `jobs: []`, `loading: false`, `error: null`, `selectedJobId: null`, and `runs: []` (or runs keyed by job id)

#### Scenario: Load jobs on panel open
- **WHEN** the AutomationPanel mounts or opens
- **THEN** the store SHALL set `loading: true`, call `automations.list`, and on success set `jobs` and `loading: false`

### Requirement: CRUD actions in store
The store SHALL expose actions that call the automations WS API.

#### Scenario: createJob action
- **WHEN** `createJob(payload)` is called with valid form data
- **THEN** the store SHALL call `automations.create`
- **AND** on success append or replace the job in `jobs`

#### Scenario: updateJob action
- **WHEN** `updateJob(id, partial)` is called
- **THEN** the store SHALL call `automations.update`
- **AND** on success update the matching entry in `jobs`

#### Scenario: deleteJob action
- **WHEN** `deleteJob(id)` is called
- **THEN** the store SHALL call `automations.delete`
- **AND** on success remove the job from `jobs`
- **AND** clear `selectedJobId` if it matched the deleted id

#### Scenario: fetchRuns action
- **WHEN** `fetchRuns(jobId, limit?)` is called
- **THEN** the store SHALL call `automations.runs`
- **AND** store the result in `runs` or `runsByJobId[jobId]`

### Requirement: Loading and error state
The store SHALL surface async operation status to the UI.

#### Scenario: Loading during list fetch
- **WHEN** `loadJobs()` is in flight
- **THEN** `loading` SHALL be true
- **AND** the AutomationPanel SHALL show a loading indicator

#### Scenario: Error on API failure
- **WHEN** any automations WS call returns an error
- **THEN** the store SHALL set `error` with a user-visible message
- **AND** SHALL set `loading` to false

#### Scenario: Clear error on retry
- **WHEN** a subsequent successful call completes
- **THEN** the store SHALL clear `error`

### Requirement: WS event handling for automations.changed
The store SHALL subscribe to `automations.changed` and keep local state in sync.

#### Scenario: Handle created or updated
- **WHEN** the client receives `automations.changed` with `action` of `created` or `updated`
- **THEN** the store SHALL call `loadJobs()` or patch the affected job in `jobs` by `jobId`

#### Scenario: Handle deleted
- **WHEN** the client receives `automations.changed` with `action: "deleted"`
- **THEN** the store SHALL remove the job with matching `jobId` from `jobs`

#### Scenario: Handle run_completed
- **WHEN** the client receives `automations.changed` with `action: "run_completed"`
- **THEN** if `selectedJobId` equals `jobId`, the store SHALL call `fetchRuns(jobId)`
- **AND** SHALL refresh last_run/status on the job in `jobs` (via reload or patch)

#### Scenario: Subscribe on app connect
- **WHEN** the WebSocket connection is established
- **THEN** the transport layer SHALL register a listener for `automations.changed` that forwards to the store

### Requirement: Selected job for detail view
The store SHALL track which job is selected for history and edit context.

#### Scenario: Select job for history
- **WHEN** the user opens execution history for a job
- **THEN** the store SHALL set `selectedJobId` to that job's id
- **AND** call `fetchRuns(jobId)`

#### Scenario: Clear selection
- **WHEN** the user closes the history/detail section
- **THEN** the store MAY clear `selectedJobId` or retain it until another job is selected

#### Scenario: getSelectedJob selector
- **WHEN** UI needs the full job object for the selected id
- **THEN** the store SHALL provide a derived value or selector returning the job from `jobs` by `selectedJobId`

### Requirement: Panel open state
The store SHALL coordinate overlay visibility with the sidebar.

#### Scenario: Open panel
- **WHEN** the user clicks Automations in AppSidebar
- **THEN** the store SHALL set `panelOpen: true` and trigger `loadJobs()`

#### Scenario: Close panel
- **WHEN** the user closes the AutomationPanel
- **THEN** the store SHALL set `panelOpen: false` without clearing `jobs` (cache for next open)
