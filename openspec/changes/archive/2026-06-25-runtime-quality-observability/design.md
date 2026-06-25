## Context

XiaoLin already has several observability primitives:

- `EventLog` persists raw `AgentEvent` streams by `session_id` and `turn_id`.
- `TurnSummary` reports end-of-turn elapsed time, iterations, tool call count, token usage, and context size.
- `CostStore` aggregates daily model tokens/cost and daily tool call counts/duration.
- `MetricsCollector` exposes process-level counters and latency summaries for Prometheus-style scraping.
- `prompt-cache-maximize-hits` adds cache hit/miss tracing and cache break diagnosis.
- `tool-quality-uplift` adds tool failure type and repetition metrics.

These are useful but fragmented. A developer investigating one bad turn must manually correlate event replay, logs, metrics, token usage, and tool behavior. The new capability adds a derived per-turn summary for offline analysis and regression comparison.

## Goals / Non-Goals

**Goals:**
- Persist one structured runtime quality summary for each completed, aborted, or errored turn.
- Keep the summary small, deterministic, and safe for local diagnostic storage.
- Capture timing, tool, token/cache/cost, context/compaction, and repetition signals at turn granularity.
- Generate rule-based diagnosis fields without calling an LLM.
- Make records queryable/exportable for developer analysis.
- Preserve `event_log` and runtime state as the source of truth; summaries are derived cache records.

**Non-Goals:**
- No frontend Diagnostics tab or MessageStream UI in this change.
- No LLM-generated explanations.
- No external APM, OpenTelemetry, tracing backend, or new service dependency.
- No storage of full prompts, full model responses, full tool outputs, secrets, or raw event stream duplicates.
- No automatic optimization decisions such as switching models, disabling tools, changing prompt cache strategy, or altering tool execution.
- No rewrite of existing `event_log`, `CostStore`, or Prometheus metric systems.

## Decisions

### D1: Persist a derived `turn_quality_summary` table

**Choice**: Add a compact SQLite table keyed by `(session_id, turn_id)` rather than computing every view by replaying `event_log`.

**Rationale**: Replaying event streams is correct but expensive for lists, trends, and repeated offline queries. A summary row makes regression analysis simple SQL while preserving raw events as the source of truth.

**Alternatives considered**:
- Event replay only: avoids a table but makes trend queries and sorting by slowest turn costly.
- Full trace/span table: powerful but duplicates event streams and drifts toward a tracing system.

### D2: Treat the summary as re-computable cache data

**Choice**: Summary rows are derived from runtime state and may be overwritten/upserted for the same `(session_id, turn_id)`.

**Rationale**: If collection logic improves, summaries can be rebuilt from event logs where enough raw data exists. This avoids treating summary fields as canonical facts.

**Implementation implication**: Store schema should include enough stable identifiers and timestamps for joins, but not raw prompt/tool payloads.

### D3: Collect hot-path timings in runtime state, not by parsing logs

**Choice**: Add a small in-memory collector for a turn that records monotonic timestamps for lifecycle milestones and aggregates tool statistics as events happen.

Suggested fields:
- `started_at`, `ended_at`, `elapsed_ms`
- `first_delta_ms`, `first_content_ms`, `first_reasoning_ms`, `first_tool_ms`
- `tool_time_ms_total`, `slowest_tool_name`, `slowest_tool_ms`
- `tool_calls_total`, `tool_failures_total`
- `compact_count`, `tokens_saved`

**Rationale**: Logs are not reliable structured inputs. Existing event emission paths already know when content, reasoning, tools, context updates, and turn endings happen.

**Alternatives considered**:
- Reconstruct all timings from `event_log.created_at`: SQLite timestamps are coarse and write batching can distort event time.
- Add timestamps to every `AgentEvent`: broader protocol churn than needed for P0.

### D4: Use deterministic diagnosis rules

**Choice**: Write rule-based classification into `diagnosis_code`, `severity`, and `evidence_json`.

Initial diagnosis codes:
- `normal`
- `provider_slow`
- `tool_slow`
- `tool_failure_loop`
- `cache_miss`
- `context_pressure`
- `high_cost`
- `many_iterations`
- `aborted`
- `error`

**Rationale**: Deterministic rules make records stable, cheap, testable, and suitable for comparing before/after optimization work.

**Precedence**: Terminal states (`aborted`, `error`) win first. Severe tool loops win over slow-provider classification. Cache/context/cost flags may be included in `evidence_json` even if they are not the primary diagnosis.

### D5: Keep labels low-cardinality and privacy-safe

**Choice**: Persist IDs, bounded enum codes, numeric metrics, model/provider names, tool names, and bounded JSON evidence. Do not persist prompt text, user message content, full tool args, full tool outputs, or arbitrary error strings.

**Rationale**: This is a local developer diagnostic feature, but it should still be safe by default and avoid metrics/table cardinality explosion.

### D6: Query/export first, UI later

**Choice**: P0 exposes data through store query functions and a developer-oriented JSON export/query route if convenient, but does not build a frontend panel.

**Rationale**: The product is still beta and the immediate value is offline analysis. UI can be designed later after the data proves useful.

## Risks / Trade-offs

- **Risk: Summary rows drift from raw events** -> Keep `event_log` as truth, upsert summaries, and make derivation deterministic so summaries can be regenerated.
- **Risk: Hot-path overhead during streaming** -> Collector updates must be in-memory O(1); SQLite write happens once at turn terminal state.
- **Risk: Missing summaries on crashes** -> P0 records normal abort/error paths but cannot guarantee process-crash records. A later startup repair job can infer incomplete turns if needed.
- **Risk: Diagnosis thresholds are arbitrary** -> Centralize thresholds as constants or config and store evidence values so thresholds can be revisited.
- **Risk: Privacy leakage through evidence** -> Evidence JSON must be schema-shaped and bounded; do not serialize raw args/output/error messages.
- **Risk: API surface becomes user-facing accidentally** -> Place routes under diagnostics naming and avoid frontend navigation in this change.

## Migration Plan

1. Add the `turn_quality_summary` table to the existing SQLite database initialization path.
2. Add store/query methods with upsert behavior.
3. Add protocol/session models for summary records and diagnosis enums.
4. Wire a runtime collector into turn setup, stream/tool/context handlers, and terminal paths.
5. Persist summaries on `TurnEnd`, `TurnAborted`, and fatal error paths.
6. Add developer query/export affordances.

Rollback is straightforward: stop writing summary rows. The table is additive and does not affect session replay, cost aggregation, model calls, or tool execution.

## Open Questions

- Should diagnosis thresholds be hard-coded for P0 or loaded from developer config?
- Should P0 expose HTTP routes immediately, or only store/query functions plus a CLI/script-friendly export?
- Should summaries include sub-agent runs folded into the parent turn, or should sub-agent turns get separate rows with a `parent_turn_id` in a later phase?
