## ADDED Requirements

### Requirement: Local embedding generation for skills
The system SHALL generate vector embeddings for all skill descriptions using a local embedding model.

#### Scenario: Embedding generated on skill load
- **WHEN** a skill is loaded into the registry
- **THEN** its description (or first 512 chars of content if no description) SHALL be embedded
- **AND** the embedding SHALL be stored in SQLite alongside the skill metadata

#### Scenario: Embedding model selection
- **WHEN** the system initializes the embedding engine
- **THEN** it SHALL use `fastembed-rs` with the `all-MiniLM-L6-v2` model
- **AND** the model SHALL run locally without network access

### Requirement: Semantic skill search
The system SHALL support searching skills by semantic similarity to a natural language query.

#### Scenario: Semantic search via tool
- **WHEN** the agent calls `search_skills` with a natural language query
- **THEN** the system SHALL embed the query and compute cosine similarity against all skill embeddings
- **AND** return skills ranked by similarity score (highest first)

#### Scenario: Minimum similarity threshold
- **WHEN** search results are computed
- **THEN** only skills with similarity score above 0.3 SHALL be returned
- **AND** the maximum number of results SHALL be configurable (default 10)

### Requirement: Embedding cache with invalidation
The system SHALL cache embeddings and invalidate them when skill content changes.

#### Scenario: Cache hit
- **WHEN** a skill's content hash matches the cached embedding's content hash
- **THEN** the cached embedding SHALL be reused without re-computation

#### Scenario: Cache miss on content change
- **WHEN** a skill's content is updated (write_skill or hot-reload detects change)
- **THEN** the embedding SHALL be re-generated and the cache updated
