## ADDED Requirements

### Requirement: File read projector
The system SHALL project file-read outputs with file identity, requested range, actual line range, content size, freshness metadata when available, and a bounded representative excerpt.

#### Scenario: File read manifest
- **WHEN** a `read_file` or equivalent output is assetized
- **THEN** the projection SHALL include path, line range, total lines or bytes, handle, and recall guidance for reading additional lines

### Requirement: Search result projector
The system SHALL project search outputs with query metadata, match counts, matched file distribution, representative matches, overflow information, and handle-based continuation guidance.

#### Scenario: Large search output manifest
- **WHEN** a search tool returns more matches than fit in the projection budget
- **THEN** the projection SHALL include total match count when available, top matched files, representative match lines, omitted count, and `output_search` guidance

### Requirement: Shell and test log projector
The system SHALL project shell and test outputs with command identity, exit status, duration when available, failure blocks, warning/error hints, tail excerpt, and handle-based recovery guidance.

#### Scenario: Failed test output
- **WHEN** a test command exits unsuccessfully with large output
- **THEN** the projection SHALL include exit status, detected failure blocks or error lines, tail excerpt, and the output handle

### Requirement: Directory listing projector
The system SHALL project directory or tree listings with root path, entry counts, representative entries, omitted counts, and paging guidance.

#### Scenario: Large directory listing
- **WHEN** a directory listing exceeds the projection budget
- **THEN** the projection SHALL include root path, count summary, representative entries, output handle, and page retrieval guidance

### Requirement: JSON and default projector
The system SHALL provide a generic projector for MCP, browser, JSON, and unknown large outputs that summarizes structure without losing raw recoverability.

#### Scenario: Large JSON output
- **WHEN** a tool returns large JSON or structured data without a specialized projector
- **THEN** the projection SHALL include top-level shape, key counts or array counts when available, representative fields, output handle, and recall guidance
