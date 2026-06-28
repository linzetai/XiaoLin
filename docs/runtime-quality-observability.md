# Runtime Quality Observability

Runtime-quality diagnostics are backend-only developer data. XiaoLin records one
`turn_quality_summary` row per terminal turn in the unified SQLite database. The
summary is rule-based and does not call an LLM.

## JSON Export

Recent turns:

```bash
curl 'http://127.0.0.1:3000/api/v1/diagnostics/runtime-quality/turns?limit=100'
```

Session-scoped turns:

```bash
curl 'http://127.0.0.1:3000/api/v1/diagnostics/runtime-quality/turns?session_id=<session_id>&limit=100'
```

Single turn:

```bash
curl 'http://127.0.0.1:3000/api/v1/diagnostics/runtime-quality/turns/<session_id>/<turn_id>'
```

Portable export:

```bash
curl 'http://127.0.0.1:3000/api/v1/diagnostics/runtime-quality/export?limit=1000'
```

## SQL Examples

Slow turns:

```sql
SELECT session_id, turn_id, elapsed_ms, diagnosis_code, severity
FROM turn_quality_summary
ORDER BY elapsed_ms DESC
LIMIT 20;
```

Cache misses:

```sql
SELECT session_id, turn_id, model, input_tokens, cache_read_tokens, cache_hit_pct
FROM turn_quality_summary
WHERE diagnosis_code = 'cache_miss'
ORDER BY started_at DESC
LIMIT 20;
```

Context pressure:

```sql
SELECT session_id, turn_id, context_tokens, context_window, context_usage_pct
FROM turn_quality_summary
WHERE diagnosis_code = 'high_context'
ORDER BY context_usage_pct DESC
LIMIT 20;
```

Slowest tools:

```sql
SELECT session_id, turn_id, slowest_tool_name, slowest_tool_ms, tool_failures_total
FROM turn_quality_summary
WHERE slowest_tool_ms IS NOT NULL
ORDER BY slowest_tool_ms DESC
LIMIT 20;
```

`evidence_json` is intentionally bounded to metrics, flags, enum-like codes, and
tool names. It must not contain prompt bodies, user messages, full tool args, or
tool outputs.

## Output Projection Metrics (Phase 8)

The turn quality summary includes per-turn output asset and recall statistics.
These enable measuring projection effectiveness without recording raw output.

### Turn-Level Aggregates

```sql
-- Projection effectiveness per turn
SELECT session_id, turn_id,
       asset_count,
       raw_output_token_estimate,
       projected_output_tokens,
       CASE WHEN raw_output_token_estimate > 0
           THEN CAST(projected_output_tokens AS REAL) / CAST(raw_output_token_estimate AS REAL)
           ELSE NULL
       END AS projection_ratio,
       recall_count,
       repeated_tool_call_indicators
FROM turn_quality_summary
WHERE asset_count > 0
ORDER BY started_at DESC
LIMIT 100;
```

```sql
-- Sessions with most output assets created
SELECT session_id,
       COUNT(*) AS turns,
       SUM(asset_count) AS total_assets,
       SUM(raw_output_token_estimate) AS raw_tokens,
       SUM(projected_output_tokens) AS projected_tokens,
       SUM(recall_count) AS total_recalls,
       SUM(repeated_tool_call_indicators) AS total_repeated
FROM turn_quality_summary
GROUP BY session_id
ORDER BY total_assets DESC
LIMIT 20;
```

```sql
-- Recall tool usage: which turns relied on recall most
SELECT session_id, turn_id,
       tool_calls_total, asset_count, recall_count
FROM turn_quality_summary
WHERE recall_count > 0
ORDER BY recall_count DESC
LIMIT 20;
```

### Prometheus Metrics

| Metric | Description |
|--------|-------------|
| `xiaolin_output_assets_created_total{tool,size_class}` | Assets created by tool and size class |
| `xiaolin_output_asset_raw_bytes_total{tool}` | Raw bytes persisted per tool |
| `xiaolin_output_asset_projected_tokens_total{tool}` | Estimated tokens after projection |
| `xiaolin_output_asset_tokens_saved_total{tool}` | Estimated tokens saved |
| `xiaolin_output_recall_total{tool,status}` | Recall invocations by tool and status |
| `xiaolin_output_recall_tokens_total{tool}` | Tokens returned by recall |
| `xiaolin_output_recall_latency_ms{tool,quantile}` | Recall latency distribution |

### Privacy

All metrics are aggregate counters and estimates. Raw output, argument values,
file paths, and user messages are never recorded in metrics.
