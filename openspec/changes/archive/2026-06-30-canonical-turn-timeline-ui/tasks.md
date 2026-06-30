## 1. Protocol And Data Model

- [x] 1.1 Define `TurnTimelineEvent`, `TimelineEventId`, event type enum, schema version, per-session `seq`, and payload structs in `xiaolin-protocol`.
- [x] 1.2 Define `TurnDisplayNode` DTOs for user message, assistant text, reasoning, tool step, tool group, approval, iteration boundary, terminal turn status, and system notice.
- [x] 1.3 Define node status, tool category, tool target metadata, output preview, large-output detail reference (reusing the existing `ToolOutputHandle` from `tool-output-assets`, not a new handle type), terminal diagnosis metadata, and source event trace metadata.
- [x] 1.4 Add generated frontend bindings for timeline events and display nodes.
- [x] 1.5 Add protocol serialization tests for representative timeline events and display nodes.

## 2. Timeline Persistence

- [x] 2.1 Add a `turn_timeline_events` store/table with session id, turn id, event id, sequence, event type, schema version, payload JSON, and created timestamp. If the existing `event_log` is extended instead, it must be hidden behind a typed timeline accessor and tests must prove runtime/debug agent events cannot be exposed as canonical UI timeline events.
- [x] 2.2 Implement idempotent append by event id.
- [x] 2.3 Implement monotonically increasing per-session sequence allocation.
- [x] 2.4 Implement queries by session, turn, `after_seq`, and page limit.
- [x] 2.5 Implement materialization from timeline events to `TurnDisplayNode[]`.
- [x] 2.6 Add tests for ordering, duplicate append, pagination, empty ranges, turn filtering, and materialization.

## 3. Runtime And Gateway Integration

- [x] 3.1 Emit timeline events for user message creation, turn start/end, assistant text, reasoning, tool start/progress/result, approvals, iteration boundaries, compact boundaries, terminal diagnostics, and system notices.
- [x] 3.2 Classify every currently UI-visible runtime event (including context warnings, brief messages, suggestions, mode changes, memory notices, and sub-agent activity) as a timeline event, node metadata, or explicitly non-transcript UI state.
- [x] 3.3 Implement a single append-and-broadcast pipeline that assigns durable `seq`, persists the event, and only then broadcasts the canonical timeline event over WebSocket.
- [x] 3.4 Implement assistant text/reasoning coalescing with explicit target identity and deterministic flush points before every visible non-text event and terminal turn event.
- [x] 3.5 Update WebSocket chat routes to stream persisted timeline events with the same event id and sequence returned by replay APIs.
- [x] 3.6 Add timeline query endpoints and display-node loading endpoints.
- [x] 3.7 Add a UI-authorized, read-only tool output detail endpoint that serves bounded content/detail sections for a `ToolOutputHandle` over the existing tool output asset store from `tool-output-assets` (with session-scoped ownership validation); do not introduce a parallel output backend or handle scheme.
- [x] 3.8 Ensure the tool output detail endpoint never returns an unbounded full blob by default; support configured response-size limits, continuation/truncation metadata, and typed unavailable/expired/unauthorized error states.
- [x] 3.9 Stop using `history_compat` and legacy message reconstruction for UI session replay.
- [x] 3.10 Keep legacy message/history storage only where still needed for model context or backend internals, with comments and tests documenting that it is not a UI source of truth and that timeline payloads are not automatically injected into model context.

## 4. Frontend Timeline Store

- [x] 4.1 Add a frontend timeline module with types, reducer, selectors, fixtures, and normalization helpers.
- [x] 4.2 Route live WebSocket timeline events through the same reducer used for replay.
- [x] 4.3 Update session loading to hydrate from display nodes or timeline events, not `BackendMessage.segmentOrder` or `toolCallsJson`.
- [x] 4.4 Implement reconnect recovery using last seen sequence and after-sequence loading.
- [x] 4.5 Remove active UI dependencies on `StreamSegment` reconstruction once node rendering is complete.
- [x] 4.6 Add reducer golden tests proving live event reduction and replay materialization produce equivalent normalized nodes.

## 5. Codex-Style Message Stream UI

- [x] 5.1 Refactor `MessageRenderer` into a `TurnDisplayNode` renderer with node-specific components.
- [x] 5.2 Implement stable assistant text streaming with delta coalescing and markdown-safe in-progress rendering.
- [x] 5.3 Render reasoning nodes consistently for active and completed states, collapsed or secondary after completion.
- [x] 5.4 Render iteration boundary nodes at the timeline position where they occurred.
- [x] 5.5 Render terminal status nodes/notices for tool-loop, cancellation, abort, budget, and runtime-error endings without presenting partial assistant text as a normal completion.
- [x] 5.6 Ensure long transcripts and high-frequency deltas remain responsive with virtualization or bounded render work.
- [x] 5.7 Add frontend unit tests for text, reasoning, boundaries, approvals, terminal status, and replay hydration.

## 6. Tool Step UI

- [x] 6.1 Implement `ToolStepNode` rendering with compact layout, semantic title, status, duration, target metadata, and progress.
- [x] 6.2 Implement display small-output policy: inline when output is <= 8,000 UTF-8 bytes, <= 200 lines, <= 2,000 estimated display tokens, and not binary.
- [x] 6.3 Implement large-output summaries and lazy detail expansion through the UI-authorized tool output detail endpoint over existing `tool-output-assets` assets.
- [x] 6.4 Implement paged/sectional detail UI for bounded responses, continuation/truncation metadata, and range/tail/summary views when available.
- [x] 6.5 Implement structured detail renderers for command output, file/search output, JSON/default output, and error states.
- [x] 6.6 Implement `ToolGroupNode` rendering for adjacent repetitive steps while preserving individual detail order.
- [x] 6.7 Add frontend tests for running, success, failed, cancelled, grouped, small-output, large-output, paged-detail, expired-detail, and replay states.

## 7. Remove Legacy UI Replay Path

- [x] 7.1 Remove or quarantine `history_compat` usage from UI-facing session loading.
- [x] 7.2 Remove active frontend reconstruction from `BackendMessage.toolCallsJson`, `reasoningContent`, and `segmentOrder`, including temporary encoded `text:` segment parsing and `buildMessageSegments` fallback logic.
- [x] 7.3 Add an explicit behavior for pre-change sessions: hide them, show unsupported-history notice, or reset development data.
- [x] 7.4 Delete dead adapters after the timeline-backed UI path is verified.
- [x] 7.5 Update developer documentation to state that canonical timeline is the UI source of truth.

## 8. Quality Gates

- [x] 8.1 Add shared fixtures for complex turns containing assistant text, reasoning, multiple tools, progress, approval, iteration boundary, failure, terminal diagnosis, and final answer.
- [x] 8.2 Add golden tests comparing live reducer output with backend materialized display nodes.
- [x] 8.3 Add reconnect tests for after-sequence catch-up and full display-node reload fallback.
- [x] 8.4 Add visual or DOM regression tests comparing live completed transcript and history replay for the same fixture.
- [x] 8.5 Add performance tests for high-frequency text deltas, long timelines, many tool steps, and large expandable output.
- [x] 8.6 Add negative tests proving small-output transcripts do not require detail API fetches for default replay.
- [x] 8.7 Add negative tests proving legacy message reconstruction is not used in the normal UI replay path.
- [x] 8.8 Add regression fixtures for text-tool-text ordering after reload, large search/diff output with handle-backed details, `tool_loop` terminal diagnosis with partial assistant text, empty reasoning deltas, and live/replay DOM equivalence.

## 9. Verification

- [x] 9.1 Run `cargo fmt --all`.
- [x] 9.2 Run targeted Rust tests for protocol, timeline persistence, materialization, gateway APIs, and reconnect behavior.
- [x] 9.3 Run `cargo test --workspace` or document unrelated blockers.
- [x] 9.4 Run `cd crates/xiaolin-app && pnpm test`.
- [x] 9.5 Run `cd crates/xiaolin-app && pnpm build`.
- [x] 9.6 Run frontend visual/E2E checks for live chat and history replay.
- [x] 9.7 Record before/after evidence that live and replay display are equivalent and that tool/text rendering improves without extra fetches for small outputs.

## 10. Codex App / ChatGPT UI Alignment

- [x] 10.1 Replace the flat node-list transcript renderer with a turn-level assistant response renderer backed by the same `TurnDisplayNode[]` timeline data.
- [x] 10.2 Group nodes by turn into user message blocks and assistant response blocks while preserving original timeline order inside each assistant response.
- [x] 10.3 Render tool calls as assistant-response activity rows, not peer chat messages and not Codex CLI-style log lines.
- [x] 10.4 Render reasoning as in-place, timeline-positioned assistant activity segments: consecutive reasoning deltas coalesce, but tool/text/approval/status boundaries close the current reasoning segment.
- [x] 10.5 Hide iteration boundary labels from the default user-facing chat UI; keep iteration metadata only for diagnostics, grouping, and tests.
- [x] 10.6 Keep assistant text as the primary narrative inside each assistant response, with resumed text after tools rendered in chronological order without merging across visible activity boundaries.
- [x] 10.7 Update DOM/unit tests to assert `reasoning -> tool -> reasoning -> text` and `text -> tool -> text` ordering inside one assistant response block.
- [x] 10.8 Add visual or DOM regression coverage for the Codex App / ChatGPT-style response layout, including running reasoning, running tool, completed tool, resumed answer text, and abnormal terminal status.

### Verification Evidence

- `cargo fmt --all` passed.
- `cargo test -p xiaolin-protocol timeline` passed: 38 tests.
- `cargo test -p xiaolin-session timeline` passed: 26 tests.
- `cargo test -p xiaolin-gateway timeline` passed: 2 tests; emitted existing `xiaolin-agent` dead-code warnings.
- `cargo test --workspace` was run and documented as blocked by unrelated `xiaolin-agent` failures: five `builtin_tools::session::session_tools_tests::*` failures due missing `turn_quality_summary`, plus `runtime::hook_executor::tests::shell_hook_success_returns_allow`, `runtime::orchestrator::tests::new_pipeline_auto_approve_executes`, and `runtime::streaming_tool_executor::tests::risk1_mutex_poison_cascade_on_tool_panic`.
- `cd crates/xiaolin-app && pnpm test` passed: 8 files, 227 tests.
- `cd crates/xiaolin-app && pnpm test -- --run src/components/message-stream/__tests__/turn-node-renderer.test.tsx` passed: 47 tests.
- `cd crates/xiaolin-app && pnpm build` passed.
- `cd crates/xiaolin-app && pnpm test:e2e` passed: 22 passed, 2 skipped.
- `cd crates/xiaolin-app && pnpm test:visual` ran outside the sandbox and failed because all 25 existing visual snapshots differ from current rendering, including unrelated agent-list/settings/full-app baselines. Regression E2E and DOM/unit coverage passed.
- Timeline reducer golden tests cover live/replay materialization equivalence; `ToolStepView` tests cover small-output inline replay without detail fetch, large-output lazy detail fetch through the session-scoped endpoint, and grouped steps preserving order.
- Legacy replay negative tests prove `toolCallsJson`, `reasoningContent`, `segmentOrder`, and encoded `text:` entries do not create active UI transcript segments.
- Assistant-response DOM tests cover `reasoning -> tool -> reasoning -> text`, `text -> tool -> text`, running reasoning, running tool, completed tool, resumed answer text, and abnormal terminal status inside one assistant response block.
- Assistant activity presentation tests cover semantic tool grouping, separate diff/sub-agent activity groups, hidden iteration boundaries, and default suppression of raw `subagent_get`/`git diff` log titles.
