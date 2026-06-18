## ADDED Requirements

### Requirement: File tools SHALL suggest similar paths on NotFound
When a file operation targets a path that does not exist, the tool SHALL search for files with the same basename under the workspace root and include up to 3 suggestions in the error message.

#### Scenario: Relative path with wrong prefix
- **WHEN** `read_file` is called with `src/main.rs` but the file exists at `/tmp/workspace123/src/main.rs`
- **THEN** the error message SHALL include `"Did you mean: /tmp/workspace123/src/main.rs?"` (or similar suggestion)

#### Scenario: No similar files found
- **WHEN** `read_file` is called with `nonexistent.rs` and no file with that basename exists in workspace
- **THEN** the error message SHALL state the file was not found without suggestions

#### Scenario: Multiple similar files
- **WHEN** `read_file` is called with a filename that matches multiple files in different directories
- **THEN** the error message SHALL list at most 3 suggestions sorted by path similarity

### Requirement: Path search SHALL be bounded
The similar-file search SHALL be limited to 3 directory levels depth and SHALL complete within 100ms for typical project sizes (< 10K files).

#### Scenario: Large project performance
- **WHEN** the workspace contains 10,000 files across nested directories
- **THEN** the similar-file search SHALL complete within 100ms
