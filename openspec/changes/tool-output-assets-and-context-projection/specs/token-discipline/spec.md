## ADDED Requirements

### Requirement: Large tool outputs use recoverable projection
The system SHALL fit large tool outputs into context using recoverable output projections instead of unrecoverable string truncation.

#### Scenario: Context pressure projects large output
- **WHEN** tool output would exceed the model-visible context budget
- **THEN** the system SHALL store the raw output as a recoverable asset and include a bounded projection with a handle in context

#### Scenario: Hard context fit preserves recovery
- **WHEN** hard context fitting removes a tool-output projection
- **THEN** the system SHALL preserve enough handle or summary state for the agent to recover the original output unless the asset has expired

### Requirement: Inline-first token discipline for useful bounded output
The system SHALL prefer direct inline inclusion for useful bounded tool output when it fits the context budget, because handle recall has a tool-call cost.

#### Scenario: Small output inline budget
- **WHEN** a tool output is classified as small by the default or configured projection size classes and including it does not exceed the active context budget
- **THEN** the system SHALL treat inline inclusion as the default representation for that output

#### Scenario: Inline output is cheaper than recall
- **WHEN** the full output fits within the configured inline budget and does not threaten the active context window
- **THEN** the system SHALL include the output inline instead of forcing handle recall

#### Scenario: Handle supplements inline content
- **WHEN** a bounded output is included inline and also stored as an asset
- **THEN** the handle SHALL be treated as a supplement for later recovery, not as a replacement for the currently useful information

### Requirement: Projection budget accounting
The system SHALL account for raw output tokens, projected output tokens, and tokens saved as separate values.

#### Scenario: Token accounting distinguishes raw and projected
- **WHEN** a large output is replaced by a manifest
- **THEN** token accounting SHALL report both the raw token estimate and the projected token estimate rather than treating the raw output as lost
