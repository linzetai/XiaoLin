## Why

XiaoLin already emits rich turn events, daily cost aggregates, provider metrics, tool failure metrics, and prompt-cache diagnostics, but those signals are scattered across logs, event replay, and aggregate stores. Developers cannot quickly answer why a specific turn was slow, expensive, failed, retried tools, missed cache, or pressured the context window.

This change introduces a backend-only runtime quality summary for each turn so beta-stage development can rely on structured diagnostic data for offline analysis and regression comparison without adding frontend UI or LLM-generated explanations.

## What Changes

- Add a persisted `turn_quality_summary` record per turn, derived from existing runtime events and execution state.
- Capture turn-level timing milestones: total elapsed time, first delta/content/reasoning/tool timing, tool duration totals, and slowest tool.
- Capture turn-level quality counters: iterations, tool call count, failures, repetition warning/force-stop counts, context usage, compaction count, token usage, cache tokens, cache hit percentage, and estimated cost.
- Add deterministic rule-based `diagnosis_code`, `severity`, and `evidence_json` fields for later analysis.
- Record summaries for normal completion, abort, and error paths.
- Provide backend query/export affordances for developers; no frontend Diagnostics tab in this change.
- Do not use an LLM to generate diagnoses.
- Do not store full prompt text, full tool outputs, or duplicate raw event streams in the summary table.

## Capabilities

### New Capabilities
- `runtime-quality-observability`: Backend-only per-turn runtime quality summaries, deterministic diagnosis classification, and developer analysis/export access.

### Modified Capabilities
- None.

## Impact

- **Backend crates**:
  - `xiaolin-protocol`: shared data model for `TurnQualitySummary`, diagnosis codes, severity, and evidence shape if exposed through internal APIs.
  - `xiaolin-session`: SQLite table and store/query methods for persisted turn quality summaries.
  - `xiaolin-agent`: turn lifecycle collection from runtime timings, tool rounds, query-loop repetition counters, usage, cache diagnostics, context/compaction events, abort/error paths.
  - `xiaolin-gateway`: optional developer query/export endpoints if selected during implementation.
- **Data**: Adds a small derived SQLite table. Raw truth remains in `event_log`, runtime state, and cost/token records; summaries are re-computable cache records.
- **APIs**: No public user-facing UI requirement. Any API added is developer/diagnostics oriented.
- **Security/privacy**: Summary records must avoid prompt bodies, full tool outputs, secrets, and high-cardinality free-form labels.
- **Dependencies**: No new external observability/APM dependency.
