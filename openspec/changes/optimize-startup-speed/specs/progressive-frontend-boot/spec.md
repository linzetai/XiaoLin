## ADDED Requirements

### Requirement: Three-phase frontend boot mode
The frontend gateway store SHALL support three modes: `shell`, `connecting`, and `ready`.

#### Scenario: Initial render before gateway ready
- **WHEN** the app starts and Gateway is not yet ready
- **THEN** the frontend SHALL enter `shell` mode and render the UI outer shell (sidebar, titlebar, chat skeleton)

#### Scenario: Transition to connecting
- **WHEN** the frontend receives GatewayInfo from IPC
- **THEN** the mode SHALL transition to `connecting` and begin WebSocket connection

#### Scenario: Transition to ready
- **WHEN** the WebSocket connection is established and backend data sync completes
- **THEN** the mode SHALL transition to `ready` and enable all interactive controls

### Requirement: Skeleton UI in shell mode
The AppLayout SHALL render a non-empty UI skeleton when in `shell` mode.

#### Scenario: Skeleton layout renders immediately
- **WHEN** mode is `shell`
- **THEN** the app SHALL display the sidebar frame, titlebar, and a placeholder chat area with skeleton loading indicators

#### Scenario: Cached session list for skeleton
- **WHEN** mode is `shell` and localStorage contains a cached session list from a previous run
- **THEN** the sidebar SHALL display cached session entries as disabled placeholders

### Requirement: Interaction controls disabled during boot
Interactive elements SHALL be disabled but visible when the frontend is not in `ready` mode.

#### Scenario: Input bar in shell/connecting mode
- **WHEN** mode is `shell` or `connecting`
- **THEN** the chat input bar SHALL be visible but disabled with a "连接中..." placeholder

#### Scenario: Navigation in shell mode
- **WHEN** mode is `shell`
- **THEN** sidebar navigation tabs SHALL be rendered but clicking them SHALL not trigger data fetches
