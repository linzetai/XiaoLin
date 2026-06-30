## Context

The current chat UI has a live model and a replay model.

Live flow:

```text
AgentEvent over WebSocket
  -> useMessageStreamChat event switch
  -> StreamSegment[]
  -> MessageRenderer / StepIndicator / ReasoningBlock
```

History flow:

```text
messages + history_items + history_compat + segment_order
  -> BackendMessage / ChatMessage
  -> reconstructed StreamSegment[]
  -> MessageRenderer / StepIndicator / ReasoningBlock
```

These two flows try to approximate the same visible transcript but do not share an authoritative event log or reducer. Every new event type, tool state, progress update, reasoning block, or persistence edge case has to be implemented twice. That is the root cause of live/history drift.

Because XiaoLin is not launched yet, this design intentionally does not preserve old UI history. The goal is to simplify the core path now and avoid long-term compatibility debt.

## Goals

- Make one canonical turn timeline the only source for UI-visible chat history.
- Make live rendering, reload, reconnect, and historical replay use the same reducer/materializer contract.
- Improve tool-call and streamed text display while changing the data model, not as a later cosmetic pass.
- Align the visible chat UI with Codex App / ChatGPT message streams, not Codex CLI terminal transcripts.
- Keep small outputs inline so common tool results remain understandable without additional fetches.
- Support large output expansion through references without making the default transcript heavy.
- Provide objective quality gates for equivalence, performance, and UI regressions.

## Non-Goals

- No migration of existing development sessions.
- No compatibility guarantee for consumers of legacy UI history fields such as `segmentOrder` and `toolCallsJson`.
- No attempt to store every provider token as a durable event when coalesced semantic events are sufficient.
- No redesign of the model-visible context projection pipeline; this change concerns UI-visible timeline and replay.
- No new tool output handle or asset scheme. Large-output detail references reuse the `ToolOutputHandle`/`ToolOutputAsset` system from the `tool-output-assets` change; this proposal only adds a UI-authorized read view, not a parallel backend.

## Decisions

### D1. Canonical Timeline As UI Source Of Truth

All UI-visible session history SHALL come from an ordered append-only turn timeline. Legacy `messages` and `history_items` may still exist for agent context or transitional backend internals, but the frontend must not reconstruct UI transcript state from them.

### D2. Store Semantic Timeline Events, Materialize Display Nodes

The durable store records semantic events. The API can return either raw events or materialized `TurnDisplayNode[]`.

This avoids storing fragile React-specific shapes while still giving the frontend a stable display contract.

### D3. One Reducer Contract For Live And Replay

The same reducer semantics apply to both sources:

```text
live websocket timeline events -> reduce -> TurnDisplayNode[]
stored timeline events         -> reduce -> TurnDisplayNode[]
```

The backend may materialize display nodes for initial load, but those nodes must be generated from the same event semantics used by the live reducer. Golden tests must prove equivalence.

### D4. Timeline Event Schema

Timeline events use stable ordering and idempotency fields:

```rust
struct TurnTimelineEvent {
    id: TimelineEventId,
    session_id: SessionId,
    turn_id: TurnId,
    seq: i64,
    event_type: TimelineEventType,
    schema_version: u16,
    payload_json: serde_json::Value,
    created_at_ms: i64,
}
```

`seq` is monotonically increasing per session. `id` is globally unique and idempotent. If an event append is retried with the same id, the store returns the existing row instead of duplicating it.

A per-session append-only `event_log` table already exists in `crates/xiaolin-session/src/event_log.rs` and stores serialized `AgentEvent` JSON keyed by `(session_id, turn_id)` with a session/turn index. The timeline store is not a second copy of that log: the existing `event_log` is the runtime/agent event record (untyped JSON, no `seq`, no idempotent id, no schema version), while the timeline store is the UI-visible semantic record with the ordering and idempotency fields above. The implementation SHOULD extend or sit alongside `event_log` rather than duplicate the append path; the decision between (a) adding `seq`/`id`/`schema_version` columns to the existing table behind a typed accessor, or (b) a separate `turn_timeline_events` table that references the same session, is deferred to implementation, but either choice must keep the two concerns (agent event log vs UI timeline) distinguishable.

### D5. Timeline Event Types

The first version should cover:

- `turn_started`
- `user_message_created`
- `assistant_text_delta`
- `assistant_text_snapshot`
- `reasoning_delta`
- `reasoning_snapshot`
- `tool_call_started`
- `tool_call_progress`
- `tool_call_finished`
- `approval_requested`
- `approval_resolved`
- `iteration_boundary`
- `assistant_message_finalized`
- `turn_finished`
- `compact_boundary`
- `system_notice`

Streaming deltas may be coalesced before durable append. The required invariant is final display equivalence, not token-by-token typing replay.

### D6. Display Node Model

The frontend renders `TurnDisplayNode` instead of legacy messages or `StreamSegment`.

```ts
type TurnDisplayNode =
  | UserMessageNode
  | AssistantTextNode
  | ReasoningNode
  | ToolStepNode
  | ToolGroupNode
  | ApprovalNode
  | IterationBoundaryNode
  | TurnStatusNode
  | SystemNoticeNode;
```

Every node has a stable `nodeId`, `turnId`, `status`, `createdAtMs`, `updatedAtMs`, and enough metadata to render in both live and replay states.

`TurnStatusNode` is the visible terminal state for non-successful or diagnostically important turn endings. It carries the turn end reason, optional terminal diagnosis code/severity/message/evidence, cancellation or error metadata when available, and source event trace metadata. A partial assistant text node MAY remain visible before the status node, but the transcript must not make a tool-loop, cancellation, or runtime failure look like a normally completed assistant response.

### D6a. Codex App / ChatGPT Message Structure

The canonical timeline is the ordering source, but the visible UI SHALL NOT render it as a flat Codex CLI-style log. The renderer groups display nodes by turn and renders them as message blocks:

```text
User message

Assistant response
  reasoning segment (at its timeline position, collapsed/secondary)
  tool activity row(s)
  assistant text
  reasoning segment
  tool activity row(s)
  assistant text / final answer
  terminal status only when abnormal
```

The assistant response block is the primary visual unit. Reasoning and tools are supporting activity within that block. This preserves real-time chronology without promoting every event to a peer chat message.

Iteration boundaries are internal timeline metadata. They MAY affect grouping, diagnostics, or test fixtures, but they SHALL NOT render visible user-facing labels such as "iteration 2" in the default chat UI.

### D7. Tool Step Display

Tool calls render as compact assistant-activity rows, not full message cards and not Codex CLI log lines. A `ToolStepNode` carries:

- tool name and semantic category
- human-readable title
- status: pending, running, succeeded, failed, cancelled
- target metadata such as path, command, URL, query, or MCP server
- progress label and numeric progress when known
- started/finished timestamps and duration
- small inline output preview when output is small
- large output reference and summary when output is large
- expandable detail sections for args, stdout/stderr, structured JSON, diff, or browser snapshot

Small output is defined by display policy, not by context-projection policy:

- UTF-8 byte length <= 8,000
- line count <= 200
- estimated display tokens <= 2,000
- no known binary payload

When all small-output thresholds are satisfied, the display node SHOULD include inline data sufficient for replay without an extra API fetch. The default chat UI SHOULD keep that output collapsed or summarized unless it is essential user-facing result content; expansion reveals the full inline preview. When any threshold is exceeded, the display node MAY include a bounded summary plus a detail reference.

The detail reference reuses the session-scoped `ToolOutputHandle` and `ToolOutputAsset` system introduced by the `tool-output-assets` change (already in flight). The timeline UI does not define its own handle scheme or output backend. The UI detail endpoint is a read-only, UI-authorized view over those assets; the existing agent-facing recall tools (`output_read`, `output_search`, `output_tail`, `output_summary`) remain the model-context path and are unchanged here. The display-side small-output policy below is independent of the model-context projection policy: display decides what is inline in the transcript, projection decides what the model sees, and the two policies are deliberately separate.

The UI detail endpoint must never return an unbounded full blob by default. It returns bounded sections, ranges, tails, summaries, or structured slices with serialized response-size limits and explicit continuation metadata when more content exists.

### D8. Text Streaming Display

Assistant text streaming coalesces frequent deltas into stable text nodes. The UI should update at frame or short time intervals, preserve markdown/code block correctness, and avoid layout shift caused by rebuilding whole messages.

Final replay should show the same assistant text content and node boundaries as the completed live turn, except that typing animation does not need to be replayed.

Text event payloads must identify the append target (`node_id` or equivalent) and carry the exact text delta or snapshot content required for replay. Buffered text must be flushed before any subsequent visible non-text event is appended, including tool start/result, reasoning, approval, iteration boundary, terminal status, and turn end. Empty text or reasoning deltas are ignored unless they intentionally update metadata.

Text nodes must not merge across visible assistant-response activity boundaries unless the event explicitly targets the same visible text segment. In normal streaming, text before a tool/activity segment and text after that segment are separate assistant text segments inside the same assistant response block, preserving chronology while keeping the answer visually coherent.

### D8a. Reasoning Segment Display

Reasoning is real-time process context, not a single global "thinking" panel and not a primary assistant answer. The UI SHALL preserve reasoning at the timeline position where it occurred:

- consecutive reasoning deltas MAY coalesce into one reasoning segment;
- a visible boundary such as tool start/result, approval, assistant text, terminal status, or turn end closes the active reasoning segment;
- later reasoning creates a new segment at its later timeline position;
- active reasoning shows a subtle live row or collapsed panel so the user can see that thinking is happening in real time;
- completed reasoning is visually secondary and collapsed by default, but expandable in place.

This avoids both extremes: it does not flatten reasoning into a CLI log, and it does not move all reasoning to a single top-level container that would scramble the perceived timeline.

### D9. Timeline Append And Broadcast Ordering

Timeline WebSocket delivery must be produced by a single append-and-broadcast path. A UI-visible timeline event is assigned its durable session `seq` and persisted successfully before it is broadcast to clients. If persistence fails, the event is not broadcast as canonical timeline state; the runtime may emit a separate non-timeline error notice. This keeps reconnect `after_seq` recovery unambiguous.

### D10. Reconnect And Detached Stream Recovery

The client records the last seen `seq` per session. On reconnect it requests all events after that sequence and feeds them into the same reducer. If the gap is too large or the client state is suspect, it reloads materialized display nodes from the backend.

### D11. Event Coverage Boundaries

The first implementation must classify every currently UI-visible live event as one of:

- represented directly by a timeline event and display node;
- represented as timeline metadata on an existing node;
- deliberately excluded from the transcript because it belongs to non-transcript UI state.

This includes assistant text, reasoning, tools, approvals, iteration boundaries, compact/compaction notices, terminal diagnostics, context warnings, brief messages, suggestions, mode changes, memory notices, and sub-agent activity. Events excluded from the transcript must not rely on legacy message reconstruction for replay.

### D12. Model Context Isolation

Timeline payloads are UI-visible replay data, not model-visible context. Existing message/history/context projection paths may continue to feed the LLM, but they must not automatically include timeline payloads, large output detail payloads, or display-only metadata unless a separate context-projection rule explicitly chooses to do so.

### D13. No Legacy Migration

Old sessions created before this change are not migrated. Acceptable behaviors are:

- hide old sessions in development builds,
- show an explicit unsupported-history notice,
- or clear local development data as part of the implementation.

The implementation should remove compatibility code from the active path rather than building adapters for old history.

## API Shape

Suggested endpoints:

```text
GET /sessions/{session_id}/timeline?after_seq=&limit=
GET /sessions/{session_id}/display-nodes?after_seq=&limit=
GET /sessions/{session_id}/turns/{turn_id}/timeline
GET /sessions/{session_id}/tool-output/{handle}   # UI-authorized read-only view over an existing ToolOutputAsset
```

The `tool-output/{handle}` endpoint is a UI-authorized, read-only view over the existing tool output asset store from the `tool-output-assets` capability. It validates session-scoped ownership and returns bounded content or detail sections the same way the agent-facing recall tools do, but scoped to UI rendering. It does not create a second output backend or handle scheme.

WebSocket events should either be timeline events directly or a thin wrapper around them:

```ts
type TimelineWsEvent = {
  kind: "timeline_event";
  event: TurnTimelineEvent;
};
```

## Frontend Architecture

Introduce a timeline module in the app, for example:

```text
src/lib/timeline/
  types.ts
  reducer.ts
  materialize.ts
  selectors.ts
  fixtures.ts
```

`useMessageStreamChat` should stop owning transcript semantics. It should subscribe to WebSocket timeline events, append them to store state, and let selectors provide render-ready nodes.

`stream-store` should load `display-nodes` or timeline events from the backend and hydrate the same store shape used by live sessions.

`MessageRenderer` should become a node renderer:

```text
TurnDisplayNode[] -> node-specific renderers
```

## UI Direction

The target is a Codex App / ChatGPT-like assistant response stream:

- user messages and assistant responses are the primary message units;
- assistant text is the primary narrative inside an assistant response;
- reasoning appears in place as subtle real-time thinking segments, collapsed or visually secondary after completion;
- tool calls appear in place as compact assistant activity rows, not as peer messages and not as a terminal log;
- tool details are available but collapsed unless the user expands them or the output is the user-facing result;
- consecutive low-value tool steps can group under a concise activity summary inside the assistant response;
- iteration boundaries are not shown as user-facing labels in the default UI;
- terminal status is visible only for abnormal or diagnostically important endings;
- history replay looks like the completed live assistant response, not a reconstructed approximation.

## Quality Strategy

Quality gates are part of the task, not a follow-up:

- reducer golden tests for live vs replay equivalence;
- persistence tests for sequence ordering, idempotency, reconnect gaps, and pagination;
- frontend unit tests for every display node type;
- visual screenshots for live and replay versions of the same fixture;
- performance tests for long sessions and high-frequency deltas;
- regression tests proving small output stays inline and large output expands through detail APIs without blocking transcript render.

## Risks

- The change touches protocol, persistence, gateway, frontend store, and rendering. Mitigate with phased implementation and fixture-driven tests.
- Backend and frontend materializers can drift if implemented independently. Mitigate by making fixtures and golden outputs shared and required.
- Tool detail APIs can reintroduce extra round trips for normal cases. Mitigate with the small-output inline policy and tests.
- Removing legacy replay can disrupt local development data. Mitigate with an explicit one-time dev data reset or unsupported-history notice.
