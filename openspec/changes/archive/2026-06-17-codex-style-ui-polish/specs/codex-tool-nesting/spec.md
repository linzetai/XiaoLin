## ADDED Requirements

### Requirement: Compact inline rendering for read-only tools
Tool calls with category "read" or "search" that appear in a consecutive sequence following a reasoning segment (with no text segment in between) SHALL render in a compact single-line format: icon (12px) + truncated path/query (max 40 chars), without border or expand capability.

#### Scenario: File read after reasoning renders inline
- **WHEN** a `file_read` tool call immediately follows a reasoning segment in the stream
- **THEN** it renders as a single line with FileText icon + file path, max height ~24px, no border

#### Scenario: Non-read tool renders normally
- **WHEN** a `shell` or `write` category tool follows reasoning
- **THEN** it renders as a full StepIndicator card as before

### Requirement: Compact tools do not affect grouping logic
The segment grouping algorithm (`groupConsecutiveSegments`) SHALL remain unchanged. Compact rendering is a presentation-layer decision only.

#### Scenario: Grouping produces same output
- **WHEN** segments are processed by `groupConsecutiveSegments`
- **THEN** the returned `GroupedSegment[]` array is identical regardless of compact rendering

### Requirement: Compact tools show status
Even in compact mode, tools SHALL indicate completion status via a small colored dot (green for success, red for error) at the end of the line.

#### Scenario: Completed read tool shows green dot
- **WHEN** a compact-rendered `file_read` has status "success"
- **THEN** a 4px green dot appears at the right end of the line
