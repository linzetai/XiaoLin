## Why

XiaoLin currently treats tool output as message text that is repeatedly truncated, micro-compacted, summarized, and finally dropped under context pressure. This damages the agent's ability to continue long-running work, causes repeated tool calls, and makes context compression a reliability risk on the core execution path.

This change makes tool output a recoverable runtime asset and makes the model-visible context a bounded projection of that asset. The goal is to improve agent task completion quality, context stability, and prompt-cache behavior without relying on temporary threshold tuning.

## What Changes

- Introduce session-scoped tool output assets that preserve raw tool output, metadata, content hashes, chunk indexes, line indexes, and lifecycle state.
- Replace large raw tool-result messages with compact, structured manifests that reference output handles and describe how to retrieve details.
- Add first-class recall tools for exact recovery: read by page/range, search within output, tail output, and request a typed summary.
- Add typed projectors for common output classes: file reads, search results, shell/test logs, directory listings, browser snapshots, MCP/default JSON, and generic large text.
- Refactor context assembly so tool-output projection is handled by a single context projection pipeline instead of scattered truncation and compaction layers.
- Track compaction and projection provenance so a result cannot be repeatedly compressed by multiple layers without visibility.
- Change LLM auto-compact guidance to preserve task state and output handles, not large tool-output bodies.
- Add quality gates and benchmarks that prove the optimization improves agent behavior, not just token counts.
- Extend runtime observability with raw-output size, projected-token size, recall usage, repeated-tool-call avoidance, and compact-after-continuation quality metrics.
- Preserve compatibility for existing persisted-output markers and old transcripts during migration.
- **BREAKING**: Runtime-internal handling of tool results changes from message-owned strings to handle-backed assets. External user-facing chat behavior should remain compatible, but runtime APIs that assume full tool output is always embedded in `ChatMessage.content` will need adaptation.

## Capabilities

### New Capabilities

- `tool-output-assets`: Session-scoped storage and lifecycle contract for raw tool output assets, handles, metadata, indexing, persistence, resume, and cleanup.
- `tool-output-recall-tools`: Built-in tools for reading, searching, tailing, and summarizing stored tool output by handle with session-scoped access control.
- `context-projection-pipeline`: Single pipeline that projects tool assets and conversation state into model-visible context under budget without destructive truncation.
- `typed-output-projectors`: Tool-type-aware manifest and summary generation for file reads, search results, shell/test logs, directory listings, browser snapshots, MCP/default JSON, and generic large output.
- `compaction-provenance`: Runtime provenance model for raw, manifest, typed summary, recalled excerpt, LLM summary, and hard-fit decisions, including prevention of repeated destructive compaction.
- `tool-output-quality-gates`: Test, benchmark, and regression requirements that measure recovery correctness, agent behavior, repeated-tool-call reduction, token projection efficiency, and cache-hit stability.
- `tool-output-observability`: Metrics and persisted runtime-quality fields for output assetization, projection, recall, token savings, and compact-after-continuation health.

### Modified Capabilities

- `token-discipline`: Context token discipline will require recoverable projection of large tool outputs instead of unrecoverable truncation when fitting context.

## Impact

- **Agent runtime**:
  - `crates/xiaolin-agent/src/runtime/tool_result_storage.rs` evolves from large-result preview persistence into the backing output-asset store or an adapter over it.
  - `crates/xiaolin-agent/src/runtime/tool_executor.rs`, `post_tool.rs`, `unified_compact.rs`, `context_compressor.rs`, and `dispatcher.rs` must stop treating raw large output as the durable context representation.
  - Built-in tool registration must include output recall tools and prompt guidance for using handles instead of rerunning expensive commands.
- **Context crate**:
  - `crates/xiaolin-context/src/engine.rs`, `pipeline.rs`, `compressor.rs`, and hard-fit behavior must preserve recoverability and respect projection provenance.
- **Session and persistence**:
  - `crates/xiaolin-session` may need tables or artifact-store integration for output assets, manifest metadata, recall events, and resume-safe handle resolution.
- **Protocol/UI**:
  - Streamed tool-result events may carry manifests plus optional display excerpts. UI can render expandable/searchable full output, while model context uses the projection.
- **Security**:
  - Output handles must be session scoped, non-guessable, and validated before recall to prevent cross-session data leakage.
- **Quality and benchmarks**:
  - Add long-output integration scenarios: large `rg`, failed test logs, large file reads, multi-turn compact/resume, and output recall after compression.
  - Add performance coverage for multi-megabyte output ingestion, indexing, search, and paging.
- **Prompt cache**:
  - Projection manifests must avoid random bytes and unstable timestamps in the model-visible prefix where possible, so this change does not regress prompt-cache hit rate.
