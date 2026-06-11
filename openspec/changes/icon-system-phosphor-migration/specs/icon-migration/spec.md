## ADDED Requirements

### Requirement: Complete Lucide to Phosphor import replacement
The system SHALL replace all `lucide-react` imports with equivalent `@phosphor-icons/react` imports across all component files. Every Lucide icon name MUST be mapped to its Phosphor equivalent using a documented name mapping table.

#### Scenario: All imports migrated
- **WHEN** the migration is complete
- **THEN** zero files SHALL contain `import ... from "lucide-react"` statements

#### Scenario: Name mapping coverage
- **WHEN** a Lucide icon has no exact Phosphor equivalent
- **THEN** the closest semantic match SHALL be used and documented in the mapping table

### Requirement: StrokeWidth to weight conversion
The system SHALL convert all `strokeWidth` prop usage to appropriate Phosphor `weight` values according to the semantic mapping: strokeWidth ≤ 1.2 → light, 1.3~1.6 → regular, 1.7~2.0 → bold (contextual), > 2.0 → bold.

#### Scenario: Thin stroke converted to light
- **WHEN** a Lucide icon had strokeWidth=1.0 or strokeWidth=1.2
- **THEN** the Phosphor equivalent SHALL use weight="light"

#### Scenario: Default stroke stays regular
- **WHEN** a Lucide icon had strokeWidth=1.5 (the token default)
- **THEN** the Phosphor equivalent SHALL use weight="regular" or omit weight (inherits from Provider)

#### Scenario: Emphasis stroke becomes bold
- **WHEN** a Lucide icon had strokeWidth ≥ 2.0 in an emphasis/CTA context
- **THEN** the Phosphor equivalent SHALL use weight="bold"

### Requirement: Lucide dependency removal
The system SHALL remove `lucide-react` from package.json dependencies after migration. The `@phosphor-icons/react` package SHALL be the sole icon library.

#### Scenario: Clean dependency
- **WHEN** `pnpm ls lucide-react` is run after migration
- **THEN** it SHALL report the package is not installed

### Requirement: ui-tokens.ts update
The system SHALL update `ui-tokens.ts` to replace Lucide-specific token definitions (ICON.sm/md/lg with strokeWidth) with Phosphor-compatible tokens (size scale without strokeWidth, weight semantics).

#### Scenario: Token file updated
- **WHEN** the migration is complete
- **THEN** `ui-tokens.ts` SHALL export ICON_SIZE, ICON_WEIGHT, and ICON_COLOR constants compatible with Phosphor's API

#### Scenario: No strokeWidth references in tokens
- **WHEN** `ui-tokens.ts` is inspected
- **THEN** it SHALL NOT contain any `strokeWidth` property definitions

### Requirement: Custom SVG compatibility
Custom SVG components (ClawIcon, AppHeader inline SVGs) SHALL remain unchanged but MUST be visually harmonious with the Phosphor icon set at their respective display sizes.

#### Scenario: ClawIcon unchanged
- **WHEN** the migration is complete
- **THEN** ClawIcon component SHALL compile and render identically to pre-migration

#### Scenario: AppHeader custom SVGs preserved
- **WHEN** AppHeader layout toggle SVGs are inspected
- **THEN** they SHALL retain their inline SVG implementation without modification
