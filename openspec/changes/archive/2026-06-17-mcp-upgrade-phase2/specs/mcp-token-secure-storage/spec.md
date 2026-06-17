## ADDED Requirements

### Requirement: OAuth tokens are stored in encrypted storage

The system SHALL store OAuth tokens (access_token, refresh_token, client credentials) in Tauri stronghold encrypted storage instead of plaintext JSON files.

#### Scenario: New token stored in stronghold

- **WHEN** an OAuth token exchange succeeds
- **THEN** the system SHALL store the token in Tauri stronghold under a key derived from the server ID
- **AND** SHALL NOT write plaintext token data to the filesystem

#### Scenario: Existing plaintext tokens auto-migrated

- **WHEN** the application starts and plaintext token files exist in `~/.local/share/com.xiaolin.desktop/mcp-tokens/`
- **THEN** the system SHALL migrate each token to stronghold
- **AND** retain the original files for 30 days as backup
- **AND** log the migration count

#### Scenario: Stronghold unavailable falls back to file storage

- **WHEN** Tauri stronghold initialization fails (e.g., corrupted vault)
- **THEN** the system SHALL fall back to the existing `FileTokenStore`
- **AND** log a warning about the fallback

### Requirement: TokenStore abstraction

The system SHALL use a `TokenStore` trait to abstract token persistence, allowing swappable backends.

#### Scenario: TokenStore trait has load/save/delete operations

- **WHEN** the OAuth module needs to persist or retrieve tokens
- **THEN** it SHALL call `TokenStore::load(server_id)`, `TokenStore::save(server_id, token)`, or `TokenStore::delete(server_id)`
- **AND** the active implementation (stronghold or file) handles the operation
