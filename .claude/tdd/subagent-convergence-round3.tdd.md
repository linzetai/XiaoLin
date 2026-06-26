# Subagent Result Convergence — Round 3 Fixes

**Source:** Code review findings on `main` branch  
**Date:** 2026-06-26  
**Status:** GREEN — all tests pass, full workspace compiles

## User Journeys

| # | Journey |
|---|---------|
| 1 | As a system operator, I want failed subagent runs to round-trip through the DB, so that `subagent_get` after restart shows the actual error |
| 2 | As a tool executor, I want DB fallbacks to use async calls, so tool execution doesn't panic on current-thread runtimes |
| 3 | As a parent agent, I want the `truncated` flag to survive GC/restarts, so I can trust the contract field |
| 4 | As a maintainer, I want `transcript_json` semantics to be unambiguous |
| 5 | As a subagent (any type), I want the output convergence contract regardless of my type |

## Test Specification

| # | What is guaranteed | Test | Result | Evidence |
|---|--------------------|------|--------|----------|
| 1 | `build_db_row` writes `"failed"` not `"failed(\"timeout...\")"` for Failed status; error goes into `result` | `build_db_row_failed_status_uses_failed_not_debug_format` | PASS | `cargo test -p xiaolin-agent -- build_db_row` |
| 2 | `truncated` flag survives `build_db_row` → `SubAgentRunRow` → `From` round-trip | `build_db_row_roundtrip_preserves_truncated_flag` | PASS | same command |
| 3 | `transcript_json` is a file path (or None), not JSON content | `build_db_row_transcript_json_stores_path_not_content` | PASS | same command |

**RED evidence:** 2 of 3 new tests failed before implementation — `build_db_row_failed_status` produced `failed("timeout after 60s")` and `truncated` was always `false` after DB round-trip.

**GREEN evidence:** all 3 pass after fixes.

## Changes Summary

### High 1: Failed status serialization
- `build_db_row` status field now uses explicit match (pending/running/completed/cancelled/failed) instead of `format!("{:?}")`
- Error message stored in `result` column for `Failed` status
- **File:** `crates/xiaolin-agent/src/subagent_manager.rs:105-113`

### High 2: Async DB fallback (no block_in_place)
- Added `get_run_async()` and `list_runs_async()` async methods
- Synchronous `get_run()`/`list_runs()` now memory-only
- All tool execute paths (`subagent_get`, `subagent_list`, `wait_agent`, `send_message`) now use `*_async` variants
- **Files:** `crates/xiaolin-agent/src/subagent_manager.rs:1318-1400`, `crates/xiaolin-agent/src/subagent.rs`

### Medium 3: truncated flag persistence
- `truncated` encoded into `token_usage_json` as `{"truncated": true}` (no schema migration needed)
- `From<SubAgentRunRow>` restores it on DB fallback
- **Files:** `crates/xiaolin-agent/src/subagent_manager.rs:116-132`, `crates/xiaolin-session/src/models.rs:122-165`

### Medium 4: transcript_json semantics
- Field comment updated from "JSON-serialized sidechain transcript" to "Path to the sidechain transcript file (legacy column name)"
- **File:** `crates/xiaolin-session/src/models.rs:106`

### Low 5: Convergence contract for all subagent types
- Contract now appended regardless of whether `def` exists (always applies to both branches of the agent_config match)
- **File:** `crates/xiaolin-agent/src/subagent.rs:563-582`

## Full Test Results

```
subagent + subagent_manager:  30 passed, 0 failed
session models + subagent:     4 passed, 0 failed
protocol events:              16 passed, 0 failed
Total:                        53 passed, 0 failed (plus 3 new build_db_row tests)
Workspace:                    compiles clean
```

## Known Gaps / Untested
- `get_run_async` DB fallback path not tested because it requires a SQLite-backed `SessionStore` — integration level, deferred
- `list_runs_async` same as above
- `wait_agent` loop polling with DB fallback — requires multi-turn integration test
