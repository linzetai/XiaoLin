## ADDED Requirements

### Requirement: Read skills from other agent tools
The system SHALL scan skill directories of other agent tools (Cursor, Codex) in read-only mode.

#### Scenario: Discover Cursor project skills
- **WHEN** `<workspace_root>/.cursor/skills/` contains skills
- **THEN** they SHALL be discovered and available as read-only skills
- **AND** they SHALL have lower priority than `.fastclaw/skills/`

#### Scenario: Discover Cursor user skills
- **WHEN** `~/.cursor/skills/` contains skills
- **THEN** they SHALL be discovered at user level with lower priority than `~/.fastclaw/skills/`

#### Scenario: Discover Codex user skills
- **WHEN** `~/.codex/skills/` contains skills
- **THEN** they SHALL be discovered at user level

### Requirement: Write-only to own directory
- **WHEN** the agent creates or modifies a skill
- **THEN** it SHALL only write to `.fastclaw/skills/` (never to `.cursor/` or `.codex/`)

### Requirement: Skill source tracking
- **WHEN** skills are discovered from multiple sources
- **THEN** each skill SHALL carry source metadata: `{ origin: "fastclaw"|"cursor"|"codex"|"shared", layer: "project"|"user", path }`

### Requirement: Full skill scan path order
The complete scan order (lowest to highest priority):
1. `~/.agents/skills/` (cross-tool shared)
2. `~/.codex/skills/` (Codex user)
3. `~/.cursor/skills/` (Cursor user)
4. `~/.fastclaw/skills/` (FastClaw user)
5. `<root>/skills/` (legacy project)
6. `<root>/.cursor/skills/` (Cursor project, read-only)
7. `<root>/.fastclaw/skills/` (FastClaw project, highest priority)
