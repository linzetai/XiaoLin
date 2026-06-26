## ADDED Requirements

### Requirement: Output read tool
The system SHALL provide a built-in `output_read` recall tool that reads a bounded portion of a tool output asset by handle.

#### Scenario: Read line range
- **WHEN** the agent calls `output_read` with a valid handle and line range
- **THEN** the tool SHALL return only that line range, line numbers, asset metadata, and whether more content exists before or after the range

#### Scenario: Read page
- **WHEN** the agent calls `output_read` with a valid handle and page identifier
- **THEN** the tool SHALL return the requested page and stable pagination metadata

#### Scenario: Unbounded read rejected
- **WHEN** the agent calls `output_read` without a line range, byte range, page, or equivalent bounded selector
- **THEN** the tool SHALL reject the request with structured guidance to choose a bounded range, page, search, or tail operation

### Requirement: Output search tool
The system SHALL provide a built-in `output_search` recall tool that searches within a stored output asset without rerunning the original tool.

#### Scenario: Search with context lines
- **WHEN** the agent calls `output_search` with a valid handle, pattern, and context line count
- **THEN** the tool SHALL return matching line numbers with bounded surrounding context from the stored asset

#### Scenario: Search result budget
- **WHEN** matches exceed the result budget
- **THEN** the tool SHALL return the first bounded result set, total match count when available, and continuation guidance

### Requirement: Output tail tool
The system SHALL provide a built-in `output_tail` recall tool for retrieving the ending lines of stored output.

#### Scenario: Tail failed command output
- **WHEN** the agent calls `output_tail` for a shell or test output handle
- **THEN** the tool SHALL return the requested number of ending lines and the original command status metadata

### Requirement: Output summary tool
The system SHALL provide a built-in `output_summary` recall tool that returns a typed summary generated from the asset without exposing the full raw output.

#### Scenario: Typed summary by mode
- **WHEN** the agent calls `output_summary` with a valid handle and summary mode
- **THEN** the tool SHALL return a bounded summary using the asset's typed projector or a generic fallback

### Requirement: Recall tool permissions
All recall tools SHALL validate handle ownership and SHALL return structured errors for unauthorized, missing, expired, or unsupported assets.

#### Scenario: Unauthorized recall denied
- **WHEN** a recall tool is called with a handle outside the current session
- **THEN** the tool SHALL deny access and SHALL NOT reveal whether the target content exists outside the allowed scope

### Requirement: Recall results are bounded and non-recursive
Recall tools SHALL return bounded excerpts with explicit range metadata and SHALL NOT emit outputs that require downstream truncation as the normal control mechanism.

#### Scenario: Recall result includes range state
- **WHEN** a recall tool returns content from an asset
- **THEN** the result SHALL include returned line or byte range, total lines or bytes when available, `has_before`, `has_after`, and continuation metadata when more content exists

#### Scenario: Recall output fits configured cap
- **WHEN** a recall tool prepares a response
- **THEN** the response SHALL fit within the recall tool's configured max lines or max bytes before it reaches generic tool-output truncation layers

#### Scenario: Recalled excerpt is not re-truncated
- **WHEN** a recall result enters the model-visible context with `recalled_excerpt` provenance
- **THEN** normal post-tool, pre-query, and content-filter processing SHALL treat it as bounded content and SHALL NOT add another truncation marker

### Requirement: Recall loop prevention
The system SHALL provide guardrails that prevent agents from repeatedly paging or rereading the same handle without narrowing the information need.

#### Scenario: Repeated same-range recall is detected
- **WHEN** the agent requests the same handle and same range repeatedly in a turn or adjacent turns
- **THEN** the runtime SHALL surface a bounded guidance response or reuse the prior excerpt instead of returning duplicate large content

#### Scenario: Linear paging is redirected to search
- **WHEN** the agent pages through the same large output repeatedly without a narrowing pattern
- **THEN** the runtime SHALL provide guidance to use `output_search`, `output_tail`, or a narrower range before continuing broad paging
