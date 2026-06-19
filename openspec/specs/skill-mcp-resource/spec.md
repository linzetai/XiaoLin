## ADDED Requirements

### Requirement: MCP servers can expose skills as resources
The system SHALL support discovering skills from MCP servers that expose `skill://` URI resources.

#### Scenario: Discover MCP skill resources
- **WHEN** an MCP server lists resources with `skill://` URI scheme
- **THEN** the system SHALL fetch each resource content
- **AND** parse it as a SKILL.md (frontmatter + markdown body)
- **AND** register it in the SkillRegistry at `Extension` layer

#### Scenario: MCP skill metadata
- **WHEN** an MCP skill resource is registered
- **THEN** its `SkillSource` SHALL have `origin: Extension` and the path SHALL reference the MCP server name

### Requirement: MCP skills hot-reload on server reconnect
MCP skill resources SHALL be refreshed when the MCP server reconnects.

#### Scenario: Server reconnect refreshes skills
- **WHEN** an MCP server disconnects and reconnects
- **THEN** the system SHALL re-fetch `skill://` resources
- **AND** update the registry with any changed or new skills
- **AND** remove skills from servers that no longer expose them

### Requirement: MCP skill isolation
MCP-provided skills SHALL not be writable or deletable through the skill management API.

#### Scenario: Cannot edit MCP skill
- **WHEN** the frontend sends `skills.update` for an MCP-provided skill
- **THEN** the backend SHALL return an error: "MCP-provided skills are read-only"

#### Scenario: Cannot delete MCP skill
- **WHEN** the frontend sends `skills.delete` for an MCP-provided skill
- **THEN** the backend SHALL return an error: "MCP-provided skills cannot be deleted"
