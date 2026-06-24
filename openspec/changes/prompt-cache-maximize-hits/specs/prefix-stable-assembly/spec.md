## ADDED Requirements

### Requirement: System messages SHALL be ordered with stable content first
The message assembly pipeline SHALL place PromptEngine static sections as the first system message(s), before any gateway-injected dynamic content.

#### Scenario: Normal message assembly
- **WHEN** `build_messages()` is called and gateway has injected skills/MCP/paths prompts
- **THEN** the resulting messages array SHALL have PromptEngine static content at index 0 (System role), followed by gateway-injected system messages, followed by dynamic sections

#### Scenario: Gateway injections do not precede static prompt
- **WHEN** `inject_skills_prompt`, `inject_runtime_paths_prompt`, or `inject_mcp_tools_prompt` is called
- **THEN** these injections SHALL NOT use `insert(0)` to prepend before the PromptEngine static block

### Requirement: Per-turn dynamic content SHALL NOT be in system prefix
Content that changes every turn (git snapshot, code_context, evolution skills) SHALL be injected into the user message context rather than as system-role prefix messages.

#### Scenario: Git snapshot placement
- **WHEN** `collect_git_snapshot()` returns content for injection
- **THEN** the git snapshot SHALL be injected as a `<system_context>` block in the last user message, NOT as a System-role message prepended to the array

#### Scenario: Code context placement  
- **WHEN** `code_context_section()` produces output (CodeGraphCache has content)
- **THEN** the code context SHALL be placed after the cache breakpoint, not in the cached prefix region

#### Scenario: Evolution skills placement
- **WHEN** `inject_relevant_skills()` finds matching skills via semantic search
- **THEN** the evolution skills content SHALL be injected after the static system prefix, not before it

### Requirement: Tool definitions SHALL maintain session-stable ordering
Tool definitions serialized for the LLM SHALL be sorted deterministically by function name and produce identical output across turns when the tool set has not changed.

#### Scenario: Consecutive turns with same tool set
- **WHEN** two consecutive LLM calls have the same set of active tools
- **THEN** the serialized tool definitions SHALL be byte-identical

#### Scenario: Tool activation changes order
- **WHEN** `tool_search` activates a new tool between turns
- **THEN** the tool definitions SHALL include the new tool in its sorted position (expected cache miss for tools block)

### Requirement: Skills injection SHALL be session-stable by default
The gateway-level skills prompt SHALL be computed once at session start and cached for the session duration, invalidated only by explicit events (skill registry version change, config reload).

#### Scenario: Consecutive turns without skill changes
- **WHEN** no skill registry changes occur between turns
- **THEN** the skills prompt content SHALL be identical across turns

#### Scenario: Skill registry version change triggers refresh
- **WHEN** a new skill is loaded or an existing skill is disabled
- **THEN** the skills prompt cache SHALL be invalidated and recomputed on the next turn

### Requirement: MCP instructions section SHALL use event-driven invalidation
The `mcp_instructions` prompt section SHALL NOT be recomputed every turn (`cache_break: false`). Instead, it SHALL be invalidated via `invalidate_sections()` when MCP server connection state changes.

#### Scenario: MCP servers unchanged between turns
- **WHEN** no MCP server connects or disconnects between two LLM calls
- **THEN** the `mcp_instructions` section SHALL return the cached value without recomputation

#### Scenario: MCP server connects
- **WHEN** a new MCP server establishes connection
- **THEN** `invalidate_sections(&["mcp_instructions"])` SHALL be called, causing recomputation on the next turn
