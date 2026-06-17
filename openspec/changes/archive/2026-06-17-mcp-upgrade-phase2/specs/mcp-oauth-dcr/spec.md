## ADDED Requirements

### Requirement: Dynamic Client Registration when no client_id configured

The system SHALL perform RFC 7591 Dynamic Client Registration when connecting to an MCP server that requires OAuth and no explicit `client_id` is configured, provided the server's OAuth metadata includes a `registration_endpoint`.

#### Scenario: Automatic DCR on first OAuth login

- **WHEN** the user triggers OAuth login for a server with no `oauth.client_id`
- **AND** the server's `/.well-known/oauth-authorization-server` includes `registration_endpoint`
- **THEN** the system SHALL POST to the `registration_endpoint` with `client_name: "XiaoLin"`, `redirect_uris`, `grant_types: ["authorization_code"]`, `response_types: ["code"]`
- **AND** persist the returned `client_id` and optional `client_secret` to secure storage

#### Scenario: DCR credentials reused on subsequent logins

- **WHEN** DCR has been performed for a server and credentials are stored
- **AND** the user triggers OAuth login again
- **THEN** the system SHALL use the stored `client_id` from DCR instead of re-registering

#### Scenario: Fallback to server URL as client_id when no registration_endpoint

- **WHEN** the server's OAuth metadata does not include `registration_endpoint`
- **AND** no explicit `client_id` is configured
- **THEN** the system SHALL use the server URL as `client_id` (current behavior)

#### Scenario: DCR registration failure is non-fatal

- **WHEN** the DCR POST request fails (network error, 4xx, 5xx)
- **THEN** the system SHALL log the error and fall back to using server URL as `client_id`
- **AND** display a warning to the user
