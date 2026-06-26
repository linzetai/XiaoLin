## Why

Recent production sessions show that XiaoLin can display Plan mode in the UI while the backend executes the turn in Agent mode, then silently ends after a runtime tool-loop force stop without producing a plan file. This is not only a Plan-mode bug: session identity, execution mode, turn start context, and terminal diagnosis are split across frontend stores, gateway memory, and runtime telemetry without one authoritative per-turn contract.

## What Changes

- Introduce an authoritative turn runtime contract for every chat turn. The chat request SHALL carry the intended runtime context, the gateway SHALL resolve the final session id before applying it, and `turn_start` SHALL echo the resolved context used by the runtime.
- Make `executionMode` a per-turn request input, not only a previous `set_mode` side effect. `set_mode` remains useful for explicit UI toggles and multi-window sync, but chat submission SHALL be self-contained.
- Make session id migration explicit. When a client sends a local/new chat id and the backend resolves a persisted session id, all runtime context SHALL be applied to the resolved session id before the turn starts.
- Extend turn terminal events with explicit end reason, diagnosis, severity, and user-visible failure guidance when the runtime ends abnormally.
- Add Plan-mode completion invariants. A Plan turn SHALL end in one of the valid Plan outcomes: plan approval pending, user clarification requested, plan artifact generated/updated, or an explicit Plan failure state.
- Align frontend state updates with backend-authoritative events. The frontend SHALL update chat mode, plan state, usage, and diagnostics from `turn_start`/`turn_end` rather than assuming local optimistic state is authoritative.
- Add regression coverage for new sessions, existing sessions, restored sessions, Plan mode, Goal mode, abnormal runtime termination, and id migration.

## Capabilities

### New Capabilities

- `turn-runtime-contract`: Defines the authoritative request/start/end contract for a chat turn, including resolved session id, execution mode, runtime context, terminal reason, and diagnostics.

### Modified Capabilities

- `ws-typed-turn`: WebSocket chat submission must include typed runtime context, especially execution mode, and apply it after session resolution before submitting `SessionOp::UserTurn`.
- `mode-attachments`: Plan/Agent mode attachments must be driven by the resolved backend execution mode echoed in `turn_start`, not by stale frontend-local mode state.
- `plan-approval-gate`: Plan-mode completion and approval state must be represented as explicit turn outcomes, including a failure state when no plan artifact or approval request is produced.

## Impact

- **Protocol / generated types**: `ChatParams`, frontend `ChatStreamParams`, generated TS protocol types, and WS event payloads gain typed runtime-context fields and terminal diagnosis fields.
- **Gateway**: WebSocket `chat` handling resolves the authoritative session id, applies requested execution mode to `SessionModeRegistry`, emits enriched `turn_start`, forwards enriched `turn_end`, and keeps legacy `set_mode` behavior for explicit toggles.
- **Agent runtime**: Turn summaries or agent steps expose terminal reason and diagnosis consistently, including `tool_loop` / no-progress force stop, budget/context/error endings, and Plan-specific invalid endings.
- **Frontend stores and stream handling**: `chat-meta-store`, `useMessageStreamChat`, `ComposerCore`, and Plan approval UI consume authoritative mode/session/result events and handle local-to-backend id migration without losing runtime context.
- **Session/quality storage**: Existing `turn_quality_summary` remains the historical quality source, but user-visible terminal diagnostics must also be carried on live stream events.
- **Tests**: Add Rust and Vitest coverage for protocol behavior plus UI stream regression tests for Plan mode first-turn new sessions and abnormal stop rendering.
