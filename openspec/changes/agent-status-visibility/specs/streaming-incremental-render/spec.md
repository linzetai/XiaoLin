## MODIFIED Requirements

### Requirement: Stream segments support reasoning type
The StreamSegment type system SHALL support a `reasoning` segment type for accumulating reasoning content during streaming.

#### Scenario: Reasoning delta arrives
- **WHEN** a `reasoning_delta` event is received during streaming
- **THEN** a segment of type `reasoning` SHALL be created or appended to in `segmentsRef`

#### Scenario: Segments serialization includes reasoning
- **WHEN** stream segments are persisted to message history on turn_end
- **THEN** reasoning segments SHALL be included with their accumulated content

### Requirement: Iteration boundary tracked in stream state
The streaming state SHALL track the current iteration number from `iteration_boundary` events.

#### Scenario: Iteration boundary event received
- **WHEN** an `iteration_boundary` event arrives with iteration N
- **THEN** `currentIteration` state SHALL be updated to N
- **AND** an iteration separator segment SHALL be inserted into the stream segments
