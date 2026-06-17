## ADDED Requirements

### Requirement: edit_file SHALL return structured error information
When `edit_file` fails, the tool result SHALL include a JSON object with `errorCode` (integer), `errorType` (string), and `recovery_hint` (string) alongside the human-readable error message.

#### Scenario: old_string not found
- **WHEN** `edit_file` is called with an `old_string` that does not exist in the target file
- **THEN** the result SHALL include `errorCode: 8`, `errorType: "not_found"`, and a `recovery_hint` suggesting to re-read the file or use `search_in_files`

#### Scenario: Multiple matches without replace_all
- **WHEN** `edit_file` is called with an `old_string` that matches multiple locations and `replace_all` is false
- **THEN** the result SHALL include `errorCode: 9`, `errorType: "ambiguous_match"`, and a `recovery_hint` suggesting to add more context or use `replace_all`

#### Scenario: File modified since last read
- **WHEN** `edit_file` is called but the file has been modified since the agent last read it
- **THEN** the result SHALL include `errorCode: 7`, `errorType: "stale_content"`, and a `recovery_hint` to re-read the file

#### Scenario: File not found
- **WHEN** `edit_file` is called on a path that does not exist
- **THEN** the result SHALL include `errorCode: 4`, `errorType: "file_not_found"`, and path suggestions if available

#### Scenario: No-op edit
- **WHEN** `edit_file` is called with `old_string` identical to `new_string`
- **THEN** the result SHALL include `errorCode: 1`, `errorType: "no_change"`, and a `recovery_hint` to modify the new_string
