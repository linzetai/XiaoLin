## Why

XiaoLin currently has two frontend truth sources for the same assistant turn: live WebSocket events are reduced into `StreamSegment[]`, while history replay reconstructs a similar shape from persisted messages, `history_items`, and compatibility adapters. This split makes tool steps, reasoning blocks, progress, approvals, detached streams, and final assistant text diverge between the live view and session replay.

The product has not launched yet, so the right fix is to remove the dual model instead of adding migration layers around it. This change makes a canonical turn timeline the only UI source of truth and uses that same model to improve the frontend display of tool calls, reasoning, and streamed text toward a Codex App / ChatGPT-like assistant response stream.

The UI reference is explicitly **not** Codex CLI. Codex CLI is a terminal-oriented step transcript where tool calls are first-class log entries. XiaoLin should instead follow Codex App / ChatGPT: user and assistant messages remain the primary visual units; reasoning and tool activity are contextual activity inside an assistant response.

## What Changes

- Introduce a canonical append-only turn timeline for UI-visible chat activity.
- Persist timeline events with stable session, turn, event id, sequence, type, schema version, payload, and timestamps.
- Add APIs that return both raw timeline events and materialized display nodes derived from those events.
- Route live WebSocket rendering and history replay through the same reducer/materializer contract.
- Replace frontend history reconstruction from legacy `messages`, `toolCallsJson`, `segmentOrder`, and `history_compat` with timeline-backed display nodes.
- Redesign the frontend stream model around `TurnDisplayNode` rather than ad hoc `StreamSegment` and message reconstruction paths.
- Improve tool-call display with compact activity rows nested inside assistant responses, semantic titles, status/progress, duration, target metadata, grouped noisy steps, and lazy details.
- Improve assistant text streaming with stable markdown-safe chunk rendering, reduced layout shift, and consistent final replay.
- Preserve small tool outputs inline in the display node so ordinary tool calls do not require an extra fetch or another tool call to be understandable.
- Reuse the session-scoped tool output handle and asset system defined by the in-flight `tool-output-assets` change as the backend for large/expandable tool details, rather than introducing a parallel handle scheme. The UI detail endpoint added here is a read-only, UI-authorized view over those existing assets.
- Remove old-session migration from scope. Existing development sessions may be discarded or hidden after this change.
- **BREAKING**: UI-facing session history APIs stop treating legacy chat messages as the replay source. Consumers that depend on `BackendMessage.segmentOrder`, `toolCallsJson`, or message-derived stream reconstruction must move to timeline APIs.

## Capabilities

### New Capabilities

- `canonical-turn-timeline`: Canonical ordered timeline contract for live events, history replay, reducer determinism, and removal of legacy UI reconstruction.
- `timeline-persistence-api`: Durable timeline storage and APIs for loading events, loading materialized display nodes, and recovering after reconnect.
- `codex-message-stream-ui`: Frontend display contract for Codex App / ChatGPT-like assistant response blocks, streamed assistant text, reasoning activity, internal iteration boundaries, virtualized transcript behavior, and live/history visual equivalence.
- `tool-step-display`: Frontend display contract for compact Codex-style tool steps, grouped tools, progress, small-output inline previews, and lazy detail expansion.

### Modified Capabilities

- `codex-step-polish`: Superseded by `tool-step-display` for step rendering behavior under the canonical timeline model.
- `codex-reasoning-block`: Reasoning rendering is now driven by `TurnDisplayNode` state instead of message reconstruction.
- `codex-iteration-divider`: Iteration dividers are now timeline display nodes and must render identically in live and replay.
- `delta-fast-path`: Streaming delta handling must feed the canonical reducer rather than a separate live-only segment path.

## Impact

- **Protocol**:
  - `crates/xiaolin-protocol/src/event.rs` gains or adapts timeline event and display node DTOs.
  - Generated frontend bindings must include timeline event payloads, display nodes, node status, and tool detail references.
  - Tool detail references in display nodes reuse the existing `ToolOutputHandle` type from the `tool-output-assets` capability; no new handle type is introduced.
- **Session persistence**:
  - `crates/xiaolin-session` adds a timeline event store with per-session sequence ordering and idempotent append semantics.
  - Existing `history_items` may remain for agent context if still needed, but must not be the UI replay source after this change.
- **Gateway/API**:
  - Session routes expose timeline event pagination, display node loading, and reconnect recovery by last sequence.
  - WebSocket chat routes emit timeline-compatible events or enough information for the same reducer to derive them.
  - The tool detail endpoint is a read-only, UI-authorized view over the existing tool output asset store (see `tool-output-assets`), not a separate output backend.
- **Frontend**:
  - `useMessageStreamChat`, `stream-store`, `MessageRenderer`, `StepIndicator`, `StepGroup`, reasoning blocks, and session loading move to the canonical display-node model and render through turn-level assistant response blocks rather than a flat CLI-style log.
  - Legacy `BackendMessage` reconstruction paths are removed from active UI flow.
- **Quality**:
  - Add reducer golden tests proving live event reduction and persisted replay produce identical display nodes.
  - Add frontend visual/regression tests for tool calls, reasoning, approvals, progress, reconnect, detached streams, and history replay.
  - Add performance tests for long timelines, frequent text deltas, many tool steps, and large expandable outputs.
