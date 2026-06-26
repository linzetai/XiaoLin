## ADDED Requirements

### Requirement: Single model-visible projection pipeline
The system SHALL route model-visible tool output representation through a single `ContextProjectionPipeline` before LLM calls.

#### Scenario: Large output represented by manifest
- **WHEN** a large output asset exists for a tool call
- **THEN** the model-visible context SHALL contain a bounded projection or manifest referencing the output handle, not the full raw output

#### Scenario: Projection owns budget decisions
- **WHEN** context is assembled under token pressure
- **THEN** the projection pipeline SHALL decide which manifests, summaries, recalled excerpts, and recent messages are included

### Requirement: Adaptive projection policy
The projection pipeline SHALL choose a projection level from output size, output type, context pressure, recency, and task relevance; it SHALL NOT replace all tool outputs with handle-only manifests.

#### Scenario: Default output size classes
- **WHEN** no custom projection thresholds are configured
- **THEN** the system SHALL classify output as small when it is at most 8,000 UTF-8 bytes, at most 200 lines, and estimated at no more than 2,000 tokens; medium when it is larger than small but at most 50,000 UTF-8 bytes, at most 1,000 lines, and estimated at no more than 12,500 tokens; and large when it exceeds any medium threshold

#### Scenario: Output size thresholds are configurable
- **WHEN** an operator configures small or medium projection thresholds
- **THEN** the projection pipeline SHALL use the configured thresholds while preserving the invariant that small outputs are eligible for full inline projection and large outputs require a recoverable asset-backed projection

#### Scenario: Context pressure can downgrade class treatment
- **WHEN** an output is small by size but including it would exceed the active projection budget or context pressure is high
- **THEN** the projection pipeline MAY treat it as medium for model-visible projection, but SHALL preserve the ability to recover the full output if it is not included inline

#### Scenario: Recent critical errors override size-only downgrade
- **WHEN** a recent failed command or test output is small by size and contains actionable error text
- **THEN** the projection pipeline SHALL keep the actionable error text inline unless doing so would make the LLM request exceed the hard context limit

#### Scenario: Small output remains inline
- **WHEN** a tool output is within the configured small-output threshold and context pressure is low or moderate
- **THEN** the model-visible context SHALL include the full output inline and MAY include an output handle as an optional recovery affordance

#### Scenario: Medium relevant output keeps key content
- **WHEN** a tool output is larger than the small-output threshold but still within the configured medium-output threshold and it is recent or task-relevant
- **THEN** the projection SHALL include enough original content or typed key excerpts for the model to make the next decision without requiring an immediate recall call

#### Scenario: Large output uses summary plus handle
- **WHEN** a tool output is too large for direct inclusion or context pressure is high
- **THEN** the projection SHALL include a typed summary, key excerpts selected for the output type, and a handle for precise recall

#### Scenario: Handle-only manifest is last resort
- **WHEN** the projection budget cannot fit useful typed excerpts for a large output
- **THEN** the projection MAY use a handle-only manifest, but it SHALL include enough metadata for the agent to choose `output_search`, `output_read`, or `output_tail` precisely

### Requirement: No negative optimization for bounded outputs
The projection pipeline SHALL avoid adding recall-tool round trips for outputs that can be safely and usefully included in the current context budget.

#### Scenario: Short failed command output is visible
- **WHEN** a failed shell or test command returns a bounded error output that fits within the projection budget
- **THEN** the projection SHALL include the actionable error text inline rather than requiring the agent to call a recall tool

#### Scenario: Small search result is visible
- **WHEN** a search tool returns a bounded number of matches that fit within the projection budget
- **THEN** the projection SHALL include those matches inline rather than replacing them with a handle-only manifest

### Requirement: Destructive compaction is not duplicated
Post-tool processing, pre-query compaction, content filtering, and hard-fit logic SHALL NOT repeatedly destructively compact the same asset-backed output.

#### Scenario: Asset-backed output skips legacy truncation
- **WHEN** a tool result has already been projected from a `ToolOutputAsset`
- **THEN** legacy truncation and microcompact layers SHALL treat it as bounded and SHALL NOT apply additional string truncation

#### Scenario: Hard fit drops recoverable projection first
- **WHEN** final context fitting must remove content
- **THEN** it SHALL prefer dropping recoverable projections before dropping current task instructions, active user input, or non-recoverable state

### Requirement: Auto-compact preserves handles
LLM auto-compact SHALL preserve output handles and the task state associated with those handles rather than attempting to summarize large raw tool output bodies as the sole fact source.

#### Scenario: Summary includes output handle
- **WHEN** auto-compact summarizes history that referenced a tool output asset
- **THEN** the compacted context SHALL retain the output handle and a concise statement of why it may be relevant

### Requirement: Prompt-cache-stable projection
Model-visible projection text SHALL be deterministic for the same asset metadata and projection configuration.

#### Scenario: Deterministic manifest formatting
- **WHEN** the same asset is projected twice with the same configuration
- **THEN** the manifest text SHALL be byte-identical except for explicitly excluded volatile fields that are not included in model-visible context
