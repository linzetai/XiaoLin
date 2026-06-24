## MODIFIED Requirements

### Requirement: CacheBreakDetector SHALL receive real system and tools hashes
The `CacheBreakDetector.pre_call_snapshot()` SHALL be called with the actual system prompt text hash and tool definition names hash before each LLM call, enabling accurate root-cause diagnosis of cache breaks.

#### Scenario: Pre-call snapshot with real data
- **WHEN** an LLM call is about to be made
- **THEN** `pre_call_snapshot` SHALL be called with a hash of all System-role message text concatenated and a hash of all tool definition names joined

#### Scenario: Cache break diagnosis accuracy
- **WHEN** the system prompt content changes between turns
- **THEN** `post_call_analyze` SHALL report `BreakCause::SystemPromptChanged` (not `Unknown`)

#### Scenario: Tools change diagnosis accuracy
- **WHEN** the tool set changes between turns (e.g., deferred tool activated)
- **THEN** `post_call_analyze` SHALL report `BreakCause::ToolsChanged` with correct prev/curr counts

### Requirement: Cache metrics SHALL be logged at appropriate levels
Cache hit/miss events SHALL be logged with sufficient detail for operational monitoring.

#### Scenario: Cache hit logging
- **WHEN** `effective_cache_read_tokens() > 0` in the LLM response
- **THEN** a `debug`-level log SHALL be emitted with `cache_read_tokens`, `prompt_tokens`, and calculated `cache_hit_pct`

#### Scenario: Cache break logging
- **WHEN** `post_call_analyze` detects a cache break
- **THEN** a `warn`-level log SHALL be emitted with the break cause, previous cache_read tokens, and current cache_read tokens
