## ADDED Requirements

### Requirement: Backend emits reasoning_delta events
The backend SHALL emit `AgentStep::ReasoningDelta` events in real-time as reasoning content arrives from the LLM provider, using the lossy step channel.

#### Scenario: Model streams reasoning content
- **WHEN** the LLM provider returns a delta with `reasoning_content` field
- **THEN** the system SHALL emit `ReasoningDelta { turn_id, content }` via step_tx within the same stream processing loop iteration

#### Scenario: Reasoning with backpressure
- **WHEN** the step channel is full (backpressure)
- **THEN** the reasoning delta MAY be dropped (lossy) without affecting agent correctness

### Requirement: Frontend displays reasoning in a collapsible block
The frontend SHALL render reasoning content in a collapsible UI block that defaults to collapsed state showing only a summary indicator.

#### Scenario: Reasoning arrives during streaming
- **WHEN** `reasoning_delta` events arrive and no content/tool output has started
- **THEN** the UI SHALL show a collapsed block with text "思考中..." and a live token/character count

#### Scenario: User expands reasoning block
- **WHEN** the user clicks the reasoning block header
- **THEN** the block SHALL expand to show the full reasoning text in monospace font with reduced contrast

#### Scenario: Content or tool output begins
- **WHEN** `content_delta` or `tool_executing` event arrives after reasoning
- **THEN** the reasoning block SHALL automatically collapse (if expanded) and stop updating

### Requirement: Reasoning block absent for non-reasoning models
The reasoning block SHALL NOT appear if zero `reasoning_delta` events are received during a turn.

#### Scenario: Model without reasoning capability
- **WHEN** no `reasoning_delta` events arrive and content_delta starts directly
- **THEN** no reasoning block SHALL be rendered
