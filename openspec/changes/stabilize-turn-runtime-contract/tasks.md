## 1. Protocol And Types

- [x] 1.1 Add optional typed `execution_mode` / `executionMode` to Rust `ChatParams` and regenerate protocol TypeScript types.
- [x] 1.2 Add `executionMode`, `requestedExecutionMode`, and `modeSource` fields to frontend `ChatStreamParams` and WebSocket chat payload construction.
- [x] 1.3 Define a stable terminal diagnosis payload for live `turn_end` events, including `endReason`, `diagnosisCode`, `severity`, bounded `evidence`, and optional user-visible message key/text.
- [x] 1.4 Add type tests or serialization tests proving `executionMode = "plan"` round-trips through WS chat params.

## 2. Gateway Turn Context

- [x] 2.1 In WebSocket `spawn_chat`, parse request execution mode before setup but defer applying it until after setup resolves the authoritative session id.
- [x] 2.2 Apply requested execution mode to `SessionModeRegistry[resolved_session_id]` before submitting `SessionOp::UserTurn`.
- [x] 2.3 Preserve backward compatibility: if request mode is absent, use existing registry mode for known sessions and Agent for unknown sessions.
- [x] 2.4 Implement Goal-mode precedence so `goalMode=true` forces effective Agent mode for the turn and records that override in the `turn_start` context.
- [x] 2.5 Enrich `turn_start` with resolved session id, effective execution mode, requested execution mode, and mode source.
- [x] 2.6 Keep `set_mode` behavior for explicit toggles and broadcasts, but ensure active chat submission request context wins for that submitted turn.

## 3. Runtime Terminal Diagnosis

- [x] 3.1 Expose runtime terminal reason and quality diagnosis from the runtime path that emits `TurnEnd`.
- [x] 3.2 Map tool-loop/no-progress force stop to live `turn_end` diagnosis instead of only persisting it in `turn_quality_summary`.
- [x] 3.3 Include bounded evidence counters for abnormal endings, such as iterations, tool calls, repeated force stops, and no-progress count.
- [x] 3.4 Ensure normal completion emits `endReason = "completed"` without false error severity.
- [x] 3.5 Add Rust tests for terminal diagnosis mapping: normal completion, tool loop force stop, context limit, budget exceeded, and cancellation where feasible.

## 4. Plan Mode Outcomes

- [x] 4.1 Track Plan-mode turn observations in the gateway/runtime stream path: plan file update, plan update, ask-question, and `exit_plan_mode` approval metadata.
- [x] 4.2 Classify Plan-mode terminal outcomes as `plan_approval_pending`, `needs_input`, `plan_artifact_updated`, or `plan_failed`.
- [x] 4.3 Emit `plan_failed` when a Plan turn ends without approval pending, clarification, or plan artifact update.
- [x] 4.4 Include plan path/existence metadata in terminal payload when available.
- [ ] 4.5 Add regression tests for Plan turn success, approval pending, clarification, tool-loop failure before plan creation, and natural end without plan artifact.

## 5. Frontend State Reconciliation

- [x] 5.1 Send `executionMode` from `useMessageStreamChat` based on the active chat's current mode, unless Goal mode overrides it.
- [x] 5.2 Update `turn_start` handling to migrate local chat id to resolved backend session id before applying mode, plan pending state, and usage.
- [x] 5.3 Preserve `executionMode`, plan file state, and plan approval state in `updateChatBackendId` as a defensive fallback.
- [x] 5.4 Reconcile visible chat mode from `turn_start.executionMode` without duplicating mode-switch brief messages.
- [x] 5.5 Ensure plan approval and continue-planning actions always use the resolved backend session id.
- [x] 5.6 Render abnormal terminal states, including `tool_loop` and `plan_failed`, as visible assistant-side status messages.

## 6. Regression Coverage

- [ ] 6.1 Add Vitest coverage for first-turn local `new-*` chat in Plan mode: request carries Plan, `turn_start` resolves backend id, store mode remains Plan.
- [ ] 6.2 Add Vitest coverage for Goal mode from a Plan-visible chat: request/effective mode reconciles to Agent for the turn.
- [ ] 6.3 Add Vitest coverage for abnormal `turn_end` rendering and usage recording after id migration.
- [ ] 6.4 Add Rust gateway tests for applying request mode after session resolution and for missing-mode backward compatibility.
- [ ] 6.5 Add Rust runtime/gateway tests for `turn_end` terminal diagnosis payload.
- [ ] 6.6 Add an integration or mocked stream regression proving a Plan turn stopped by `tool_loop` shows no-plan-produced feedback instead of silently ending.

## 7. Verification

- [x] 7.1 Run `cargo fmt --all`.
- [x] 7.2 Run targeted Rust tests for protocol/gateway/runtime changes.
- [x] 7.3 Run `cargo test --workspace` or document any unrelated blocker.
- [ ] 7.4 Run `cd crates/xiaolin-app && pnpm test`.
- [x] 7.5 Run `cd crates/xiaolin-app && pnpm build`.
- [ ] 7.6 If frontend behavior changes visually, run a Tauri/UI smoke test for first-turn Plan mode and abnormal stop display.
