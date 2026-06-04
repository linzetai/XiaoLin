## ADDED Requirements

### Requirement: Automation list view
The system SHALL display all cron automations in a table within the AutomationPanel overlay.

#### Scenario: List displays job metadata
- **WHEN** the AutomationPanel opens and jobs exist in CronJobStore
- **THEN** the panel SHALL render a table with columns: name, schedule (human-readable or expression), status, last run time, and row actions
- **AND** status SHALL reflect `JobStatus` (idle, running, failed, disabled) with distinct visual styling

#### Scenario: Last run display
- **WHEN** a job has `last_run` set
- **THEN** the last run column SHALL show a formatted relative or absolute timestamp
- **WHEN** a job has never run
- **THEN** the last run column SHALL show "Never" or equivalent empty label

#### Scenario: Row actions
- **WHEN** a job row is displayed
- **THEN** the row SHALL provide actions: enable/disable toggle, edit, delete, and view history
- **WHEN** the user toggles enable off
- **THEN** the system SHALL call `automations.update` with `enabled: false` and refresh the list

### Requirement: Empty state
The AutomationPanel SHALL show a friendly empty state when no automations exist.

#### Scenario: No automations
- **WHEN** `automations.list` returns an empty array
- **THEN** the panel SHALL display an empty state message explaining that no automations are configured
- **AND** SHALL provide a primary "Create automation" button that opens the create form

### Requirement: Create automation form
The AutomationPanel SHALL provide a form to create new automations.

#### Scenario: Open create form
- **WHEN** the user clicks "Create automation" from the list header or empty state
- **THEN** the panel SHALL open a create form (modal or inline section) with fields: name, schedule, action type, action-specific fields, enabled, notify_channels

#### Scenario: Submit valid create
- **WHEN** the user fills required fields with valid values and submits
- **THEN** the system SHALL call `automations.create` with the form payload
- **AND** on success close the form and show the new job in the list

#### Scenario: Submit invalid schedule
- **WHEN** the user submits an invalid cron expression
- **THEN** the form SHALL display a validation error and SHALL NOT call the API

#### Scenario: Cancel create
- **WHEN** the user cancels the create form
- **THEN** the form SHALL close without persisting changes

### Requirement: Edit automation form
The AutomationPanel SHALL allow editing existing automations.

#### Scenario: Open edit form
- **WHEN** the user clicks edit on a job row
- **THEN** the panel SHALL open an edit form pre-filled with the job's current name, schedule, action, enabled, and notify_channels

#### Scenario: Submit valid update
- **WHEN** the user modifies fields and submits
- **THEN** the system SHALL call `automations.update` with `id` and changed fields
- **AND** on success close the form and reflect updates in the list

### Requirement: Delete confirmation
The AutomationPanel SHALL require confirmation before deleting an automation.

#### Scenario: Confirm delete
- **WHEN** the user clicks delete on a job row
- **THEN** the panel SHALL show a confirmation dialog naming the job
- **WHEN** the user confirms
- **THEN** the system SHALL call `automations.delete` with the job id
- **AND** remove the job from the list on success

#### Scenario: Cancel delete
- **WHEN** the user dismisses the confirmation dialog
- **THEN** no API call SHALL be made and the job SHALL remain in the list

### Requirement: Execution history panel
The AutomationPanel SHALL display per-job execution history.

#### Scenario: Open history for a job
- **WHEN** the user clicks view history on a job row
- **THEN** the panel SHALL call `automations.runs` with `job_id` and a default limit
- **AND** display a list of runs with started_at, ended_at, status, and truncated output/error

#### Scenario: History empty
- **WHEN** a job has no recorded runs
- **THEN** the history section SHALL show "No runs yet"

#### Scenario: Close history
- **WHEN** the user closes the history view or selects another job
- **THEN** the history section SHALL collapse or switch context without closing the entire AutomationPanel

### Requirement: Panel overlay behavior
The AutomationPanel SHALL render as an overlay triggered from the sidebar, not a full-page route.

#### Scenario: Open from sidebar
- **WHEN** the user clicks Automations in AppSidebar
- **THEN** the AutomationPanel overlay SHALL open above the main chat layout
- **AND** the chat session underneath SHALL remain unchanged

#### Scenario: Close overlay
- **WHEN** the user clicks the panel close button or presses Escape
- **THEN** the overlay SHALL close and return focus to the main layout

### Requirement: Cron schedule helper in form
The create/edit form SHALL integrate the cron expression helper (presets + custom).

#### Scenario: Select preset schedule
- **WHEN** the user selects a preset (every hour, daily, weekly)
- **THEN** the schedule field SHALL be populated with the corresponding cron expression
- **AND** a human-readable summary SHALL be shown below the field

#### Scenario: Custom schedule
- **WHEN** the user selects "Custom" and enters a cron expression
- **THEN** the form SHALL validate the expression before submit
- **AND** update the human-readable summary when the expression is valid
