## ADDED Requirements

### Requirement: Watch channel for gateway readiness notification
The system SHALL use a `tokio::sync::watch` channel to notify consumers when the embedded Gateway becomes ready, replacing all polling-based readiness checks.

#### Scenario: Gateway starts successfully
- **WHEN** the embedded Gateway finishes initialization and axum begins accepting connections
- **THEN** the watch channel SHALL send `GatewayStartupState::Running { info }` with zero delay

#### Scenario: Gateway startup fails
- **WHEN** the embedded Gateway encounters a fatal error during initialization
- **THEN** the watch channel SHALL send `GatewayStartupState::Failed { error }` immediately

### Requirement: IPC command uses watch channel
The `get_gateway_info` Tauri IPC command SHALL await the watch channel instead of polling a Mutex lock.

#### Scenario: Frontend requests gateway info before ready
- **WHEN** the frontend calls `get_gateway_info` while Gateway is still starting
- **THEN** the command SHALL block on `watch_rx.changed().await` until the state transitions to Running or Failed, with a 30-second timeout

#### Scenario: Frontend requests gateway info after ready
- **WHEN** the frontend calls `get_gateway_info` after Gateway is already running
- **THEN** the command SHALL return the GatewayInfo immediately from the current watch value

### Requirement: No HTTP health probe during startup
The `probe_gateway` HTTP polling loop in `embedded.rs` SHALL be removed entirely.

#### Scenario: Gateway startup without polling
- **WHEN** `GatewayProcess::start` is called
- **THEN** it SHALL await readiness via the watch channel, not via HTTP GET /health polling
