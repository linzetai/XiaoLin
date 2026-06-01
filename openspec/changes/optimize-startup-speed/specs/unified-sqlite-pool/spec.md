## ADDED Requirements

### Requirement: Single SQLite database file
The system SHALL use a single SQLite database file (`fastclaw.db`) for all persistent stores instead of separate files for sessions, evolution, and cron.

#### Scenario: Fresh installation
- **WHEN** FastClaw starts for the first time with no existing database
- **THEN** a single `fastclaw.db` SHALL be created containing all tables (sessions, event_log, feedback, trajectory, skills, cron_jobs, notifications, prompt_distiller)

#### Scenario: Shared connection pool
- **WHEN** StateBuilder initializes Phase 1
- **THEN** a single `SqlitePool` SHALL be created and passed to all subsequent phases for reuse

### Requirement: Automatic migration from legacy databases
The system SHALL detect and migrate data from legacy split databases on upgrade.

#### Scenario: Legacy databases detected
- **WHEN** FastClaw starts and finds existing `sessions.db`, `evolution.db`, or `cron.db` files
- **THEN** the system SHALL migrate all tables and data into `fastclaw.db` within a transaction

#### Scenario: Migration success
- **WHEN** migration completes successfully
- **THEN** the legacy database files SHALL be renamed to `*.db.bak` (not deleted)

#### Scenario: Migration failure
- **WHEN** migration fails due to any error
- **THEN** the transaction SHALL be rolled back, legacy files SHALL remain untouched, and the system SHALL fall back to using legacy split databases for this session

### Requirement: Store constructors accept shared pool
All store types (SessionStore, EventLog, FeedbackStore, TrajectoryStore, SkillStore, CronJobStore, NotificationStore, PromptDistiller) SHALL accept an existing `SqlitePool` as a constructor parameter.

#### Scenario: SessionStore with shared pool
- **WHEN** SessionStore is initialized with a shared pool
- **THEN** it SHALL use the provided pool instead of opening a new database connection

#### Scenario: CronJobStore with shared pool
- **WHEN** CronJobStore is initialized with a shared pool
- **THEN** it SHALL create its tables in the shared database and use the provided pool
