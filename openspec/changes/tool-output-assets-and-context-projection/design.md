## Context

The current runtime stores tool results primarily as `ChatMessage.content`. Large outputs can be transformed by several independent layers: execution-time truncation, persisted-output previewing, post-tool microcompact, pre-query time and tier compaction, `ContentFilterHook`, LLM auto-compact, and final hard context fitting. Each layer is locally reasonable, but together they make raw tool output a fragile fact source.

The failure mode is visible in long coding sessions: the agent sees a partial preview, later sees only a compact marker, then must either guess, re-run the tool, or spend turns recovering information. This hurts tool quality, context compression, prompt-cache stability, and user trust because output loss happens on the core path between observing the workspace and acting on it.

The new architecture separates three concepts that are currently conflated:

- Raw tool output: durable fact source.
- Model-visible projection: bounded context representation.
- UI display: user-facing rendering and expansion.

```text
Tool Runtime
    |
    | raw output + metadata
    v
Tool Output Store  ---> indexes: chunks, lines, search metadata
    |
    | output handle
    v
Typed Projector ---> compact manifest / summary / excerpt
    |
    v
Context Projection Pipeline ---> LLM messages
    |
    v
Recall Tools: output_read / output_search / output_tail / output_summary
```

## Goals / Non-Goals

**Goals:**

- Preserve every large tool output as a recoverable session-scoped asset before any model-context projection.
- Replace destructive string truncation with stable manifests and typed summaries.
- Provide exact recall tools so the model can recover specific pages, line ranges, search matches, and tails without rerunning expensive commands.
- Centralize context projection decisions in one pipeline with provenance and budget accounting.
- Prevent repeated compression of the same output by multiple layers.
- Make LLM auto-compact preserve output handles and task state instead of summarizing large raw output bodies.
- Add quality gates that measure agent behavior: repeated tool-call reduction, recall success, compact-after-continuation correctness, and prompt-cache stability.
- Keep old transcripts and `<persisted-output>` markers readable during migration.

**Non-Goals:**

- Do not cache LLM responses.
- Do not remove normal small tool-result messages; small outputs can still be embedded directly when under budget.
- Do not make output assets globally shared across sessions.
- Do not introduce semantic vector search as a required first version; exact line/chunk/pattern recall is the baseline.
- Do not change external MCP tool protocols unless needed to wrap their outputs after execution.

## Decisions

### D1: Store raw output as a session-scoped asset before projection

Every tool execution result above a small inline threshold is written to a `ToolOutputAsset` before it is added to the model-visible message list. The asset stores:

- non-guessable handle id
- session id, turn id, tool call id, tool name, arguments digest
- success/exit status and stderr/stdout split where available
- raw bytes/text path
- content hash, byte count, line count, estimated tokens
- chunk index and line index
- projector kind and lifecycle timestamps

Small outputs may remain inline, but the same abstraction should support storing all outputs when configured for debugging.

Alternative considered: keep the existing `ToolResultStorage` preview files and teach the agent to `read_file` them. That preserves bytes but keeps recovery implicit, path-based, and easy to break under sandbox/session changes. A handle-backed asset API gives stronger access control and a better model contract.

### D2: Use handles, not filesystem paths, as the recovery contract

The model sees output handles such as `out_...` and recall affordances. Recall tools validate that the handle belongs to the current session before reading content. The manifest may include a UI/debug path only outside model context.

Alternative considered: expose persisted file paths. This leaks implementation details into prompts, creates path stability issues, and risks cross-session access if a file path is copied.

### D3: Project outputs through typed projectors

The system chooses a projector from tool metadata and output shape:

- `read_file`: path, requested range, actual line range, mtime/hash when available, representative excerpt.
- `search`: pattern, root, total matches, files matched, top matches, overflow count.
- `shell_test`: command, exit code, failure blocks, warnings, tail, duration.
- `list_tree`: root, entry counts, representative entries, omitted pages.
- `browser_snapshot`: URL/title, selected DOM/text summary, interaction affordances.
- `json_default`: top-level shape, important keys, array counts, representative records.
- `generic_text`: head/tail plus size and recall instructions.

Alternative considered: one generic head/tail summarizer. It is simple but misses the facts that matter most for search results and test failures, and it encourages re-runs.

### D4: Introduce a single ContextProjectionPipeline

The pipeline owns model-visible projection under token budget. Existing layers become inputs or safety fallbacks:

- Tool execution produces assets and initial manifests.
- Post-tool processing records metrics and iteration state, but does not destructively microcompact fresh raw output.
- Pre-query projection decides which manifests, recalled excerpts, summaries, and recent messages fit.
- `ContentFilterHook` becomes a last-resort safety guard, not a normal output-management layer.
- Hard context fit drops recoverable projections before dropping active task state.

Alternative considered: tune each existing layer. That reduces symptoms but leaves no single place to reason about what the model can recover.

### D5: Track projection provenance in messages and metrics

Every model-visible representation of tool output carries provenance:

- `raw_inline`
- `asset_manifest`
- `typed_summary`
- `recalled_excerpt`
- `llm_summary`
- `hard_fit_notice`

The projection pipeline must skip destructive transformations for content that is already a bounded projection. Metrics record the transition from raw size to projected size and whether the asset remains recoverable.

Alternative considered: rely on string markers like `[faded]` and `[summarized]`. Markers are useful for migration, but they are not a durable contract and can drift across crates.

### D6: Teach auto-compact to preserve state plus handles

LLM auto-compact receives compactable conversation history that already refers to output handles. Its prompt must preserve task intent, decisions, touched files, active plans, errors, and output handles. It should not attempt to embed large tool outputs in the summary.

Alternative considered: keep asking the summarizer to include code snippets and tool outputs. That makes summaries long and lossy, and it fights the output asset model.

### D7: Make quality and performance gates first-class

Implementation is not complete until tests and benchmarks show:

- full raw output can be recovered after compaction and session resume
- same output is not repeatedly destructively compressed
- large-output tasks avoid unnecessary tool re-runs
- recall tools retrieve the expected lines/matches/tails
- projection tokens stay bounded under long sessions
- prompt cache hit rate does not regress from unstable manifest bytes

Alternative considered: ship the architecture first and evaluate quality later. This area is core-path reliability, so quality evidence must be part of the work, not a follow-up.

## Risks / Trade-offs

- [Risk] Asset storage can grow quickly on long sessions. → Mitigation: session-scoped retention policy, size caps, cleanup jobs, and UI/runtime warnings before deletion.
- [Risk] Recall tools add extra tool calls. → Mitigation: manifests include enough typed summary for common decisions; recall is for precision, not every output.
- [Risk] Projectors can hide important details. → Mitigation: projector snapshot tests, generic fallback with head/tail, and exact recall for all raw content.
- [Risk] Handles can leak data across sessions if validation is weak. → Mitigation: non-guessable ids, session binding, store-level authorization, and negative tests.
- [Risk] Manifest instability can harm prompt cache. → Mitigation: omit random paths/timestamps from model-visible text, use stable ordering and deterministic formatting.
- [Risk] Migration from message-owned output touches many runtime paths. → Mitigation: adapter over existing `ToolResultStorage`, compatibility reader for `<persisted-output>`, and incremental replacement behind a feature gate.
- [Risk] UI expectations differ from model projection. → Mitigation: stream both display excerpt and asset metadata; UI expansion reads asset store while model sees the context projection.

## Migration Plan

1. Add output asset data model and store behind the existing runtime path without changing model-visible behavior.
2. Generate handles and persist raw large outputs while continuing to emit existing previews.
3. Add recall tools and validate session-scoped access.
4. Introduce typed projectors and model-visible manifests for selected tools under a feature gate.
5. Add `ContextProjectionPipeline` and route pre-query context assembly through it.
6. Disable destructive post-tool microcompact for asset-backed outputs, then reduce `ContentFilterHook` to fallback behavior.
7. Update LLM auto-compact prompt and restoration logic to preserve handles.
8. Add observability fields and benchmark dashboards/queries.
9. Turn on asset projection by default after quality gates pass.
10. Keep `<persisted-output>` compatibility until old sessions naturally age out.

Rollback strategy: disable asset projection and fall back to current preview/truncation behavior while retaining raw output writes. Because handles are additive in the transcript, rollback should not require deleting stored assets.

## Open Questions

- Should output assets live in `xiaolin-session` tables, `artifact_store`, or a dedicated runtime store with SQLite metadata and filesystem blobs?
- What default retention cap is appropriate per session and per workspace?
- Should recall tools be exposed to all agents or only coding/runtime agents?
- Should projectors be implemented as Rust traits in `xiaolin-agent`, `xiaolin-context`, or a new crate to avoid dependency cycles?
- How much UI work belongs in the first implementation phase versus backend-only quality validation?
