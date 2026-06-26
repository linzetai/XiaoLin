## ADDED Requirements

### Requirement: Mode attachments use resolved backend mode
Plan and Agent mode attachment injection SHALL be based on the execution mode applied to the resolved backend session id for the current turn.

#### Scenario: Plan attachment after local id migration
- **WHEN** a first-turn chat request uses local id `new-*`
- **AND** the request includes `executionMode = "plan"`
- **AND** setup resolves backend session id `S`
- **THEN** the runtime MUST inject Plan mode attachments for that turn using `SessionModeRegistry[S]`

#### Scenario: No Plan attachment when Goal mode overrides Plan
- **WHEN** a chat request includes `goalMode = true`
- **AND** the frontend previously displayed Plan mode
- **THEN** the runtime MUST NOT inject Plan mode attachments for that turn

### Requirement: Mode counters track authoritative transitions
Plan and Agent mode turn counters SHALL advance according to the effective backend mode for the resolved session id, not according to client-local ids or optimistic UI state.

#### Scenario: Plan turn counter increments on resolved session
- **WHEN** a Plan request with local id `new-*` resolves to session `S`
- **THEN** the Plan turn counter for session `S` MUST increment according to existing attachment throttling rules

#### Scenario: Stale local mode does not affect backend counters
- **WHEN** frontend local state says Plan for a chat id that is not the resolved backend session id
- **THEN** runtime mode counters MUST ignore that stale local id
