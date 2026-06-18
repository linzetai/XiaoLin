## ADDED Requirements

### Requirement: System prompt SHALL have static/dynamic boundary
The system prompt assembly SHALL insert a boundary marker between static content (role, coding rules, tool routing) and dynamic content (MCP instructions, environment, memory). This enables LLM API prompt caching for the static portion.

#### Scenario: Boundary marker present
- **WHEN** the system prompt is assembled for an LLM API call
- **THEN** the prompt SHALL contain a `cache_control` breakpoint between static and dynamic sections

#### Scenario: Static content stable across turns
- **WHEN** two consecutive turns occur without configuration changes
- **THEN** the static portion of the system prompt SHALL be byte-identical, enabling prompt cache hits

### Requirement: Tool definitions SHALL be session-stable
Tool schema definitions sent to the LLM SHALL be deterministic within a session — the same set of active tools SHALL produce the same JSON output across turns. This prevents prompt cache invalidation due to non-semantic ordering or formatting differences.

#### Scenario: Stable ordering
- **WHEN** tool definitions are serialized for two consecutive LLM calls with the same tool set
- **THEN** the serialized output SHALL be identical
