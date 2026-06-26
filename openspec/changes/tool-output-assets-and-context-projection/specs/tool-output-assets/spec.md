## ADDED Requirements

### Requirement: Raw tool output asset creation
The system SHALL create a session-scoped `ToolOutputAsset` for every tool result whose raw output exceeds the configured inline threshold before any truncation, compaction, or model-visible projection is applied.

#### Scenario: Default asset creation threshold
- **WHEN** no custom asset creation threshold is configured
- **THEN** the system SHALL create an asset for every output classified as medium or large by the projection size classes and SHALL NOT be required to create an asset for small output unless debug assetization is enabled

#### Scenario: Large output is stored before projection
- **WHEN** a tool returns output larger than the inline threshold
- **THEN** the system SHALL persist the complete raw output and SHALL generate a tool output handle before building the model-visible tool result

#### Scenario: Small output may remain inline
- **WHEN** a tool returns output at or below the inline threshold
- **THEN** the system MAY keep the output inline and SHALL still support storing it as an asset when output-asset debugging is enabled

### Requirement: Asset metadata and indexes
Each `ToolOutputAsset` SHALL record enough metadata and indexes to support recovery, auditing, projection, and cleanup without depending on the original `ChatMessage.content`.

#### Scenario: Asset metadata is recorded
- **WHEN** a tool output asset is created
- **THEN** it SHALL record session id, turn id, tool call id, tool name, argument digest, success status, content hash, byte count, line count, estimated token count, creation time, and projector kind

#### Scenario: Line and chunk indexes are available
- **WHEN** a text tool output asset is created
- **THEN** the system SHALL create line and chunk indexes that allow bounded reads by line range, byte range, or page

### Requirement: Resume-safe handles
Tool output handles SHALL remain resolvable after session reload as long as the session's output retention policy has not expired the asset.

#### Scenario: Handle resolves after resume
- **WHEN** a session is restored and its transcript contains an output handle
- **THEN** recall tools SHALL be able to resolve that handle to the original asset if it has not expired

#### Scenario: Expired handle is explicit
- **WHEN** a recall tool is invoked for an expired handle
- **THEN** the tool SHALL return a structured error explaining that the asset expired and SHALL NOT silently return partial or unrelated content

### Requirement: Session-scoped access control
Tool output assets SHALL be accessible only from the session that created them unless a future explicit sharing capability authorizes otherwise.

#### Scenario: Cross-session handle access denied
- **WHEN** a session attempts to recall an output handle created by another session
- **THEN** the system SHALL deny access with a structured authorization error

#### Scenario: Handles are non-guessable
- **WHEN** the system creates an output handle
- **THEN** the handle SHALL include sufficient entropy or an equivalent unguessable identifier so sequential guessing is not a viable access path

### Requirement: Cleanup policy
The system SHALL apply a bounded retention policy for output assets by session, workspace, and total storage size.

#### Scenario: Cleanup preserves live transcript references
- **WHEN** cleanup evaluates assets referenced by active or recently restorable transcripts
- **THEN** it SHALL preserve those assets unless the configured hard storage cap requires explicit expiration

#### Scenario: Cleanup records expiration
- **WHEN** an asset is expired by retention policy
- **THEN** the system SHALL record expiration metadata so later recall attempts can distinguish expiration from missing or unauthorized handles
