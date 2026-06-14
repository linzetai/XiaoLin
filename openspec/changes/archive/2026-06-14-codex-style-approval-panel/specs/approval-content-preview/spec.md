## ADDED Requirements

### Requirement: FileWrite PendingAction carries optional content
`PendingAction::FileWrite` SHALL include a field `content: Option<String>`. When present, it contains a preview of the file content to be written, truncated to at most 2000 characters at a valid UTF-8 boundary.

#### Scenario: Short file content included in full
- **WHEN** a file write tool is called with content of 500 characters
- **THEN** `PendingAction::FileWrite.content` is `Some(full_content)`

#### Scenario: Long file content truncated
- **WHEN** a file write tool is called with content of 5000 characters
- **THEN** `PendingAction::FileWrite.content` is `Some(first_2000_chars)` truncated at a char boundary

#### Scenario: No content field still serializes correctly
- **WHEN** `PendingAction::FileWrite { path: "x", content: None }` is serialized
- **THEN** the JSON does NOT contain the key `"content"`

### Requirement: ApplyPatch PendingAction carries optional diff
`PendingAction::ApplyPatch` SHALL include a field `diff: Option<String>`. When present, it contains a preview of the patch diff, truncated to at most 2000 characters at a valid UTF-8 boundary.

#### Scenario: Short diff included in full
- **WHEN** an edit_file tool is called with a diff of 800 characters
- **THEN** `PendingAction::ApplyPatch.diff` is `Some(full_diff)`

#### Scenario: Long diff truncated
- **WHEN** an edit_file tool is called with a diff of 4000 characters
- **THEN** `PendingAction::ApplyPatch.diff` is `Some(first_2000_chars)` truncated at a char boundary

### Requirement: Content extraction from tool arguments
The `FileWriteRuntime::to_pending_action` method SHALL extract the `content` field from the tool arguments JSON and include it (truncated) in the returned `PendingAction::FileWrite`.

#### Scenario: Content extracted from write_file args
- **WHEN** tool args contain `{"path": "/tmp/foo.txt", "content": "hello world"}`
- **THEN** `to_pending_action` returns `FileWrite { path: "/tmp/foo.txt", content: Some("hello world") }`

### Requirement: Diff extraction from edit tool arguments
The `FileEditRuntime::to_pending_action` method SHALL extract diff-like content from the tool arguments (e.g., `old_string`/`new_string` or `diff` field) and include a formatted preview in the returned `PendingAction::ApplyPatch`.

#### Scenario: Diff constructed from old/new strings
- **WHEN** tool args contain `{"path": "x.rs", "old_string": "foo", "new_string": "bar"}`
- **THEN** `to_pending_action` returns `ApplyPatch { paths: ["x.rs"], diff: Some("-foo\n+bar") }`

### Requirement: Truncation uses char boundary safety
All content/diff truncation SHALL use `str::floor_char_boundary` (or equivalent) to avoid splitting multi-byte UTF-8 characters.

#### Scenario: CJK content truncated safely
- **WHEN** content contains 1000 CJK characters (3 bytes each, 3000 bytes total)
- **THEN** truncation occurs at a valid character boundary, not mid-character
