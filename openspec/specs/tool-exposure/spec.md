## ADDED Requirements

### Requirement: Tool self-declares exposure level
Each tool SHALL declare its exposure level via `Tool::exposure()` trait method, returning `ToolExposure::Direct` (default) or `ToolExposure::Deferred`.

#### Scenario: Default exposure is Direct
- **WHEN** a tool does not override `exposure()`
- **THEN** `exposure()` returns `ToolExposure::Direct`

#### Scenario: Deferred tool declaration
- **WHEN** `ExitPlanModeTool` implements `exposure()` returning `ToolExposure::Deferred`
- **THEN** the tool is excluded from `eager_definitions()` by default

### Requirement: ToolProfile promotes and demotes tools based on mode
The system SHALL support `ToolProfile` structs that define promote (Deferred→Direct) and demote (Direct→hidden) rules. Each `ExecutionMode` SHALL have a predefined profile.

#### Scenario: Plan mode promotes exit_plan_mode
- **WHEN** current mode is `ExecutionMode::Plan`
- **THEN** `ToolProfile::for_mode(Plan)` includes `exit_plan_mode` in `promote` list
- **THEN** `definitions_with_profile(profile)` includes `exit_plan_mode` in the result

#### Scenario: Plan mode demotes enter_plan_mode
- **WHEN** current mode is `ExecutionMode::Plan`
- **THEN** `enter_plan_mode` is excluded from `definitions_with_profile(profile)` result

#### Scenario: Agent mode uses default profile
- **WHEN** current mode is `ExecutionMode::Agent`
- **THEN** `ToolProfile::for_mode(Agent)` returns a default profile with no promote/demote overrides

### Requirement: ToolRegistry provides profile-aware definitions
`ToolRegistry` SHALL expose a `definitions_with_profile(&ToolProfile)` method that returns tool definitions filtered according to both `Tool::exposure()` and the profile's promote/demote rules.

#### Scenario: Profile promotes a deferred tool
- **WHEN** tool `exit_plan_mode` has `exposure() == Deferred`
- **AND** profile.promote contains `"exit_plan_mode"`
- **THEN** `definitions_with_profile(profile)` includes `exit_plan_mode`

#### Scenario: Profile demotes an eager tool
- **WHEN** tool `enter_plan_mode` has `exposure() == Direct`
- **AND** profile.demote contains `"enter_plan_mode"`
- **THEN** `definitions_with_profile(profile)` excludes `enter_plan_mode`

### Requirement: AgentToolsConfig.profile connects to predefined profiles
The `AgentToolsConfig.profile` field SHALL resolve to a predefined `ToolProfile` when set (e.g. `"plan"`, `"readonly"`).

#### Scenario: Profile field resolves
- **WHEN** agent config has `tools.profile = "readonly"`
- **THEN** the sub-agent's tool definitions are filtered using the readonly profile
- **THEN** write/execute tools are excluded from the sub-agent's tool list
