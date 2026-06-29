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

- [ ] 6.1 Implement `ToolStepNode` rendering with compact layout, semantic title, status, duration, target metadata, and progress.
- [ ] 6.2 Implement display small-output policy: inline when output is <= 8,000 UTF-8 bytes, <= 200 lines, <= 2,000 estimated display tokens, and not binary.
- [ ] 6.3 Implement large-output summaries and lazy detail expansion through the UI-authorized tool output detail endpoint over existing `tool-output-assets` assets.
- [ ] 6.4 Implement paged/sectional detail UI for bounded responses, continuation/truncation metadata, and range/tail/summary views when available.
- [ ] 6.5 Implement structured detail renderers for command output, file/search output, JSON/default output, and error states.
- [ ] 6.6 Implement `ToolGroupNode` rendering for adjacent repetitive steps while preserving individual detail order.
- [ ] 6.7 Add frontend tests for running, success, failed, cancelled, grouped, small-output, large-output, paged-detail, expired-detail, and replay states.

## 7. Remove Legacy UI Replay Path

- [ ] 7.1 Remove or quarantine `history_compat` usage from UI-facing session loading.
- [ ] 7.2 Remove active frontend reconstruction from `BackendMessage.toolCallsJson`, `reasoningContent`, and `segmentOrder`, including temporary encoded `text:` segment parsing and `buildMessageSegments` fallback logic.
- [ ] 7.3 Add an explicit behavior for pre-change sessions: hide them, show unsupported-history notice, or reset development data.
- [ ] 7.4 Delete dead adapters after the timeline-backed UI path is verified.
- [ ] 7.5 Update developer documentation to state that canonical timeline is the UI source of truth.

## 8. Quality Gates

- [ ] 8.1 Add shared fixtures for complex turns containing assistant text, reasoning, multiple tools, progress, approval, iteration boundary, failure, terminal diagnosis, and final answer.
- [ ] 8.2 Add golden tests comparing live reducer output with backend materialized display nodes.
- [ ] 8.3 Add reconnect tests for after-sequence catch-up and full display-node reload fallback.
- [ ] 8.4 Add visual or DOM regression tests comparing live completed transcript and history replay for the same fixture.
- [ ] 8.5 Add performance tests for high-frequency text deltas, long timelines, many tool steps, and large expandable output.
- [ ] 8.6 Add negative tests proving small-output transcripts do not require detail API fetches for default replay.
- [ ] 8.7 Add negative tests proving legacy message reconstruction is not used in the normal UI replay path.
- [ ] 8.8 Add regression fixtures for text-tool-text ordering after reload, large search/diff output with handle-backed details, `tool_loop` terminal diagnosis with partial assistant text, empty reasoning deltas, and live/replay DOM equivalence.

## 9. Verification

- [ ] 9.1 Run `cargo fmt --all`.
- [ ] 9.2 Run targeted Rust tests for protocol, timeline persistence, materialization, gateway APIs, and reconnect behavior.
- [ ] 9.3 Run `cargo test --workspace` or document unrelated blockers.
- [ ] 9.4 Run `cd crates/xiaolin-app && pnpm test`.
- [ ] 9.5 Run `cd crates/xiaolin-app && pnpm build`.
- [ ] 9.6 Run frontend visual/E2E checks for live chat and history replay.
- [ ] 9.7 Record before/after evidence that live and replay display are equivalent and that tool/text rendering improves without extra fetches for small outputs.
