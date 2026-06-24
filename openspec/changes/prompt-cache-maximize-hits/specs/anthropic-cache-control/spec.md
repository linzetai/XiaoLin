## ADDED Requirements

### Requirement: Anthropic system field SHALL use content block array format
When sending requests to Anthropic's Messages API, the `system` field SHALL be formatted as an array of content blocks (not a plain string) to enable per-block `cache_control` annotations.

#### Scenario: Standard request with cache_control
- **WHEN** the system prompt contains both static and dynamic sections separated by `DYNAMIC_BOUNDARY`
- **THEN** the Anthropic request SHALL contain `system` as a JSON array with at least two text blocks: the static block with `cache_control: {"type": "ephemeral"}` and the dynamic block without `cache_control`

#### Scenario: Single system message without boundary
- **WHEN** the system prompt does not contain `DYNAMIC_BOUNDARY` (e.g., override_prompt or agent_prompt)
- **THEN** the entire system text SHALL be sent as a single content block with `cache_control: {"type": "ephemeral"}`

#### Scenario: Empty system prompt
- **WHEN** no system messages exist in the message array
- **THEN** the `system` field SHALL be omitted from the request (same as current behavior)

### Requirement: Static block SHALL be byte-identical across consecutive turns
The static content block sent to Anthropic SHALL produce byte-identical JSON between consecutive LLM calls within the same session, unless a cache-invalidating event occurs (tool activation, config change).

#### Scenario: Two consecutive turns without config changes
- **WHEN** two LLM calls occur within the same session without tool activation or config changes
- **THEN** the first `system` content block SHALL have identical `text` content in both requests

#### Scenario: Tool activation changes static content
- **WHEN** `tool_search` activates a deferred tool between turns
- **THEN** the static block MAY change (expected cache miss), and `CacheBreakDetector` SHALL report `ToolsChanged`

### Requirement: Anthropic response cache tokens SHALL be parsed correctly
The provider SHALL continue to parse `cache_read_input_tokens` and `cache_creation_input_tokens` from both streaming and non-streaming responses.

#### Scenario: Cache hit response
- **WHEN** Anthropic returns `usage.cache_read_input_tokens > 0`
- **THEN** `Usage.cache_read_tokens` SHALL reflect this value and `CostTracker` SHALL apply the discounted rate

#### Scenario: Cache creation response
- **WHEN** Anthropic returns `usage.cache_creation_input_tokens > 0` (first call with new prefix)
- **THEN** `Usage.cache_creation_tokens` SHALL reflect this value and `CostTracker` SHALL apply the creation surcharge rate
