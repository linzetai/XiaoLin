## ADDED Requirements

### Requirement: MCP tool calls respect approval configuration

The system SHALL check `tools_ask` and `requires_confirmation` for MCP tool calls (`mcp__*` prefixed) before execution. MCP tools MUST NOT bypass the approval pipeline via `execute_unguarded` when the active behavior preset or user config requires confirmation.

#### Scenario: MCP tool in "Suggest edits" mode requires confirmation

- **WHEN** the active behavior preset has `tools_ask: ["mcp__*"]`
- **AND** the agent invokes an MCP tool `mcp__github__create_issue`
- **THEN** the system SHALL present an approval prompt to the user before executing the tool call
- **AND** the tool SHALL NOT execute until the user approves

#### Scenario: MCP tool in "Auto edit" mode executes without confirmation

- **WHEN** the active behavior preset has `tools_allow: ["mcp__*"]`
- **AND** the agent invokes an MCP tool
- **THEN** the system SHALL execute the tool call without an approval prompt

#### Scenario: Session-level approval cache prevents repeated prompts

- **WHEN** the user approves an MCP tool call with "Approve for session"
- **AND** the same tool is invoked again in the same session
- **THEN** the system SHALL execute without prompting again (cached approval)

#### Scenario: Glob pattern matching for MCP server-level approval

- **WHEN** the user config has `tools_ask: ["mcp__github__*"]`
- **AND** the agent invokes `mcp__github__create_issue`
- **THEN** the system SHALL require approval
- **BUT** `mcp__filesystem__read_file` SHALL execute without approval

#### Scenario: Denied MCP tools are blocked

- **WHEN** the user config has `tools_deny: ["mcp__dangerous__*"]`
- **AND** the agent invokes `mcp__dangerous__delete_all`
- **THEN** the system SHALL block the tool call and return an error to the agent
