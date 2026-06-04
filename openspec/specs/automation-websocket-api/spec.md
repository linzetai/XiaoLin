## ADDED Requirements

### Requirement: automations.list WebSocket method
The system SHALL handle `automations.list` requests and return all cron jobs from CronJobStore.

#### Scenario: List all jobs
- **WHEN** a client sends `{ "type": "automations.list", "params": {} }`
- **THEN** the system SHALL call `CronJobStore::list()` (or equivalent)
- **AND** return `{ "jobs": [ CronJob, ... ] }` with fields: id, name, schedule, action, enabled, status, last_run, next_run, run_count, error_count, last_error, notify_channels, created_at

#### Scenario: List failure
- **WHEN** the database query fails
- **THEN** the system SHALL return an error response with a descriptive message

### Requirement: automations.create WebSocket method
The system SHALL handle `automations.create` to insert a new cron job.

#### Scenario: Create with required fields
- **WHEN** a client sends `{ "type": "automations.create", "params": { "name": "...", "schedule": "0 9 * * *", "action": { "type": "agent_chat", "agent_id": "...", "message": "..." }, "enabled": true, "notify_channels": [] } }`
- **THEN** the system SHALL validate the cron expression and action shape
- **AND** persist the job via `CronJobStore::upsert`
- **AND** return `{ "job": CronJob }` or `{ "jobId": "<id>" }`
- **AND** broadcast `automations.changed` with `action: "created"`

#### Scenario: Create with invalid schedule
- **WHEN** the schedule expression is not parseable by the scheduler
- **THEN** the system SHALL return an error and SHALL NOT persist the job

#### Scenario: Create Webhook action
- **WHEN** action type is `webhook` with a valid url
- **THEN** the system SHALL store a `JobAction::Webhook` variant and return the created job

### Requirement: automations.update WebSocket method
The system SHALL handle `automations.update` to modify an existing cron job.

#### Scenario: Update existing job
- **WHEN** a client sends `{ "type": "automations.update", "params": { "id": "<job_id>", "name": "...", "schedule": "...", "enabled": false, ... } }`
- **THEN** the system SHALL load the job by id, merge provided fields, and upsert
- **AND** return the updated `CronJob`
- **AND** broadcast `automations.changed` with `action: "updated"` and `jobId`

#### Scenario: Update unknown job
- **WHEN** the id does not exist
- **THEN** the system SHALL return an error: job not found

#### Scenario: Toggle enabled only
- **WHEN** the client sends only `{ "id": "<job_id>", "enabled": false }`
- **THEN** the system SHALL update enabled status without requiring other fields

### Requirement: automations.delete WebSocket method
The system SHALL handle `automations.delete` to remove a cron job.

#### Scenario: Delete existing job
- **WHEN** a client sends `{ "type": "automations.delete", "params": { "id": "<job_id>" } }`
- **THEN** the system SHALL call `CronJobStore::delete(id)`
- **AND** return `{ "ok": true }`
- **AND** broadcast `automations.changed` with `action: "deleted"` and `jobId`

#### Scenario: Delete unknown job
- **WHEN** the id does not exist
- **THEN** the system SHALL return an error: job not found

### Requirement: automations.runs WebSocket method
The system SHALL handle `automations.runs` to return execution history for a job.

#### Scenario: List runs with default limit
- **WHEN** a client sends `{ "type": "automations.runs", "params": { "job_id": "<job_id>" } }`
- **THEN** the system SHALL return the most recent runs (default limit 20) as `{ "runs": [ CronJobRun, ... ] }`

#### Scenario: List runs with custom limit
- **WHEN** a client sends `{ "type": "automations.runs", "params": { "job_id": "<job_id>", "limit": 50 } }`
- **THEN** the system SHALL return at most 50 runs ordered by started_at descending

#### Scenario: Runs for unknown job
- **WHEN** the job_id does not exist
- **THEN** the system MAY return an empty runs array or a not-found error (implementation SHALL document chosen behavior; prefer empty array if job was deleted)

### Requirement: automations.changed WebSocket event
The system SHALL broadcast `automations.changed` when automation data changes.

#### Scenario: Broadcast on CRUD
- **WHEN** a job is created, updated, or deleted via `automations.*` handlers or Agent cron_tool
- **THEN** the gateway SHALL broadcast `{ "type": "automations.changed", "data": { "action": "created"|"updated"|"deleted", "jobId": "<id>" } }` to connected clients

#### Scenario: Broadcast on run completion
- **WHEN** a cron job finishes execution and a CronJobRun record is written
- **THEN** the gateway MAY broadcast `{ "type": "automations.changed", "data": { "action": "run_completed", "jobId": "<id>" } }`

#### Scenario: Client receives event
- **WHEN** a connected client receives `automations.changed`
- **THEN** the client SHALL be able to refresh its local job list or run history without a full page reload

### Requirement: Protocol registration
The automations WS methods SHALL be registered in the gateway protocol dispatch.

#### Scenario: Route automations methods
- **WHEN** an incoming WS message type starts with `automations.`
- **THEN** the gateway SHALL dispatch to the automations handler module
- **AND** SHALL NOT require HTTP REST for the same operations

### Requirement: Backend cron event broadcast (prerequisite)
后端现有 `ws/cron.rs` 处理 `cron.list_jobs` 等方法，但**未广播** job 执行完成/失败事件。前端 `transport.ts` 已订阅 `cron.job.complete` 和 `cron.job.failed`，但后端从未发射。本 change 的 `automations.changed` 事件可替代或包含这些事件语义。

#### Scenario: Existing gap — cron events not emitted
- **WHEN** CronScheduler 执行一个 job 并写入 `cron_job_runs` 记录
- **THEN** 后端当前**不会**广播任何 WS 事件
- **AND** 实现 `automations.changed` 时 MUST 在 CronScheduler 执行路径中添加广播逻辑
- **AND** 可选：同时补充 `cron.job.complete` / `cron.job.failed` 事件以保持向后兼容

#### Scenario: Legacy cron.* method coexistence
- **WHEN** 现有 `cron.list_jobs` / `cron.upsert_job` 等 WS 方法仍存在
- **THEN** `automations.*` 方法与 `cron.*` 方法 MAY 共存（`automations.*` 为新前端 UI 专用，`cron.*` 供 Agent tool 调用保持兼容）
- **AND** 两组方法操作同一 `CronJobStore`
