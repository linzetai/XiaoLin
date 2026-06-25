## 1. Data Model and Storage

- [x] 1.1 Define `TurnQualitySummary`, diagnosis code, severity, and bounded evidence model in the shared protocol/session layer.
- [x] 1.2 Add `turn_quality_summary` SQLite table creation to the unified database initialization path.
- [x] 1.3 Add indexes for recent-turn lookup, session lookup, diagnosis lookup, and slow-turn sorting.
- [x] 1.4 Implement upsert-by-`(session_id, turn_id)` persistence so terminal paths can safely write once or update an existing partial record.
- [x] 1.5 Implement query methods for recent summaries, summaries by session, and single-turn lookup.

## 2. Runtime Collection

- [x] 2.1 Add an in-memory per-turn quality collector initialized during turn setup.
- [x] 2.2 Record monotonic timing milestones for first content delta, first reasoning delta, first model delta, and first tool start.
- [x] 2.3 Aggregate tool call count, failure count, total tool time, slowest tool name, and slowest tool duration from tool execution/result paths.
- [x] 2.4 Capture repetition warning and force-stop counts from existing query-loop repetition detection.
- [x] 2.5 Capture context usage, compression flag, tokens saved, and compaction count from context usage and compact boundary events.
- [x] 2.6 Capture usage, cache read/create tokens, cache hit percentage, estimated cost, model, provider, agent id, elapsed time, and iteration count at terminal state.

## 3. Terminal Path Persistence

- [x] 3.1 Persist a summary on normal `TurnEnd`.
- [x] 3.2 Persist a summary on `TurnAborted` with `diagnosis_code=aborted`.
- [x] 3.3 Persist a summary on fatal runtime or stream error with `diagnosis_code=error`.
- [x] 3.4 Ensure failed summary writes log warnings but do not fail the user turn or block streaming completion.
- [x] 3.5 Ensure missing optional values persist as null or documented zero values consistently.

## 4. Rule-Based Diagnosis

- [x] 4.1 Implement deterministic diagnosis precedence for terminal states, tool failure loops, tool slowdowns, provider slowness, cache miss, context pressure, high cost, many iterations, and normal turns.
- [x] 4.2 Centralize P0 thresholds for slow first delta, slow tool, tool-time dominance, low cache hit percentage, context pressure, high cost, and many iterations.
- [x] 4.3 Generate bounded `evidence_json` containing only numeric values, enum codes, tool names, and low-cardinality identifiers used by the selected rule.
- [x] 4.4 Include secondary flags in evidence when relevant without changing the primary `diagnosis_code`.
- [x] 4.5 Verify diagnosis generation never calls an LLM and never serializes prompt, message, tool argument, or tool output bodies.

## 5. Developer Query and Export

- [x] 5.1 Add backend query/export affordance for recent runtime quality summaries.
- [x] 5.2 Add backend query/export affordance for summaries filtered by session id.
- [x] 5.3 Add backend query/export affordance for a specific turn id.
- [x] 5.4 Keep diagnostics endpoints or export helpers developer-oriented and do not add frontend navigation or MessageStream UI.
- [x] 5.5 Document example SQL or JSON export usage for slow turn, cache miss, context pressure, and slowest-tool analysis.

## 6. Tests and Validation

- [x] 6.1 Add storage tests for table creation, upsert behavior, recent lookup, session lookup, and single-turn lookup.
- [x] 6.2 Add collector tests for timing milestones, no-tool turns, multi-tool aggregation, and failed tool aggregation.
- [x] 6.3 Add diagnosis tests for `normal`, `provider_slow`, `tool_slow`, `tool_failure_loop`, `cache_miss`, `context_pressure`, `high_cost`, `many_iterations`, `aborted`, and `error`.
- [x] 6.4 Add privacy tests confirming summaries and evidence do not contain prompt bodies, user message bodies, full tool args, or full tool outputs.
- [x] 6.5 Add an integration-style test that drives a synthetic turn to completion and verifies one `turn_quality_summary` row is persisted.
- [x] 6.6 Run targeted Rust tests for the affected crates.
- [x] 6.7 Run `openspec validate runtime-quality-observability --strict` and resolve any spec formatting or scenario issues.
