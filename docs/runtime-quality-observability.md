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
