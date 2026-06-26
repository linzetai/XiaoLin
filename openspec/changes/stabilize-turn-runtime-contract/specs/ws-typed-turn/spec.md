## ADDED Requirements

### Requirement: WebSocket UserTurn includes typed execution mode context
WebSocket chat submission SHALL carry the request execution mode through typed chat parameters and into the setup/runtime boundary without relying only on a previous `set_mode` RPC.

#### Scenario: ChatParams accepts execution mode
- **WHEN** the gateway deserializes WebSocket `chat` params containing `executionMode = "plan"`
- **THEN** typed `ChatParams` MUST preserve that value as execution mode context

#### Scenario: TypedTurnData sees resolved mode
- **WHEN** the gateway submits `SessionOp::UserTurn` after resolving setup
- **THEN** the typed turn data or associated runtime services MUST reflect the effective execution mode for the resolved session id

### Requirement: Request mode wins for the submitted turn
When a chat request includes `executionMode`, that value SHALL be applied to the resolved backend session for the submitted turn, unless a higher-precedence mode such as Goal mode explicitly overrides it.

#### Scenario: Previous registry mode differs from request
- **WHEN** `SessionModeRegistry[S]` is Agent
- **AND** the chat request for session `S` includes `executionMode = "plan"`
- **THEN** the submitted turn MUST run in Plan mode

#### Scenario: Goal mode overrides request mode
- **WHEN** the chat request includes `executionMode = "plan"`
- **AND** the chat request includes `goalMode = true`
- **THEN** the submitted turn MUST run in Agent mode
