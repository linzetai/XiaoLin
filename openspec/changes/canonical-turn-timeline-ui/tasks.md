## 1. Protocol And Data Model

- [ ] 1.1 Define `TurnTimelineEvent`, `TimelineEventId`, event type enum, schema version, per-session `seq`, and payload structs in `xiaolin-protocol`.
- [ ] 1.2 Define `TurnDisplayNode` DTOs for user message, assistant text, reasoning, tool step, tool group, approval, iteration boundary, and system notice.
- [ ] 1.3 Define node status, tool category, tool target metadata, output preview, large-output detail reference (reusing the existing `ToolOutputHandle` from `tool-output-assets`, not a new handle type), and source event trace metadata.
- [ ] 1.4 Add generated frontend bindings for timeline events and display nodes.
- [ ] 1.5 Add protocol serialization tests for representative timeline events and display nodes.

## 2. Timeline Persistence

- [ ] 2.1 Add a `turn_timeline_events` store/table (or extend the existing `event_log` table with `seq`/event id/`schema_version` columns behind a typed accessor) with session id, turn id, event id, sequence, event type, schema version, payload JSON, and created timestamp. The agent event log and the UI timeline must remain distinguishable regardless of which option is chosen.
- [ ] 2.2 Implement idempotent append by event id.
- [ ] 2.3 Implement monotonically increasing per-session sequence allocation.
- [ ] 2.4 Implement queries by session, turn, `after_seq`, and page limit.
- [ ] 2.5 Implement materialization from timeline events to `TurnDisplayNode[]`.
- [ ] 2.6 Add tests for ordering, duplicate append, pagination, empty ranges, turn filtering, and materialization.

## 3. Runtime And Gateway Integration

- [ ] 3.1 Emit timeline events for user message creation, turn start/end, assistant text, reasoning, tool start/progress/result, approvals, iteration boundaries, compact boundaries, and system notices.
- [ ] 3.2 Persist timeline events before they are required for replay durability.
- [ ] 3.3 Update WebSocket chat routes to stream timeline-compatible events.
- [ ] 3.4 Add timeline query endpoints and display-node loading endpoints.
- [ ] 3.5 Add a UI-authorized, read-only tool output detail endpoint that serves bounded content/detail sections for a `ToolOutputHandle` over the existing tool output asset store from `tool-output-assets` (with session-scoped ownership validation); do not introduce a parallel output backend or handle scheme.
- [ ] 3.6 Stop using `history_compat` and legacy message reconstruction for UI session replay.
- [ ] 3.7 Keep legacy message/history storage only where still needed for model context or backend internals, with comments documenting that it is not a UI source of truth.

## 4. Frontend Timeline Store

- [ ] 4.1 Add a frontend timeline module with types, reducer, selectors, fixtures, and normalization helpers.
- [ ] 4.2 Route live WebSocket timeline events through the same reducer used for replay.
- [ ] 4.3 Update session loading to hydrate from display nodes or timeline events, not `BackendMessage.segmentOrder` or `toolCallsJson`.
- [ ] 4.4 Implement reconnect recovery using last seen sequence and after-sequence loading.
- [ ] 4.5 Remove active UI dependencies on `StreamSegment` reconstruction once node rendering is complete.
- [ ] 4.6 Add reducer golden tests proving live event reduction and replay materialization produce equivalent normalized nodes.

## 5. Codex-Style Message Stream UI

- [ ] 5.1 Refactor `MessageRenderer` into a `TurnDisplayNode` renderer with node-specific components.
- [ ] 5.2 Implement stable assistant text streaming with delta coalescing and markdown-safe in-progress rendering.
- [ ] 5.3 Render reasoning nodes consistently for active and completed states, collapsed or secondary after completion.
- [ ] 5.4 Render iteration boundary nodes at the timeline position where they occurred.
- [ ] 5.5 Ensure long transcripts and high-frequency deltas remain responsive with virtualization or bounded render work.
- [ ] 5.6 Add frontend unit tests for text, reasoning, boundaries, approvals, and replay hydration.

## 6. Tool Step UI

- [ ] 6.1 Implement `ToolStepNode` rendering with compact layout, semantic title, status, duration, target metadata, and progress.
- [ ] 6.2 Implement display small-output policy: inline when output is <= 8,000 UTF-8 bytes, <= 200 lines, <= 2,000 estimated display tokens, and not binary.
- [ ] 6.3 Implement large-output summaries and lazy detail expansion through the UI-authorized tool output detail endpoint over existing `tool-output-assets` assets.
- [ ] 6.4 Implement structured detail renderers for command output, file/search output, JSON/default output, and error states.
- [ ] 6.5 Implement `ToolGroupNode` rendering for adjacent repetitive steps while preserving individual detail order.
- [ ] 6.6 Add frontend tests for running, success, failed, cancelled, grouped, small-output, large-output, expired-detail, and replay states.

## 7. Remove Legacy UI Replay Path

- [ ] 7.1 Remove or quarantine `history_compat` usage from UI-facing session loading.
- [ ] 7.2 Remove active frontend reconstruction from `BackendMessage.toolCallsJson`, `reasoningContent`, and `segmentOrder`.
- [ ] 7.3 Add an explicit behavior for pre-change sessions: hide them, show unsupported-history notice, or reset development data.
- [ ] 7.4 Delete dead adapters after the timeline-backed UI path is verified.
- [ ] 7.5 Update developer documentation to state that canonical timeline is the UI source of truth.

## 8. Quality Gates

- [ ] 8.1 Add shared fixtures for complex turns containing assistant text, reasoning, multiple tools, progress, approval, iteration boundary, failure, and final answer.
- [ ] 8.2 Add golden tests comparing live reducer output with backend materialized display nodes.
- [ ] 8.3 Add reconnect tests for after-sequence catch-up and full display-node reload fallback.
- [ ] 8.4 Add visual or DOM regression tests comparing live completed transcript and history replay for the same fixture.
- [ ] 8.5 Add performance tests for high-frequency text deltas, long timelines, many tool steps, and large expandable output.
- [ ] 8.6 Add negative tests proving small-output transcripts do not require detail API fetches for default replay.
- [ ] 8.7 Add negative tests proving legacy message reconstruction is not used in the normal UI replay path.

## 9. Verification

- [ ] 9.1 Run `cargo fmt --all`.
- [ ] 9.2 Run targeted Rust tests for protocol, timeline persistence, materialization, gateway APIs, and reconnect behavior.
- [ ] 9.3 Run `cargo test --workspace` or document unrelated blockers.
- [ ] 9.4 Run `cd crates/xiaolin-app && pnpm test`.
- [ ] 9.5 Run `cd crates/xiaolin-app && pnpm build`.
- [ ] 9.6 Run frontend visual/E2E checks for live chat and history replay.
- [ ] 9.7 Record before/after evidence that live and replay display are equivalent and that tool/text rendering improves without extra fetches for small outputs.
