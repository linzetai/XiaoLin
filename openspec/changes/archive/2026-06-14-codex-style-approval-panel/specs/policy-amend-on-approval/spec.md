## ADDED Requirements

### Requirement: ApprovalDecision has ApprovedWithPolicyAmend variant
The `ApprovalDecision` enum SHALL include a variant `ApprovedWithPolicyAmend { prefix: Vec<String> }` that represents the user's decision to approve and persist the command prefix as an ExecPolicy rule.

When included in `available_decisions`, the `prefix` field SHALL contain the auto-extracted suggested prefix (not empty). The frontend directly uses this prefix value when the user selects this option.

#### Scenario: Variant serializes correctly
- **WHEN** `ApprovalDecision::ApprovedWithPolicyAmend { prefix: vec!["npm".into()] }` is serialized
- **THEN** the JSON output is `{"decision": "approved_with_policy_amend", "prefix": ["npm"]}`

#### Scenario: Variant deserializes correctly
- **WHEN** JSON `{"decision": "approved_with_policy_amend", "prefix": ["cargo", "build"]}` is deserialized
- **THEN** the result is `ApprovalDecision::ApprovedWithPolicyAmend { prefix: vec!["cargo", "build"] }`

#### Scenario: available_decisions carries prefix in-place
- **WHEN** orchestrator builds `available_decisions` for a Medium-risk ShellCommand `npm install`
- **THEN** the list includes `ApprovedWithPolicyAmend { prefix: vec!["npm"] }` with the prefix pre-filled

### Requirement: Orchestrator persists policy rule on ApprovedWithPolicyAmend
When the orchestrator receives `ApprovedWithPolicyAmend { prefix }`, it SHALL:
1. Add the prefix as a session rule via `PolicyEngine::add_session_rule()` for immediate effect
2. Persist the prefix to the project ExecPolicy file via `blocking_append_allow_prefix_rule()`
3. Allow the current tool call to proceed (same as `Approved`)

#### Scenario: Prefix persisted to disk
- **WHEN** user approves `npm install` with policy amend and prefix `["npm"]`
- **THEN** the project exec_policy file contains a rule allowing commands starting with `"npm"`
- **AND** subsequent `npm` commands are auto-approved without prompting

#### Scenario: Session rule takes immediate effect
- **WHEN** user approves with policy amend prefix `["cargo"]`
- **THEN** the next `cargo build` command in the same session is auto-approved by `PolicyEngine::evaluate()`

### Requirement: Policy amend only available for Medium-risk ShellCommand
The `ApprovedWithPolicyAmend` option SHALL only be included in `available_decisions` when:
- The action is `PendingAction::ShellCommand`
- The inferred `risk_level` is `Medium` (not `High`)

#### Scenario: Medium risk shell shows amend option
- **WHEN** a ShellCommand with `risk_level: Medium` triggers approval
- **THEN** `available_decisions` includes `ApprovedWithPolicyAmend`

#### Scenario: High risk shell hides amend option
- **WHEN** a ShellCommand with `risk_level: High` (e.g., `sudo rm -rf /`) triggers approval
- **THEN** `available_decisions` does NOT include `ApprovedWithPolicyAmend`

#### Scenario: FileWrite hides amend option
- **WHEN** a FileWrite action triggers approval
- **THEN** `available_decisions` does NOT include `ApprovedWithPolicyAmend`

### Requirement: Prefix extraction from command
The orchestrator SHALL extract the command prefix from `PendingAction::ShellCommand.command` by taking the first token (space-split). This prefix is placed directly into the `ApprovedWithPolicyAmend` variant in `available_decisions`.

#### Scenario: Single-word command prefix
- **WHEN** command is `npm install lodash`
- **THEN** `available_decisions` includes `ApprovedWithPolicyAmend { prefix: ["npm"] }`

#### Scenario: Multi-token command
- **WHEN** command is `cargo build --release`
- **THEN** `available_decisions` includes `ApprovedWithPolicyAmend { prefix: ["cargo"] }`
