## ADDED Requirements

### Requirement: when_to_use frontmatter field
The `SkillFrontmatter` struct SHALL support an optional `when_to_use: String` field. When present, it SHALL be used in search relevance scoring and skill listing.

#### Scenario: when_to_use parsed from frontmatter
- **WHEN** a SKILL.md contains `when_to_use: "Use when deploying backend services"`
- **THEN** `SkillEntry.frontmatter.when_to_use` SHALL be `Some("Use when deploying backend services")`

#### Scenario: when_to_use absent
- **WHEN** a SKILL.md does not contain `when_to_use`
- **THEN** `SkillEntry.frontmatter.when_to_use` SHALL be `None`

### Requirement: when_to_use search weight
The keyword search algorithm SHALL weight `when_to_use` matches at 2.0, equal to `description` (2.0) in XiaoLin's weight scheme. Note: Claude Code uses `whenToUse: 2.0` but with `description: 1.0` — XiaoLin preserves its own relative weights (name 3.0 > tags 2.5 > when_to_use = description 2.0 > content 1.0).

#### Scenario: Search matches when_to_use
- **WHEN** a search query "deploy" matches a skill's `when_to_use` field "Use when deploying backend services"
- **THEN** the relevance score SHALL include a +2.0 contribution from the when_to_use match

### Requirement: when_to_use in Compact listing
When `prompt_mode` is `Compact` and a skill has `when_to_use`, the listing entry SHALL include the when_to_use text as an additional line.

#### Scenario: Compact mode with when_to_use
- **WHEN** a skill has `when_to_use: "For database migrations"` and prompt_mode is Compact
- **THEN** the listing SHALL include a line like `  when: For database migrations` after the description

#### Scenario: Compact mode without when_to_use
- **WHEN** a skill has no `when_to_use` and prompt_mode is Compact
- **THEN** the listing SHALL not include a `when:` line

### Requirement: Stable embedding content hash
The skill embedding cache SHALL use `blake3` for content hashing instead of `std::collections::hash_map::DefaultHasher`. This ensures cache keys are stable across Rust compiler versions.

#### Scenario: Hash stability across builds
- **WHEN** the same skill content is hashed after a Rust toolchain update
- **THEN** the hash value SHALL be identical to the previous build's hash

#### Scenario: Cache hit on unchanged content
- **WHEN** a skill's content has not changed between app restarts
- **THEN** the embedding cache SHALL return a hit and skip re-computation
