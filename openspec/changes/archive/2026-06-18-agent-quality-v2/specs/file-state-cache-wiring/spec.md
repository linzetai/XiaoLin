## ADDED Requirements

### Requirement: FileStateCache SHALL be initialized and scoped per turn
The agent runtime SHALL create an `Arc<FileStateCache>` instance during turn setup and scope it via `with_file_state_cache` for the entire turn execution, enabling read dedup in all file tools.

#### Scenario: Cache initialized on turn start
- **WHEN** a new agent turn begins via `execute_unified`
- **THEN** a `FileStateCache` instance SHALL be created and scoped via `with_file_state_cache` before any tool execution occurs

#### Scenario: Read dedup active within a turn
- **WHEN** `read_file` is called for a file that has already been read in the same turn and the file content has not changed
- **THEN** the tool SHALL return a `FILE_UNCHANGED_STUB` message instead of re-reading the full file content

### Requirement: FileStateCache SHALL update on write operations
After `write_file` or `edit_file` successfully modifies a file, the cache SHALL be updated with the new content hash and mtime so subsequent stale checks reflect the latest state.

#### Scenario: Cache updated after edit
- **WHEN** `edit_file` successfully modifies `src/main.rs`
- **AND** `read_file` is called for `src/main.rs` again in the same turn
- **THEN** `read_file` SHALL return the updated content (not stale cached version)
