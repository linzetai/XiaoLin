## 1. Data Model And Store Foundation

- [x] 1.1 Define `ToolOutputHandle`, `ToolOutputAsset`, lifecycle state, projection provenance, and structured recall error types.
- [x] 1.2 Define projection size class configuration with defaults: small <= 8,000 UTF-8 bytes, <= 200 lines, <= 2,000 estimated tokens; medium <= 50,000 UTF-8 bytes, <= 1,000 lines, <= 12,500 estimated tokens; large above any medium threshold.
- [x] 1.3 Decide store placement (`xiaolin-session`, artifact store, or dedicated runtime store) and document the chosen storage boundary in code comments.
- [x] 1.4 Implement session-scoped output metadata persistence with session id, turn id, tool call id, tool name, argument digest, status, sizes, hashes, and projector kind.
- [x] 1.5 Implement raw output blob persistence with atomic writes and no overwrite of existing asset blobs.
- [x] 1.6 Implement line index, chunk/page index, and content hash generation for text outputs.
- [x] 1.7 Implement retention metadata and expiration records for removed assets.
- [x] 1.8 Add unit tests for size class classification, asset creation, metadata persistence, exact raw recovery, index generation, expiration records, and non-UTF8 or mixed-output handling.

## 2. Runtime Integration For Asset Creation

- [x] 2.1 Integrate asset creation into the tool execution path before any output truncation or model-visible projection.
- [x] 2.2 Adapt `tool_result_storage.rs` into the new store path or provide a compatibility adapter over the new asset store.
- [x] 2.3 Preserve small-output inline behavior while allowing debug/config mode to assetize all outputs.
- [x] 2.4 Replace path-based large-output recovery hints with handle-based recovery hints in model-visible messages.
- [x] 2.5 Add compatibility parsing for legacy `<persisted-output>` markers and map them to legacy provenance.
- [x] 2.6 Add integration tests proving large outputs are stored before post-tool processing, pre-query compaction, and content filtering can alter them.

## 3. Recall Tools

- [x] 3.1 Implement `output_read` with line range, byte range, and page-based reads plus stable pagination metadata.
- [x] 3.2 Implement `output_search` with bounded match results, context lines, total count when available, and continuation guidance.
- [x] 3.3 Implement `output_tail` with line-count bounds and shell/test status metadata.
- [x] 3.4 Implement `output_summary` using typed projectors with generic fallback.
- [x] 3.5 Register recall tools in the built-in tool registry and add concise prompt guidance for handle-based recovery.
- [x] 3.6 Enforce session-scoped authorization and non-disclosing errors for unauthorized handles.
- [x] 3.7 Add unit and integration tests for successful recall, expired handles, missing handles, unauthorized cross-session access, paging, search overflow, and tail output.

## 4. Typed Output Projectors

- [x] 4.1 Define a projector trait/interface that accepts asset metadata and raw/index access without creating crate dependency cycles.
- [x] 4.2 Implement `read_file` projector with path, requested range, actual range, freshness metadata when available, representative excerpt, and recall guidance.
- [x] 4.3 Implement search/grep projector with pattern/root metadata, match counts, file distribution, representative matches, omitted counts, and `output_search` guidance.
- [x] 4.4 Implement shell/test projector with command, exit status, duration when available, failure blocks, warnings/errors, tail excerpt, and handle.
- [x] 4.5 Implement directory/tree projector with root path, entry counts, representative entries, omitted counts, and page guidance.
- [x] 4.6 Implement JSON/default projector with top-level shape, key/array counts, representative fields, handle, and recall guidance.
- [x] 4.7 Implement generic text projector for unknown large output with deterministic head/tail summary and recall guidance.
- [x] 4.8 Add snapshot tests for every projector, including stable byte formatting and no volatile timestamps or blob paths in model-visible text.

## 5. Context Projection Pipeline

- [x] 5.1 Introduce `ContextProjectionPipeline` as the single owner of model-visible output projection under token budget.
- [x] 5.2 Route pre-query context assembly through the projection pipeline before LLM calls.
- [x] 5.3 Change post-tool processing so it records metrics and iteration state but does not destructively microcompact fresh asset-backed outputs.
- [x] 5.4 Change `ContentFilterHook` so asset manifests, typed summaries, recalled excerpts, and legacy projections are treated as bounded content and not re-truncated under normal conditions.
- [x] 5.5 Change hard context fitting to drop recoverable projections before active task instructions, current user input, or non-recoverable state.
- [x] 5.6 Add projection budget accounting for raw tokens, projected tokens, and saved tokens.
- [x] 5.7 Implement adaptive projection policy so small outputs stay inline, medium relevant outputs keep key content, large outputs use typed summary plus handle, and handle-only manifests are a last resort.
- [x] 5.8 Add tests that pass one large output through post-tool, pre-query, content filter, and hard-fit paths without nested truncation markers.
- [x] 5.9 Add tests proving bounded failed command output and small search results are visible inline without requiring immediate recall.

## 6. Auto-Compact And Prompt Behavior

- [x] 6.1 Update auto-compact prompt guidance to preserve output handles, task intent, decisions, active plans, touched files, and error state instead of embedding large raw output bodies.
- [x] 6.2 Ensure compaction summaries retain why each output handle matters when the compacted history referenced it.
- [x] 6.3 Update runtime restoration logic so file/skill/plan restoration coexists with output-handle restoration.
- [x] 6.4 Update agent tool guidance to prefer `output_read`, `output_search`, or `output_tail` over rerunning expensive commands when a handle is available.
- [x] 6.5 Add tests for compacted histories that retain handle references and allow precise post-compact recall.

## 7. Provenance And Migration Safety

- [ ] 7.1 Attach projection provenance to model-visible output representations and recall-tool results.
- [ ] 7.2 Record provenance transitions from raw output to manifest, typed summary, recalled excerpt, LLM summary, and hard-fit removal.
- [ ] 7.3 Replace string-marker-only compaction checks with provenance-aware checks while preserving legacy marker recognition.
- [ ] 7.4 Add migration tests for restored transcripts containing `[faded]`, `[summarized]`, `[recall-available]`, and `<persisted-output>`.
- [ ] 7.5 Add negative tests proving asset-backed projections are not repeatedly destructively compacted by old and new paths together.

## 8. Observability And Runtime Quality

- [ ] 8.1 Add metrics for asset creation count, raw bytes, raw token estimate, line count, projected token estimate, and tokens saved.
- [ ] 8.2 Add metrics for recall tool success, failure type, returned token estimate, and latency.
- [ ] 8.3 Extend turn quality summary persistence with asset count, raw output token estimate, projected output tokens, recall count, and repeated-tool-call indicators.
- [ ] 8.4 Correlate repeated same/equivalent tool calls with prior available output handles where possible.
- [ ] 8.5 Add documentation or SQL examples for measuring output projection effectiveness across sessions.
- [ ] 8.6 Add tests proving metrics avoid recording sensitive raw output content.

## 9. Quality Gates And Benchmarks

- [ ] 9.1 Add raw recovery tests proving exact line-range recall after projection, compaction, and session resume.
- [ ] 9.2 Add large `rg` integration scenario proving the agent can locate relevant matches via recall tools without rerunning the original search.
- [ ] 9.3 Add failed test log scenario proving shell/test projector surfaces failure blocks and recall can fetch exact surrounding lines.
- [ ] 9.4 Add large file-read scenario proving projected file output can recover arbitrary line ranges after compaction.
- [ ] 9.5 Add multi-turn long-session scenario proving compact-after-continuation remains correct with output handles.
- [ ] 9.6 Add performance benchmarks for multi-megabyte ingestion, indexing, search, and page reads with documented thresholds.
- [ ] 9.7 Add token budget tests proving projection tokens remain bounded and raw/projected accounting is reported separately.
- [ ] 9.8 Add prompt-cache stability tests proving deterministic manifest formatting across repeated context assemblies.
- [ ] 9.9 Add no-negative-optimization benchmark proving small/medium output scenarios do not require extra recall calls compared with direct inline output.
- [ ] 9.10 Add recall-loop benchmark proving repeated broad paging or same-range recall is detected and redirected.

## 10. UI And Protocol Integration

- [ ] 10.1 Decide the streamed event shape for tool-result manifests, display excerpts, asset handles, and UI expansion metadata.
- [ ] 10.2 Update protocol types and generated frontend types for output asset metadata if stream payloads change.
- [ ] 10.3 Update tool result UI rendering to display bounded excerpts with affordances for full output expansion/search where applicable.
- [ ] 10.4 Ensure UI expansion reads through authorized backend APIs instead of exposing raw blob paths.
- [ ] 10.5 Add frontend tests or mocked stream tests for manifest rendering, expired asset display, and large output expansion.

## 11. Verification

- [ ] 11.1 Run `cargo fmt --all`.
- [ ] 11.2 Run targeted Rust tests for output asset store, recall tools, projectors, projection pipeline, provenance, and quality metrics.
- [ ] 11.3 Run long-output integration and benchmark scenarios added by this change.
- [ ] 11.4 Run `cargo test --workspace` or document unrelated blockers.
- [ ] 11.5 Run `cd crates/xiaolin-app && pnpm test` if protocol/UI code changes.
- [ ] 11.6 Run `cd crates/xiaolin-app && pnpm build` if protocol/UI code changes.
- [ ] 11.7 Record before/after evidence for repeated tool calls, recall success, projected token savings, and prompt-cache stability before marking the change complete.
