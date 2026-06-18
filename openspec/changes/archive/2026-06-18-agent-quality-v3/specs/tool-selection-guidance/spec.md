## ADDED Requirements

### Requirement: Tool Selection Decision Tree in system prompt
The system prompt SHALL include a structured decision tree (Step 0-3) that guides the agent through tool selection for each user request.

#### Scenario: Agent uses glob before read_file for unknown file locations
- **WHEN** the agent needs to find a file whose exact path is unknown
- **THEN** the agent MUST use `glob` first to discover the path, then `read_file` with the discovered path

#### Scenario: Agent prefers dedicated tools over shell_exec
- **WHEN** the agent needs to read a file, search content, or list files
- **THEN** the agent MUST use `read_file`, `search_in_files`, or `glob` respectively, not `shell_exec`

#### Scenario: Agent does not call tools when unnecessary
- **WHEN** the agent can answer the user's question from existing context
- **THEN** the agent SHALL respond directly without making tool calls

### Requirement: Few-shot tool selection examples
The system prompt SHALL include 3-5 concrete examples of correct tool selection for common tasks.

#### Scenario: Few-shot examples cover common patterns
- **WHEN** the system prompt is assembled
- **THEN** it SHALL include examples for: finding files by pattern, searching code by content, reading specific files, and editing files

### Requirement: Search-before-declare-missing rule
The system prompt SHALL require the agent to perform at least one `glob` or `search_in_files` call before declaring that a file or function does not exist.

#### Scenario: Agent searches before creating new files
- **WHEN** the agent considers creating a new file that might already exist
- **THEN** the agent MUST first search with `glob` to confirm the file does not exist
