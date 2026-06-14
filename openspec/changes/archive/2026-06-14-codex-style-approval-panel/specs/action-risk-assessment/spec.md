## ADDED Requirements

### Requirement: ActionRiskLevel enum exists in protocol
The system SHALL define an `ActionRiskLevel` enum in `xiaolin-protocol/src/approval.rs` with variants `Low`, `Medium`, `High`, serialized as snake_case.

#### Scenario: Enum serializes correctly
- **WHEN** `ActionRiskLevel::High` is serialized to JSON
- **THEN** the output is `"high"`

#### Scenario: Enum deserializes correctly
- **WHEN** JSON string `"low"` is deserialized
- **THEN** the result is `ActionRiskLevel::Low`

### Requirement: ApprovalRequired event carries risk_level
The `AgentEvent::ApprovalRequired` variant SHALL include a field `risk_level: Option<ActionRiskLevel>`. When `None`, it SHALL be omitted from serialized JSON.

#### Scenario: risk_level present in event
- **WHEN** orchestrator emits `ApprovalRequired` with `risk_level: Some(ActionRiskLevel::High)`
- **THEN** the serialized JSON includes `"risk_level": "high"`

#### Scenario: risk_level absent in event
- **WHEN** orchestrator emits `ApprovalRequired` with `risk_level: None`
- **THEN** the serialized JSON does NOT contain the key `"risk_level"`

### Requirement: Shell commands classified by risk rules
The orchestrator SHALL classify `PendingAction::ShellCommand` risk level using pattern matching on the command string:
- High: command contains `rm -rf`, `sudo`, `chmod 777`, `mkfs`, `dd if=`, `curl.*|.*sh`, `wget.*|.*sh`, or targets paths outside workspace
- Medium: all other shell commands

#### Scenario: Destructive command is High risk
- **WHEN** a shell command `rm -rf /important` is pending approval
- **THEN** `risk_level` is `High`

#### Scenario: Safe command is Medium risk
- **WHEN** a shell command `ls -la` is pending approval
- **THEN** `risk_level` is `Medium`

#### Scenario: Sudo command is High risk
- **WHEN** a shell command `sudo apt install foo` is pending approval
- **THEN** `risk_level` is `High`

### Requirement: File operations classified by path
The orchestrator SHALL classify `PendingAction::FileWrite` and `ApplyPatch` risk level by comparing the target path to the workspace root:
- High: path is outside the workspace directory
- Medium: path is inside the workspace directory

#### Scenario: File write inside workspace is Medium
- **WHEN** a file write to `/home/user/project/src/main.rs` is pending with workspace `/home/user/project`
- **THEN** `risk_level` is `Medium`

#### Scenario: File write outside workspace is High
- **WHEN** a file write to `/etc/hosts` is pending with workspace `/home/user/project`
- **THEN** `risk_level` is `High`

### Requirement: Network access classified as Medium
The orchestrator SHALL classify `PendingAction::NetworkAccess` as `Medium` risk level.

#### Scenario: Network access risk
- **WHEN** a network access to `api.example.com:443` is pending
- **THEN** `risk_level` is `Medium`
