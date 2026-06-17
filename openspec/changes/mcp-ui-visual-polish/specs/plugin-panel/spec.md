## MODIFIED Requirements

### Requirement: Empty state visual enhancement
The MCP empty state SHALL display an enhanced visual with: (1) a floating-animated icon, (2) two CTA buttons ("Browse Directory" as primary, "Add Manually" as secondary ghost button), and (3) improved spacing and typography.

#### Scenario: Empty state shows dual CTA buttons
- **WHEN** no MCP servers are installed and the Installed sub-view is active
- **THEN** the empty state SHALL display two buttons: a primary "Browse Directory" button that switches to Explore view, and a secondary "Add Manually" button that opens the AddServerModal

#### Scenario: Empty state icon has float animation
- **WHEN** the empty state is rendered
- **THEN** the PuzzlePiece icon container SHALL have the `pv-float` animation class applied

## ADDED Requirements

### Requirement: PluginRow brand color indicator
Each PluginRow in the Installed list SHALL display a small color indicator derived from the server's brand color or category color, providing visual consistency with the Explore panel cards.

#### Scenario: PluginRow shows category color dot
- **WHEN** a PluginRow is rendered for a server whose ID matches a registry entry
- **THEN** the row SHALL display a 3px-wide color strip (or a colored icon background) on the left side using the entry's brandColor or category color

#### Scenario: PluginRow for non-registry server uses default color
- **WHEN** a PluginRow is rendered for a server not found in the registry
- **THEN** the row SHALL use the default tint color for the color indicator

### Requirement: PluginRow icon from registry
Each PluginRow SHALL display the Phosphor icon from the registry entry (if matched), replacing the generic status dot with a more informative icon+color combination.

#### Scenario: PluginRow shows registry-matched icon
- **WHEN** a PluginRow is rendered for a server with id "github" that matches the registry
- **THEN** the row SHALL display the GithubLogo icon with the github entry's brand color background

#### Scenario: PluginRow for unmatched server shows default icon
- **WHEN** a PluginRow is rendered for a server not in the registry
- **THEN** the row SHALL display a PuzzlePiece icon with tint color background
