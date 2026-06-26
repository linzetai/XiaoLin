## ADDED Requirements

### Requirement: Plan turn terminal outcome is explicit
Every turn that starts in Plan mode SHALL end with an explicit Plan outcome in the terminal event.

#### Scenario: Plan approval pending outcome
- **WHEN** `exit_plan_mode` returns metadata with `approval_pending = true`
- **THEN** `turn_end` MUST include a Plan outcome equivalent to `plan_approval_pending`

#### Scenario: Plan clarification outcome
- **WHEN** a Plan-mode turn ends by asking the user a clarification question
- **THEN** the stream MUST expose a Plan outcome equivalent to `needs_input`

#### Scenario: Plan artifact updated outcome
- **WHEN** a Plan-mode turn updates or creates the plan artifact and then ends normally
- **THEN** `turn_end` MUST expose a Plan outcome equivalent to `plan_artifact_updated`

### Requirement: Plan mode failure is not silent
A Plan-mode turn that ends without approval pending, clarification, or a plan artifact update SHALL be reported as a Plan failure rather than an ordinary successful turn.

#### Scenario: Tool loop stops Plan before plan creation
- **WHEN** a Plan-mode turn is stopped by runtime tool-loop protection before any plan artifact is produced
- **THEN** `turn_end` MUST include a Plan outcome equivalent to `plan_failed`
- **THEN** the frontend MUST render that no plan file was produced

#### Scenario: Natural end without plan artifact
- **WHEN** a Plan-mode turn reaches normal completion without plan artifact update, approval pending, or clarification
- **THEN** `turn_end` MUST include a Plan outcome equivalent to `plan_failed`

### Requirement: Plan approval actions use resolved session id
Frontend approval and continue-planning actions SHALL target the backend session id announced by `turn_start` or `turn_end`, not a stale local chat id.

#### Scenario: Approve after first-turn id migration
- **WHEN** a Plan approval card is shown for a session that began as local id `new-*`
- **AND** the backend resolved the session to id `S`
- **THEN** the approval RPC MUST use `sessionId = "S"`
