## MODIFIED Requirements

### Requirement: ContentDelta 携带预序列化字节
The live streaming delta path SHALL route every `ContentDelta` into the canonical timeline reducer as timeline-compatible events while preserving the existing `raw_bytes` transport optimization.

#### Scenario: ContentDelta feeds the timeline reducer
- **WHEN** the runtime emits a `ContentDelta` event during a live turn
- **THEN** the gateway SHALL derive or forward a timeline-compatible event into the same reducer used for replay
- **AND** the frontend SHALL NOT accumulate the delta into a live-only segment array with separate semantics

#### Scenario: raw_bytes fast path is preserved for transport only
- **WHEN** `ContentDelta` carries `raw_bytes`
- **THEN** the gateway SHALL still use `raw_bytes` directly for SSE formatting
- **AND** the raw_bytes optimization SHALL NOT imply a separate live-only transcript model
- **AND** the same delta SHALL also be representable as a timeline event for replay

### Requirement: HTTP UserTurn 跳过冗余 messages 序列化
When `typed_data` is set, `SessionOp::UserTurn` SHALL keep using an empty placeholder for the `messages` field to avoid serializing the full message list. This optimization is independent of the canonical timeline: user message creation is also recorded as a `user_message_created` timeline event for UI replay.

#### Scenario: typed_data present keeps messages empty
- **WHEN** HTTP `handle_stream` builds `SessionOp::UserTurn` and `typed_data` is `Some`
- **THEN** `messages` MUST remain an empty `Value::Array(vec![])`
- **AND** a `user_message_created` timeline event SHALL be emitted for the canonical timeline

#### Scenario: session actor still extracts from typed_data
- **WHEN** the session actor receives `UserTurn` with `typed_data` set
- **THEN** the actor SHALL extract messages from `typed_data` and ignore the `messages` field
- **AND** the timeline event emission SHALL NOT depend on the `messages` field being populated
