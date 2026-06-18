## ADDED Requirements

### Requirement: suggest_path_under_cwd path correction
When a file path resolves to a location under the CWD's parent directory but not under CWD itself, the system SHALL attempt to correct the path by re-rooting it under CWD and suggest the corrected path if it exists.

#### Scenario: Agent uses path missing repo directory prefix
- **WHEN** the agent calls `read_file` with path `/tmp/src/lib.rs` but CWD is `/tmp/myrepo/` and `/tmp/myrepo/src/lib.rs` exists
- **THEN** the error message SHALL include "Did you mean /tmp/myrepo/src/lib.rs?"

#### Scenario: Correction path does not exist
- **WHEN** the corrected path also does not exist
- **THEN** the system SHALL fall back to `find_similar_files` suggestions

### Requirement: Enhanced FileNotFound error messages
All FileNotFound errors from `read_file` and `edit_file` SHALL include both `suggestPathUnderCwd` and `findSimilarFiles` suggestions, plus the current working directory.

#### Scenario: Error message includes CWD context
- **WHEN** `read_file` fails with FileNotFound
- **THEN** the error message SHALL include "Current working directory: {cwd}" to help the agent construct correct paths

#### Scenario: Combined suggestions in priority order
- **WHEN** both `suggest_path_under_cwd` and `find_similar_files` return results
- **THEN** the `suggest_path_under_cwd` result SHALL be shown first (higher priority), followed by `find_similar_files` results
