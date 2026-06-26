## ADDED Requirements

### Requirement: Raw recovery tests
The implementation SHALL include tests proving that raw output can be recovered exactly after projection, compaction, and session resume.

#### Scenario: Recover exact line after compaction
- **WHEN** a large output is projected, compacted, and then recalled by line range
- **THEN** the recalled content SHALL match the original raw output for that range

### Requirement: Repeated compaction regression tests
The implementation SHALL include regression tests proving that the same asset-backed output is not repeatedly destructively compacted by multiple layers.

#### Scenario: One output one projection decision
- **WHEN** a large output passes through post-tool processing, pre-query projection, content filtering, and hard-fit checks
- **THEN** the output SHALL retain a single recoverable projection path without nested truncation markers

### Requirement: Agent behavior quality benchmarks
The implementation SHALL include integration or benchmark scenarios that measure whether output assets improve agent behavior.

#### Scenario: Large search does not require rerun
- **WHEN** the agent needs details from a large search result after context projection
- **THEN** the benchmark SHALL verify the agent can use recall tools to locate relevant matches without rerunning the original search command

#### Scenario: Compact continuation remains correct
- **WHEN** a long task compacts context after using large tool outputs
- **THEN** the benchmark SHALL verify the agent can continue the task using preserved handles and recalled excerpts

#### Scenario: Bounded output does not add recall round trip
- **WHEN** a task uses only small or medium tool outputs that fit the configured projection budget
- **THEN** the benchmark SHALL verify the asset/projection path does not increase required recall tool calls compared with direct inline output

#### Scenario: Large output recall replaces rerun
- **WHEN** a task needs additional details from a large projected output
- **THEN** the benchmark SHALL verify recall calls replace rerunning the original expensive tool and total repeated original-tool calls decrease

#### Scenario: Recall loop does not occur
- **WHEN** the agent works with a large output handle across multiple turns
- **THEN** the benchmark SHALL verify it does not repeatedly request the same broad pages or ranges after recall-loop guardrails are triggered

### Requirement: Performance gates
The implementation SHALL include performance coverage for ingesting, indexing, searching, and paging multi-megabyte outputs.

#### Scenario: Multi-megabyte output operations are bounded
- **WHEN** a multi-megabyte output is stored and indexed
- **THEN** the performance test SHALL assert bounded ingestion, search, and page-read latency according to documented thresholds

### Requirement: Prompt cache regression gate
The implementation SHALL verify that model-visible manifests do not introduce avoidable byte instability into stable prompt prefixes.

#### Scenario: Stable manifest bytes
- **WHEN** the same output asset metadata is projected across repeated context assemblies
- **THEN** the projection SHALL remain byte-stable and SHALL NOT include volatile timestamps or local filesystem blob paths in model-visible text
