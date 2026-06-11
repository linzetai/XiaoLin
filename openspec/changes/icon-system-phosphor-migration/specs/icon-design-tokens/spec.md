## ADDED Requirements

### Requirement: Icon size scale tokens
The system SHALL define a size scale with 6 named levels (xs/sm/md/lg/xl/2xl) mapping to specific pixel values (12/14/16/20/24/32). All icon usage MUST reference these tokens instead of hardcoded numeric values.

#### Scenario: Component uses size token
- **WHEN** a component renders an icon
- **THEN** the icon size MUST be one of the defined scale values (12, 14, 16, 20, 24, or 32)

#### Scenario: No hardcoded sizes in components
- **WHEN** code review checks icon usage
- **THEN** no component SHALL use a numeric size value not present in the scale (e.g., size=13, size=15, size=11)

### Requirement: Icon weight semantic mapping
The system SHALL define a weight semantic mapping that associates UI contexts with Phosphor weight values: regular (default), light (decorative/window controls), bold (emphasis/CTA), fill (active/selected state), thin (large empty state icons).

#### Scenario: Default weight applied
- **WHEN** an icon is rendered without explicit weight
- **THEN** the icon SHALL display with `regular` weight

#### Scenario: Active state uses fill weight
- **WHEN** a navigation item or tab is in active/selected state
- **THEN** its icon SHALL use `fill` weight to indicate selection

#### Scenario: Window control icons use light weight
- **WHEN** window control buttons (minimize, maximize, close) render icons
- **THEN** they SHALL use `light` weight for a refined appearance

### Requirement: IconContext global configuration
The system SHALL provide an `IconContext.Provider` at the app root with default values: size=14, weight="regular", color="currentColor". Components MAY override these defaults via local props when contextually appropriate.

#### Scenario: Global defaults applied
- **WHEN** any Phosphor icon component renders without explicit props
- **THEN** it SHALL inherit size=14, weight="regular", color="currentColor" from the Provider

#### Scenario: Local override takes precedence
- **WHEN** a component passes explicit size or weight props to an icon
- **THEN** the local props SHALL override the global IconContext defaults

### Requirement: Icon color semantic tokens
The system SHALL define color semantic tokens for icons: default (currentColor), muted (--fill-quaternary), secondary (--fill-secondary), accent (--fill-accent), danger (--red), success (--green). Icons MUST use these tokens instead of hardcoded color values.

#### Scenario: Muted icon color
- **WHEN** an icon is used as a secondary/decorative element
- **THEN** it SHALL use the muted color token (var(--fill-quaternary))

#### Scenario: Accent icon color
- **WHEN** an icon represents a brand or primary action
- **THEN** it SHALL use the accent color token (var(--fill-accent))
