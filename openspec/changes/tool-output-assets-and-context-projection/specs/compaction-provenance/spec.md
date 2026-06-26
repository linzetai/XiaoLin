## ADDED Requirements

### Requirement: Projection provenance labels
Every model-visible representation of tool output SHALL carry provenance describing how it was derived.

#### Scenario: Manifest provenance
- **WHEN** raw output is replaced by an asset manifest
- **THEN** the projection SHALL be labeled with provenance `asset_manifest`

#### Scenario: Recalled excerpt provenance
- **WHEN** output content is reintroduced through a recall tool
- **THEN** the recalled content SHALL be labeled with provenance `recalled_excerpt`

### Requirement: Provenance prevents repeated destructive compaction
Compaction and filtering layers SHALL inspect provenance before transforming tool-output content.

#### Scenario: Already projected content is skipped
- **WHEN** content provenance indicates `asset_manifest`, `typed_summary`, `recalled_excerpt`, or `llm_summary`
- **THEN** downstream compaction SHALL NOT apply destructive truncation to that content unless the hard context limit cannot otherwise be satisfied

### Requirement: Compaction transition records
The runtime SHALL record transitions from raw output to projection, typed summary, recalled excerpt, LLM summary, or hard-fit removal.

#### Scenario: Transition is recorded
- **WHEN** the projection pipeline changes the representation of a tool output
- **THEN** it SHALL record source provenance, destination provenance, raw token estimate, projected token estimate, and output handle

### Requirement: Compatibility with legacy markers
The system SHALL recognize legacy compaction markers and persisted-output tags during migration and map them to provenance where possible.

#### Scenario: Legacy persisted output marker
- **WHEN** a restored transcript contains a legacy `<persisted-output>` marker
- **THEN** the system SHALL treat it as a recoverable legacy projection and SHALL avoid applying nested truncation markers
