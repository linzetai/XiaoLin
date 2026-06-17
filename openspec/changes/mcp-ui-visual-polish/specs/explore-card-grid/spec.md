## ADDED Requirements

### Requirement: Grid layout for Explore panel
The Explore panel SHALL render MCP server entries in a responsive grid layout (2 columns on standard viewport, 1 column on narrow viewport) instead of the current single-column list.

#### Scenario: Standard viewport renders 2-column grid
- **WHEN** the Explore panel is displayed on a viewport wider than 480px
- **THEN** MCP server cards SHALL be arranged in a 2-column grid with 12px gap

#### Scenario: Narrow viewport collapses to single column
- **WHEN** the Explore panel is displayed on a viewport 480px or narrower
- **THEN** MCP server cards SHALL stack in a single column

### Requirement: Card visual structure with brand color
Each MCP server card SHALL display a vertically-stacked layout containing: (1) a brand-colored icon area (40x40px rounded square with 10% opacity brand color background), (2) server name in 14px semibold, (3) author name in 11px muted text, (4) category badge with category color, (5) one-line description, and (6) an install/installed action at the bottom.

#### Scenario: Card displays brand color icon area
- **WHEN** an MCP registry entry has a `brandColor` field
- **THEN** the card icon area background SHALL use that brandColor at 10% opacity, and the icon SHALL use the brandColor as its foreground color

#### Scenario: Card displays author attribution
- **WHEN** an MCP registry entry has an `author` field
- **THEN** the card SHALL display the author name below the server name in 11px muted text

#### Scenario: Card without brandColor uses category fallback
- **WHEN** an MCP registry entry has no `brandColor` field
- **THEN** the card SHALL use the existing category color system as fallback

### Requirement: Card hover micro-interaction
Each card SHALL respond to hover with a subtle upward translation and shadow increase.

#### Scenario: Card hover animation
- **WHEN** user hovers over a card
- **THEN** the card SHALL translate upward by 2px and increase its box-shadow from `shadow-sm` to `shadow-md` with a 200ms ease transition

### Requirement: Registry data schema extension
The `mcp-registry.json` entries SHALL support additional optional fields: `brandColor` (hex string), `author` (string), and `tags` (string array).

#### Scenario: Registry entries include brand metadata
- **WHEN** the registry JSON is loaded
- **THEN** entries MAY include `brandColor`, `author`, and `tags` fields without breaking existing entries that lack these fields

### Requirement: Tags display on cards
Cards SHALL display tags as small pill badges below the description when the entry has a non-empty `tags` array.

#### Scenario: Card with tags shows tag badges
- **WHEN** an MCP registry entry has `tags: ["official", "popular"]`
- **THEN** the card SHALL display two small pill badges reading "official" and "popular" in muted styling

#### Scenario: Card without tags shows no badge row
- **WHEN** an MCP registry entry has no `tags` field or an empty array
- **THEN** the card SHALL not render a tags row
