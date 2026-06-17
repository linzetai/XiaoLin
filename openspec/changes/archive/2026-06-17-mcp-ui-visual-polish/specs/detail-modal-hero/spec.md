## ADDED Requirements

### Requirement: Hero header section in McpDetailModal
The McpDetailModal SHALL display a Hero header section at the top containing: (1) a large icon (48px) with brand-colored background, (2) server name in 18px semibold, (3) status badge, (4) server description text, and (5) a subtle gradient color bar using the server's category color.

#### Scenario: Hero renders with registry metadata
- **WHEN** the detail modal opens for a server whose ID matches a registry entry
- **THEN** the hero section SHALL display the registry entry's icon, description, and brand color gradient

#### Scenario: Hero renders for non-registry servers
- **WHEN** the detail modal opens for a manually-added server not in the registry
- **THEN** the hero section SHALL display a default icon (PuzzlePiece) with the tint color and the server ID as the title

### Requirement: Searchable and collapsible tools list
The tools section in McpDetailModal SHALL be collapsible (defaulting to expanded) and SHALL include a search input when tool count exceeds 5.

#### Scenario: Tools section with search
- **WHEN** the server has more than 5 tools
- **THEN** a search input SHALL appear in the tools section header, filtering tools by name or description as the user types

#### Scenario: Tools section collapse toggle
- **WHEN** user clicks the tools section header
- **THEN** the tools list SHALL toggle between expanded and collapsed state with a smooth height transition

### Requirement: Edit configuration entry point
The McpDetailModal SHALL provide an "Edit" button that opens the AddServerModal pre-filled with the current server's configuration.

#### Scenario: Edit button opens AddServerModal with prefill
- **WHEN** user clicks the "Edit" button in the detail modal
- **THEN** the AddServerModal SHALL open with `prefill` containing the server's id, command, args, transport, and url from the detail data

### Requirement: Gradient accent bar
The Hero header SHALL include a 3px gradient bar at the top of the modal using the category color, transitioning from the color at 40% opacity on the left to transparent on the right.

#### Scenario: Gradient bar renders with category color
- **WHEN** the detail modal opens for a server with category "development"
- **THEN** a 3px gradient bar SHALL render at the top of the modal using the development category color (blue)
