## ADDED Requirements

### Requirement: Phase-aware status indicator replaces generic dots
The streaming message row SHALL display a textual phase indicator instead of anonymous bouncing dots, reflecting the current agent execution phase.

#### Scenario: Initial connection phase
- **WHEN** `turn_start` is received and no `content_delta`, `reasoning_delta`, or `tool_executing` has arrived for 300ms
- **THEN** the indicator SHALL display "连接模型中..." with a subtle animation

#### Scenario: Thinking phase
- **WHEN** `reasoning_delta` events are arriving but no `content_delta` or `tool_executing` has occurred
- **THEN** the indicator SHALL display "思考中..." with a rotating label animation

#### Scenario: Planning next step phase
- **WHEN** the last `tool_result` has been received and no new `content_delta` or `tool_executing` arrives for 300ms
- **THEN** the indicator SHALL display "规划下一步..." indicating the agent is preparing the next iteration

#### Scenario: Content streaming phase
- **WHEN** `content_delta` events are actively arriving
- **THEN** the phase indicator SHALL be hidden (markdown cursor provides sufficient feedback)

#### Scenario: Tool execution phase
- **WHEN** `tool_executing` event arrives
- **THEN** the phase indicator SHALL be hidden (StepIndicator provides feedback)

### Requirement: Phase indicator has debounce to prevent flicker
The phase indicator SHALL NOT appear for phases that last less than 300ms to avoid visual noise.

#### Scenario: Fast LLM response
- **WHEN** the first `content_delta` arrives within 300ms of `turn_start`
- **THEN** no "连接模型中..." phase SHALL be displayed
