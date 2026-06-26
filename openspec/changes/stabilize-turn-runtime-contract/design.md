## Context

XiaoLin currently treats a chat turn as the composition of several loosely coupled states:

- Frontend `ChatMeta.executionMode` drives the composer and visual mode.
- Gateway `SessionModeRegistry` stores execution mode by backend session id.
- Runtime `ExecutionModeState` drives plan attachments, tool filtering, and mode-specific behavior.
- Runtime quality diagnosis is persisted after the fact in `turn_quality_summary`.
- Live UI stream events receive `turn_start`, deltas, tools, and `turn_end`, but not the full terminal diagnosis.

This split works when the frontend already has a stable backend session id and no abnormal runtime ending occurs. It fails for new sessions because the frontend can optimistically set Plan mode on a local `new-*` id, while the gateway later resolves a different persisted session id. It also fails operationally because a runtime force stop such as `tool_loop` is visible in persisted quality data but not as a user-facing live event.

The design goal is to make each chat turn self-describing and authoritative: the request states the intended runtime context, the gateway resolves the final session id, the runtime starts with that resolved context, and terminal events explain how the turn ended.

## Goals / Non-Goals

**Goals:**

- Make WebSocket chat submission self-contained for execution mode and future runtime context.
- Apply mode changes to the resolved backend session id before the runtime turn starts.
- Make `turn_start` the frontend's authoritative source for resolved session id and effective execution mode.
- Make `turn_end` expose terminal reason and diagnosis so abnormal endings are visible without querying quality tables.
- Define Plan-mode valid endings so a turn that neither creates a plan nor requests approval/clarification is reported as failure.
- Preserve compatibility for existing explicit `set_mode` UI interactions and multi-window broadcasts.
- Cover new-session, restored-session, id-migration, Plan-mode, Goal-mode, and abnormal-stop regressions.

**Non-Goals:**

- Do not replace the session actor or `TypedTurnData` architecture.
- Do not persist execution mode as a durable session preference unless a separate product decision requires it.
- Do not remove `set_mode`; it remains useful for UI toggles outside an active turn.
- Do not expose private reasoning content as part of terminal diagnostics.
- Do not alter tool-loop detection thresholds as part of this change, except for Plan-specific terminal classification.

## Decisions

### D1: Treat chat submission as the authoritative turn-context boundary

The frontend SHALL include `executionMode` in `ChatStreamParams`, and the protocol `ChatParams` SHALL expose a typed optional `execution_mode` field with `serde` alias `executionMode`.

The gateway SHALL resolve the final session id via existing setup logic, then apply the requested execution mode to that resolved id before submitting `SessionOp::UserTurn`. If the field is absent, the gateway uses the current registry mode for existing sessions and Agent mode for unknown sessions.

Alternative considered: keep relying on `set_mode` before `chat`. This preserves current shape but leaves a race between mode-setting and session-id resolution. It also cannot make a single chat request replayable or testable in isolation.

### D2: `turn_start` echoes the effective runtime context

`turn_start` SHALL include:

- `session_id` / `sessionId`: resolved backend session id.
- `executionMode`: effective mode used for this turn.
- `requestedExecutionMode`: when provided by the client.
- `modeSource`: `request`, `registry`, or `default`.
- existing model/agent/resolve metadata.

The frontend SHALL update chat id and chat execution mode from this event. Local optimistic mode remains allowed for instant UI feedback, but it is not the final source of truth.

Alternative considered: only send `mode_change`. This misses cases where the final mode equals an existing backend mode but differs from stale frontend state, and it does not carry the session-id migration point.

### D3: Keep `set_mode`, but make it non-authoritative for active turns

`set_mode` continues to update `SessionModeRegistry` and broadcast `mode_change` for explicit toggles. During an active `chat` request, the request context wins for that turn because it is applied after session resolution.

If `goalMode=true`, the gateway continues to force Agent mode for the turn, but it SHALL expose this in `turn_start` as the effective execution mode and mode source. The frontend SHALL not remain visually in Plan mode during a Goal turn.

Alternative considered: make execution mode only a persisted session property. That makes toggles simpler but adds persistence semantics that are not currently required and still leaves first-turn local id ambiguity.

### D4: Terminal events carry user-visible diagnosis

Runtime already computes quality diagnosis such as `tool_loop`, context/budget conditions, and repeated tool force-stop counts. The live `turn_end` event SHALL carry a compact terminal envelope:

- `endReason`: stable machine-readable reason, such as `completed`, `plan_approval_pending`, `tool_loop`, `context_limit`, `budget_exceeded`, `cancelled`, or `error`.
- `diagnosisCode`: quality diagnosis when available.
- `severity`: `info`, `warning`, or `error`.
- `userMessage`: localized or localizable guidance suitable for UI display.
- optional `evidence`: bounded counters such as iterations, tool call count, repeated force stops, and no-progress count.

The quality table remains the historical record. The stream event is the live UX contract.

Alternative considered: have the frontend query `turn_quality_summary` after every turn. That adds latency, races with persistence, and does not help detached streams or transient failures.

### D5: Plan mode has explicit valid terminal outcomes

A Plan-mode turn is valid if it ends with one of these outcomes:

- `plan_approval_pending`: `exit_plan_mode` produced approval metadata.
- `needs_input`: the assistant asked a clarification question.
- `plan_artifact_updated`: a plan file or plan update event was produced and the assistant ended normally.
- `plan_failed`: the runtime ended abnormally or ended normally without a plan artifact, approval request, or clarification.

`plan_failed` SHALL be represented in `turn_end` so the UI can show that no plan was produced. This is not a substitute for better prompting; it is the invariant that prevents silent failure.

Alternative considered: only improve the Plan prompt to tell the model to write the plan. Prompting helps but does not guarantee protocol correctness.

### D6: Migrate frontend state through backend-authoritative events

`updateChatBackendId` SHALL preserve local metadata during id migration, including execution mode and plan state, but the stream handler SHALL prefer `turn_start.executionMode` as the effective mode. This gives two layers of safety:

1. Local migration does not drop state.
2. Backend echo corrects stale or cross-window state.

The frontend SHALL use the resolved session id for usage, plan meta hydration, approval actions, and subsequent messages after receiving `turn_start`.

Alternative considered: only fix `updateChatBackendId`. This addresses the observed first-turn Plan bug but leaves request-level replayability and backend/frontend divergence unsolved.

## Risks / Trade-offs

- **Protocol field drift** → Generate/update protocol TS types and add compile-time tests for `executionMode` serialization.
- **Double mode updates causing UI flicker** → Treat local mode changes as optimistic and reconcile on `turn_start`; avoid adding duplicate brief messages when the authoritative event confirms the same mode.
- **Backward compatibility with older clients** → Make `executionMode` optional. Missing field preserves current registry/default behavior.
- **Goal mode conflict with Plan mode** → Define precedence explicitly: `goalMode=true` forces effective Agent mode for that turn and is echoed in `turn_start`.
- **Overloading `turn_end` with too much data** → Keep diagnosis evidence bounded and machine-readable; detailed quality evidence remains in `turn_quality_summary`.
- **Plan artifact detection false negatives** → Base Plan outcomes on existing plan-file/update/approval events observed during the turn plus plan store state at end; add tests for each path.

## Migration Plan

1. Add protocol fields and frontend transport typing without changing behavior.
2. Apply requested execution mode after session resolution in the gateway and echo effective context in `turn_start`.
3. Update frontend stream handling to migrate chat id and mode from `turn_start`.
4. Add terminal diagnosis fields to runtime/gateway `turn_end` forwarding.
5. Add Plan-mode outcome classification and UI rendering for `plan_failed`.
6. Add regression tests for first-turn Plan mode with local id migration, existing session Plan mode, Goal-mode precedence, and `tool_loop` abnormal stop.
7. Run Rust and frontend test suites and archive quality evidence in the change tasks.

Rollback strategy: because new request/event fields are optional additive fields, rollback can remove frontend consumption first, then gateway emission. Existing `set_mode` and default registry behavior remain as compatibility fallback.

## Open Questions

- Should execution mode become durable session metadata in a future change, or remain an in-memory/runtime preference?
- Should `userMessage` be produced by backend as localized text, or should backend send stable reason codes and let frontend i18n render text?
- Should Plan-mode `plan_artifact_updated` require a persisted plan file, or is an in-memory `plan_update` sufficient for some future workflows?
