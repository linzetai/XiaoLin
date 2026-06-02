## ADDED Requirements

### Requirement: Plan mode instructions as per-turn attachment
Plan mode instructions SHALL be injected as per-turn user-role attachment messages instead of system prompt sections. The attachment SHALL be invisible to the user in the UI.

#### Scenario: First turn in Plan mode
- **WHEN** agent enters Plan mode and starts first turn
- **THEN** a full plan mode attachment (~800 tokens) is injected into the message list
- **THEN** the attachment includes plan file path, read-only constraints, and exit instructions

#### Scenario: Subsequent turns with throttling
- **WHEN** agent is in Plan mode and has completed N turns since last attachment
- **AND** N < turns_between (default 5)
- **THEN** no attachment is injected

#### Scenario: Sparse reminder
- **WHEN** agent is in Plan mode and turns_between threshold is reached
- **AND** attachment count since last full reminder is not divisible by full_every_n
- **THEN** a sparse reminder (~100 tokens) is injected

#### Scenario: Full reminder cycle
- **WHEN** agent is in Plan mode and attachment count since start is at positions 1, 6, 11...
- **THEN** a full plan mode attachment is injected

### Requirement: Mode transition state tracking
`ExecutionModeState` SHALL track mode transitions for attachment injection decisions.

#### Scenario: Plan mode entry tracking
- **WHEN** mode transitions from Agent to Plan
- **THEN** plan_turn_counter resets to 0
- **THEN** has_exited_plan is set to false

#### Scenario: Plan mode exit tracking
- **WHEN** mode transitions from Plan to Agent
- **THEN** has_exited_plan is set to true

#### Scenario: Plan mode reentry detection
- **WHEN** mode transitions from Agent to Plan
- **AND** has_exited_plan is true
- **THEN** a one-time reentry attachment is prepended before the regular plan attachment

### Requirement: Plan mode section removed from system prompt
The plan mode guidance section in `session_guidance_section()` SHALL be removed when mode-attachments are active, to avoid duplicate instructions.

#### Scenario: System prompt without plan instructions
- **WHEN** mode-attachments feature is enabled
- **AND** agent is in Plan mode
- **THEN** `session_guidance_section()` does not include the Plan mode guidance block
- **THEN** plan mode instructions are delivered exclusively through attachments
