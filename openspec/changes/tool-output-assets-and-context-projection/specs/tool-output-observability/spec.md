## ADDED Requirements

### Requirement: Output asset metrics
The runtime SHALL record metrics for output asset creation, raw size, projected size, and token savings.

#### Scenario: Asset creation metric
- **WHEN** a tool output asset is created
- **THEN** the runtime SHALL record tool name, raw byte count, raw token estimate, line count, and output handle category without recording sensitive raw content

#### Scenario: Projection savings metric
- **WHEN** an asset is projected into model-visible context
- **THEN** the runtime SHALL record projected token estimate and tokens saved relative to the raw estimate

### Requirement: Recall metrics
The runtime SHALL record recall tool usage and outcomes.

#### Scenario: Recall success metric
- **WHEN** `output_read`, `output_search`, `output_tail`, or `output_summary` succeeds
- **THEN** the runtime SHALL record recall tool name, handle category, returned token estimate, and latency

#### Scenario: Recall failure metric
- **WHEN** a recall tool fails due to missing, expired, or unauthorized handle
- **THEN** the runtime SHALL record the structured failure type without exposing raw content

### Requirement: Runtime quality fields
Turn quality summaries SHALL include output projection and recall indicators sufficient to evaluate optimization quality over time.

#### Scenario: Turn quality includes projection stats
- **WHEN** a turn completes after using tool output assets
- **THEN** the persisted quality summary SHALL include asset count, projected output tokens, raw output token estimate, recall count, and repeated-tool-call indicators where available

### Requirement: Repeated tool call avoidance signal
The runtime SHALL expose metrics that help determine whether output recall reduced unnecessary reruns.

#### Scenario: Same command rerun after projection
- **WHEN** the agent reruns a tool with the same or equivalent arguments after a recoverable output handle was available
- **THEN** the runtime SHALL record a repeated-tool-call signal linked to the prior output handle when correlation is possible
